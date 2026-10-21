//! Comptes en ligne (Libre.fm, Last.fm, ListenBrainz) : les écoutes sont
//! récupérées directement, sans fichier d'export.
//!
//! Seul le nom d'utilisateur est demandé (plus une clé d'API pour Last.fm) : ces
//! services publient l'historique des profils publics, aucun mot de passe n'est
//! nécessaire ni stocké.
//!
//! Chaque compte est une source comme une autre : ses écoutes passent par la même
//! fusion que les fichiers, une écoute déjà importée d'un CSV (même titre à
//! ±90 s, ±120 s si le CSV n'a que la minute) n'est donc pas comptée deux fois.
//!
//! Synchronisation par fenêtres [début, fin] parcourues de la plus récente à la
//! plus ancienne, page par page. La position est enregistrée après chaque page :
//! une synchronisation interrompue (fermeture, réseau) reprend où elle en était.

use crate::import::{formats, ingest_into, new_source, Reject};
use crate::{Error, Library, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const USER_AGENT: &str = concat!("LibrestFM/", env!("CARGO_PKG_VERSION"));
/// Recouvrement avec la synchronisation précédente : Last.fm accepte des écoutes
/// envoyées jusqu'à 14 jours après coup. Les lignes relues sont reconnues (même
/// source, même heure) : une ou deux pages de plus par synchronisation.
const OVERLAP: i64 = 14 * 86_400;

/// Fin du ralentissement imposé par un service (secondes Unix), 0 sinon.
pub static THROTTLED_UNTIL: AtomicI64 = AtomicI64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Librefm,
    Lastfm,
    Listenbrainz,
}

