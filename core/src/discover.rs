//! Découvertes : des titres jamais écoutés, proposés à partir des artistes
//! qu'on écoute le plus.
//!
//! Aucune base complète n'est téléchargée. Pour chaque artiste de départ, deux
//! services gratuits donnent ses voisins : ListenBrainz (artistes écoutés dans
//! les mêmes sessions par des millions d'auditeurs, par identifiant MusicBrainz)
//! et Deezer (artistes liés). Les voisins proches de plusieurs départs passent
//! devant ; on prend leurs titres phares sur Deezer (avec extrait de 30 s), sans
//! ce qui a déjà été écouté, ni les lives et remixes. Les avis (« j'aime »,
//! « pas pour moi ») pèsent sur la sélection suivante. Les réponses des
//! services sont gardées un mois : une nouvelle sélection coûte peu.

use crate::images::{API as DEEZER, get_json as deezer};
use crate::normalize::{artist_key, is_live, title_key};
use crate::{Error, Library, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const DAY: i64 = 86_400;
/// Clé de titre sans rien entre parenthèses ni après un tiret : « Song (2003 Edit) »
/// et « Song - Remastered » valent « Song ». Large exprès : mieux vaut écarter un
/// titre de trop que proposer une version de ce qu'on connaît déjà.
fn loose_title(title: &str) -> String {
    let base = title.split(['(', '[']).next().unwrap_or(title).split(" - ").next().unwrap_or(title);
    let k = title_key(base);
    if k.is_empty() { title_key(title) } else { k }
}

/// Durée de vie des réponses gardées.
const CACHE_TTL: i64 = 30 * DAY;
/// Artistes de départ de la sélection de la semaine.
const SEEDS: usize = 20;
/// Voisins retenus par artiste de départ et par service.
const NEIGHBOURS: usize = 15;
/// Artistes candidats dont on va chercher les titres.
const CANDIDATES: usize = 40;
const NEW_SIZE: usize = 30;
const KNOWN_SIZE: usize = 20;
const LISTENBRAINZ: &str = "https://labs.api.listenbrainz.org/similar-artists/json?algorithm=session_based_days_7500_session_300_contribution_5_threshold_10_limit_100_filter_True_skip_30&artist_mbids=";

/// Où chercher : sélection de la semaine (artistes les plus écoutés) ou autour d'un artiste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum Scope {
    Weekly,
    Artist(i64),
}

impl Scope {
    fn key(self) -> String {
        match self {
            Scope::Weekly => "weekly".into(),
            Scope::Artist(id) => format!("artist:{id}"),
        }
    }
}

/// Un titre proposé.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Found {
    pub deezer_id: i64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub cover: Option<String>,
    pub duration_s: Option<i64>,
    /// Artistes écoutés qui ont mené à ce titre (les plus proches d'abord).
    pub because: Vec<String>,
    /// Sources du rapprochement : « listenbrainz », « deezer ».
    pub sources: Vec<String>,
    pub score: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selection {
    /// Titres d'artistes jamais écoutés.
    pub new_artists: Vec<Found>,
    /// Titres jamais écoutés d'artistes déjà écoutés.
    pub known_artists: Vec<Found>,
    pub computed_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DiscoverStatus {
    pub running: bool,
    /// Étapes faites et prévues (artistes de départ, puis candidats).
    pub done: usize,
    pub total: usize,
    pub current: Option<String>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------- accès réseau, avec cache ----------

static LB_SLOT: LazyLock<Mutex<Instant>> = LazyLock::new(|| Mutex::new(Instant::now()));
static LB_AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::AgentBuilder::new().user_agent(concat!("LibrestFM/", env!("CARGO_PKG_VERSION"))).timeout(Duration::from_secs(30)).build()
});

/// ListenBrainz : une requête par seconde au plus, par politesse.
fn listenbrainz(url: &str) -> Result<Value> {
    let wait = {
        let mut next = LB_SLOT.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let slot = (*next).max(now);
        *next = slot + Duration::from_secs(1);
        slot - now
    };
    std::thread::sleep(wait);
    match LB_AGENT.get(url).call() {
        Ok(r) => crate::json_body(r).map_err(|e| Error::Http(e.to_string())),
        Err(ureq::Error::Status(400 | 404, _)) => Ok(Value::Array(vec![])),
        Err(e) => Err(Error::Http(e.to_string())),
    }
}

