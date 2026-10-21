//! Enrichissement des titres par ReccoBeats (api.reccobeats.com, gratuit, sans clé).
//!
//! Seuls l'artiste, le titre et éventuellement l'album sont nécessaires. Pour
//! limiter le nombre de requêtes (le service en accepte environ 35 par 40 s),
//! on cherche dans cet ordre :
//! 1. l'album, quand plusieurs titres en attente en viennent (2 requêtes pour tout l'album) ;
//! 2. le titre seul (1 requête) ;
//! 3. le catalogue de l'artiste, en dernier recours (mis en cache).
//!
//! Les caractéristiques sont ensuite demandées par lots de 40 titres.
//! Les appels réseau se font sans tenir le verrou de la base.

use crate::normalize::{album_key, artist_key, artist_names, fold, title_key};
use crate::{Error, Library, Result};
use regex::Regex;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const BASE: &str = "https://api.reccobeats.com/v1";
const PROVIDER: &str = "reccobeats";
const USER_AGENT: &str = concat!("LibrestFM/", env!("CARGO_PKG_VERSION"));
/// Mesuré : refus (429) à la 35e requête au rythme d'une par
/// seconde. On reste juste en dessous.
const MIN_INTERVAL: Duration = Duration::from_millis(1200);
const CATALOG_TTL: i64 = 30 * 86_400;
/// Fiches homonymes examinées (première page de chacune).
const MAX_HOMONYMS: usize = 8;
/// Pages de 50 titres lues par fiche d'artiste : 25 couvre les gros catalogues
/// (1 024 titres pour Imagine Dragons, dont « Monster » au-delà de la 10e page).
const MAX_PAGES: usize = 25;
const FEATURES_BATCH: usize = 40;
/// Échecs réseau d'affilée avant d'abandonner (service indisponible, pas de réseau).
const MAX_CONSECUTIVE_ERRORS: usize = 3;

/// Requêtes envoyées depuis le lancement (diagnostic).
pub static REQUESTS: AtomicUsize = AtomicUsize::new(0);
/// Heure Unix jusqu'à laquelle ReccoBeats nous a demandé d'attendre (0 = pas de pause).
static THROTTLED_UNTIL: AtomicI64 = AtomicI64::new(0);
static DEBUG: LazyLock<Option<Instant>> = LazyLock::new(|| std::env::var_os("LIBRESTFM_DEBUG").map(|_| Instant::now()));

fn debug(msg: impl FnOnce() -> String) {
    if let Some(t0) = *DEBUG {
        eprintln!("{:.1} {}", t0.elapsed().as_secs_f64(), msg());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RbTrack {
    pub id: String,
    pub title: String,
    /// Identifiant Spotify, dernier segment de `href`.
    pub spotify_id: String,
    pub isrc: Option<String>,
    pub popularity: i64,
    pub artists: Vec<String>,
}

fn parse_track(v: &Value) -> Option<RbTrack> {
    let href = v.get("href")?.as_str()?;
    Some(RbTrack {
        id: v.get("id")?.as_str()?.to_owned(),
        title: v.get("trackTitle")?.as_str()?.to_owned(),
        spotify_id: href.rsplit('/').next()?.to_owned(),
        isrc: v.get("isrc").and_then(Value::as_str).map(str::to_owned),
        popularity: v.get("popularity").and_then(Value::as_i64).unwrap_or(0),
        artists: names(v),
    })
}

fn names(v: &Value) -> Vec<String> {
    v.get("artists")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

// ---------- rapprochement ----------

static VERSION_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(live|remix|rmx|version|edit|instrumental|karaoke|acoustic|acoustique|demo|cover|reprise|session|mix|orchestral|sped up|slowed)\b").unwrap()
});
static LOOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[\(\[].*?[\)\]]|\s+-\s+.*$").unwrap());

fn loose_key(title: &str) -> String {
    fold(&LOOSE.replace_all(title, ""))
}

/// Meilleur candidat pour `title`, ou None. Priorité : titre identique, puis
/// titre identique sans parenthèses ni suffixe ; une version (live, remix...)
/// absente du titre cherché est pénalisée ; à égalité, le plus populaire.
pub fn choose<'a>(title: &str, candidates: &'a [RbTrack]) -> Option<&'a RbTrack> {
    let exact = title_key(title);
    let loose = loose_key(title);
    let wanted_version = VERSION_WORDS.is_match(title);
    candidates
        .iter()
        .filter_map(|c| {
            let score = if title_key(&c.title) == exact {
                2
            } else if loose_key(&c.title) == loose {
                1
            } else {
                return None;
            };
            let unwanted = !wanted_version && VERSION_WORDS.is_match(&c.title);
            Some(((score, !unwanted, c.popularity), c))
        })
        .max_by_key(|(k, _)| *k)
        .map(|(_, c)| c)
}

