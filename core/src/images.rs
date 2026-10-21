//! Photos d'artiste et pochettes d'album, via l'API publique de Deezer (sans clé).
//!
//! Les images sont téléchargées une fois puis gardées dans la base : l'interface
//! n'affiche que des données locales. Pour un titre, on prend la pochette de
//! l'album sur lequel il est le plus écouté.

use crate::normalize::{album_key, artist_key, fold};
use crate::stats::Kind;
use crate::{Error, Library, Result};
use base64::Engine;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

pub(crate) const API: &str = "https://api.deezer.com";
/// Deezer accepte environ 50 requêtes par 5 s ; on reste à 8 par seconde.
const MIN_INTERVAL: Duration = Duration::from_millis(125);
const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Empreinte de l'image vide que Deezer renvoie quand il n'a pas de photo.
const EMPTY_HASH: &str = "d41d8cd98f00b204e9800998ecf8427e";

/// Créneau d'envoi partagé par tous les appels (plusieurs fiches peuvent charger en même temps).
static NEXT_SLOT: LazyLock<Mutex<Instant>> = LazyLock::new(|| Mutex::new(Instant::now()));
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::AgentBuilder::new()
        .user_agent(concat!("LibrestFM/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
});

fn wait_slot() {
    let wait = {
        let mut next = NEXT_SLOT.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let slot = (*next).max(now);
        *next = slot + MIN_INTERVAL;
        slot - now
    };
    std::thread::sleep(wait);
}

pub(crate) fn get_json(url: &str) -> Result<Value> {
    // Deezer signale un dépassement de quota dans le corps de la réponse (code 4) : quelques
    // nouveaux essais espacés, pas indéfiniment.
    for _ in 0..4 {
        wait_slot();
        let v: Value = crate::json_body(AGENT.get(url).call().map_err(|e| Error::Http(e.to_string()))?).map_err(|e| Error::Http(e.to_string()))?;
        if v.get("error").and_then(|e| e.get("code")).and_then(Value::as_i64) != Some(4) {
            return Ok(v);
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    Err(Error::Http("Deezer : quota dépassé".into()))
}

fn get_bytes(url: &str) -> Result<(String, Vec<u8>)> {
    wait_slot();
    let resp = AGENT.get(url).call().map_err(|e| Error::Http(e.to_string()))?;
    let mime = resp.content_type().to_owned();
    let mut buf = Vec::new();
    resp.into_reader().take(MAX_BYTES).read_to_end(&mut buf)?;
    Ok((mime, buf))
}

fn usable(url: Option<&str>) -> Option<&str> {
    url.filter(|u| !u.is_empty() && !u.contains("/artist//") && !u.contains("/cover//") && !u.contains(EMPTY_HASH))
}

/// URL de la photo de l'artiste : homonyme exact le plus suivi.
pub fn find_artist(name: &str) -> Result<Option<String>> {
    let v = get_json(&format!("{API}/search/artist?q={}&limit=25", urlencoding::encode(name)))?;
    let key = artist_key(name);
    let best = v
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| artist_key(n) == key))
        .max_by_key(|a| a.get("nb_fan").and_then(Value::as_i64).unwrap_or(0));
    Ok(best.and_then(|a| usable(a.get("picture_medium").and_then(Value::as_str))).map(str::to_owned))
}

/// URL de la pochette : même artiste, titre identique de préférence, sinon
/// identique hors parenthèses (« The Blue Notebooks (15 Years) »).
pub fn find_album(artist: &str, album: &str) -> Result<Option<String>> {
    let cover = |a: Value| usable(a.get("cover_medium").and_then(Value::as_str)).map(str::to_owned);
    Ok(find_album_value(artist, album)?.and_then(cover))
}

/// Identifiant Deezer de l'album (même rapprochement que pour la pochette).
pub fn find_album_id(artist: &str, album: &str) -> Result<Option<i64>> {
    Ok(find_album_value(artist, album)?.and_then(|a| a.get("id")?.as_i64()))
}

/// Fiche complète d'un album Deezer : date d'édition, durée, label, titres et leurs durées.
pub fn deezer_album(id: i64) -> Result<Option<Value>> {
    let v = get_json(&format!("{API}/album/{id}"))?;
    Ok(v.get("error").is_none().then_some(v))
}