impl Library {
    fn cached(&self, url: &str) -> Result<Option<Value>> {
        let row: Option<(String, i64)> =
            self.conn.query_row("SELECT body, fetched_at FROM disc_cache WHERE url = ?", [url], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        Ok(row.filter(|r| now() - r.1 < CACHE_TTL).and_then(|r| serde_json::from_str(&r.0).ok()))
    }

    fn remember(&self, url: &str, v: &Value) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO disc_cache (url, body, fetched_at) VALUES (?, ?, ?)", params![url, v.to_string(), now()])?;
        Ok(())
    }
}

/// Réponse gardée si récente, sinon demandée (sans tenir le verrou de la base pendant l'appel).
fn fetch(lib: &Mutex<Library>, url: &str, get: fn(&str) -> Result<Value>) -> Result<Value> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = lock().cached(url)? {
        return Ok(v);
    }
    let v = get(url)?;
    lock().remember(url, &v)?;
    Ok(v)
}

/// Identifiant Deezer d'un artiste : même nom, le plus suivi.
fn deezer_artist(lib: &Mutex<Library>, name: &str) -> Result<Option<i64>> {
    let v = fetch(lib, &format!("{DEEZER}/search/artist?q={}&limit=25", urlencoding::encode(name)), deezer)?;
    let key = artist_key(name);
    Ok(v.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| artist_key(n) == key))
        .max_by_key(|a| a.get("nb_fan").and_then(Value::as_i64).unwrap_or(0))
        .and_then(|a| a.get("id")?.as_i64()))
}

/// Titres phares d'un artiste Deezer.
fn top_tracks(lib: &Mutex<Library>, deezer_id: i64) -> Result<Vec<Value>> {
    let v = fetch(lib, &format!("{DEEZER}/artist/{deezer_id}/top?limit=8"), deezer)?;
    Ok(v.get("data").and_then(Value::as_array).cloned().unwrap_or_default())
}

// ---------- calcul ----------

struct Seed {
    name: String,
    mbid: Option<String>,
    weight: f64,
}

#[derive(Default)]
struct Candidate {
    name: String,
    deezer_id: Option<i64>,
    score: f64,
    /// Départ → force du lien (pour les raisons affichées).
    because: HashMap<String, f64>,
    sources: HashSet<&'static str>,
}

impl Library {
    /// Artistes de départ : les plus écoutés récemment (chaque écoute pondérée par son
    /// ancienneté, demi-vie de 3 mois, comme pour les mixes).
    fn discover_seeds(&self, scope: Scope) -> Result<Vec<Seed>> {
        let only = if let Scope::Artist(id) = scope { id } else { 0 };
        let limit = if only == 0 { SEEDS } else { 1 };
        let end: i64 = self.conn.query_row("SELECT COALESCE(MAX(ts), 0) FROM scrobble", [], |r| r.get(0))?;
        let mut weight: HashMap<i64, f64> = HashMap::new();
        let mut st = self.conn.prepare("SELECT artist_id, ts FROM scrobble WHERE ?1 = 0 OR artist_id = ?1")?;
        for row in st.query_map([only], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (a, ts) = row?;
            *weight.entry(a).or_default() += crate::mixes::recency(end - ts);
        }
        let mut ranked: Vec<(i64, f64)> = weight.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut info = self.conn.prepare("SELECT a.name, COALESCE(a.mbid, i.mbid) FROM artist a LEFT JOIN artist_info i ON i.artist_id = a.id WHERE a.id = ?")?;
        let mut rows = Vec::new();
        for (id, w) in ranked {
            let (name, mbid): (String, Option<String>) = info.query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if crate::normalize::is_placeholder_artist(&name) {
                continue;
            }
            rows.push((name, mbid, w));
            if rows.len() == limit {
                break;
            }
        }
        let top = rows.iter().map(|r| r.2).fold(f64::MIN_POSITIVE, f64::max);
        Ok(rows.into_iter().map(|(name, mbid, w)| Seed { name, mbid, weight: (w / top).sqrt() }).collect())
    }