/// Meilleur candidat crédité à `artist` : au nom entier d'abord, sinon à l'un des
/// artistes d'un nom collectif (« A & B »).
pub fn pick<'a>(title: &str, tracks: &'a [RbTrack], artist: &str) -> Option<&'a RbTrack> {
    let keys: Vec<String> = artist_names(artist).iter().map(|n| artist_key(n)).collect();
    let credited = |k: &[String]| tracks.iter().filter(|t| t.artists.iter().any(|a| k.contains(&artist_key(a)))).cloned().collect::<Vec<_>>();
    let full = choose(title, &credited(&keys[..1])).map(|c| c.id.clone());
    let id = full.or_else(|| choose(title, &credited(&keys)).map(|c| c.id.clone()))?;
    tracks.iter().find(|t| t.id == id)
}

// ---------- client HTTP ----------

pub struct ReccoBeats {
    agent: ureq::Agent,
    next_slot: Instant,
    cancel: Option<Arc<AtomicBool>>,
}

impl Default for ReccoBeats {
    fn default() -> Self {
        Self::new()
    }
}

impl ReccoBeats {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new().user_agent(USER_AGENT).timeout(Duration::from_secs(20)).build();
        Self { agent, next_slot: Instant::now(), cancel: None }
    }

    /// Attend jusqu'à `until`, par petites tranches pour réagir à l'annulation.
    fn sleep_until(&self, until: Instant) -> Result<()> {
        while Instant::now() < until {
            if self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
            std::thread::sleep((until - Instant::now()).min(Duration::from_millis(200)));
        }
        Ok(())
    }

    /// GET poli : une requête toutes les `MIN_INTERVAL`, pause imposée par le
    /// serveur respectée (Retry-After), 404 et 400 → None.
    fn get(&mut self, url: &str) -> Result<Option<Value>> {
        let mut failures = 0;
        loop {
            self.sleep_until(self.next_slot)?;
            self.next_slot = Instant::now() + MIN_INTERVAL;
            REQUESTS.fetch_add(1, Ordering::Relaxed);
            debug(|| format!("GET {url}"));
            match self.agent.get(url).call() {
                Ok(resp) => {
                    THROTTLED_UNTIL.store(0, Ordering::Relaxed);
                    return crate::json_body(resp).map(Some).map_err(|e| Error::Http(e.to_string()));
                }
                // 400 : requête refusée (recherche trop courte, ex. l'album « BE »). Pas de résultat.
                Err(ureq::Error::Status(404 | 400, _)) => return Ok(None),
                Err(ureq::Error::Status(429, resp)) => {
                    // Pas une erreur : on attend le délai demandé puis on reprend.
                    let secs = resp.header("Retry-After").and_then(|s| s.parse().ok()).unwrap_or(30u64).max(1) + 1;
                    debug(|| format!("429 Retry-After {secs}s"));
                    THROTTLED_UNTIL.store(chrono::Utc::now().timestamp() + secs as i64, Ordering::Relaxed);
                    self.next_slot = Instant::now() + Duration::from_secs(secs);
                }
                Err(ureq::Error::Status(code, _)) if code >= 500 && failures < 3 => {
                    failures += 1;
                    self.next_slot = Instant::now() + Duration::from_secs(2 << failures);
                }
                Err(ureq::Error::Transport(_)) if failures < 3 => {
                    failures += 1;
                    self.next_slot = Instant::now() + Duration::from_secs(2 << failures);
                }
                Err(e) => return Err(Error::Http(e.to_string())),
            }
        }
    }

    fn content(v: Option<Value>) -> Vec<Value> {
        v.and_then(|v| v.get("content").and_then(Value::as_array).cloned()).unwrap_or_default()
    }

    fn paged_tracks(&mut self, path: &str, page: usize) -> Result<(Vec<RbTrack>, usize)> {
        let v = self.get(&format!("{BASE}/{path}?size=50&page={page}"))?;
        let total_pages = v.as_ref().and_then(|v| v.get("totalPages")).and_then(Value::as_u64).unwrap_or(0) as usize;
        Ok((Self::content(v).iter().filter_map(parse_track).collect(), total_pages))
    }

    /// Titres de la ou des fiches ReccoBeats de cet artiste. Un nom courant a
    /// parfois des dizaines de fiches homonymes (22 « Queen »), renvoyées dans un
    /// ordre variable : on lit la première page de quelques-unes et on ne charge
    /// en entier que les deux dont les titres sont les plus populaires.
    /// Nom collectif (« A & B ») sans fiche à ce nom : celle du premier artiste qui en a une.
    pub fn artist_catalog(&mut self, name: &str) -> Result<Vec<RbTrack>> {
        const KEEP: usize = 2;
        let mut ids: Vec<String> = Vec::new();
        for n in artist_names(name) {
            let q = urlencoding::encode(&n);
            let found = Self::content(self.get(&format!("{BASE}/artist/search?searchText={q}&size=50"))?);
            let key = artist_key(&n);
            ids = found
                .iter()
                .filter(|a| a.get("name").and_then(Value::as_str).is_some_and(|x| artist_key(x) == key))
                .filter_map(|a| a.get("id").and_then(Value::as_str).map(str::to_owned))
                .take(MAX_HOMONYMS)
                .collect();
            if !ids.is_empty() {
                break;
            }
        }
        let mut firsts = Vec::new();
        for id in &ids {
            let (items, total_pages) = self.paged_tracks(&format!("artist/{id}/track"), 0)?;
            let best = items.iter().map(|t| t.popularity).max().unwrap_or(-1);
            firsts.push((best, id, items, total_pages));
        }
        firsts.sort_by_key(|f| std::cmp::Reverse(f.0));
        let mut tracks: Vec<RbTrack> = Vec::new();
        for (_, id, items, total_pages) in firsts.into_iter().take(KEEP) {
            tracks.extend(items);
            for page in 1..total_pages.min(MAX_PAGES) {
                let (items, _) = self.paged_tracks(&format!("artist/{id}/track"), page)?;
                if items.is_empty() {
                    break;
                }
                tracks.extend(items);
            }
        }
        tracks.sort_by(|a, b| a.spotify_id.cmp(&b.spotify_id));
        tracks.dedup_by(|a, b| a.spotify_id == b.spotify_id);
        Ok(tracks)
    }

    /// Titres de l'album `album` de `artist` (les deux éditions les plus proches).
    pub fn album_tracks(&mut self, artist: &str, album: &str) -> Result<Vec<RbTrack>> {
        let q = urlencoding::encode(album);
        let found = Self::content(self.get(&format!("{BASE}/album/search?searchText={q}&size=25"))?);
        let keys: Vec<String> = artist_names(artist).iter().map(|n| artist_key(n)).collect();
        let (exact, loose) = (album_key(album), loose_key(album));
        let mut editions: Vec<(bool, bool, &str)> = found
            .iter()
            .filter(|a| names(a).iter().any(|n| keys.contains(&artist_key(n))))
            .filter_map(|a| {
                let name = a.get("name")?.as_str()?;
                let is_exact = album_key(name) == exact;
                (is_exact || loose_key(name) == loose).then(|| {
                    let is_album = a.get("albumType").and_then(Value::as_str) == Some("album");
                    (is_exact, is_album, a.get("id").and_then(Value::as_str).unwrap_or_default())
                })
            })
            .collect();
        editions.sort_by_key(|e| std::cmp::Reverse((e.0, e.1)));
        let mut tracks = Vec::new();
        for (_, _, id) in editions.into_iter().take(2) {
            let (items, total_pages) = self.paged_tracks(&format!("album/{id}/track"), 0)?;
            tracks.extend(items);
            for page in 1..total_pages.min(3) {
                tracks.extend(self.paged_tracks(&format!("album/{id}/track"), page)?.0);
            }
        }
        Ok(tracks)
    }

    /// Recherche plein texte du titre, filtrée sur l'artiste : d'abord les titres crédités
    /// au nom entier, sinon à l'un des artistes d'un nom collectif (« A & B »).
    pub fn search(&mut self, artist: &str, title: &str) -> Result<Option<RbTrack>> {
        let q = urlencoding::encode(title);
        let found: Vec<RbTrack> = Self::content(self.get(&format!("{BASE}/track/search?searchText={q}&size=50"))?).iter().filter_map(parse_track).collect();
        Ok(pick(title, &found, artist).cloned())
    }

    /// Caractéristiques par identifiant Spotify.
    pub fn features(&mut self, spotify_ids: &[&str]) -> Result<HashMap<String, Value>> {
        let mut out = HashMap::new();
        for chunk in spotify_ids.chunks(FEATURES_BATCH) {
            let ids: Vec<String> = chunk.iter().map(|id| format!("ids={}", urlencoding::encode(id))).collect();
            for f in Self::content(self.get(&format!("{BASE}/audio-features?{}", ids.join("&")))?) {
                if let Some(id) = f.get("href").and_then(Value::as_str).and_then(|h| h.rsplit('/').next()) {
                    out.insert(id.to_owned(), f);
                }
            }
        }
        Ok(out)
    }
}