fn find_album_value(artist: &str, album: &str) -> Result<Option<Value>> {
    let (akey, exact) = (artist_key(artist), album_key(album));
    let loose = |s: &str| fold(s.split(['(', '[']).next().unwrap_or(s));
    let title = |a: &Value| a.get("title").and_then(Value::as_str).unwrap_or_default().to_owned();
    let artist_of = |a: &Value| a.get("artist").and_then(|x| x.get("name")).and_then(Value::as_str).map(artist_key).unwrap_or_default();
    let results = |q: String| -> Result<Vec<Value>> {
        let v = get_json(&format!("{API}/search/album?q={}&limit=25", urlencoding::encode(&q)))?;
        Ok(v.get("data").and_then(Value::as_array).cloned().unwrap_or_default())
    };

    let strict = results(format!("artist:\"{artist}\" album:\"{album}\""))?;
    let same_artist: Vec<&Value> = strict.iter().filter(|a| artist_of(a) == akey).collect();
    let pick = same_artist
        .iter()
        .find(|a| album_key(&title(a)) == exact)
        .or_else(|| same_artist.iter().find(|a| loose(&title(a)) == loose(album)));
    if let Some(a) = pick {
        return Ok(Some((*a).clone()));
    }
    // Crédit différent chez Deezer (« Bill Evans » pour « Bill Evans Trio ») :
    // recherche sur le seul titre de base (Deezer exige tous les mots : « Trio » ou
    // « (Remastered) » feraient échouer), artiste identique ou inclus dans l'autre,
    // même titre (hors mentions d'édition).
    let base = crate::normalize::album_base(album);
    let free = results(base.clone())?;
    let close = |k: &str| k == akey || (k.len() >= 4 && akey.len() >= 4 && (k.contains(&akey) || akey.contains(k)));
    let same_title = |t: &str| album_key(t) == exact || fold(&crate::normalize::album_base(t)) == fold(&base);
    Ok(free.into_iter().find(|a| close(&artist_of(a)) && same_title(&title(a))))
}

/// Ce qu'il faut chercher pour une entité.
enum Query {
    Artist(String),
    Album { artist: String, title: String },
}

/// Clé de cache : une image par artiste ou par album (un titre réutilise son album).
type Slot = (&'static str, i64);

impl Library {
    /// Artiste ou album dont l'image représente cette entité.
    fn image_slot(&self, kind: Kind, id: i64) -> Result<Option<(Slot, Query)>> {
        let c = &self.conn;
        Ok(match kind {
            Kind::Artist => c
                .query_row("SELECT name FROM artist WHERE id = ?", [id], |r| r.get(0))
                .optional()?
                .map(|name| (("artist", id), Query::Artist(name))),
            Kind::Album => c
                .query_row("SELECT a.name, al.title FROM album al JOIN artist a ON a.id = al.artist_id WHERE al.id = ?", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?
                .map(|(artist, title)| (("album", id), Query::Album { artist, title })),
            Kind::Track => {
                let album: Option<i64> = c
                    .query_row(
                        "SELECT album_id FROM scrobble WHERE track_id = ? AND album_id IS NOT NULL GROUP BY album_id ORDER BY COUNT(*) DESC LIMIT 1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                match album {
                    Some(a) => self.image_slot(Kind::Album, a)?,
                    None => None,
                }
            }
        })
    }

    /// `Some(Some(url))` : image en cache ; `Some(None)` : déjà cherchée, aucune ; `None` : jamais cherchée.
    fn cached_image(&self, slot: Slot) -> Result<Option<Option<String>>> {
        let row: Option<(String, Option<String>, Option<Vec<u8>>)> = self
            .conn
            .query_row("SELECT status, mime, data FROM image WHERE kind = ? AND entity_id = ?", params![slot.0, slot.1], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        Ok(row.map(|(status, mime, data)| match (status.as_str(), mime, data) {
            ("ok", Some(m), Some(d)) => Some(format!("data:{m};base64,{}", base64::engine::general_purpose::STANDARD.encode(d))),
            _ => None,
        }))
    }

    fn store_image(&self, slot: Slot, found: Option<(String, Vec<u8>, String)>) -> Result<()> {
        let (status, mime, data, url) = match found {
            Some((mime, data, url)) => ("ok", Some(mime), Some(data), Some(url)),
            None => ("not_found", None, None, None),
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO image (kind, entity_id, status, mime, data, source_url, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![slot.0, slot.1, status, mime, data, url, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Images déjà en cache pour une liste d'entités (data URL), sans réseau.
    pub fn cached_images(&self, kind: Kind, ids: &[i64]) -> Result<HashMap<i64, String>> {
        let mut out = HashMap::new();
        for &id in ids {
            if let Some((slot, _)) = self.image_slot(kind, id)?
                && let Some(Some(url)) = self.cached_image(slot)?
            {
                out.insert(id, url);
            }
        }
        Ok(out)
    }
}

/// Image d'une entité (data URL), téléchargée si besoin. Le verrou de la base
/// n'est pas tenu pendant les appels réseau.
pub fn image(lib: &Mutex<Library>, kind: Kind, id: i64) -> Result<Option<String>> {
    let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
    let found = lock().image_slot(kind, id)?;
    let Some((slot, query)) = found else { return Ok(None) };
    let cached = lock().cached_image(slot)?;
    if let Some(c) = cached {
        return Ok(c);
    }
    let url = match &query {
        Query::Artist(name) => find_artist(name)?,
        Query::Album { artist, title } => find_album(artist, title)?,
    };
    let fetched = match url {
        Some(u) => {
            let (mime, data) = get_bytes(&u)?;
            mime.starts_with("image/").then_some((mime, data, u))
        }
        None => None,
    };
    let l = lock();
    l.store_image(slot, fetched)?;
    l.cached_image(slot).map(Option::flatten)
}