    /// Ce qui est déjà connu : artistes écoutés, titres écoutés (artiste + titre normalisés).
    fn known(&self) -> Result<(HashSet<String>, HashSet<(String, String)>)> {
        let mut st = self.conn.prepare("SELECT DISTINCT a.key FROM scrobble s JOIN artist a ON a.id = s.artist_id")?;
        let artists = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut st = self.conn.prepare(
            "SELECT DISTINCT a.key, t.title FROM scrobble s JOIN track t ON t.id = COALESCE(s.raw_track_id, s.track_id) JOIN artist a ON a.id = t.artist_id",
        )?;
        let tracks = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, loose_title(&r.get::<_, String>(1)?))))?
            .collect::<rusqlite::Result<_>>()?;
        Ok((artists, tracks))
    }

    /// Avis donnés : titre → +1 / -1, et somme par artiste (clé normalisée).
    fn discover_feedback(&self) -> Result<(HashMap<i64, i64>, HashMap<String, i64>)> {
        let mut st = self.conn.prepare("SELECT deezer_id, artist, verdict FROM disc_feedback")?;
        let mut tracks = HashMap::new();
        let mut artists: HashMap<String, i64> = HashMap::new();
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
            let (id, artist, v) = row?;
            tracks.insert(id, v);
            *artists.entry(artist_key(&artist)).or_default() += v;
        }
        Ok((tracks, artists))
    }

    /// Avis sur un titre proposé (+1 j'aime, -1 pas pour moi, 0 retire l'avis).
    pub fn discover_verdict(&self, deezer_id: i64, artist: &str, verdict: i64) -> Result<()> {
        if verdict == 0 {
            self.conn.execute("DELETE FROM disc_feedback WHERE deezer_id = ?", [deezer_id])?;
        } else {
            self.conn.execute(
                "INSERT OR REPLACE INTO disc_feedback (deezer_id, artist, verdict, at) VALUES (?, ?, ?, ?)",
                params![deezer_id, artist, verdict.signum(), now()],
            )?;
        }
        Ok(())
    }

    /// Dernière sélection calculée (sans les titres écartés depuis).
    pub fn selection(&self, scope: Scope) -> Result<Selection> {
        let body: Option<String> = self.conn.query_row("SELECT body FROM disc_result WHERE scope = ?", [scope.key()], |r| r.get(0)).optional()?;
        let mut d: Selection = body.and_then(|b| serde_json::from_str(&b).ok()).unwrap_or_default();
        let (verdicts, _) = self.discover_feedback()?;
        for list in [&mut d.new_artists, &mut d.known_artists] {
            list.retain(|f| verdicts.get(&f.deezer_id) != Some(&-1));
        }
        Ok(d)
    }

    /// Sélection de la semaine à refaire (jamais faite : non, il faut la demander une fois).
    pub fn discover_due(&self) -> Result<bool> {
        let at: Option<i64> = self.conn.query_row("SELECT computed_at FROM disc_result WHERE scope = 'weekly'", [], |r| r.get(0)).optional()?;
        Ok(at.is_some_and(|t| now() - t > 7 * DAY))
    }
}