// ---------- côté base ----------

#[derive(Debug, Clone, Serialize)]
pub struct PendingTrack {
    pub track_id: i64,
    pub title: String,
    /// Album sur lequel ce titre est le plus écouté.
    pub album: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingArtist {
    pub artist_id: i64,
    pub name: String,
    pub tracks: Vec<PendingTrack>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EnrichStatus {
    pub ok: i64,
    pub not_found: i64,
    pub errors: i64,
    pub pending: i64,
    /// Part des écoutes dont le titre a des caractéristiques.
    pub plays_covered: f64,
    pub current_artist: Option<String>,
    pub running: bool,
    /// Pause imposée par ReccoBeats : heure Unix de reprise.
    pub throttled_until: Option<i64>,
}

impl Library {
    /// Artistes ayant des titres sans caractéristiques, les plus écoutés d'abord.
    pub fn pending_artists(&self, limit: usize) -> Result<Vec<PendingArtist>> {
        let mut st = self.conn.prepare(
            "SELECT t.artist_id, a.name, t.id, t.title,
                    (SELECT al.title FROM scrobble s2 JOIN album al ON al.id = s2.album_id
                     WHERE s2.track_id = t.id GROUP BY s2.album_id ORDER BY COUNT(*) DESC LIMIT 1),
                    COUNT(*) AS plays
             FROM track t JOIN artist a ON a.id = t.artist_id JOIN scrobble s ON s.track_id = t.id
             WHERE NOT EXISTS (SELECT 1 FROM track_features f WHERE f.track_id = t.id)
             GROUP BY t.id ORDER BY SUM(COUNT(*)) OVER (PARTITION BY t.artist_id) DESC, t.artist_id, plays DESC",
        )?;
        let mut out: Vec<PendingArtist> = Vec::new();
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, PendingTrack { track_id: r.get(2)?, title: r.get(3)?, album: r.get(4)? }))
        })?;
        for row in rows {
            let (artist_id, name, track) = row?;
            match out.last_mut() {
                Some(p) if p.artist_id == artist_id => p.tracks.push(track),
                _ => {
                    if out.len() == limit {
                        break;
                    }
                    out.push(PendingArtist { artist_id, name, tracks: vec![track] });
                }
            }
        }
        Ok(out)
    }

    pub fn enrich_status(&self) -> Result<EnrichStatus> {
        let (ok, not_found, errors, pending, covered, plays): (i64, i64, i64, i64, i64, i64) = self.conn.query_row(
            "WITH played AS (SELECT track_id, COUNT(*) AS n FROM scrobble GROUP BY track_id)
             SELECT COALESCE(SUM(f.status = 'ok'), 0), COALESCE(SUM(f.status = 'not_found'), 0),
                    COALESCE(SUM(f.status = 'error'), 0), COALESCE(SUM(f.status IS NULL), 0),
                    COALESCE(SUM(CASE WHEN f.status = 'ok' THEN p.n END), 0), COALESCE(SUM(p.n), 0)
             FROM played p LEFT JOIN track_features f ON f.track_id = p.track_id",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        let until = THROTTLED_UNTIL.load(Ordering::Relaxed);
        Ok(EnrichStatus {
            ok,
            not_found,
            errors,
            pending,
            plays_covered: if plays > 0 { covered as f64 / plays as f64 } else { 0.0 },
            current_artist: None,
            running: false,
            throttled_until: (until > chrono::Utc::now().timestamp()).then_some(until),
        })
    }

    /// Vide les caches devenus inutiles : catalogues ReccoBeats périmés ou d'artistes
    /// sans titre en attente, réponses des découvertes de plus d'un mois. Renvoie le
    /// nombre d'entrées supprimées. La place est rendue par `compact_if_worth`.
    pub fn purge_caches(&self) -> Result<usize> {
        let now = chrono::Utc::now().timestamp();
        let mut n = self.conn.execute(
            "DELETE FROM reccobeats_artist_cache WHERE fetched_at < ?1 OR NOT EXISTS (
               SELECT 1 FROM track t JOIN scrobble s ON s.track_id = t.id
               WHERE t.artist_id = reccobeats_artist_cache.artist_id
                 AND NOT EXISTS (SELECT 1 FROM track_features f WHERE f.track_id = t.id))",
            [now - CATALOG_TTL],
        )?;
        n += self.conn.execute("DELETE FROM disc_cache WHERE fetched_at < ?", [now - 30 * 86_400])?;
        Ok(n)
    }

    /// Compacte la base si plus d'un quart du fichier (et au moins 5 Mo) est libre.
    /// `VACUUM` synchronise le disque à chaque page (plusieurs secondes) : on écrit
    /// plutôt une copie compacte, synchronisée une fois, qui remplace le fichier.
    pub fn compact_if_worth(&mut self) -> Result<bool> {
        let (free, total, page): (i64, i64, i64) = self.conn.query_row(
            "SELECT (SELECT freelist_count FROM pragma_freelist_count), (SELECT page_count FROM pragma_page_count), (SELECT page_size FROM pragma_page_size)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if free * page < 5 << 20 || free * 4 < total {
            return Ok(false);
        }
        let Some(path) = self.conn.path().filter(|p| !p.is_empty()).map(std::path::PathBuf::from) else {
            self.conn.execute_batch("VACUUM")?;
            return Ok(true);
        };
        let tmp = path.with_extension("compact");
        self.backup_to(&tmp)?;
        std::fs::File::open(&tmp)?.sync_all()?;
        drop(std::mem::replace(&mut self.conn, rusqlite::Connection::open_in_memory()?));
        // Un journal resté là serait rejoué sur la copie compacte.
        for ext in ["sqlite-wal", "sqlite-shm"] {
            let _ = std::fs::remove_file(path.with_extension(ext));
        }
        let swap = std::fs::rename(&tmp, &path);
        *self = Library::open(&path)?;
        swap?;
        Ok(true)
    }

    fn cached_catalog(&self, artist_id: i64) -> Result<Option<Vec<RbTrack>>> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row("SELECT fetched_at, tracks FROM reccobeats_artist_cache WHERE artist_id = ?", [artist_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(row
            .filter(|(at, _)| chrono::Utc::now().timestamp() - at < CATALOG_TTL)
            .and_then(|(_, json)| serde_json::from_str(&json).ok()))
    }

    fn store_catalog(&self, artist_id: i64, tracks: &[RbTrack]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO reccobeats_artist_cache (artist_id, fetched_at, tracks) VALUES (?, ?, ?)",
            params![artist_id, chrono::Utc::now().timestamp(), serde_json::to_string(tracks).unwrap_or_default()],
        )?;
        Ok(())
    }

    fn store_features(&self, track_id: i64, m: Option<&RbTrack>, f: Option<&Value>, status: &str) -> Result<()> {
        let num = |k: &str| f.and_then(|f| f.get(k)).and_then(Value::as_f64);
        let int = |k: &str| f.and_then(|f| f.get(k)).and_then(Value::as_i64);
        self.conn.execute(
            "INSERT OR REPLACE INTO track_features (track_id, status, provider, external_id, matched_title, matched_artist, isrc,
               danceability, energy, valence, acousticness, instrumentalness, speechiness, liveness, tempo, loudness, key, mode, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                track_id, status, PROVIDER, m.map(|m| &m.spotify_id), m.map(|m| &m.title),
                m.map(|m| m.artists.join(", ")), m.and_then(|m| m.isrc.as_ref()),
                num("danceability"), num("energy"), num("valence"), num("acousticness"), num("instrumentalness"),
                num("speechiness"), num("liveness"), num("tempo"), num("loudness"),
                int("key").filter(|k| (0..12).contains(k)), int("mode"), chrono::Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    /// Oublie les échecs pour retenter ces titres (après une panne réseau par exemple).
    pub fn reset_enrich_failures(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM track_features WHERE status IN ('error', 'not_found')", [])?)
    }
}

/// Rapproche les titres en attente d'un artiste. Ne demande pas encore les
/// caractéristiques : elles sont regroupées par lots entre artistes.
fn match_artist(lib: &Mutex<Library>, rb: &mut ReccoBeats, p: &PendingArtist) -> Result<Vec<(i64, Option<RbTrack>)>> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let mut found: HashMap<i64, RbTrack> = HashMap::new();
    let missing = |found: &HashMap<i64, RbTrack>| p.tracks.iter().filter(|t| !found.contains_key(&t.track_id)).collect::<Vec<_>>();

    // 1. Par album, quand l'album couvre au moins deux titres en attente.
    let mut by_album: HashMap<String, (&str, Vec<&PendingTrack>)> = HashMap::new();
    for t in &p.tracks {
        if let Some(a) = &t.album {
            by_album.entry(album_key(a)).or_insert((a.as_str(), vec![])).1.push(t);
        }
    }
    for (album, tracks) in by_album.values().filter(|(_, v)| v.len() >= 2) {
        let candidates = rb.album_tracks(&p.name, album)?;
        for t in tracks {
            if let Some(m) = choose(&t.title, &candidates) {
                found.insert(t.track_id, m.clone());
            }
        }
    }
    // 2. Titre par titre.
    for t in missing(&found) {
        if let Some(m) = rb.search(&p.name, &t.title)? {
            found.insert(t.track_id, m);
        }
    }
    // 3. Catalogue de l'artiste pour les derniers.
    let rest = missing(&found);
    if !rest.is_empty() {
        // Lu dans une variable : dans le `match`, le verrou serait tenu jusqu'à
        // la fin et `store_catalog` ci-dessous se bloquerait sur lui-même.
        let cached = lock().cached_catalog(p.artist_id)?;
        let catalog = match cached {
            Some(c) => c,
            None => {
                let c = rb.artist_catalog(&p.name)?;
                lock().store_catalog(p.artist_id, &c)?;
                c
            }
        };
        for t in rest {
            if let Some(m) = choose(&t.title, &catalog) {
                found.insert(t.track_id, m.clone());
            }
        }
    }
    Ok(p.tracks.iter().map(|t| (t.track_id, found.remove(&t.track_id))).collect())
}

