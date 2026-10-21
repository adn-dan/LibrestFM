//! Paroles (LRCLIB, gratuit, sans compte) : synchronisées en priorité (format LRC,
//! une heure par ligne), sinon le texte brut. Cherchées à la demande (page d'un
//! titre, titre en cours d'écoute) et gardées dans la base, y compris « rien
//! trouvé » (nouvel essai après 30 jours).

use crate::normalize::{artist_key, title_key};
use crate::{Library, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const BASE: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!("LibrestFM/", env!("CARGO_PKG_VERSION"));
/// Écart de durée toléré entre le titre écouté et la version trouvée (secondes).
const DURATION_SLACK: f64 = 4.0;
/// Délai avant de chercher à nouveau des paroles introuvables.
const RETRY_AFTER: i64 = 30 * 86_400;

static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::AgentBuilder::new().user_agent(USER_AGENT).timeout(Duration::from_secs(15)).build());

/// Une ligne de paroles synchronisées.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    pub ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Lyrics {
    /// Paroles synchronisées, dans l'ordre (vide : pas de version synchronisée).
    pub lines: Vec<Line>,
    /// Texte brut (aussi présent quand les paroles sont synchronisées, s'il existe).
    pub plain: Option<String>,
    pub instrumental: bool,
}

impl Lyrics {
    pub fn found(&self) -> bool {
        !self.lines.is_empty() || self.plain.is_some() || self.instrumental
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

/// Lignes « [mm:ss.xx] texte » (plusieurs heures possibles sur une ligne) ; le reste est ignoré.
pub fn parse_lrc(lrc: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(end) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
            let tag = &rest[1..end + 1];
            let Some((m, s)) = tag.split_once(':') else { break };
            let (Ok(m), Ok(s)) = (m.trim().parse::<i64>(), s.trim().parse::<f64>()) else { break };
            times.push(m * 60_000 + (s * 1000.0).round() as i64);
            rest = &rest[end + 2..];
        }
        let text = rest.trim();
        for ms in times {
            out.push(Line { ms, text: text.to_string() });
        }
    }
    out.sort_by_key(|l| l.ms);
    out
}

fn key(artist: &str, title: &str) -> String {
    format!("{}\u{1f}{}", artist_key(artist), title_key(title))
}

/// Serveur surchargé (503) ou réseau coupé : deux nouveaux essais espacés.
fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<Option<T>> {
    for attempt in 0.. {
        match AGENT.get(url).call() {
            Ok(r) => return Ok(Some(crate::json_body(r).map_err(|e| crate::Error::Http(e.to_string()))?)),
            Err(ureq::Error::Status(404, _)) => return Ok(None),
            Err(ureq::Error::Status(503 | 429, _) | ureq::Error::Transport(_)) if attempt < 2 => {
                std::thread::sleep(Duration::from_millis(1500 * (attempt + 1)));
            }
            Err(e) => return Err(crate::Error::Http(e.to_string())),
        }
    }
    unreachable!()
}

