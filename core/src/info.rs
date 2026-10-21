//! Informations sur les albums et les artistes :
//! - album : date de première sortie (MusicBrainz, release group), durée, nombre
//!   de titres, label et durée de chaque titre (Deezer, une requête par album) ;
//! - artiste : pays et ville d'origine, groupe ou personne, années d'activité (MusicBrainz).
//!
//! Deezer donne la date de l'édition (une réédition 2018 d'un album de 2004) :
//! on la garde à part et on préfère la date MusicBrainz.

use crate::images::{deezer_album, find_album_id};
use crate::normalize::{album_key, fold, title_key};
use crate::{Error, Library, Result};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earliest_and_most_precise_date() {
        let mut d = ["2020-09-18", "1960", "1960-01-27", "2008-08-20"];
        d.sort_by(|a, b| earlier(a, b));
        assert_eq!(d[0], "1960-01-27");
        assert_eq!(base_title("Giant Steps (60th Anniversary Super Deluxe Edition) [2020 Remaster]"), "Giant Steps");
        assert_eq!(base_title("Debut..."), "Debut...");
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AlbumInfo {
    pub status: String,
    /// Première sortie : « 2004-02-26 », « 2004-02 » ou « 2004 ».
    pub release_date: Option<String>,
    /// Date de l'édition trouvée chez Deezer (réédition éventuelle).
    pub edition_date: Option<String>,
    pub duration_s: Option<i64>,
    pub nb_tracks: Option<i64>,
    pub label: Option<String>,
    /// album, single, ep, compile…
    pub record_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ArtistInfo {
    pub status: String,
    /// Person, Group, Orchestra, Choir…
    pub kind: Option<String>,
    /// Code pays ISO (FR, US…).
    pub country: Option<String>,
    /// Ville ou région d'origine (begin-area), sinon zone principale.
    pub area: Option<String>,
    pub begin: Option<String>,
    pub end: Option<String>,
}

fn mb(url: &str, cancel: Option<&AtomicBool>) -> Result<Option<Value>> {
    crate::credits::get(url, cancel)
}

/// Titre sans mentions d'édition : « Giant Steps (60th Anniversary…) [2020 Remaster] » → « Giant Steps ».
fn base_title(album: &str) -> &str {
    album.split(['(', '[']).next().unwrap_or(album).split(" - ").next().unwrap_or(album).trim()
}

/// Date de première sortie (MusicBrainz) : la plus ancienne parmi les release
/// groups de cet artiste portant ce titre, rééditions et anniversaires compris.
fn mb_release_date(artist: &str, album: &str, cancel: Option<&AtomicBool>) -> Result<Option<(String, Option<String>)>> {
    let base = base_title(album);
    let bkey = fold(base);
    // Recherche exacte avec l'artiste, puis plus large sans lui : certains noms
    // (« Can ») sont ignorés par la recherche, certaines graphies diffèrent (« Ex Military » / « Exmilitary »).
    let queries = [
        format!("releasegroup:\"{}\" AND artist:\"{}\"", crate::credits::lucene(base), crate::credits::lucene(artist)),
        format!("releasegroup:({})", crate::credits::lucene(base)),
        format!("releasegroup:{}", bkey),
    ];
    for q in queries {
        let Some(v) = mb(&format!("https://musicbrainz.org/ws/2/release-group?fmt=json&limit=50&query={}", urlencoding::encode(&q)), cancel)? else {
            continue;
        };
        if let Some(found) = earliest_group(&v, artist, &bkey, album) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Le release group le plus ancien de cet artiste portant ce titre, dans une réponse de recherche.
fn earliest_group(v: &Value, artist: &str, bkey: &str, album: &str) -> Option<(String, Option<String>)> {
    v.get("release-groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|g| crate::credits::credit_matches(g, artist, None))
        .filter(|g| g.get("title").and_then(Value::as_str).is_some_and(|t| fold(base_title(t)) == bkey || album_key(t) == album_key(album)))
        .filter_map(|g| {
            let date = g.get("first-release-date").and_then(Value::as_str).filter(|d| !d.is_empty())?;
            Some((date.to_owned(), g.get("primary-type").and_then(Value::as_str).map(str::to_lowercase)))
        })
        .min_by(|a, b| earlier(&a.0, &b.0))
}

/// Ordre chronologique de dates ISO partielles ; à année égale, la plus précise d'abord
/// (« 1960-01-27 » avant « 1960 »).
fn earlier(a: &str, b: &str) -> std::cmp::Ordering {
    let year = |d: &str| d.get(..4).unwrap_or(d).to_owned();
    year(a).cmp(&year(b)).then(b.len().cmp(&a.len())).then(a.cmp(b))
}

/// Artiste MusicBrainz : par MBID si connu, sinon homonyme exact le mieux classé.
fn mb_artist(name: &str, mbid: Option<&str>, cancel: Option<&AtomicBool>) -> Result<Option<Value>> {
    if let Some(id) = mbid {
        return mb(&format!("https://musicbrainz.org/ws/2/artist/{id}?fmt=json"), cancel);
    }
    // Par nom, puis par alias : le nom principal peut être dans une autre écriture (坂本龍一).
    for field in ["artist", "alias"] {
        let q = format!("{field}:\"{}\"", crate::credits::lucene(name));
        let Some(v) = mb(&format!("https://musicbrainz.org/ws/2/artist?fmt=json&limit=10&query={}", urlencoding::encode(&q)), cancel)? else {
            continue;
        };
        let best = v
            .get("artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|a| crate::credits::artist_matches(a, name))
            .max_by_key(|a| a.get("score").and_then(Value::as_i64).unwrap_or(0))
            .cloned();
        if best.is_some() {
            return Ok(best);
        }
    }
    Ok(None)
}

impl Library {
    pub fn album_info(&self, album_id: i64) -> Result<Option<AlbumInfo>> {
        Ok(self
            .conn
            .query_row(
                "SELECT status, release_date, edition_date, duration_s, nb_tracks, label, record_type FROM album_info WHERE album_id = ?",
                [album_id],
                |r| {
                    Ok(AlbumInfo {
                        status: r.get(0)?,
                        release_date: r.get(1)?,
                        edition_date: r.get(2)?,
                        duration_s: r.get(3)?,
                        nb_tracks: r.get(4)?,
                        label: r.get(5)?,
                        record_type: r.get(6)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn artist_info(&self, artist_id: i64) -> Result<Option<ArtistInfo>> {
        Ok(self
            .conn
            .query_row("SELECT status, kind, country, area, begin, end FROM artist_info WHERE artist_id = ?", [artist_id], |r| {
                Ok(ArtistInfo { status: r.get(0)?, kind: r.get(1)?, country: r.get(2)?, area: r.get(3)?, begin: r.get(4)?, end: r.get(5)? })
            })
            .optional()?)
    }

    /// Albums puis artistes sans informations, les plus écoutés d'abord.
    fn pending_info(&self, limit: usize) -> Result<Vec<(bool, i64)>> {
        let mut st = self.conn.prepare(
            "SELECT 1, s.album_id, COUNT(*) FROM scrobble s WHERE s.album_id IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM album_info i WHERE i.album_id = s.album_id) GROUP BY s.album_id
             UNION ALL
             SELECT 0, s.artist_id, COUNT(*) FROM scrobble s
             WHERE NOT EXISTS (SELECT 1 FROM artist_info i WHERE i.artist_id = s.artist_id) GROUP BY s.artist_id
             ORDER BY 3 DESC LIMIT ?",
        )?;
        let rows = st.query_map([limit as i64], |r| Ok((r.get::<_, i64>(0)? == 1, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn info_status(&self) -> Result<crate::credits::CreditsStatus> {
        let (ok, not_found, errors, pending): (i64, i64, i64, i64) = self.conn.query_row(
            "WITH items AS (
               SELECT i.status FROM (SELECT DISTINCT album_id AS id FROM scrobble WHERE album_id IS NOT NULL) a
                 LEFT JOIN album_info i ON i.album_id = a.id
               UNION ALL
               SELECT i.status FROM (SELECT DISTINCT artist_id AS id FROM scrobble) a LEFT JOIN artist_info i ON i.artist_id = a.id)
             SELECT COALESCE(SUM(status = 'ok'), 0), COALESCE(SUM(status = 'not_found'), 0),
                    COALESCE(SUM(status = 'error'), 0), COALESCE(SUM(status IS NULL), 0) FROM items",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        // Part des écoutes dont l'album a une date de sortie connue.
        let covered: f64 = self.conn.query_row(
            "SELECT COALESCE(AVG(i.release_date IS NOT NULL), 0) FROM scrobble s LEFT JOIN album_info i ON i.album_id = s.album_id",
            [],
            |r| r.get(0),
        )?;
        Ok(crate::credits::CreditsStatus { ok, not_found, errors, pending, refresh: 0, plays_covered: covered, current: None, running: false })
    }

    pub fn reset_info_failures(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM album_info WHERE status IN ('error', 'not_found')", [])?
            + self.conn.execute("DELETE FROM artist_info WHERE status IN ('error', 'not_found')", [])?)
    }
}

/// Récupère et enregistre les informations d'un album.
pub fn fetch_album(lib: &Mutex<Library>, album_id: i64, cancel: Option<&AtomicBool>) -> Result<String> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let q: Option<(String, String)> = lock()
        .conn
        .query_row("SELECT a.name, al.title FROM album al JOIN artist a ON a.id = al.artist_id WHERE al.id = ?", [album_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((artist, title)) = q else { return Ok(String::new()) };
    let label = format!("{artist} – {title}");

    let result = (|| -> Result<(AlbumInfo, Vec<(String, i64)>)> {
        let mut info = AlbumInfo::default();
        let mut tracks = Vec::new();
        if let Some(id) = find_album_id(&artist, &title)?
            && let Some(d) = deezer_album(id)?
        {
            info.edition_date = d.get("release_date").and_then(Value::as_str).filter(|s| !s.starts_with("0000")).map(str::to_owned);
            info.duration_s = d.get("duration").and_then(Value::as_i64);
            info.nb_tracks = d.get("nb_tracks").and_then(Value::as_i64);
            info.label = d.get("label").and_then(Value::as_str).map(str::to_owned);
            info.record_type = d.get("record_type").and_then(Value::as_str).map(str::to_owned);
            tracks = d
                .get("tracks")
                .and_then(|t| t.get("data"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| Some((t.get("title")?.as_str()?.to_owned(), t.get("duration")?.as_i64()?)))
                .collect();
        }
        if let Some((date, kind)) = mb_release_date(&artist, &title, cancel)? {
            info.release_date = Some(date);
            info.record_type = info.record_type.or(kind);
        }
        // La plus ancienne des deux.
        info.release_date = match (info.release_date.take(), info.edition_date.clone()) {
            (Some(a), Some(b)) => Some(if earlier(&b, &a).is_lt() { b } else { a }),
            (a, b) => a.or(b),
        };
        info.status = if info.release_date.is_some() || info.duration_s.is_some() { "ok" } else { "not_found" }.into();
        Ok((info, tracks))
    })();

    let l = lock();
    let (info, tracks) = match result {
        Ok(r) => r,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => {
            l.conn.execute(
                "INSERT OR REPLACE INTO album_info (album_id, status, fetched_at) VALUES (?, 'error', ?)",
                params![album_id, chrono::Utc::now().timestamp()],
            )?;
            return Err(e);
        }
    };
    l.conn.execute(
        "INSERT OR REPLACE INTO album_info (album_id, status, release_date, edition_date, duration_s, nb_tracks, label, record_type, fetched_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            album_id, info.status, info.release_date, info.edition_date, info.duration_s, info.nb_tracks, info.label, info.record_type,
            chrono::Utc::now().timestamp()
        ],
    )?;
    // Durée des titres de l'album que tu as écoutés, rapprochés par titre.
    let mut st = l.conn.prepare("SELECT DISTINCT t.id, t.title FROM scrobble s JOIN track t ON t.id = s.track_id WHERE s.album_id = ?")?;
    let mine: Vec<(i64, String)> = st.query_map([album_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id, t) in mine {
        let key = title_key(&t);
        if let Some((_, secs)) = tracks.iter().find(|(dt, _)| title_key(dt) == key) {
            l.conn.execute("UPDATE track SET duration_ms = ? WHERE id = ?", params![secs * 1000, id])?;
        }
    }
    Ok(label)
}

/// Récupère et enregistre l'origine d'un artiste.
pub fn fetch_artist(lib: &Mutex<Library>, artist_id: i64, cancel: Option<&AtomicBool>) -> Result<String> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let q: Option<(String, Option<String>)> =
        lock().conn.query_row("SELECT name, mbid FROM artist WHERE id = ?", [artist_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((name, mbid)) = q else { return Ok(String::new()) };
    let found = match mb_artist(&name, mbid.as_deref(), cancel) {
        Ok(f) => f,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => {
            lock().conn.execute(
                "INSERT OR REPLACE INTO artist_info (artist_id, status, fetched_at) VALUES (?, 'error', ?)",
                params![artist_id, chrono::Utc::now().timestamp()],
            )?;
            return Err(e);
        }
    };
    let s = |v: &Value, path: &[&str]| -> Option<String> {
        let mut cur = v;
        for p in path {
            cur = cur.get(p)?;
        }
        cur.as_str().filter(|x| !x.is_empty()).map(str::to_owned)
    };
    let l = lock();
    match found {
        Some(a) => {
            let country = s(&a, &["country"]).or_else(|| {
                a.get("area")?.get("iso-3166-1-codes")?.get(0)?.as_str().map(str::to_owned)
            });
            l.conn.execute(
                "INSERT OR REPLACE INTO artist_info (artist_id, status, mbid, kind, country, area, begin, end, fetched_at)
                 VALUES (?, 'ok', ?, ?, ?, ?, ?, ?, ?)",
                params![
                    artist_id,
                    s(&a, &["id"]),
                    s(&a, &["type"]),
                    country,
                    s(&a, &["begin-area", "name"]).or_else(|| s(&a, &["area", "name"])),
                    s(&a, &["life-span", "begin"]),
                    s(&a, &["life-span", "end"]),
                    chrono::Utc::now().timestamp()
                ],
            )?;
            if mbid.is_none() {
                l.conn.execute("UPDATE artist SET mbid = ? WHERE id = ? AND mbid IS NULL", params![s(&a, &["id"]), artist_id])?;
            }
        }
        None => {
            l.conn.execute(
                "INSERT OR REPLACE INTO artist_info (artist_id, status, fetched_at) VALUES (?, 'not_found', ?)",
                params![artist_id, chrono::Utc::now().timestamp()],
            )?;
        }
    }
    Ok(name)
}

/// Albums et artistes en attente, les plus écoutés d'abord.
pub fn run(lib: &Mutex<Library>, cancel: Arc<AtomicBool>, mut progress: impl FnMut(crate::credits::CreditsStatus)) -> Result<crate::credits::CreditsStatus> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let mut consecutive_errors = 0;
    'outer: loop {
        let items = lock().pending_info(50)?;
        if items.is_empty() {
            break;
        }
        for (is_album, id) in items {
            if cancel.load(Ordering::Relaxed) {
                break 'outer;
            }
            let r = if is_album { fetch_album(lib, id, Some(&cancel)) } else { fetch_artist(lib, id, Some(&cancel)) };
            let label = match r {
                Ok(l) => {
                    consecutive_errors = 0;
                    Some(l)
                }
                Err(Error::Cancelled) => break 'outer,
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= 3 {
                        return Err(e);
                    }
                    None
                }
            };
            let mut st = lock().info_status()?;
            st.running = true;
            st.current = label;
            progress(st);
        }
    }
    let st = lock().info_status()?;
    progress(st.clone());
    Ok(st)
}