/// Demande les caractéristiques des titres trouvés et les enregistre.
fn flush(lib: &Mutex<Library>, rb: &mut ReccoBeats, batch: &mut Vec<(i64, RbTrack)>) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = batch.iter().map(|(_, m)| m.spotify_id.as_str()).collect();
    let feats = rb.features(&ids)?;
    let l = lib.lock().unwrap_or_else(|e| e.into_inner());
    for (track_id, m) in batch.drain(..) {
        let f = feats.get(&m.spotify_id);
        l.store_features(track_id, Some(&m), f, if f.is_some() { "ok" } else { "not_found" })?;
    }
    Ok(())
}

/// Boucle d'enrichissement : traite les artistes en attente jusqu'à épuisement
/// ou annulation. `progress` est appelé après chaque artiste.
pub fn run(lib: &Mutex<Library>, cancel: Arc<AtomicBool>, mut progress: impl FnMut(EnrichStatus)) -> Result<EnrichStatus> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let mut rb = ReccoBeats::new();
    rb.cancel = Some(cancel.clone());
    let mut batch: Vec<(i64, RbTrack)> = Vec::new();
    let mut consecutive_errors = 0;
    let mut result = Ok(());
    'outer: loop {
        // On exclut les titres déjà trouvés et en attente de caractéristiques.
        let artists = lock().pending_artists(20 + batch.len())?;
        let artists: Vec<PendingArtist> = artists
            .into_iter()
            .filter_map(|mut p| {
                p.tracks.retain(|t| !batch.iter().any(|(id, _)| *id == t.track_id));
                (!p.tracks.is_empty()).then_some(p)
            })
            .collect();
        if artists.is_empty() {
            break;
        }
        for p in &artists {
            if cancel.load(Ordering::Relaxed) {
                break 'outer;
            }
            let step = match_artist(lib, &mut rb, p).and_then(|matches| {
                let l = lock();
                for (track_id, m) in matches {
                    match m {
                        Some(m) => batch.push((track_id, m)),
                        None => l.store_features(track_id, None, None, "not_found")?,
                    }
                }
                drop(l);
                if batch.len() >= FEATURES_BATCH { flush(lib, &mut rb, &mut batch) } else { Ok(()) }
            });
            match step {
                Ok(()) => consecutive_errors = 0,
                Err(Error::Cancelled) => break 'outer,
                Err(e) => {
                    debug(|| format!("erreur pour {} : {e}", p.name));
                    // Titres marqués en erreur pour ne pas boucler dessus ;
                    // « Réessayer les échecs » les relance.
                    let l = lock();
                    for t in &p.tracks {
                        if !batch.iter().any(|(id, _)| *id == t.track_id) {
                            l.store_features(t.track_id, None, None, "error")?;
                        }
                    }
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        result = Err(e);
                        break 'outer;
                    }
                }
            }
            let mut st = lock().enrich_status()?;
            st.running = true;
            st.current_artist = Some(p.name.clone());
            progress(st);
        }
    }
    // Les titres déjà trouvés ne sont pas perdus, même en cas d'arrêt.
    rb.cancel = None;
    if let Err(e) = flush(lib, &mut rb, &mut batch) {
        result = result.and(Err(e));
    }
    result?;
    lock().purge_caches()?;
    let st = lock().enrich_status()?;
    progress(st.clone());
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, pop: i64) -> RbTrack {
        RbTrack { id: title.into(), title: title.into(), spotify_id: title.into(), isrc: None, popularity: pop, artists: vec![] }
    }

    #[test]
    fn prefers_studio_over_more_popular_live() {
        let c = [t("Belle Journée - Live à l'Olympia", 50), t("Belle Journée", 30)];
        assert_eq!(choose("Belle Journée", &c).unwrap().title, "Belle Journée");
    }

    #[test]
    fn loose_match_when_only_a_suffixed_version_exists() {
        let c = [t("ABC - Bonus Track", 40), t("Autre titre", 90)];
        assert_eq!(choose("ABC", &c).unwrap().title, "ABC - Bonus Track");
        let c = [t("LENTAMENTE - Cap.1: Inicio", 60)];
        assert_eq!(choose("Lentamente", &c).unwrap().title, "LENTAMENTE - Cap.1: Inicio");
    }

    #[test]
    fn avoids_unwanted_versions_but_keeps_wanted_ones() {
        let c = [t("D.A.N.C.E. (Orchestral Version)", 60), t("D.A.N.C.E.", 50)];
        assert_eq!(choose("D.A.N.C.E.", &c).unwrap().title, "D.A.N.C.E.");
        let c = [t("So What", 40), t("So What - Live at Montreux", 60)];
        assert_eq!(choose("So What - Live at Montreux", &c).unwrap().title, "So What - Live at Montreux");
    }

    #[test]
    fn collective_name_matches_one_of_its_artists() {
        let rb = |id: &str, artists: &[&str]| RbTrack { id: id.into(), title: "Freedom".into(), spotify_id: id.into(), isrc: None, popularity: 50, artists: artists.iter().map(|a| a.to_string()).collect() };
        let list = vec![rb("other", &["Somebody"]), rb("ah", &["Sam Cole", "Ada Reed"])];
        assert_eq!(pick("Freedom", &list, "Sam Cole & Ada Reed").map(|t| t.id.as_str()), Some("ah"));
        // Crédité au nom entier : préféré à un titre d'un seul des artistes.
        let list = vec![rb("solo", &["Ada Reed"]), rb("duo", &["Sam Cole & Ada Reed"])];
        assert_eq!(pick("Freedom", &list, "Sam Cole & Ada Reed").map(|t| t.id.as_str()), Some("duo"));
        assert_eq!(pick("Freedom", &list, "Nobody"), None);
    }

    #[test]
    fn no_match() {
        assert!(choose("Get Lucky", &[t("Get Back", 90)]).is_none());
    }
}