/// Meilleure version : même artiste et même titre, synchronisée d'abord, puis la plus proche en durée.
fn best(records: Vec<Record>, artist: &str, title: &str, duration_s: Option<f64>) -> Option<Record> {
    let (a, t) = (artist_key(artist), title_key(title));
    records
        .into_iter()
        .filter(|r| artist_key(&r.artist_name) == a && title_key(&r.track_name) == t)
        .filter(|r| match (duration_s, r.duration) {
            (Some(want), Some(got)) => (want - got).abs() <= DURATION_SLACK,
            _ => true,
        })
        .filter(|r| r.instrumental || r.synced_lyrics.is_some() || r.plain_lyrics.is_some())
        .min_by(|x, y| {
            let gap = |r: &Record| match (duration_s, r.duration) {
                (Some(w), Some(g)) => (w - g).abs(),
                _ => 0.0,
            };
            (x.synced_lyrics.is_none(), gap(x)).partial_cmp(&(y.synced_lyrics.is_none(), gap(y))).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Cherche sur LRCLIB : la version exacte (avec album et durée), sinon la recherche.
fn fetch(artist: &str, title: &str, album: Option<&str>, duration_s: Option<f64>) -> Result<Lyrics> {
    let enc = |s: &str| urlencoding::encode(s).into_owned();
    let mut exact = None;
    if let Some(d) = duration_s {
        let mut url = format!("{BASE}/get?artist_name={}&track_name={}&duration={}", enc(artist), enc(title), d.round());
        if let Some(al) = album.filter(|a| !a.is_empty()) {
            url += &format!("&album_name={}", enc(al));
        }
        exact = get_json::<Record>(&url)?;
    }
    // Version exacte synchronisée : pas besoin de chercher mieux.
    let rec = match exact {
        Some(r) if r.synced_lyrics.is_some() || r.instrumental => Some(r),
        other => {
            let url = format!("{BASE}/search?artist_name={}&track_name={}", enc(artist), enc(title));
            let found = get_json::<Vec<Record>>(&url)?.unwrap_or_default();
            best(found, artist, title, duration_s).or(other)
        }
    };
    Ok(rec.map_or_else(Lyrics::default, |r| Lyrics {
        lines: r.synced_lyrics.as_deref().map(parse_lrc).unwrap_or_default(),
        plain: r.plain_lyrics.filter(|p| !p.trim().is_empty()),
        instrumental: r.instrumental,
    }))
}

impl Library {
    fn cached_lyrics(&self, key: &str, now: i64) -> Result<Option<Lyrics>> {
        let row: Option<(Option<String>, Option<String>, bool, i64)> = self
            .conn
            .query_row("SELECT synced, plain, instrumental, fetched_at FROM lyrics WHERE key = ?", [key], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        Ok(row.and_then(|(synced, plain, instrumental, at)| {
            let l = Lyrics { lines: synced.as_deref().map(parse_lrc).unwrap_or_default(), plain, instrumental };
            (l.found() || now - at < RETRY_AFTER).then_some(l)
        }))
    }

    fn store_lyrics(&self, key: &str, l: &Lyrics, now: i64) -> Result<()> {
        let synced = (!l.lines.is_empty())
            .then(|| l.lines.iter().map(|x| format!("[{:02}:{:05.2}] {}", x.ms / 60_000, (x.ms % 60_000) as f64 / 1000.0, x.text)).collect::<Vec<_>>().join("\n"));
        self.conn.execute(
            "INSERT OR REPLACE INTO lyrics (key, synced, plain, instrumental, fetched_at) VALUES (?, ?, ?, ?, ?)",
            params![key, synced, l.plain, l.instrumental, now],
        )?;
        Ok(())
    }
}

/// Paroles d'un titre : base d'abord, sinon LRCLIB (verrou relâché pendant la requête).
pub fn get(lib: &Mutex<Library>, artist: &str, title: &str, album: Option<&str>, duration_ms: Option<i64>) -> Result<Lyrics> {
    get_any(lib, &[artist], title, album, duration_ms)
}

/// Comme `get`, en essayant plusieurs artistes dans l'ordre (artiste corrigé, artiste de
/// l'album, artiste annoncé par le lecteur) : un titre joué sous le nom d'un invité
/// (« Zoé » pour « Nova feat. Zoé ») trouve ses paroles sous l'artiste principal.
pub fn get_any(lib: &Mutex<Library>, artists: &[&str], title: &str, album: Option<&str>, duration_ms: Option<i64>) -> Result<Lyrics> {
    let now = chrono::Utc::now().timestamp();
    let duration_s = duration_ms.filter(|d| *d > 0).map(|d| d as f64 / 1000.0);
    let mut seen = Vec::new();
    let mut last = Lyrics::default();
    for artist in artists.iter().map(|a| a.trim()).filter(|a| !a.is_empty()) {
        let k = key(artist, title);
        if seen.contains(&k) {
            continue;
        }
        seen.push(k.clone());
        let cached = lib.lock().unwrap_or_else(|e| e.into_inner()).cached_lyrics(&k, now)?;
        let l = match cached {
            Some(l) => l,
            None => {
                let l = fetch(artist, title, album, duration_s)?;
                lib.lock().unwrap_or_else(|e| e.into_inner()).store_lyrics(&k, &l, now)?;
                l
            }
        };
        if l.found() {
            return Ok(l);
        }
        last = l;
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines_are_parsed_and_sorted() {
        let l = parse_lrc("[ar: Someone]\n[00:12.50] Hello\n[01:02.00][00:05.10] Again\n[00:20.00]\nplain line");
        assert_eq!(
            l,
            vec![
                Line { ms: 5_100, text: "Again".into() },
                Line { ms: 12_500, text: "Hello".into() },
                Line { ms: 20_000, text: "".into() },
                Line { ms: 62_000, text: "Again".into() },
            ]
        );
    }

    #[test]
    fn synced_version_preferred_then_closest_duration() {
        let rec = |name: &str, d: f64, synced: bool| Record {
            track_name: name.into(),
            artist_name: "Nova".into(),
            duration: Some(d),
            instrumental: false,
            plain_lyrics: Some("x".into()),
            synced_lyrics: synced.then(|| "[00:01.00] x".into()),
        };
        let list = vec![rec("Le retour [Explicit]", 268.2, false), rec("Le Retour", 290.0, true), rec("Le retour", 268.3, true), rec("Le retour", 268.0, false)];
        let b = best(list, "Nova", "Le retour", Some(268.0)).unwrap();
        assert_eq!((b.duration, b.synced_lyrics.is_some()), (Some(268.3), true));
    }

    #[test]
    fn cache_keeps_found_and_retries_missing_later() {
        let lib = Library::open_in_memory().unwrap();
        let l = Lyrics { lines: vec![Line { ms: 1000, text: "a".into() }], plain: Some("a".into()), instrumental: false };
        lib.store_lyrics("k", &l, 0).unwrap();
        assert_eq!(lib.cached_lyrics("k", 10 * RETRY_AFTER).unwrap().unwrap().lines, l.lines);
        lib.store_lyrics("none", &Lyrics::default(), 0).unwrap();
        assert!(lib.cached_lyrics("none", 10).unwrap().is_some());
        assert!(lib.cached_lyrics("none", RETRY_AFTER + 1).unwrap().is_none());
    }
}