/// Calcule une sélection et la garde. Le verrou de la base n'est pris que le
/// temps des lectures et écritures, jamais pendant les appels réseau.
pub fn run(lib: &Mutex<Library>, scope: Scope, cancel: Arc<AtomicBool>, mut progress: impl FnMut(DiscoverStatus)) -> Result<DiscoverStatus> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let (seeds, (known_artists, known_tracks), (verdicts, artist_verdicts)) = {
        let l = lock();
        (l.discover_seeds(scope)?, l.known()?, l.discover_feedback()?)
    };
    let mut st = DiscoverStatus { running: true, done: 0, total: seeds.len() + CANDIDATES, current: None };
    let mut step = |st: &mut DiscoverStatus, label: &str| -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        st.done += 1;
        st.current = Some(label.to_owned());
        progress(st.clone());
        Ok(())
    };

    // 1. Voisins de chaque artiste de départ.
    let mut cands: HashMap<String, Candidate> = HashMap::new();
    let mut seed_ids: Vec<(i64, f64, String)> = Vec::new();
    for seed in &seeds {
        step(&mut st, &seed.name)?;
        let mut add = |name: &str, deezer_id: Option<i64>, sim: f64, source: &'static str| {
            let key = artist_key(name);
            if key.is_empty() || key == artist_key(&seed.name) {
                return;
            }
            let c = cands.entry(key).or_insert_with(|| Candidate { name: name.to_owned(), ..Default::default() });
            c.deezer_id = c.deezer_id.or(deezer_id);
            c.score += seed.weight * sim;
            *c.because.entry(seed.name.clone()).or_default() += seed.weight * sim;
            c.sources.insert(source);
        };
        if let Some(mbid) = &seed.mbid {
            let v = fetch(lib, &format!("{LISTENBRAINZ}{mbid}"), listenbrainz)?;
            let list: Vec<&Value> = v.as_array().into_iter().flatten().take(NEIGHBOURS).collect();
            let top = list.first().and_then(|a| a.get("score")?.as_f64()).unwrap_or(1.0).max(1.0);
            for a in list {
                if let (Some(name), Some(score)) = (a.get("name").and_then(Value::as_str), a.get("score").and_then(Value::as_f64)) {
                    add(name, None, score / top, "listenbrainz");
                }
            }
        }
        if let Some(id) = deezer_artist(lib, &seed.name)? {
            seed_ids.push((id, seed.weight, seed.name.clone()));
            let v = fetch(lib, &format!("{DEEZER}/artist/{id}/related?limit=20"), deezer)?;
            for (i, a) in v.get("data").and_then(Value::as_array).into_iter().flatten().take(NEIGHBOURS).enumerate() {
                if let Some(name) = a.get("name").and_then(Value::as_str) {
                    add(name, a.get("id").and_then(Value::as_i64), 0.8 * (1.0 - i as f64 / 20.0), "deezer");
                }
            }
        }
    }

    // 2. Candidats : artistes jamais écoutés, les plus proches de plusieurs départs d'abord.
    let mut ranked: Vec<Candidate> = cands
        .into_iter()
        .filter(|(k, _)| !known_artists.contains(k))
        .map(|(k, mut c)| {
            // Avis sur l'artiste : chaque « j'aime » le pousse, chaque « pas pour moi » l'écarte.
            let v = artist_verdicts.get(&k).copied().unwrap_or(0);
            c.score *= if v >= 0 { 1.0 + 0.5 * v as f64 } else { 0.5f64.powi(-v as i32) };
            // Proche de plusieurs départs : bonus.
            c.score *= 1.0 + 0.15 * (c.because.len() as f64 - 1.0).max(0.0);
            c
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    ranked.truncate(CANDIDATES);
    st.total = st.done + ranked.len() + seed_ids.len().min(10);

    let keep = |t: &Value, artist: &str| -> Option<(i64, String)> {
        let id = t.get("id")?.as_i64()?;
        let title = t.get("title").and_then(Value::as_str)?.to_owned();
        // Déjà écouté sous n'importe lequel de ses artistes : un titre en featuring
        // est souvent enregistré au nom de l'invité.
        let loose = loose_title(&title);
        let contributors = t.get("contributors").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("name")?.as_str());
        let heard = std::iter::once(artist).chain(contributors).any(|a| known_tracks.contains(&(artist_key(a), loose.clone())));
        let ok = !is_live(&title) && verdicts.get(&id) != Some(&-1) && !heard;
        ok.then_some((id, title))
    };
    let found = |t: &Value, title: String, artist: &str, because: Vec<String>, sources: Vec<String>, score: f64| Found {
        deezer_id: t.get("id").and_then(Value::as_i64).unwrap_or(0),
        title,
        artist: artist.to_owned(),
        album: t.pointer("/album/title").and_then(Value::as_str).map(str::to_owned),
        cover: t.pointer("/album/cover_medium").and_then(Value::as_str).filter(|u| !u.is_empty()).map(str::to_owned),
        duration_s: t.get("duration").and_then(Value::as_i64),
        because,
        sources,
        score,
    };

    // 3. Titres phares des candidats : deux au plus par artiste.
    let mut new_artists = Vec::new();
    for c in &ranked {
        step(&mut st, &c.name)?;
        let id = match c.deezer_id {
            Some(id) => Some(id),
            None => deezer_artist(lib, &c.name)?,
        };
        let Some(id) = id else { continue };
        let mut because: Vec<(&String, &f64)> = c.because.iter().collect();
        because.sort_by(|a, b| b.1.total_cmp(a.1));
        let because: Vec<String> = because.into_iter().take(3).map(|(n, _)| n.clone()).collect();
        let mut sources: Vec<String> = c.sources.iter().map(|s| s.to_string()).collect();
        sources.sort();
        let mut seen = HashSet::new();
        for (i, t) in top_tracks(lib, id)?.iter().enumerate() {
            let artist = t.pointer("/artist/name").and_then(Value::as_str).unwrap_or(&c.name).to_owned();
            let Some((_, title)) = keep(t, &artist) else { continue };
            if !seen.insert(loose_title(&title)) {
                continue;
            }
            let liked = verdicts.get(&t.get("id").and_then(Value::as_i64).unwrap_or(0)) == Some(&1);
            let score = c.score / (1.0 + 0.35 * i as f64) * if liked { 1.5 } else { 1.0 };
            new_artists.push(found(t, title, &artist, because.clone(), sources.clone(), score));
            if seen.len() == 2 {
                break;
            }
        }
    }
    new_artists.sort_by(|a, b| b.score.total_cmp(&a.score));
    new_artists.truncate(NEW_SIZE);

    // 4. Titres phares jamais écoutés des artistes de départ eux-mêmes.
    let mut known_new = Vec::new();
    for (id, weight, name) in seed_ids.iter().take(10) {
        step(&mut st, name)?;
        let mut n = 0;
        for (i, t) in top_tracks(lib, *id)?.iter().enumerate() {
            let artist = t.pointer("/artist/name").and_then(Value::as_str).unwrap_or(name).to_owned();
            // Le top d'un artiste contient parfois des titres d'autres artistes où il est invité.
            if artist_key(&artist) != artist_key(name) {
                continue;
            }
            let Some((_, title)) = keep(t, &artist) else { continue };
            known_new.push(found(t, title, &artist, vec![name.clone()], vec!["deezer".into()], weight / (1.0 + 0.35 * i as f64)));
            n += 1;
            if n == 2 {
                break;
            }
        }
    }
    known_new.sort_by(|a, b| b.score.total_cmp(&a.score));
    known_new.truncate(KNOWN_SIZE);

    let d = Selection { new_artists, known_artists: known_new, computed_at: Some(now()) };
    lock().conn.execute(
        "INSERT OR REPLACE INTO disc_result (scope, body, computed_at) VALUES (?, ?, ?)",
        params![scope.key(), serde_json::to_string(&d).map_err(|e| Error::Http(e.to_string()))?, now()],
    )?;
    lock().purge_caches()?;
    st.running = false;
    st.current = None;
    progress(st.clone());
    Ok(st)
}

