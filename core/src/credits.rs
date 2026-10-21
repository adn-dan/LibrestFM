//! Crédits détaillés des titres (auteurs, compositeurs, producteurs, musiciens
//! et leurs instruments…) depuis MusicBrainz (gratuit, sans clé, 1 requête/s).
//!
//! Les auteurs et compositeurs sont attachés à l'œuvre, communs à toutes ses
//! versions ; producteurs et musiciens dépendent de l'enregistrement précis.
//! On cherche donc l'enregistrement le mieux documenté : candidats par ISRC
//! puis par recherche, classés (album écouté, sortie officielle), et on garde
//! le premier qui a des crédits d'enregistrement (3 essais au plus).

use crate::normalize::{album_key, artist_key, artist_key_no_article, artist_names, fold, is_live, title_key};
use crate::{Error, Library, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const API: &str = "https://musicbrainz.org/ws/2";
/// MusicBrainz demande un User-Agent identifiant l'application.
const USER_AGENT: &str = concat!("LibrestFM/", env!("CARGO_PKG_VERSION"), " ( desktop application )");
/// Limite officielle : une requête par seconde.
const MIN_INTERVAL: Duration = Duration::from_millis(1100);
const MAX_LOOKUPS: usize = 3;
const INC: &str = "artist-credits+artist-rels+work-rels+work-level-rels";

static NEXT_SLOT: LazyLock<Mutex<Instant>> = LazyLock::new(|| Mutex::new(Instant::now()));
static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::AgentBuilder::new().user_agent(USER_AGENT).timeout(Duration::from_secs(20)).build());

/// GET MusicBrainz au rythme imposé (partagé avec le module info).
pub(crate) fn get(url: &str, cancel: Option<&AtomicBool>) -> Result<Option<Value>> {
    for attempt in 0..5 {
        let wait = {
            let mut next = NEXT_SLOT.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let slot = (*next).max(now);
            *next = slot + MIN_INTERVAL;
            slot - now
        };
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
            std::thread::sleep((until - Instant::now()).min(Duration::from_millis(200)));
        }
        match AGENT.get(url).call() {
            Ok(r) => return crate::json_body(r).map(Some).map_err(|e| Error::Http(e.to_string())),
            Err(ureq::Error::Status(404 | 400, _)) => return Ok(None),
            // 503 : débit dépassé, on ralentit tout le monde.
            Err(ureq::Error::Status(503, _)) | Err(ureq::Error::Transport(_)) if attempt < 4 => {
                *NEXT_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now() + Duration::from_secs(2 << attempt);
            }
            Err(e) => return Err(Error::Http(e.to_string())),
        }
    }
    Err(Error::Http(format!("MusicBrainz ne répond pas : {url}")))
}