impl Service {
    fn code(self) -> &'static str {
        match self {
            Service::Librefm => "librefm",
            Service::Lastfm => "lastfm",
            Service::Listenbrainz => "listenbrainz",
        }
    }

    fn from_code(s: &str) -> Option<Self> {
        match s {
            "librefm" => Some(Service::Librefm),
            "lastfm" => Some(Service::Lastfm),
            "listenbrainz" => Some(Service::Listenbrainz),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Service::Librefm => "Libre.fm",
            Service::Lastfm => "Last.fm",
            Service::Listenbrainz => "ListenBrainz",
        }
    }

    /// Délai minimal entre deux requêtes : Libre.fm coupe vite (Cloudflare),
    /// Last.fm tolère environ 5 requêtes/s, ListenBrainz 30 par 10 s.
    fn interval(self) -> Duration {
        match self {
            Service::Librefm => Duration::from_millis(2000),
            Service::Lastfm => Duration::from_millis(400),
            Service::Listenbrainz => Duration::from_millis(500),
        }
    }

    fn page_size(self) -> usize {
        match self {
            Service::Lastfm => 200,
            _ => 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: i64,
    pub service: Service,
    pub username: String,
    pub source_id: i64,
    /// Toutes les écoutes jusqu'à cette date sont récupérées (None : jamais synchronisé jusqu'au bout).
    pub synced_until: Option<i64>,
    /// Synchronisation en cours ou interrompue : (début, position, fin) de la fenêtre.
    pub pending: Option<(i64, i64, i64)>,
    pub synced_at: Option<i64>,
    pub error: Option<String>,
    /// Écoutes de ce compte dans la bibliothèque.
    pub plays: i64,
    /// Écoutes trouvées seulement dans ce compte (absentes des fichiers importés).
    pub exclusive: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncStatus {
    pub running: bool,
    pub account_id: Option<i64>,
    pub label: Option<String>,
    /// Avancement estimé dans la fenêtre en cours (0 à 1).
    pub progress: f64,
    pub fetched: usize,
    pub added: usize,
    pub merged: usize,
    pub throttled_until: i64,
}

// ---------- requêtes ----------

struct Client {
    agent: ureq::Agent,
    next: Instant,
}

enum Fetch {
    Json(Value),
    /// Utilisateur, ressource ou plage vide (404, ou erreur 6/7 côté Libre.fm et Last.fm).
    NotFound,
}

impl Client {
    fn new() -> Self {
        Self { agent: ureq::AgentBuilder::new().user_agent(USER_AGENT).timeout(Duration::from_secs(30)).build(), next: Instant::now() }
    }

    fn wait(&self, until: Instant, cancel: &AtomicBool) -> Result<()> {
        while Instant::now() < until {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            std::thread::sleep((until - Instant::now()).min(Duration::from_millis(200)));
        }
        Ok(())
    }

    fn get(&mut self, service: Service, url: &str, cancel: &AtomicBool) -> Result<Fetch> {
        for attempt in 0..6 {
            self.wait(self.next, cancel)?;
            self.next = Instant::now() + service.interval();
            let backoff = |secs: u64| {
                THROTTLED_UNTIL.store(chrono::Utc::now().timestamp() + secs as i64, Ordering::Relaxed);
                Instant::now() + Duration::from_secs(secs)
            };
            match self.agent.get(url).call() {
                Ok(r) => {
                    THROTTLED_UNTIL.store(0, Ordering::Relaxed);
                    // ListenBrainz annonce son quota : on s'arrête avant de le dépasser.
                    if r.header("X-RateLimit-Remaining").and_then(|v| v.parse::<i64>().ok()).is_some_and(|n| n <= 1) {
                        let secs = r.header("X-RateLimit-Reset-In").and_then(|v| v.parse().ok()).unwrap_or(10);
                        self.next = Instant::now() + Duration::from_secs(secs + 1);
                    }
                    let v: Value = crate::json_body(r).map_err(|_| Error::Http(format!("{} : réponse illisible", service.label())))?;
                    match api_error(&v) {
                        None => return Ok(Fetch::Json(v)),
                        Some(ApiError::NotFound) => return Ok(Fetch::NotFound),
                        Some(ApiError::Retry) if attempt < 5 => self.next = backoff(15 << attempt),
                        Some(ApiError::Retry) => break,
                        Some(ApiError::Fatal(e)) => return Err(e),
                    }
                }
                Err(ureq::Error::Status(404, _)) => return Ok(Fetch::NotFound),
                Err(ureq::Error::Status(code, r)) if (code == 429 || code >= 500) && attempt < 5 => {
                    let secs = r.header("Retry-After").and_then(|v| v.parse().ok()).unwrap_or(15u64 << attempt);
                    self.next = backoff(secs);
                }
                // Last.fm répond 403 à une clé refusée, le code d'erreur étant dans le corps.
                Err(ureq::Error::Status(code, r)) => match api_error(&crate::json_body(r).unwrap_or(Value::Null)) {
                    Some(ApiError::NotFound) => return Ok(Fetch::NotFound),
                    Some(ApiError::Retry) if attempt < 5 => self.next = backoff(15 << attempt),
                    Some(ApiError::Fatal(e)) => return Err(e),
                    _ => return Err(Error::Http(format!("{} : HTTP {code}", service.label()))),
                },
                // Jamais l'adresse dans le message : elle contient la clé d'API.
                Err(ureq::Error::Transport(_)) if attempt < 5 => self.next = backoff(5 << attempt),
                Err(ureq::Error::Transport(_)) => break,
            }
        }
        Err(Error::Http(format!("{} ne répond pas", service.label())))
    }
}

/// Erreur signalée par l'API Last.fm / Libre.fm dans une réponse.
enum ApiError {
    /// 6, 7 : utilisateur introuvable, plage vide.
    NotFound,
    /// 29 : trop de requêtes ; 8, 11, 16 : erreur passagère du service.
    Retry,
    Fatal(Error),
}

fn api_error(v: &Value) -> Option<ApiError> {
    let code = v.pointer("/error").and_then(|e| e.get("code").or(Some(e))).and_then(Value::as_i64)?;
    Some(match code {
        6 | 7 => ApiError::NotFound,
        29 | 8 | 11 | 16 => ApiError::Retry,
        17 => ApiError::Fatal(Error::Invalid("private_profile")),
        10 | 26 => ApiError::Fatal(Error::Invalid("lastfm_key")),
        _ => ApiError::Fatal(Error::Http(v["message"].as_str().unwrap_or("erreur du service").to_owned())),
    })
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Racine de l'API compatible Last.fm 2.0 (Libre.fm accepte n'importe quelle clé).
fn audioscrobbler(service: Service, key: Option<&str>, method: &str, user: &str) -> String {
    let (root, key) = match service {
        Service::Librefm => ("https://libre.fm/2.0/", "librestfm"),
        _ => ("https://ws.audioscrobbler.com/2.0/", key.unwrap_or("")),
    };
    format!("{root}?method={method}&user={}&api_key={}&format=json", enc(user), enc(key))
}

/// Une page d'écoutes, de la plus récente à la plus ancienne, avant `to` (inclus) et après `from`.
struct Page {
    rows: Vec<formats::Row>,
    /// Plus ancienne date de la page.
    oldest: Option<i64>,
}

type Item = (Option<i64>, std::result::Result<crate::import::RawScrobble, Reject>);

/// (date, écoute) : la date est gardée même pour une écoute refusée, pour avancer dans l'historique.
fn lastfm_row(v: &Value) -> Option<Item> {
    // L'écoute en cours n'a pas de date : elle sera récupérée une fois finie.
    if v.pointer("/@attr/nowplaying").is_some() {
        return None;
    }
    let ts = v.pointer("/date/uts").and_then(|u| u.as_str().and_then(|s| s.parse().ok()).or(u.as_i64())).map(|t| (t, true));
    let text = |p: &str| v.pointer(p).and_then(Value::as_str).filter(|s| !s.is_empty());
    let artist = text("/artist/#text").or(text("/artist/name"));
    Some((ts.map(|t| t.0), formats::build(ts, artist, text("/name"), text("/album/#text"), [text("/mbid"), text("/artist/mbid"), text("/album/mbid")])))
}

fn fetch_page(client: &mut Client, acc: &Account, key: Option<&str>, from: i64, to: i64, cancel: &AtomicBool) -> Result<Page> {
    let size = acc.service.page_size();
    match acc.service {
        Service::Librefm | Service::Lastfm => {
            let mut url = audioscrobbler(acc.service, key, "user.getrecenttracks", &acc.username);
            // `to` exclusif ou non selon le service : la seconde `to` est toujours demandée.
            url.push_str(&format!("&limit={size}&to={}", to + 1));
            if from > 0 {
                url.push_str(&format!("&from={from}"));
            }
            let v = match client.get(acc.service, &url, cancel)? {
                Fetch::Json(v) => v,
                Fetch::NotFound => return Ok(Page { rows: vec![], oldest: None }),
            };
            let rt = &v["recenttracks"];
            // Une seule écoute : Last.fm renvoie un objet au lieu d'une liste.
            let list: Vec<&Value> = match &rt["track"] {
                Value::Array(a) => a.iter().collect(),
                Value::Object(_) => vec![&rt["track"]],
                _ => vec![],
            };
            Ok(page_of(list.iter().filter_map(|t| lastfm_row(t)).collect(), from))
        }
        Service::Listenbrainz => {
            let url = format!("https://api.listenbrainz.org/1/user/{}/listens?count={size}&max_ts={}", enc(&acc.username), to + 1);
            let v = match client.get(acc.service, &url, cancel)? {
                Fetch::Json(v) => v,
                Fetch::NotFound => return Err(Error::NotFound(acc.username.clone())),
            };
            let rows = v.pointer("/payload/listens").and_then(Value::as_array).cloned().unwrap_or_default();
            Ok(page_of(rows.iter().map(|l| (l["listened_at"].as_i64(), formats::json_row(formats::Format::ListenBrainz, l))).collect(), from))
        }
    }
}

/// Numérote les lignes et écarte ce qui précède le début de la fenêtre.
fn page_of(items: Vec<Item>, from: i64) -> Page {
    let oldest = items.iter().filter_map(|i| i.0).min();
    let rows = items.into_iter().filter(|i| i.0.is_none_or(|t| t >= from)).enumerate().map(|(n, i)| (n + 1, i.1)).collect();
    Page { rows, oldest }
}

/// Vérifie que le compte existe ; renvoie son nombre d'écoutes s'il est connu.
fn check(client: &mut Client, service: Service, user: &str, key: Option<&str>) -> Result<Option<i64>> {
    let never = AtomicBool::new(false);
    let (url, ptr) = match service {
        Service::Listenbrainz => (format!("https://api.listenbrainz.org/1/user/{}/listen-count", enc(user)), "/payload/count"),
        _ => (audioscrobbler(service, key, "user.getinfo", user), "/user/playcount"),
    };
    match client.get(service, &url, &never)? {
        Fetch::Json(v) => Ok(v.pointer(ptr).and_then(|c| c.as_i64().or(c.as_str().and_then(|s| s.parse().ok())))),
        Fetch::NotFound => Err(Error::NotFound(format!("{} : {user}", service.label()))),
    }
}

// ---------- bibliothèque ----------

impl Library {
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut st = self.conn.prepare(
            "SELECT a.id, a.service, a.username, a.source_id, a.synced_until, a.run_from, a.run_to, a.run_top, a.synced_at, a.error,
                    (SELECT COUNT(*) FROM scrobble_source x WHERE x.source_id = a.source_id),
                    (SELECT COUNT(*) FROM scrobble_source x WHERE x.source_id = a.source_id AND NOT EXISTS
                       (SELECT 1 FROM scrobble_source y WHERE y.scrobble_id = x.scrobble_id AND y.source_id <> x.source_id))
             FROM account a ORDER BY a.id",
        )?;
        let rows = st.query_map([], |r| {
            let pending = match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<i64>>(6)?, r.get::<_, Option<i64>>(7)?) {
                (Some(f), Some(t), Some(top)) => Some((f, t, top)),
                _ => None,
            };
            Ok(Account {
                id: r.get(0)?,
                service: Service::from_code(&r.get::<_, String>(1)?).unwrap_or(Service::Librefm),
                username: r.get(2)?,
                source_id: r.get(3)?,
                synced_until: r.get(4)?,
                pending,
                synced_at: r.get(8)?,
                error: r.get(9)?,
                plays: r.get(10)?,
                exclusive: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn account(&self, id: i64) -> Result<Account> {
        self.accounts()?.into_iter().find(|a| a.id == id).ok_or_else(|| Error::NotFound(format!("compte {id}")))
    }

    fn api_key(&self, id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT api_key FROM account WHERE id = ?", [id], |r| r.get(0))?)
    }

    fn save_position(&self, id: i64, run: Option<(i64, i64, i64)>, synced_until: Option<i64>, error: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE account SET run_from = ?2, run_to = ?3, run_top = ?4, synced_until = COALESCE(?5, synced_until), error = ?6,
               synced_at = CASE WHEN ?5 IS NULL THEN synced_at ELSE strftime('%s','now') END WHERE id = ?1",
            params![id, run.map(|r| r.0), run.map(|r| r.1), run.map(|r| r.2), synced_until, error],
        )?;
        Ok(())
    }

    /// Supprime un compte et les écoutes qu'il était seul à contenir.
    pub fn remove_account(&mut self, id: i64) -> Result<usize> {
        let source: i64 = self.conn.query_row("SELECT source_id FROM account WHERE id = ?", [id], |r| r.get(0))?;
        self.remove_source(source) // le compte part avec sa source
    }
}

/// Ajoute un compte après avoir vérifié qu'il existe (requête réseau, sans verrou).
pub fn add_account(lib: &Mutex<Library>, service: Service, username: &str, api_key: Option<&str>) -> Result<Account> {
    let username = username.trim().trim_start_matches('@');
    let key = api_key.map(str::trim).filter(|k| !k.is_empty());
    if username.is_empty() {
        return Err(Error::NotFound(String::new()));
    }
    if service == Service::Lastfm && key.is_none() {
        return Err(Error::Http("clé d'API Last.fm manquante".into()));
    }
    check(&mut Client::new(), service, username, key)?;
    let mut guard = lib.lock().unwrap_or_else(|e| e.into_inner());
    let tx = guard.conn.transaction()?;
    let sha = format!("account:{}:{}", service.code(), username.to_lowercase());
    let known: Option<i64> = tx.query_row("SELECT a.id FROM account a JOIN source s ON s.id = a.source_id WHERE s.sha256 = ?", [&sha], |r| r.get(0)).optional()?;
    let id = match known {
        Some(id) => {
            tx.execute("UPDATE account SET api_key = COALESCE(?, api_key) WHERE id = ?", params![key, id])?;
            id
        }
        None => {
            let source = new_source(&tx, &format!("{} : {username}", service.label()), &sha, &format!("{}_account", service.code()))?;
            tx.execute(
                "INSERT INTO account (service, username, api_key, source_id) VALUES (?, ?, ?, ?)",
                params![service.code(), username, key, source],
            )?;
            tx.last_insert_rowid()
        }
    };
    tx.commit()?;
    guard.account(id)
}

/// Synchronise les comptes (tous, ou un seul), avec reprise. Renvoie le nombre d'écoutes ajoutées.
pub fn run(lib: &Mutex<Library>, only: Option<i64>, cancel: Arc<AtomicBool>, mut progress: impl FnMut(&SyncStatus)) -> Result<usize> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let accounts: Vec<Account> = lock().accounts()?.into_iter().filter(|a| only.is_none_or(|id| a.id == id)).collect();
    let mut st = SyncStatus { running: true, ..Default::default() };
    let mut client = Client::new();
    let mut last_error = None;
    for acc in accounts {
        st.account_id = Some(acc.id);
        st.label = Some(format!("{} : {}", acc.service.label(), acc.username));
        st.progress = 0.0;
        progress(&st);
        let key = lock().api_key(acc.id)?;
        let now = chrono::Utc::now().timestamp();
        // Reprise d'une fenêtre interrompue, sinon tout ce qui est plus récent que la dernière fois.
        let (from, mut to, top) = acc.pending.unwrap_or((acc.synced_until.map_or(0, |s| s - OVERLAP), now, now));
        // Premier passage : avancement d'après le nombre d'écoutes du profil ; ensuite, d'après les dates.
        let total = if from == 0 { check(&mut client, acc.service, &acc.username, key.as_deref()).ok().flatten() } else { None };
        let mut seen = acc.plays;
        let result: Result<()> = (|| loop {
            if cancel.load(Ordering::Relaxed) {
                break Err(Error::Cancelled);
            }
            let page = fetch_page(&mut client, &acc, key.as_deref(), from, to, &cancel)?;
            let Some(oldest) = page.oldest else {
                break Ok(());
            };
            if cancel.load(Ordering::Relaxed) {
                break Err(Error::Cancelled); // compte retiré pendant la requête
            }
            seen += page.rows.len() as i64;
            let n = page.rows.len();
            {
                let mut guard = lock();
                let tx = guard.conn.transaction()?;
                let rep = ingest_into(&tx, acc.source_id, "", &format!("{}_account", acc.service.code()), page.rows)?;
                tx.commit()?;
                st.fetched += rep.rows_read;
                st.added += rep.added;
                st.merged += rep.merged;
                // Page suivante : jusqu'à la plus ancienne écoute de celle-ci, incluse (deux
                // écoutes à la même seconde, coupées par la fin de page, ne se perdent pas ;
                // celles déjà lues sont reconnues). Sans recul (page d'une seule seconde) : avant.
                to = if oldest < to { oldest } else { oldest - 1 };
                guard.save_position(acc.id, Some((from, to, top)), None, None)?;
            }
            st.progress = match total {
                Some(t) if t > 0 => (seen as f64 / t as f64).min(0.99),
                _ if from > 0 => ((top - to) as f64 / (top - from).max(1) as f64).clamp(0.0, 0.99),
                _ => 0.0,
            };
            st.throttled_until = THROTTLED_UNTIL.load(Ordering::Relaxed);
            progress(&st);
            if oldest < from || n == 0 {
                break Ok(());
            }
        })();
        match result {
            Ok(()) => lock().save_position(acc.id, None, Some(top), None)?,
            Err(Error::Cancelled) => break,
            Err(e) => {
                // Code compris : l'interface le traduit (comme les erreurs des commandes).
                lock().save_position(acc.id, Some((from, to, top)), None, Some(&format!("{}\u{1f}{e}", e.code())))?;
                last_error = Some(e);
            }
        }
    }
    // Corrections sûres et écoutes « .mp4 » sur ce qui vient d'arriver.
    lock().auto_clean()?;
    st.running = false;
    progress(&st);
    match last_error {
        Some(e) if st.added == 0 => Err(e),
        _ => Ok(st.added),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lastfm_json_rows() {
        let v: Value = serde_json::from_str(
            r##"{"recenttracks":{"track":[
              {"artist":{"#text":"Daft Punk","mbid":""},"name":"Get Lucky","album":{"#text":"Random Access Memories"},"@attr":{"nowplaying":"true"}},
              {"artist":{"#text":"Daft Punk","mbid":""},"name":"Get Lucky","mbid":"","album":{"#text":"Random Access Memories","mbid":""},"date":{"uts":"1710100000","#text":"x"}},
              {"artist":{"#text":"Daft Punk"},"name":"clip.mp4","album":{"#text":""},"date":{"uts":"1710090000"}}
            ],"@attr":{"total":"2"}}}"##,
        )
        .unwrap();
        let rows: Vec<_> = v["recenttracks"]["track"].as_array().unwrap().iter().filter_map(lastfm_row).collect();
        assert_eq!(rows.len(), 2, "l'écoute en cours est ignorée");
        let s = rows[0].1.as_ref().unwrap();
        assert_eq!((s.ts, s.precise, s.artist.as_str(), s.album.as_deref()), (1710100000, true, "Daft Punk", Some("Random Access Memories")));
        assert_eq!(rows[1].1.as_ref().err(), Some(&Reject::RecordingError));
        let page = page_of(rows, 1710095000);
        assert_eq!((page.rows.len(), page.oldest), (1, Some(1710090000)));
    }

    #[test]
    fn account_plays_merge_with_csv() {
        // Une écoute du CSV (minute près) et la même renvoyée par l'API (seconde près) : une seule écoute.
        let lib = Library::open_in_memory().unwrap();
        let lib = Mutex::new(lib);
        {
            let mut g = lib.lock().unwrap();
            g.import_bytes("x.csv", b"Daft Punk,Discovery,One More Time,10 Mar 2024 20:00\n").unwrap();
            let tx = g.conn.transaction().unwrap();
            let src = new_source(&tx, "Libre.fm : x", "account:librefm:x", "librefm_account").unwrap();
            let raw = formats::build(Some((1710100835, true)), Some("Daft Punk"), Some("One More Time"), Some("Discovery"), [None; 3]);
            let other = formats::build(Some((1710101200, true)), Some("Daft Punk"), Some("Aerodynamic"), Some("Discovery"), [None; 3]);
            let rep = ingest_into(&tx, src, "", "librefm_account", vec![(1, raw.clone()), (2, other)]).unwrap();
            assert_eq!((rep.added, rep.merged), (1, 1));
            // Page relue (reprise, recouvrement) : rien de neuf.
            let rep = ingest_into(&tx, src, "", "librefm_account", vec![(1, raw)]).unwrap();
            assert_eq!((rep.added, rep.merged), (0, 1));
            tx.commit().unwrap();
            let n: i64 = g.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap();
            assert_eq!(n, 2);
        }
    }

    /// Une écoute fusionnée avec une autre source (heure de
    /// l'autre source gardée) ne doit pas revenir quand la page est relue.
    #[test]
    fn merged_play_does_not_come_back_on_reread() {
        // Export précis (Spotify, ListenBrainz…) à 1710100800, compte à +20 s : fusion, puis page relue.
        let lib = Library::open_in_memory().unwrap();
        let mut g = lib;
        g.import_bytes("lb.csv", b"artist,title,timestamp\nDaft Punk,One More Time,1710100800\n").unwrap();
        let tx = g.conn.transaction().unwrap();
        let src = new_source(&tx, "Libre.fm : x", "account:librefm:x", "librefm_account").unwrap();
        let raw = formats::build(Some((1710100820, true)), Some("Daft Punk"), Some("One More Time"), None, [None; 3]);
        let rep = ingest_into(&tx, src, "", "librefm_account", vec![(1, raw.clone())]).unwrap();
        println!("1re synchro {:?}", (rep.added, rep.merged));
        let rep = ingest_into(&tx, src, "", "librefm_account", vec![(1, raw)]).unwrap();
        println!("recouvrement {:?}", (rep.added, rep.merged));
        tx.commit().unwrap();
        let n: i64 = g.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "écoute recomptée à la synchro suivante");
    }
}