/// Adresse fraîche de l'extrait de 30 s (les adresses Deezer expirent).
pub fn preview(deezer_id: i64) -> Result<Option<String>> {
    let v = deezer(&format!("{DEEZER}/track/{deezer_id}"))?;
    Ok(v.get("preview").and_then(Value::as_str).filter(|u| !u.is_empty()).map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_titles_match_loosely() {
        assert_eq!(loose_title("Don't Stop 'Til You Get Enough (2003 Edit)"), loose_title("Don't Stop 'Til You Get Enough"));
        assert_eq!(loose_title("Get Lucky - Radio Edit"), loose_title("Get Lucky"));
        assert_ne!(loose_title("(Where Are) The Good Days?"), "");
    }

    #[test]
    fn rejected_tracks_leave_the_selection() {
        let lib = Library::open_in_memory().unwrap();
        let f = |id: i64| Found {
            deezer_id: id,
            title: format!("t{id}"),
            artist: "A".into(),
            album: None,
            cover: None,
            duration_s: None,
            because: vec![],
            sources: vec![],
            score: 1.0,
        };
        let d = Selection { new_artists: vec![f(1), f(2)], known_artists: vec![], computed_at: Some(0) };
        lib.conn.execute("INSERT INTO disc_result VALUES ('weekly', ?, 0)", [serde_json::to_string(&d).unwrap()]).unwrap();
        lib.discover_verdict(2, "A", -1).unwrap();
        let ids: Vec<i64> = lib.selection(Scope::Weekly).unwrap().new_artists.iter().map(|f| f.deezer_id).collect();
        assert_eq!(ids, [1]);
        assert!(lib.discover_due().unwrap(), "sélection vieille de plus d'une semaine");
    }
}