/// Échappe une valeur pour une requête de recherche (syntaxe Lucene).
pub(crate) fn lucene(s: &str) -> String {
    s.chars().fold(String::new(), |mut out, c| {
        if r#"+-&|!(){}[]^"~*?:\/"#.contains(c) {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Credit {
    pub person_mbid: String,
    pub name: String,
    /// Type de relation MusicBrainz (writer, composer, lyricist, producer,
    /// instrument, vocal, engineer, mix…), ou `credited` pour l'artiste crédité.
    pub role: String,
    /// Précision : instrument (« bass guitar »), type de voix (« lead vocals »)…
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Fetched {
    pub recording_mbid: Option<String>,
    /// Langues des paroles (ISO 639-3), toutes œuvres confondues, séparées par des virgules.
    pub work_language: Option<String>,
    pub length_ms: Option<i64>,
    pub credits: Vec<Credit>,
    /// Artistes crédités dans l'ordre, tels qu'écrits sur l'enregistrement.
    pub artist_credit: Vec<CreditName>,
}

/// Un nom de l'« artist-credit » MusicBrainz : « Nova » + « feat. » + « Zoé ».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreditName {
    pub mbid: String,
    pub name: String,
    /// Liaison avec le nom suivant (« feat. », « & »…), vide pour le dernier.
    pub join: String,
}

fn parse_artist_credit(v: &Value) -> Vec<CreditName> {
    v.get("artist-credit")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let a = c.get("artist")?;
            Some(CreditName {
                mbid: a.get("id")?.as_str()?.to_owned(),
                name: c.get("name").or(a.get("name"))?.as_str()?.to_owned(),
                join: c.get("joinphrase").and_then(Value::as_str).unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

/// Un artiste MusicBrainz correspond-il à ce nom ? Nom, nom de tri inversé
/// (« Sakamoto, Ryūichi » pour 坂本龍一) et alias, article « The » toléré.
pub(crate) fn artist_matches(a: &Value, name: &str) -> bool {
    let want = artist_key_no_article(name);
    let same = |s: &str| artist_key_no_article(s) == want;
    let sort_reversed = |s: &str| match s.split_once(", ") {
        Some((last, first)) => format!("{first} {last}"),
        None => s.to_owned(),
    };
    a.get("name").and_then(Value::as_str).is_some_and(same)
        || a.get("sort-name").and_then(Value::as_str).is_some_and(|s| same(&sort_reversed(s)) || same(s))
        || a.get("aliases").and_then(Value::as_array).into_iter().flatten().any(|x| {
            x.get("name").and_then(Value::as_str).is_some_and(same) || x.get("sort-name").and_then(Value::as_str).is_some_and(|s| same(&sort_reversed(s)))
        })
}

/// Un enregistrement ou un release group est-il de cet artiste ? (identifiant si connu, sinon nom)
pub(crate) fn credit_matches(rec: &Value, name: &str, artist_mbid: Option<&str>) -> bool {
    rec.get("artist-credit").and_then(Value::as_array).into_iter().flatten().any(|c| {
        let a = c.get("artist");
        artist_mbid.is_some_and(|m| a.and_then(|a| a.get("id")).and_then(Value::as_str) == Some(m))
            || c.get("name").and_then(Value::as_str).is_some_and(|n| artist_key_no_article(n) == artist_key_no_article(name))
            || a.is_some_and(|a| artist_matches(a, name))
    })
}

enum Source<'a> {
    /// Enregistrements portant l'ISRC reconnu par ReccoBeats (1 requête).
    Isrc(&'a str),
    /// Recherche titre + artiste (1 requête).
    Search,
}

/// Titre sans version ni précision : « Je t'ai manqué - Live à l'Olympia / 2008 » → « Je t'ai manqué ».
pub(crate) fn base_track_title(title: &str) -> &str {
    let t = title.split(['(', '[']).next().unwrap_or(title);
    let t = t.split(" - ").next().unwrap_or(t);
    let t = t.split(" / ").next().unwrap_or(t);
    let t = t.trim();
    if t.is_empty() { title } else { t }
}

/// Enregistrements candidats d'une source, classés du plus au moins prometteur.
fn candidates(
    artist: &str,
    artist_mbid: Option<&str>,
    title: &str,
    album: Option<&str>,
    source: Source,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<String>> {
    let tkey = title_key(title);
    let loose = |s: &str| fold(base_track_title(s));
    let title_ok = |r: &Value| {
        let t = r.get("title").and_then(Value::as_str).unwrap_or_default();
        title_key(t) == tkey || loose(t) == loose(title)
    };
    let artist_ok = |r: &Value| credit_matches(r, artist, artist_mbid);
    let matches = |r: &Value| title_ok(r) && artist_ok(r);
    let mut ranked: Vec<(i64, String)> = Vec::new();
    let mut push = |r: &Value, bonus: i64| {
        let Some(id) = r.get("id").and_then(Value::as_str) else { return };
        let releases = r.get("releases").and_then(Value::as_array).cloned().unwrap_or_default();
        let on_album = album.is_some_and(|a| {
            releases.iter().any(|rel| rel.get("title").and_then(Value::as_str).is_some_and(|t| album_key(t) == album_key(a) || loose(t) == loose(a)))
        });
        let official_album = releases.iter().any(|rel| {
            rel.get("status").and_then(Value::as_str) == Some("Official")
                && rel.get("release-group").and_then(|g| g.get("primary-type")).and_then(Value::as_str) == Some("Album")
        });
        let score = bonus + if on_album { 100 } else { 0 } + if official_album { 20 } else { 0 };
        match ranked.iter_mut().find(|(_, i)| i == id) {
            Some(existing) => existing.0 = existing.0.max(score),
            None => ranked.push((score, id.to_owned())),
        }
    };
    match source {
        Source::Isrc(isrc) => {
            // L'ISRC désigne l'enregistrement : l'artiste OU le titre suffit à confirmer
            // (titre en japonais, « Pt. I » / « Part 1 », artiste crédité autrement…).
            if let Some(v) = get(&format!("{API}/isrc/{isrc}?fmt=json&inc=artist-credits+releases"), cancel)? {
                for r in v.get("recordings").and_then(Value::as_array).into_iter().flatten().filter(|r| artist_ok(r) || title_ok(r)) {
                    push(r, 0);
                }
            }
        }
        Source::Search => {
            // Du plus précis au plus large ; on s'arrête dès qu'un enregistrement correspond.
            // Sans l'artiste en dernier recours : certains noms (« Can ») sont ignorés par la recherche.
            let base = base_track_title(title);
            let mut queries = Vec::new();
            // Identifiant d'artiste connu : indépendant de l'écriture du nom (japonais, « The »…).
            if let Some(m) = artist_mbid {
                queries.push(format!("recording:\"{}\" AND arid:{m}", lucene(title)));
                if base != title {
                    queries.push(format!("recording:\"{}\" AND arid:{m}", lucene(base)));
                }
            }
            queries.push(format!("recording:\"{}\" AND artist:\"{}\"", lucene(title), lucene(artist)));
            if base != title {
                queries.push(format!("recording:\"{}\" AND artist:\"{}\"", lucene(base), lucene(artist)));
            }
            queries.push(format!("recording:({})", lucene(base)));
            for q in queries {
                let mut found = 0;
                if let Some(v) = get(&format!("{API}/recording?fmt=json&limit=50&query={}", urlencoding::encode(&q)), cancel)? {
                    for r in v.get("recordings").and_then(Value::as_array).into_iter().flatten().filter(|r| matches(r)) {
                        push(r, 0);
                        found += 1;
                    }
                }
                if found > 0 {
                    break;
                }
            }
        }
    }
    ranked.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    Ok(ranked.into_iter().map(|(_, id)| id).collect())
}

fn parse_recording(v: &Value) -> Fetched {
    let mut f = Fetched {
        recording_mbid: v.get("id").and_then(Value::as_str).map(str::to_owned),
        length_ms: v.get("length").and_then(Value::as_i64),
        artist_credit: parse_artist_credit(v),
        ..Default::default()
    };
    let mut add = |a: &Value, role: &str, detail: String| {
        if let (Some(id), Some(name)) = (a.get("id").and_then(Value::as_str), a.get("name").and_then(Value::as_str)) {
            let c = Credit { person_mbid: id.into(), name: name.into(), role: role.into(), detail };
            if !f.credits.contains(&c) {
                f.credits.push(c);
            }
        }
    };
    for c in v.get("artist-credit").and_then(Value::as_array).into_iter().flatten() {
        if let Some(a) = c.get("artist") {
            add(a, "credited", String::new());
        }
    }
    let attrs = |r: &Value| {
        r.get("attributes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()
    };
    for r in v.get("relations").and_then(Value::as_array).into_iter().flatten() {
        let ty = r.get("type").and_then(Value::as_str).unwrap_or_default();
        if let Some(a) = r.get("artist") {
            add(a, ty, attrs(r));
        }
        if let Some(w) = r.get("work") {
            // Une œuvre peut avoir plusieurs langues (« languages ») ; un titre, plusieurs œuvres.
            let mut langs: Vec<String> = f.work_language.as_deref().map(|l| l.split(',').map(str::to_owned).collect()).unwrap_or_default();
            let found = w.get("languages").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>());
            for l in found.unwrap_or_else(|| w.get("language").and_then(Value::as_str).map(str::to_owned).into_iter().collect()) {
                if !langs.contains(&l) {
                    langs.push(l);
                }
            }
            f.work_language = (!langs.is_empty()).then(|| langs.join(","));
            for wr in w.get("relations").and_then(Value::as_array).into_iter().flatten() {
                if let Some(a) = wr.get("artist") {
                    add(a, wr.get("type").and_then(Value::as_str).unwrap_or_default(), attrs(wr));
                }
            }
        }
    }
    f
}

/// Crédits d'un titre : enregistrement le mieux documenté parmi les candidats.
pub fn fetch(
    artist: &str,
    artist_mbid: Option<&str>,
    title: &str,
    album: Option<&str>,
    mbid: Option<&str>,
    isrc: Option<&str>,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Fetched>> {
    // Sources essayées dans l'ordre ; on s'arrête au premier enregistrement qui
    // a des crédits d'enregistrement (producteur, musiciens…). Cas courant : 2 requêtes.
    let mut best: Option<Fetched> = None;
    let mut seen: Vec<String> = Vec::new();
    let mut lookups = 0;
    let phases: Vec<Option<Source>> = match (mbid, isrc) {
        (Some(_), _) => vec![None, Some(Source::Search)],
        (None, Some(i)) => vec![Some(Source::Isrc(i)), Some(Source::Search)],
        (None, None) => vec![Some(Source::Search)],
    };
    for phase in phases {
        let ids = match phase {
            None => mbid.map(|m| vec![m.to_owned()]).unwrap_or_default(),
            Some(src) => candidates(artist, artist_mbid, title, album, src, cancel)?,
        };
        for id in ids {
            if seen.contains(&id) || lookups >= MAX_LOOKUPS {
                continue;
            }
            seen.push(id.clone());
            lookups += 1;
            let Some(v) = get(&format!("{API}/recording/{id}?fmt=json&inc={INC}"), cancel)? else { continue };
            let f = parse_recording(&v);
            let recording_level = f.credits.iter().any(|c| !matches!(c.role.as_str(), "credited" | "writer" | "composer" | "lyricist"));
            if best.as_ref().is_none_or(|b| f.credits.len() > b.credits.len()) {
                best = Some(f);
            }
            if recording_level {
                return Ok(best);
            }
        }
    }
    Ok(best)
}

// ---------- base ----------

#[derive(Debug, Clone, Serialize)]
pub struct TrackCredit {
    pub person_id: i64,
    /// La personne est aussi un artiste de l'historique : sa fiche artiste.
    pub artist_id: Option<i64>,
    pub name: String,
    pub role: String,
    pub detail: String,
    /// Titres de ta bibliothèque auxquels cette personne a participé.
    pub tracks_in_library: i64,
}

/// Artiste crédité d'un titre, avec sa fiche quand il est aussi dans l'historique.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CreditedArtist {
    pub person_id: i64,
    pub artist_id: Option<i64>,
    pub name: String,
    pub join: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackCredits {
    /// none : pas encore cherchés ; ok ; not_found ; error.
    pub status: String,
    pub recording_mbid: Option<String>,
    pub work_language: Option<String>,
    pub credits: Vec<TrackCredit>,
    pub artist_credit: Vec<CreditedArtist>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumCredit {
    pub person_id: i64,
    pub artist_id: Option<i64>,
    pub name: String,
    pub role: String,
    pub detail: String,
    /// Titres de l'album sur lesquels cette personne a ce rôle.
    pub tracks: i64,
    /// Titres de toute ta bibliothèque auxquels elle a participé.
    pub tracks_in_library: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumCredits {
    /// Titres de l'album présents dans ta bibliothèque.
    pub tracks_total: i64,
    /// Titres dont les crédits ont déjà été cherchés.
    pub tracks_done: i64,
    /// Titres pas encore cherchés, les plus écoutés d'abord.
    pub pending: Vec<i64>,
    pub credits: Vec<AlbumCredit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonTrack {
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    pub artist_id: i64,
    pub plays: i64,
    /// « producer », « instrument: guitar »…
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonDetail {
    pub id: i64,
    /// Fiche artiste qui la remplace, si la personne est aussi un artiste écouté.
    pub artist_id: Option<i64>,
    pub mbid: String,
    pub name: String,
    pub total_plays: i64,
    /// (rôle, nombre de titres)
    pub roles: Vec<(String, i64)>,
    pub tracks: Vec<PersonTrack>,
}

/// Personne créditée chez deux artistes, avec ses rôles chez l'autre artiste.
#[derive(Debug, Clone, Serialize)]
pub struct LinkPerson {
    pub id: i64,
    pub artist_id: Option<i64>,
    pub name: String,
    pub roles: Vec<String>,
}

/// Un artiste relié par des personnes en commun (membres, producteurs, musiciens…).
#[derive(Debug, Clone, Serialize)]
pub struct ArtistLink {
    pub artist: crate::stats::Ranked,
    pub people: Vec<LinkPerson>,
}

/// Ce qu'un artiste a fait chez un autre artiste (producteur, musicien, auteur…).
#[derive(Debug, Clone, Serialize)]
pub struct CreditedFor {
    pub artist: crate::stats::Ranked,
    /// « producer », « instrument: guitar »…
    pub roles: Vec<String>,
    /// (titre, nom, écoutes), les plus écoutés d'abord.
    pub tracks: Vec<(i64, String, i64)>,
}

/// Rôles techniques : un ingénieur du son relie des artistes sans rapport.
const TECHNICAL_ROLES: [&str; 5] = ["engineer", "mix", "recording", "editor", "mastering"];

#[derive(Debug, Clone, Default, Serialize)]
pub struct CreditsStatus {
    pub ok: i64,
    pub not_found: i64,
    pub errors: i64,
    pub pending: i64,
    /// Crédits : fiches anciennes dont l'ordre des artistes est à relire.
    pub refresh: i64,
    pub plays_covered: f64,
    pub current: Option<String>,
    pub running: bool,
}

/// Ce qu'il faut pour chercher les crédits d'un titre.
#[derive(Debug, Clone)]
pub struct CreditQuery {
    pub track_id: i64,
    pub artist_id: i64,
    pub artist_mbid: Option<String>,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub mbid: Option<String>,
    pub isrc: Option<String>,
}

impl Library {
    fn credit_query(&self, track_id: i64) -> Result<Option<CreditQuery>> {
        Ok(self
            .conn
            .query_row(
                "SELECT a.name, t.title, t.mbid, f.isrc,
                        (SELECT al.title FROM scrobble s JOIN album al ON al.id = s.album_id
                         WHERE s.track_id = t.id GROUP BY s.album_id ORDER BY COUNT(*) DESC LIMIT 1),
                        a.id, COALESCE(a.mbid, ai.mbid)
                 FROM track t JOIN artist a ON a.id = t.artist_id
                 LEFT JOIN track_features f ON f.track_id = t.id AND f.status = 'ok'
                 LEFT JOIN artist_info ai ON ai.artist_id = a.id
                 WHERE t.id = ?",
                [track_id],
                |r| {
                    Ok(CreditQuery {
                        track_id,
                        artist: r.get(0)?,
                        title: r.get(1)?,
                        mbid: r.get(2)?,
                        isrc: r.get(3)?,
                        album: r.get(4)?,
                        artist_id: r.get(5)?,
                        artist_mbid: r.get(6)?,
                    })
                },
            )
            .optional()?)
    }

    /// Titres sans crédits, les plus écoutés d'abord.
    pub fn pending_credits(&self, limit: usize) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare(
            "SELECT s.track_id FROM scrobble s WHERE NOT EXISTS (SELECT 1 FROM track_credits c WHERE c.track_id = s.track_id)
             GROUP BY s.track_id ORDER BY COUNT(*) DESC LIMIT ?",
        )?;
        let ids = st.query_map([limit as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    fn store_credits(&self, track_id: i64, found: Option<&Fetched>, status: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM credit WHERE track_id = ?", [track_id])?;
        if let Some(f) = found {
            for c in &f.credits {
                tx.execute("INSERT OR IGNORE INTO person (mbid, name) VALUES (?, ?)", params![c.person_mbid, c.name])?;
                let pid: i64 = tx.query_row("SELECT id FROM person WHERE mbid = ?", [&c.person_mbid], |r| r.get(0))?;
                tx.execute(
                    "INSERT OR IGNORE INTO credit (track_id, person_id, role, detail) VALUES (?, ?, ?, ?)",
                    params![track_id, pid, c.role, c.detail],
                )?;
            }
        }
        if let Some(ms) = found.and_then(|f| f.length_ms) {
            tx.execute("UPDATE track SET duration_ms = ? WHERE id = ? AND duration_ms IS NULL", params![ms, track_id])?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO track_credits (track_id, status, recording_mbid, work_language, length_ms, fetched_at, artist_credit) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                track_id,
                status,
                found.and_then(|f| f.recording_mbid.clone()),
                found.and_then(|f| f.work_language.clone()),
                found.and_then(|f| f.length_ms),
                chrono::Utc::now().timestamp(),
                found.map(|f| serde_json::to_string(&f.artist_credit).unwrap_or_default()),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn track_credits(&self, track_id: i64) -> Result<TrackCredits> {
        let head: Option<(String, Option<String>, Option<String>)> = self
            .conn
            .query_row("SELECT status, recording_mbid, work_language FROM track_credits WHERE track_id = ?", [track_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((status, recording_mbid, work_language)) = head else {
            return Ok(TrackCredits { status: "none".into(), recording_mbid: None, work_language: None, credits: vec![], artist_credit: vec![] });
        };
        let mut st = self.conn.prepare(
            "SELECT p.id, p.name, c.role, c.detail,
                    (SELECT COUNT(DISTINCT c2.track_id) FROM credit c2 WHERE c2.person_id = p.id)
             FROM credit c JOIN person p ON p.id = c.person_id WHERE c.track_id = ? ORDER BY c.role, p.name",
        )?;
        let credits = st
            .query_map([track_id], |r| {
                Ok(TrackCredit { person_id: r.get(0)?, artist_id: None, name: r.get(1)?, role: r.get(2)?, detail: r.get(3)?, tracks_in_library: r.get(4)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let artists = self.person_artists()?;
        let credits = credits.into_iter().map(|c| TrackCredit { artist_id: artists.get(&c.person_id).copied(), ..c }).collect();
        Ok(TrackCredits { status, recording_mbid, work_language, credits, artist_credit: self.artist_credit(track_id)? })
    }

    /// Artistes crédités dans l'ordre MusicBrainz, quand il y en a plusieurs et
    /// que l'artiste de tes écoutes en fait partie (sinon la fiche trouvée est
    /// douteuse et on s'en tient aux écoutes).
    pub fn artist_credit(&self, track_id: i64) -> Result<Vec<CreditedArtist>> {
        let row: Option<(Option<String>, String, i64)> = self
            .conn
            .query_row(
                "SELECT c.artist_credit, a.name, a.id FROM track t JOIN artist a ON a.id = t.artist_id
                 LEFT JOIN track_credits c ON c.track_id = t.id WHERE t.id = ?",
                [track_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((Some(json), artist, artist_id)) = row else { return Ok(vec![]) };
        let names: Vec<CreditName> = serde_json::from_str(&json).unwrap_or_default();
        if names.len() < 2 {
            return Ok(vec![]);
        }
        let artists = self.person_artists()?;
        let artist_id = crate::aliases::Resolver::load(&self.conn)?.get(crate::stats::Kind::Artist, artist_id);
        let mut person = self.conn.prepare_cached("SELECT id FROM person WHERE mbid = ?")?;
        let mut out = Vec::new();
        for n in names {
            let Some(person_id) = person.query_row([&n.mbid], |r| r.get::<_, i64>(0)).optional()? else { return Ok(vec![]) };
            out.push(CreditedArtist { person_id, artist_id: artists.get(&person_id).copied(), name: n.name, join: n.join });
        }
        let key = artist_key_no_article(&artist);
        let ours = out.iter().any(|c| c.artist_id == Some(artist_id) || artist_key_no_article(&c.name) == key);
        Ok(if ours { out } else { vec![] })
    }

    /// Fiche lue avant que l'ordre des artistes soit gardé, avec plusieurs
    /// artistes crédités : son enregistrement MusicBrainz, à relire.
    fn stale_artist_credit(&self, track_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT recording_mbid FROM track_credits c WHERE track_id = ? AND status = 'ok' AND artist_credit IS NULL
                 AND recording_mbid IS NOT NULL
                 AND (SELECT COUNT(*) FROM credit WHERE track_id = c.track_id AND role = 'credited') > 1",
                [track_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Titres dont l'ordre des artistes est à relire, les plus écoutés d'abord.
    fn pending_artist_credits(&self, limit: usize) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare(
            "SELECT c.track_id FROM track_credits c JOIN scrobble s ON s.track_id = c.track_id
             WHERE c.status = 'ok' AND c.artist_credit IS NULL AND c.recording_mbid IS NOT NULL
             AND (SELECT COUNT(*) FROM credit WHERE track_id = c.track_id AND role = 'credited') > 1
             GROUP BY c.track_id ORDER BY COUNT(*) DESC LIMIT ?",
        )?;
        let ids = st.query_map([limit as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Titres dont l'année d'origine reste à chercher, les plus écoutés d'abord.
    fn pending_original_years(&self, limit: usize) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare(
            "SELECT c.track_id FROM track_credits c JOIN scrobble s ON s.track_id = c.track_id
             WHERE c.status IN ('ok', 'not_found') AND c.first_release IS NULL
             GROUP BY c.track_id ORDER BY COUNT(*) DESC LIMIT ?",
        )?;
        let ids = st.query_map([limit as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    fn store_first_release(&self, track_id: i64, date: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE track_credits SET first_release = ? WHERE track_id = ?", params![date.unwrap_or(""), track_id])?;
        Ok(())
    }

    fn store_artist_credit(&self, track_id: i64, names: &[CreditName]) -> Result<()> {
        for n in names {
            self.conn.execute("INSERT OR IGNORE INTO person (mbid, name) VALUES (?, ?)", params![n.mbid, n.name])?;
        }
        self.conn.execute(
            "UPDATE track_credits SET artist_credit = ? WHERE track_id = ?",
            params![serde_json::to_string(names).unwrap_or_default(), track_id],
        )?;
        Ok(())
    }

    /// Crédits cumulés des titres d'un album (ceux que tu as écoutés sur cet album).
    pub fn album_credits(&self, album_id: i64) -> Result<AlbumCredits> {
        let mut st = self.conn.prepare(
            "SELECT s.track_id, c.status IS NOT NULL FROM scrobble s LEFT JOIN track_credits c ON c.track_id = s.track_id
             WHERE s.album_id = ? GROUP BY s.track_id ORDER BY COUNT(*) DESC",
        )?;
        let tracks: Vec<(i64, bool)> = st.query_map([album_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut st = self.conn.prepare(
            "SELECT p.id, p.name, c.role, c.detail, COUNT(DISTINCT c.track_id),
                    (SELECT COUNT(DISTINCT c2.track_id) FROM credit c2 WHERE c2.person_id = p.id)
             FROM credit c JOIN person p ON p.id = c.person_id
             WHERE c.track_id IN (SELECT DISTINCT track_id FROM scrobble WHERE album_id = ?1)
             GROUP BY p.id, c.role, c.detail ORDER BY 5 DESC, p.name",
        )?;
        let credits = st
            .query_map([album_id], |r| {
                Ok(AlbumCredit {
                    person_id: r.get(0)?,
                    artist_id: None,
                    name: r.get(1)?,
                    role: r.get(2)?,
                    detail: r.get(3)?,
                    tracks: r.get(4)?,
                    tracks_in_library: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let artists = self.person_artists()?;
        let credits = credits.into_iter().map(|c| AlbumCredit { artist_id: artists.get(&c.person_id).copied(), ..c }).collect();
        Ok(AlbumCredits {
            tracks_total: tracks.len() as i64,
            tracks_done: tracks.iter().filter(|t| t.1).count() as i64,
            pending: tracks.iter().filter(|t| !t.1).map(|t| t.0).collect(),
            credits,
        })
    }

    pub fn person(&self, id: i64) -> Result<PersonDetail> {
        let (mbid, name): (String, String) = self
            .conn
            .query_row("SELECT mbid, name FROM person WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("person {id}")))?;
        let mut st = self.conn.prepare(
            "SELECT t.id, t.title, a.name, a.id, (SELECT COUNT(*) FROM scrobble s WHERE s.track_id = t.id),
                    group_concat(CASE WHEN c.detail = '' THEN c.role ELSE c.role || ': ' || c.detail END, '|')
             FROM credit c JOIN track t ON t.id = c.track_id JOIN artist a ON a.id = t.artist_id
             WHERE c.person_id = ? GROUP BY t.id ORDER BY 5 DESC",
        )?;
        let tracks: Vec<PersonTrack> = st
            .query_map([id], |r| {
                let roles: String = r.get(5)?;
                Ok(PersonTrack {
                    track_id: r.get(0)?,
                    title: r.get(1)?,
                    artist: r.get(2)?,
                    artist_id: r.get(3)?,
                    plays: r.get(4)?,
                    roles: roles.split('|').map(str::to_owned).collect(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut st = self.conn.prepare("SELECT role, COUNT(DISTINCT track_id) FROM credit WHERE person_id = ? GROUP BY role ORDER BY 2 DESC")?;
        let roles = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let artist_id = self.person_artists()?.get(&id).copied();
        Ok(PersonDetail { id, artist_id, mbid, name, total_plays: tracks.iter().map(|t| t.plays).sum(), roles, tracks })
    }

    /// Personnes qui sont aussi un artiste de l'historique (même identifiant
    /// MusicBrainz, jamais le seul nom) : personne → artiste corrigé le plus écouté.
    pub fn person_artists(&self) -> Result<std::collections::HashMap<i64, i64>> {
        use std::collections::HashMap;
        let resolver = crate::aliases::Resolver::load(&self.conn)?;
        let mut st = self.conn.prepare(
            "SELECT p.id, x.artist_id FROM person p JOIN (
               SELECT artist_id, mbid FROM artist_info WHERE mbid IS NOT NULL
               UNION SELECT id, mbid FROM artist WHERE mbid IS NOT NULL) x ON x.mbid = p.mbid",
        )?;
        let pairs: Vec<(i64, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut plays = self.conn.prepare_cached("SELECT COUNT(*) FROM scrobble WHERE artist_id = ?")?;
        let mut best: HashMap<i64, (i64, i64)> = HashMap::new();
        for (person, artist) in pairs {
            let artist = resolver.get(crate::stats::Kind::Artist, artist);
            let n: i64 = plays.query_row([artist], |r| r.get(0))?;
            if n > 0 && best.get(&person).is_none_or(|b| n > b.1) {
                best.insert(person, (artist, n));
            }
        }
        Ok(best.into_iter().map(|(p, (a, _))| (p, a)).collect())
    }

    /// Ce que l'artiste a fait chez d'autres artistes écoutés, d'après les crédits
    /// où il figure comme personne (producteur, musicien, auteur…).
    pub fn artist_credits(&self, artist_id: i64) -> Result<Vec<CreditedFor>> {
        use std::collections::{BTreeSet, HashMap};
        let persons: Vec<i64> = self.person_artists()?.into_iter().filter(|x| x.1 == artist_id).map(|x| x.0).collect();
        let resolver = crate::aliases::Resolver::load(&self.conn)?;
        let mut by_artist: HashMap<i64, (BTreeSet<String>, HashMap<i64, (String, i64)>)> = HashMap::new();
        for p in persons {
            for t in self.person(p)?.tracks {
                let other = resolver.get(crate::stats::Kind::Artist, t.artist_id);
                if other == artist_id || t.plays == 0 {
                    continue;
                }
                let e = by_artist.entry(other).or_default();
                e.0.extend(t.roles);
                e.1.insert(t.track_id, (t.title, t.plays));
            }
        }
        let mut out = Vec::new();
        for (other, (roles, tracks)) in by_artist {
            let mut tracks: Vec<(i64, String, i64)> = tracks.into_iter().map(|(id, (title, n))| (id, title, n)).collect();
            tracks.sort_by_key(|t| std::cmp::Reverse(t.2));
            let plays = tracks.iter().map(|t| t.2).sum();
            out.push(CreditedFor { artist: self.ranked(crate::stats::Kind::Artist, other, plays)?, roles: roles.into_iter().collect(), tracks });
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.artist.plays));
        Ok(out)
    }

    /// Artistes écoutés qui partagent des personnes créditées avec `artist_id`,
    /// les plus liés d'abord (nombre de personnes, puis écoutes).
    pub fn artist_links(&self, artist_id: i64, limit: usize) -> Result<Vec<ArtistLink>> {
        use std::collections::{BTreeSet, HashMap, HashSet};
        let resolver = crate::aliases::Resolver::load(&self.conn)?;
        let artist = |id: i64| resolver.get(crate::stats::Kind::Artist, id);
        let mut st = self.conn.prepare(
            "SELECT c.person_id, p.name, t.artist_id, CASE WHEN c.detail = '' THEN c.role ELSE c.role || ': ' || c.detail END
             FROM credit c JOIN track t ON t.id = c.track_id JOIN person p ON p.id = c.person_id",
        )?;
        let rows: Vec<(i64, String, i64, String)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, artist(r.get(2)?), r.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|r: &(i64, String, i64, String)| !TECHNICAL_ROLES.contains(&r.3.split(':').next().unwrap_or_default()))
            .collect();
        // L'artiste lui-même, crédité ailleurs, est dans « Crédité chez d'autres artistes ».
        let person_artist = self.person_artists()?;
        let own: HashSet<i64> = rows.iter().filter(|r| r.2 == artist_id && person_artist.get(&r.0) != Some(&artist_id)).map(|r| r.0).collect();
        let mut links: HashMap<i64, HashMap<i64, (String, BTreeSet<String>)>> = HashMap::new();
        for (person, name, other, role) in rows {
            if other != artist_id && own.contains(&person) {
                links.entry(other).or_default().entry(person).or_insert_with(|| (name, BTreeSet::new())).1.insert(role);
            }
        }
        let mut plays = self.conn.prepare("SELECT COUNT(*) FROM scrobble WHERE artist_id = ?")?;
        let mut out = Vec::new();
        for (other, people) in links {
            let n: i64 = plays.query_row([other], |r| r.get(0))?;
            if n == 0 {
                continue;
            }
            let mut people: Vec<LinkPerson> =
                people.into_iter().map(|(id, (name, roles))| LinkPerson { id, artist_id: person_artist.get(&id).copied(), name, roles: roles.into_iter().collect() }).collect();
            people.sort_by(|a, b| a.name.cmp(&b.name));
            out.push(ArtistLink { artist: self.ranked(crate::stats::Kind::Artist, other, n)?, people });
        }
        out.sort_by_key(|l| std::cmp::Reverse((l.people.len(), l.artist.plays)));
        out.truncate(limit);
        Ok(out)
    }

    pub fn credits_status(&self) -> Result<CreditsStatus> {
        let (ok, not_found, errors, pending, covered, plays): (i64, i64, i64, i64, i64, i64) = self.conn.query_row(
            "WITH played AS (SELECT track_id, COUNT(*) AS n FROM scrobble GROUP BY track_id)
             SELECT COALESCE(SUM(c.status = 'ok'), 0), COALESCE(SUM(c.status = 'not_found'), 0),
                    COALESCE(SUM(c.status = 'error'), 0), COALESCE(SUM(c.status IS NULL), 0),
                    COALESCE(SUM(CASE WHEN c.status = 'ok' THEN p.n END), 0), COALESCE(SUM(p.n), 0)
             FROM played p LEFT JOIN track_credits c ON c.track_id = p.track_id",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        let refresh: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM track_credits c WHERE c.status = 'ok' AND c.artist_credit IS NULL AND c.recording_mbid IS NOT NULL
             AND (SELECT COUNT(*) FROM credit WHERE track_id = c.track_id AND role = 'credited') > 1
             AND EXISTS (SELECT 1 FROM scrobble WHERE track_id = c.track_id)",
            [],
            |r| r.get(0),
        )?;
        let years: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM track_credits c WHERE c.status IN ('ok', 'not_found') AND c.first_release IS NULL
             AND EXISTS (SELECT 1 FROM scrobble WHERE track_id = c.track_id)",
            [],
            |r| r.get(0),
        )?;
        let refresh = refresh + years;
        Ok(CreditsStatus {
            ok,
            not_found,
            errors,
            pending,
            refresh,
            plays_covered: if plays > 0 { covered as f64 / plays as f64 } else { 0.0 },
            current: None,
            running: false,
        })
    }

    pub fn reset_credit_failures(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM track_credits WHERE status IN ('error', 'not_found')", [])?)
    }
}

/// Cherche et enregistre les crédits d'un titre (sans tenir le verrou pendant le réseau).
pub fn fetch_track(lib: &Mutex<Library>, track_id: i64, cancel: Option<&AtomicBool>) -> Result<()> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let q = lock().credit_query(track_id)?;
    let Some(mut q) = q else { return Ok(()) };
    // Identifiant de l'artiste pas encore connu : on récupère d'abord sa fiche (une fois par artiste).
    let known = lock().artist_info(q.artist_id)?.is_some();
    if q.artist_mbid.is_none() && !known {
        let _ = crate::info::fetch_artist(lib, q.artist_id, cancel);
        q = lock().credit_query(track_id)?.unwrap_or(q);
    }
    match fetch(&q.artist, q.artist_mbid.as_deref(), &q.title, q.album.as_deref(), q.mbid.as_deref(), q.isrc.as_deref(), cancel) {
        Ok(Some(f)) if !f.credits.is_empty() => lock().store_credits(track_id, Some(&f), "ok"),
        Ok(_) => lock().store_credits(track_id, None, "not_found"),
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Err(e) => {
            lock().store_credits(track_id, None, "error")?;
            Err(e)
        }
    }
}

/// Relit l'ordre des artistes crédités d'une fiche ancienne (une requête).
pub fn refresh_artist_credit(lib: &Mutex<Library>, track_id: i64, cancel: Option<&AtomicBool>) -> Result<()> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mbid) = lock().stale_artist_credit(track_id)? else { return Ok(()) };
    // Enregistrement disparu de MusicBrainz : liste vide, on ne le redemande plus.
    let names = get(&format!("{API}/recording/{mbid}?fmt=json&inc=artist-credits"), cancel)?.map(|v| parse_artist_credit(&v)).unwrap_or_default();
    lock().store_artist_credit(track_id, &names)
}

/// Première sortie d'une œuvre parmi les résultats de recherche MusicBrainz : même titre
/// (sans mention de version), même artiste (ou l'un d'un nom collectif), hors live et remix
/// si le titre écouté n'en est pas un.
fn earliest_release(v: &Value, artist: &str, title: &str) -> Option<String> {
    let keys: Vec<String> = artist_names(artist).iter().map(|n| artist_key(n)).collect();
    let base = |t: &str| title_key(t.split(['(', '[']).next().unwrap_or(t).split(" - ").next().unwrap_or(t));
    let (want, live) = (base(title), is_live(title));
    v.get("recordings")?
        .as_array()?
        .iter()
        .filter(|r| r.get("title").and_then(Value::as_str).is_some_and(|t| base(t) == want && (live || !is_live(t))))
        .filter(|r| {
            r.get("artist-credit").and_then(Value::as_array).is_some_and(|ac| {
                ac.iter().any(|c| c.get("name").and_then(Value::as_str).is_some_and(|n| keys.contains(&artist_key(n))))
            })
        })
        .filter_map(|r| r.get("first-release-date").and_then(Value::as_str).filter(|d| d.len() >= 4 && !d.starts_with("0000")))
        .min()
        .map(str::to_owned)
}

/// Cherche et garde l'année d'origine d'un titre (une ou deux requêtes).
pub fn find_original_year(lib: &Mutex<Library>, track_id: i64, cancel: Option<&AtomicBool>) -> Result<()> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let Some(q) = lock().credit_query(track_id)? else { return Ok(()) };
    let lucene = |s: &str| s.replace(['\\', '"'], " ");
    let base = q.title.split(['(', '[']).next().unwrap_or(&q.title).split(" - ").next().unwrap_or(&q.title).trim().to_owned();
    let search = |query: String| get(&format!("{API}/recording?fmt=json&limit=100&query={}", urlencoding::encode(&query)), cancel);
    // Avec l'artiste d'abord ; certains noms courts ne passent pas la recherche (« FEX ») :
    // alors le titre seul, filtré ici sur l'artiste.
    let mut date = search(format!("recording:\"{}\" AND artist:\"{}\"", lucene(&base), lucene(&q.artist)))?
        .and_then(|v| earliest_release(&v, &q.artist, &q.title));
    if date.is_none() {
        date = search(format!("recording:\"{}\"", lucene(&base)))?.and_then(|v| earliest_release(&v, &q.artist, &q.title));
    }
    lock().store_first_release(track_id, date.as_deref())
}

/// Récupère les crédits des titres en attente, les plus écoutés d'abord.
pub fn run(lib: &Mutex<Library>, cancel: Arc<AtomicBool>, mut progress: impl FnMut(CreditsStatus)) -> Result<CreditsStatus> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let mut consecutive_errors = 0;
    let mut stored = 0;
    'outer: loop {
        // D'abord les titres jamais cherchés, puis l'ordre des artistes des fiches anciennes.
        // Enfin l'année d'origine de chaque titre.
        let mut ids = lock().pending_credits(50)?;
        let refresh = ids.is_empty();
        if refresh {
            ids = lock().pending_artist_credits(50)?;
        }
        let years = ids.is_empty();
        if years {
            ids = lock().pending_original_years(50)?;
        }
        if ids.is_empty() {
            break;
        }
        for id in ids {
            if cancel.load(Ordering::Relaxed) {
                break 'outer;
            }
            let label = lock().credit_query(id)?.map(|q| format!("{} – {}", q.artist, q.title));
            let done = if years {
                find_original_year(lib, id, Some(&cancel))
            } else if refresh {
                refresh_artist_credit(lib, id, Some(&cancel))
            } else {
                fetch_track(lib, id, Some(&cancel))
            };
            match done {
                Ok(()) => {
                    consecutive_errors = 0;
                    stored += 1;
                }
                Err(Error::Cancelled) => break 'outer,
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= 3 {
                        return Err(e);
                    }
                }
            }
            let mut st = lock().credits_status()?;
            st.running = true;
            st.current = label;
            progress(st);
        }
    }
    // Crédits nouveaux : un titre écouté sous le nom d'un invité rejoint l'artiste principal.
    if stored > 0 {
        lock().auto_clean()?;
    }
    let st = lock().credits_status()?;
    progress(st.clone());
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording(credit: Value) -> Value {
        serde_json::json!({ "id": "rec", "artist-credit": credit })
    }

    #[test]
    fn featured_artist_listened_shows_main_artist_first() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.import_bytes("h.csv", "artist,title,album,timestamp\nZoé,Une nuit de plus,,1700000000\nZoé,Solo,,1700001000\n".as_bytes()).unwrap();
        let track = |t: &str| lib.conn.query_row("SELECT id FROM track WHERE title = ?", [t], |r| r.get::<_, i64>(0)).unwrap();
        let (duo, solo) = (track("Une nuit de plus"), track("Solo"));
        let f = parse_recording(&recording(serde_json::json!([
            { "name": "Nova", "joinphrase": " feat. ", "artist": { "id": "m-nova", "name": "Nova" } },
            { "name": "Zoé", "joinphrase": "", "artist": { "id": "m-zoe", "name": "Zoé" } },
        ])));
        lib.store_credits(duo, Some(&f), "ok").unwrap();
        let c = lib.artist_credit(duo).unwrap();
        assert_eq!(c.iter().map(|c| format!("{}{}", c.name, c.join)).collect::<String>(), "Nova feat. Zoé");
        assert_eq!(lib.track_credits(duo).unwrap().artist_credit, c);

        // Fiche trouvée sans l'artiste écouté : douteuse, rien d'affiché.
        let other = parse_recording(&recording(serde_json::json!([
            { "name": "A", "joinphrase": " & ", "artist": { "id": "m-a", "name": "A" } },
            { "name": "B", "joinphrase": "", "artist": { "id": "m-b", "name": "B" } },
        ])));
        lib.store_credits(solo, Some(&other), "ok").unwrap();
        assert!(lib.artist_credit(solo).unwrap().is_empty());

        // Fiche ancienne (ordre non gardé) avec deux artistes : à relire.
        lib.conn.execute("UPDATE track_credits SET artist_credit = NULL", []).unwrap();
        assert_eq!(lib.pending_artist_credits(10).unwrap().len(), 2);
        assert!(lib.artist_credit(duo).unwrap().is_empty());
        lib.store_artist_credit(duo, &f.artist_credit).unwrap();
        assert_eq!(lib.pending_artist_credits(10).unwrap(), vec![solo]);
        assert_eq!(lib.artist_credit(duo).unwrap().len(), 2);
    }

    #[test]
    fn original_year_is_the_earliest_matching_recording() {
        let v = serde_json::json!({ "recordings": [
            { "title": "Jusqu’à demain (Remasterisé en 2002)", "first-release-date": "2022-10-28", "artist-credit": [{ "name": "Jean Martin" }] },
            { "title": "Jusqu’à demain", "first-release-date": "1981-09-03", "artist-credit": [{ "name": "Jean Martin" }] },
            { "title": "Jusqu’à demain (live)", "first-release-date": "1979", "artist-credit": [{ "name": "Jean Martin" }] },
            { "title": "Jusqu’à demain", "first-release-date": "1970", "artist-credit": [{ "name": "Quelqu'un d'autre" }] },
            { "title": "Jusqu’à demain", "artist-credit": [{ "name": "Jean Martin" }] }
        ]});
        assert_eq!(earliest_release(&v, "Jean Martin", "Jusqu'à demain (Remasterisé en 2002)").as_deref(), Some("1981-09-03"));
        assert_eq!(earliest_release(&v, "Jean Martin", "Jusqu'à demain (Live)").as_deref(), Some("1979"));
        assert_eq!(earliest_release(&v, "Nobody", "Jusqu'à demain"), None);
    }

    #[test]
    fn featured_artist_plays_go_to_main_artist() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.import_bytes("h.csv", "artist,title,album,timestamp\nZoé,Une nuit de plus,La route du nord,1700000000\nZoé,Une nuit de plus,La route du nord,1700001000\nZoé,Solo,Zoé,1700002000\nNova,Simple,La route du nord,1700003000\n".as_bytes()).unwrap();
        let track = |l: &Library, t: &str| l.conn.query_row("SELECT id FROM track WHERE title = ?", [t], |r| r.get::<_, i64>(0)).unwrap();
        let plays = |l: &Library, a: &str| -> i64 {
            l.conn.query_row("SELECT COUNT(*) FROM scrobble s JOIN artist a ON a.id = s.artist_id WHERE a.name = ?", [a], |r| r.get(0)).unwrap()
        };
        let duo = track(&lib, "Une nuit de plus");
        let credit = |main: &str| {
            parse_recording(&recording(serde_json::json!([
                { "name": main, "joinphrase": " feat. ", "artist": { "id": format!("m-{main}"), "name": main } },
                { "name": "Zoé", "joinphrase": "", "artist": { "id": "m-zoe", "name": "Zoé" } },
            ])))
        };
        lib.store_credits(duo, Some(&credit("Nova")), "ok").unwrap();
        assert!(lib.auto_clean().unwrap() >= 1);
        assert_eq!((plays(&lib, "Nova"), plays(&lib, "Zoé")), (3, 1));
        // La page du titre (désormais chez Nova) garde « Nova feat. Zoé ».
        let now: i64 = lib.conn.query_row("SELECT track_id FROM scrobble WHERE raw_track_id = ?", [duo], |r| r.get(0)).unwrap();
        assert_eq!(lib.artist_credit(now).unwrap().len(), 2);
        // Titre joué sur le téléphone sous le nom de l'invité : retrouvé chez Nova.
        let found = lib.find_track("Zoé", "Une nuit de plus", None).unwrap().unwrap();
        assert_eq!((found.id, found.artist.as_str()), (now, "Nova"));
        assert_eq!(lib.find_track("Inconnu", "Une nuit de plus", Some("Nova")).unwrap().map(|t| t.id), Some(now));
        assert_eq!(lib.find_track("Inconnu", "Une nuit de plus", None).unwrap(), None);
        // L'album de l'invité (même nom) a suivi : un seul « La route du nord », chez Nova.
        let albums: Vec<(String, String)> = lib
            .conn
            .prepare("SELECT DISTINCT a.name, al.title FROM scrobble s JOIN album al ON al.id = s.album_id JOIN artist a ON a.id = al.artist_id WHERE al.key = ?")
            .unwrap()
            .query_map([crate::normalize::album_key("La route du nord")], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(albums, [("Nova".to_string(), "La route du nord".to_string())]);
        // Annulable.
        let op = lib.merge_ops().unwrap().into_iter().find(|o| o.rule == "featured" && o.kind == crate::stats::Kind::Track).unwrap();
        lib.undo_merge(op.id).unwrap();
        assert_eq!(plays(&lib, "Zoé"), 3);
        // L'album est revenu avec le titre.
        assert!(!lib.merge_ops().unwrap().iter().any(|o| o.rule == "featured"));

        // Artiste principal absent de l'historique : rien.
        let mut lib = Library::open_in_memory().unwrap();
        lib.import_bytes("h.csv", "artist,title,album,timestamp\nZoé,Une nuit de plus,,1700000000\nNova,Simple,,1700003000\n".as_bytes()).unwrap();
        let duo = track(&lib, "Une nuit de plus");
        lib.store_credits(duo, Some(&credit("Inconnu")), "ok").unwrap();
        assert!(!lib.suggestions().unwrap().iter().any(|s| s.rule == "featured"));

        // Simple liste « A & B » (duo, générique) : l'artiste écouté reste.
        let duet = parse_recording(&recording(serde_json::json!([
            { "name": "Nova", "joinphrase": " & ", "artist": { "id": "m-Nova", "name": "Nova" } },
            { "name": "Zoé", "joinphrase": "", "artist": { "id": "m-zoe", "name": "Zoé" } },
        ])));
        lib.store_credits(duo, Some(&duet), "ok").unwrap();
        assert!(!lib.suggestions().unwrap().iter().any(|s| s.rule == "featured"));
    }
}
