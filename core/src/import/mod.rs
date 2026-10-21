//! Import d'un fichier d'export et fusion avec la bibliothèque existante.
//!
//! Règles de fusion, pensées pour résister aux imports désordonnés :
//! - un fichier déjà importé (même contenu, SHA-256) est ignoré ;
//! - une écoute du même titre à ±90 s (±120 s si une des sources n'a que la
//!   minute) d'une écoute existante est la même écoute : on ajoute seulement la
//!   source. Exception : deux lignes d'un même fichier à des heures différentes
//!   sont deux écoutes (un morceau court réécouté) ;
//! - retirer une source supprime les écoutes qu'elle était seule à contenir.

pub mod formats;

use crate::normalize::{album_key, artist_key, title_key};
use crate::{Library, Result};
use formats::Format;
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct RawScrobble {
    pub ts: i64,
    pub precise: bool,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub track_mbid: Option<String>,
    pub artist_mbid: Option<String>,
    pub album_mbid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reject {
    MissingArtist,
    MissingTitle,
    BadDate,
    DateOutOfRange,
    TooShort,
    NotMusic,
    /// Titre en « .mp4 » : erreur d'enregistrement.
    RecordingError,
    Malformed,
}

/// Avant 2000, c'est presque toujours une date à zéro (1970) d'un scrobbler buggé.
const MIN_TS: i64 = 946_684_800;
/// Fenêtre de recherche autour d'une écoute (la règle fine est `same_play`).
const TOLERANCE_MINUTE: i64 = 120;

/// Deux heures d'une même écoute vue par deux sources ? Précises : ±90 s. Une heure à la
/// minute (tronquée : l'écoute a commencé entre `m` et `m + 59`) contre une précise : la
/// précise entre 30 s avant et 89 s après `m` (une fenêtre symétrique de ±120 s prenait
/// la réécoute voisine d'un titre court). Deux à la minute : ±60 s.
pub(crate) fn same_play(a: i64, a_precise: bool, b: i64, b_precise: bool) -> bool {
    match (a_precise, b_precise) {
        (true, true) => (a - b).abs() <= 90,
        (false, true) => (-30..=89).contains(&(b - a)),
        (true, false) => (-30..=89).contains(&(a - b)),
        (false, false) => (a - b).abs() <= 60,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Imported,
    AlreadyImported,
    Unrecognized,
}

#[derive(Debug, Clone, Serialize)]
pub struct RejectCount {
    /// Motif, traduit par l'interface.
    pub reason: Reject,
    pub count: usize,
    /// Quelques numéros de ligne pour aller voir dans le fichier.
    pub sample_lines: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overlap {
    pub source_id: i64,
    pub file_name: String,
    pub shared: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub file_name: String,
    pub status: ImportStatus,
    pub source_id: Option<i64>,
    pub format: Option<String>,
    pub rows_read: usize,
    pub added: usize,
    pub merged: usize,
    pub rejected: usize,
    pub rejects: Vec<RejectCount>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    /// Autres sources qui contenaient déjà une partie de ces écoutes.
    pub overlaps: Vec<Overlap>,
}

impl ImportReport {
    pub(crate) fn empty(file_name: &str, status: ImportStatus) -> Self {
        Self {
            file_name: file_name.into(), status, source_id: None, format: None, rows_read: 0,
            added: 0, merged: 0, rejected: 0, rejects: vec![], first_ts: None, last_ts: None, overlaps: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub id: i64,
    pub file_name: String,
    pub format: String,
    pub imported_at: i64,
    pub rows_read: i64,
    pub rows_added: i64,
    pub rows_merged: i64,
    pub rows_rejected: i64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    /// Écoutes présentes uniquement dans cette source.
    pub exclusive: i64,
}

/// Texte d'un fichier quel que soit son encodage (BOM UTF-8/16, sinon UTF-8, sinon Windows-1252).
pub fn decode(bytes: &[u8]) -> String {
    if let Some((enc, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode_without_bom_handling(&bytes[bom_len..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

/// (année*100 + mois, année*10000 + mois*100 + jour, jour de la semaine 0 = lundi, heure), en heure locale.
pub fn local_fields(ts: i64) -> (i64, i64, i64, i64) {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    match Local.timestamp_opt(ts, 0).earliest() {
        Some(d) => {
            let ym = d.year() as i64 * 100 + d.month() as i64;
            (ym, ym * 100 + d.day() as i64, d.weekday().num_days_from_monday() as i64, d.hour() as i64)
        }
        None => (197001, 19700101, 3, 0),
    }
}

/// Empreinte du fuseau horaire de la machine (décalages en hiver et en été sur plusieurs
/// années, changements d'heure compris) : mois, jour et heure des écoutes en dépendent.
pub fn timezone_print() -> String {
    use chrono::{Local, Offset, TimeZone};
    [2005, 2015, 2025]
        .iter()
        .flat_map(|y| [(*y, 1), (*y, 7)])
        .filter_map(|(y, m)| Local.with_ymd_and_hms(y, m, 15, 12, 0, 0).earliest())
        .map(|d| d.offset().fix().local_minus_utc().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

impl Library {
    /// Mois, jour, jour de la semaine et heure de chaque écoute recalculés dans le fuseau
    /// actuel (machine changée de fuseau depuis l'import) ; renvoie le nombre d'écoutes.
    pub fn refresh_local_fields(&self) -> Result<usize> {
        let block = self.atomic()?;
        let rows: Vec<(i64, i64)> = self.conn.prepare("SELECT id, ts FROM scrobble")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        {
            let mut up = self.conn.prepare("UPDATE scrobble SET ym = ?, ymd = ?, dow = ?, hour = ? WHERE id = ?")?;
            for (id, ts) in &rows {
                let (ym, ymd, dow, hour) = local_fields(*ts);
                up.execute(params![ym, ymd, dow, hour, id])?;
            }
        }
        block.commit()?;
        Ok(rows.len())
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Library {
    pub fn import_file(&mut self, path: &std::path::Path) -> Result<ImportReport> {
        let bytes = std::fs::read(path)?;
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.import_bytes(&name, &bytes)
    }

    pub fn import_bytes(&mut self, file_name: &str, bytes: &[u8]) -> Result<ImportReport> {
        let sha = format!("{:x}", Sha256::digest(bytes));
        let known: Option<i64> =
            self.conn.query_row("SELECT id FROM source WHERE sha256 = ?", [&sha], |r| r.get(0)).optional()?;
        if let Some(id) = known {
            let mut rep = ImportReport::empty(file_name, ImportStatus::AlreadyImported);
            rep.source_id = Some(id);
            return Ok(rep);
        }
        let text = decode(bytes);
        let Some((format, rows)) = formats::parse(&text) else {
            return Ok(ImportReport::empty(file_name, ImportStatus::Unrecognized));
        };
        let tx = self.conn.transaction()?;
        let rep = ingest(&tx, file_name, &sha, format, rows)?;
        tx.commit()?;
        Ok(rep)
    }

    pub fn sources(&self) -> Result<Vec<SourceInfo>> {
        let mut st = self.conn.prepare(
            "SELECT s.id, s.file_name, s.format, s.imported_at, s.rows_read, s.rows_added, s.rows_merged,
                    s.rows_rejected, s.first_ts, s.last_ts,
                    (SELECT COUNT(*) FROM scrobble_source a WHERE a.source_id = s.id AND NOT EXISTS
                       (SELECT 1 FROM scrobble_source b WHERE b.scrobble_id = a.scrobble_id AND b.source_id <> s.id))
             FROM source s ORDER BY s.imported_at, s.id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(SourceInfo {
                id: r.get(0)?, file_name: r.get(1)?, format: r.get(2)?, imported_at: r.get(3)?,
                rows_read: r.get(4)?, rows_added: r.get(5)?, rows_merged: r.get(6)?, rows_rejected: r.get(7)?,
                first_ts: r.get(8)?, last_ts: r.get(9)?, exclusive: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Supprime les écoutes en « .mp4 » déjà en base (importées avant cette règle).
    pub fn purge_recording_errors(&self) -> Result<usize> {
        let ids: Vec<i64> = {
            let mut st = self.conn.prepare("SELECT id, title FROM track")?;
            st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .filter_map(|r| r.ok())
                .filter(|(_, t)| crate::normalize::is_recording_error(t))
                .map(|(id, _)| id)
                .collect()
        };
        let mut n = 0;
        for id in ids {
            n += self.conn.execute("DELETE FROM scrobble WHERE raw_track_id = ?1 OR (raw_track_id IS NULL AND track_id = ?1)", [id])?;
        }
        Ok(n)
    }

    /// Retire les doubles envois : dans une même source, le même titre relancé
    /// avant la moitié de sa durée (scrobbler qui envoie deux fois, ou deux
    /// appareils en même temps). La première écoute reste. Titre de durée
    /// inconnue : moins de 30 s d'écart entre deux heures précises. Une relecture de la source les fait revenir ; ils
    /// repartent au nettoyage suivant. Chaque retrait est gardé (double_send) :
    /// listé dans le nettoyage et rétablissable, et un envoi rétabli n'est plus retiré.
    /// Une écoute qu'une autre source confirme sans avoir la première reste.
    pub fn drop_double_sends(&self) -> Result<usize> {
        let mut n = 0;
        let now = chrono::Utc::now().timestamp();
        // Trois envois rapprochés : le 3e est comparé au 1er une fois le 2e retiré.
        loop {
            let pairs: Vec<(i64, i64)> = {
                let mut st = self.conn.prepare(
                    "SELECT DISTINCT c.id, c.prev_id FROM (
                       SELECT s.id, s.ts, s.track_id, t.duration_ms, s.precise,
                              LAG(s.id) OVER w AS prev_id, LAG(s.ts) OVER w AS prev_ts, LAG(s.track_id) OVER w AS prev_track,
                              LAG(s.precise) OVER w AS prev_precise
                       FROM scrobble s JOIN scrobble_source x ON x.scrobble_id = s.id JOIN track t ON t.id = s.track_id
                       WINDOW w AS (PARTITION BY x.source_id ORDER BY s.ts, s.id)) AS c
                     WHERE c.track_id = c.prev_track
                       AND ((c.duration_ms > 0 AND (c.ts - c.prev_ts) * 2000 < c.duration_ms)
                            -- Durée inconnue : moins de 30 s entre deux heures précises reste un double
                            -- (un scrobble exige au moins 30 s d'écoute).
                            OR (c.precise AND c.prev_precise AND c.ts - c.prev_ts < 30))
                       AND NOT EXISTS (SELECT 1 FROM scrobble_source y WHERE y.scrobble_id = c.id AND NOT EXISTS
                             (SELECT 1 FROM scrobble_source z WHERE z.scrobble_id = c.prev_id AND z.source_id = y.source_id))
                       AND NOT EXISTS (SELECT 1 FROM scrobble r JOIN double_send d ON d.raw_track_id = r.raw_track_id AND d.ts = r.ts
                                       WHERE r.id = c.id AND d.restored = 1)",
                )?;
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
            };
            if pairs.is_empty() {
                return Ok(n);
            }
            // Seulement ceux qui suivent une écoute gardée.
            let doubles: HashSet<i64> = pairs.iter().map(|p| p.0).collect();
            for (id, prev) in pairs {
                if !doubles.contains(&prev) {
                    // Écoutes réunies dans celle-ci (autre graphie, autre source) : elles partent
                    // avec elle (sinon un recalcul les ferait revenir) et reviennent avec elle.
                    self.conn.execute(
                        "INSERT OR IGNORE INTO double_send (ts, precise, raw_track_id, raw_album_id, sources, dropped_at)
                         SELECT ts, precise, raw_track_id, raw_album_id,
                                (SELECT group_concat(x, ',') FROM (
                                   SELECT source_id || ':' || COALESCE(ts, '') AS x FROM scrobble_source WHERE scrobble_id = ?1
                                   UNION ALL SELECT sources FROM scrobble_fold WHERE keep_id = ?1)), ?2
                         FROM scrobble WHERE id = ?1",
                        params![id, now],
                    )?;
                    self.conn.execute("DELETE FROM scrobble_fold WHERE keep_id = ?", [id])?;
                    n += self.conn.execute("DELETE FROM scrobble WHERE id = ?", [id])?;
                }
            }
        }
    }

    /// Doubles envois retirés (non rétablis) : (artiste, titre, heure), du plus récent au plus ancien.
    pub fn double_sends(&self) -> Result<Vec<(String, String, i64)>> {
        let mut st = self.conn.prepare(
            "SELECT a.name, t.title, d.ts FROM double_send d JOIN track t ON t.id = d.raw_track_id JOIN artist a ON a.id = t.artist_id
             WHERE d.restored = 0 AND EXISTS (SELECT 1 FROM source x WHERE ',' || d.sources LIKE '%,' || x.id || ':%')
             ORDER BY d.ts DESC",
        )?;
        let r = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(r)
    }

    /// Rétablit tous les doubles envois retirés (ils ne seront plus retirés) ; renvoie leur nombre.
    pub fn restore_double_sends(&self) -> Result<usize> {
        self.restore_doubles(None)
    }

    /// Après l'annulation d'une fusion : un envoi retiré qui n'est plus le double d'une écoute
    /// gardée (même titre corrigé, même source, moins de la moitié de sa durée avant) revient.
    pub fn revalidate_double_sends(&self) -> Result<usize> {
        let resolver = crate::aliases::Resolver::load(&self.conn)?;
        let rows: Vec<(i64, i64, i64, String)> = self
            .conn
            .prepare("SELECT id, ts, raw_track_id, sources FROM double_send WHERE restored = 0")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut stale = HashSet::new();
        for (id, ts, raw, sources) in rows {
            let track = resolver.get(crate::stats::Kind::Track, raw);
            let duration: Option<i64> = self.conn.query_row("SELECT duration_ms FROM track WHERE id = ?", [track], |r| r.get(0)).optional()?.flatten();
            let srcs: Vec<i64> = sources.split(',').filter_map(|x| x.split(':').next()?.parse().ok()).collect();
            let still = match duration.filter(|d| *d > 0) {
                Some(d) => srcs.iter().any(|s| {
                    self.conn
                        .query_row(
                            "SELECT 1 FROM scrobble s JOIN scrobble_source x ON x.scrobble_id = s.id
                             WHERE s.track_id = ?1 AND x.source_id = ?2 AND s.ts < ?3 AND (?3 - s.ts) * 2000 < ?4",
                            params![track, s, ts, d],
                            |_| Ok(()),
                        )
                        .is_ok()
                }),
                None => false,
            };
            if !still {
                stale.insert(id);
            }
        }
        if stale.is_empty() { Ok(0) } else { self.restore_doubles(Some(&stale)) }
    }

    fn restore_doubles(&self, only: Option<&HashSet<i64>>) -> Result<usize> {
        let rows: Vec<(i64, i64, bool, i64, Option<i64>, String)> = self
            .conn
            .prepare("SELECT id, ts, precise, raw_track_id, raw_album_id, sources FROM double_send WHERE restored = 0")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .filter(|r| r.as_ref().map_or(true, |r| only.is_none_or(|o| o.contains(&r.0))))
            .collect::<rusqlite::Result<_>>()?;
        let tx = self.atomic()?;
        let mut n = 0;
        for (id, ts, precise, track, album, sources) in rows {
            tx.execute("UPDATE double_send SET restored = 1 WHERE id = ?", [id])?;
            // Sources retirées entre-temps : l'écoute ne revient pas.
            let alive: Vec<(i64, Option<i64>)> = sources
                .split(',')
                .filter_map(|it| {
                    let mut p = it.split(':');
                    Some((p.next()?.parse().ok()?, p.next().and_then(|x| x.parse().ok())))
                })
                .filter(|(s, _)| tx.query_row("SELECT 1 FROM source WHERE id = ?", [s], |_| Ok(())).is_ok())
                .collect();
            if alive.is_empty() {
                continue;
            }
            let (ym, ymd, dow, hour) = local_fields(ts);
            tx.execute(
                "INSERT INTO scrobble (ts, track_id, artist_id, album_id, raw_track_id, raw_album_id, precise, ym, ymd, dow, hour)
                 SELECT ?1, ?2, artist_id, ?3, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM track WHERE id = ?2",
                params![ts, track, album, precise, ym, ymd, dow, hour],
            )?;
            let sid = tx.last_insert_rowid();
            for (s, t) in alive {
                tx.execute("INSERT OR IGNORE INTO scrobble_source (scrobble_id, source_id, ts) VALUES (?, ?, ?)", params![sid, s, t])?;
            }
            n += 1;
        }
        tx.commit()?;
        if n > 0 {
            self.rebuild_canonical()?;
        }
        Ok(n)
    }

    /// Retire une source ; renvoie le nombre d'écoutes supprimées.
    pub fn remove_source(&mut self, source_id: i64) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let n = tx.execute(
            "DELETE FROM scrobble WHERE id IN (
               SELECT a.scrobble_id FROM scrobble_source a WHERE a.source_id = ?1 AND NOT EXISTS
                 (SELECT 1 FROM scrobble_source b WHERE b.scrobble_id = a.scrobble_id AND b.source_id <> ?1))",
            [source_id],
        )?;
        tx.execute("DELETE FROM source WHERE id = ?", [source_id])?;
        // Doubles envois qui ne venaient que de cette source : plus rien à rétablir.
        tx.execute(
            "DELETE FROM double_send WHERE NOT EXISTS (SELECT 1 FROM source x WHERE ',' || double_send.sources LIKE '%,' || x.id || ':%')",
            [],
        )?;
        tx.commit()?;
        Ok(n)
    }
}

struct Ids<'a> {
    tx: &'a rusqlite::Connection,
    artists: HashMap<String, i64>,
    albums: HashMap<(i64, String), i64>,
    tracks: HashMap<(i64, String), i64>,
}

/// Nombre d'octets hors ASCII : « Björk » l'emporte sur « Bjork ».
fn diacritics(s: &str) -> i64 {
    (s.len() - s.chars().count()) as i64
}

impl Ids<'_> {
    /// Nom affiché : la graphie la plus complète vue jusqu'ici. Ça compte aussi
    /// pour l'enrichissement, les recherches distantes étant sensibles aux accents.
    fn prefer_spelling(&self, table: &str, id: i64, name: &str) -> rusqlite::Result<()> {
        let score = diacritics(name);
        if score > 0 {
            let col = if table == "artist" { "name" } else { "title" };
            self.tx.execute(
                &format!("UPDATE {table} SET {col} = ?1 WHERE id = ?2 AND length(CAST({col} AS BLOB)) - length({col}) < ?3"),
                params![name, id, score],
            )?;
        }
        Ok(())
    }

    fn artist(&mut self, name: &str, mbid: Option<&str>) -> rusqlite::Result<i64> {
        let key = artist_key(name);
        if let Some(&id) = self.artists.get(&key) {
            self.prefer_spelling("artist", id, name)?;
            return Ok(id);
        }
        self.tx.execute("INSERT OR IGNORE INTO artist (name, key, mbid) VALUES (?, ?, ?)", params![name, key, mbid])?;
        let id = self.tx.query_row("SELECT id FROM artist WHERE key = ?", [&key], |r| r.get(0))?;
        self.prefer_spelling("artist", id, name)?;
        if mbid.is_some() {
            self.tx.execute("UPDATE artist SET mbid = ? WHERE id = ? AND mbid IS NULL", params![mbid, id])?;
        }
        self.artists.insert(key, id);
        Ok(id)
    }

    fn titled(&mut self, table: &str, artist_id: i64, title: &str, key: String, mbid: Option<&str>) -> rusqlite::Result<i64> {
        let cache = if table == "album" { &mut self.albums } else { &mut self.tracks };
        if let Some(&id) = cache.get(&(artist_id, key.clone())) {
            self.prefer_spelling(table, id, title)?;
            return Ok(id);
        }
        self.tx.execute(
            &format!("INSERT OR IGNORE INTO {table} (artist_id, title, key, mbid) VALUES (?, ?, ?, ?)"),
            params![artist_id, title, key, mbid],
        )?;
        let id = self.tx.query_row(
            &format!("SELECT id FROM {table} WHERE artist_id = ? AND key = ?"),
            params![artist_id, key],
            |r| r.get(0),
        )?;
        if mbid.is_some() {
            self.tx.execute(&format!("UPDATE {table} SET mbid = ? WHERE id = ? AND mbid IS NULL"), params![mbid, id])?;
        }
        self.prefer_spelling(table, id, title)?;
        let cache = if table == "album" { &mut self.albums } else { &mut self.tracks };
        cache.insert((artist_id, key), id);
        Ok(id)
    }
}

fn ingest(tx: &Transaction, file_name: &str, sha: &str, format: Format, rows: Vec<formats::Row>) -> Result<ImportReport> {
    let source_id = new_source(tx, file_name, sha, format.code())?;
    ingest_into(tx, source_id, file_name, format.code(), rows)
}

pub(crate) fn new_source(tx: &rusqlite::Connection, name: &str, sha: &str, format: &str) -> Result<i64> {
    tx.execute(
        "INSERT INTO source (file_name, sha256, format, imported_at, rows_read, rows_added, rows_merged, rows_rejected)
         VALUES (?, ?, ?, ?, 0, 0, 0, 0)",
        params![name, sha, format, now()],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Ajoute des écoutes à une source existante (fichier, ou compte synchronisé
/// page après page) ; ses compteurs s'additionnent.
pub(crate) fn ingest_into(tx: &Transaction, source_id: i64, file_name: &str, format: &str, rows: Vec<formats::Row>) -> Result<ImportReport> {
    let mut ids = Ids { tx, artists: HashMap::new(), albums: HashMap::new(), tracks: HashMap::new() };
    let mut rep = ImportReport::empty(file_name, ImportStatus::Imported);
    rep.source_id = Some(source_id);
    rep.format = Some(format.into());
    rep.rows_read = rows.len();
    let mut rejects: BTreeMap<Reject, (usize, Vec<usize>)> = BTreeMap::new();
    let max_ts = now() + 86_400;

    let mut find = tx.prepare(
        "SELECT s.id, s.ts, s.precise, s.album_id,
                EXISTS (SELECT 1 FROM scrobble_source x WHERE x.scrobble_id = s.id AND x.source_id = ?1),
                (SELECT x.ts FROM scrobble_source x WHERE x.scrobble_id = s.id AND x.source_id = ?1)
         FROM scrobble s WHERE s.track_id = ?2 AND s.ts BETWEEN ?3 AND ?4 ORDER BY ABS(s.ts - ?5)",
    )?;
    let mut insert = tx.prepare(
        "INSERT INTO scrobble (ts, track_id, artist_id, album_id, raw_track_id, raw_album_id, precise, ym, ymd, dow, hour)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;
    // Corrections déjà décidées (alias) : appliquées dès l'import.
    let mut resolver = crate::aliases::Resolver::load(tx)?;
    let mut artist_of = tx.prepare("SELECT artist_id FROM track WHERE id = ?")?;
    // Doubles envois déjà retirés : (titre brut, heure, précise), pour reconnaître la même
    // écoute relue (même source) ou venue d'un autre export, dans la tolérance habituelle.
    let mut dropped = tx.prepare("SELECT raw_track_id, ts, precise FROM double_send WHERE restored = 0 AND ts BETWEEN ? AND ?")?;
    // Le lien garde l'heure telle que la source la donne (l'écoute peut avoir été
    // recalée par une source plus précise). Lien ancien sans heure : complété.
    let mut link = tx.prepare(
        "INSERT INTO scrobble_source (scrobble_id, source_id, ts) VALUES (?1, ?2, ?3)
         ON CONFLICT (scrobble_id, source_id) DO UPDATE SET ts = COALESCE(scrobble_source.ts, excluded.ts)",
    )?;
    // Liens anciens, sans heure : une écoute déjà attribuée à une ligne de cette
    // lecture ne peut plus être reprise par une autre ligne de la même source.
    let mut claimed: HashSet<i64> = HashSet::new();

    for (line, row) in rows {
        let raw = match row.and_then(|r| if r.ts < MIN_TS || r.ts > max_ts { Err(Reject::DateOutOfRange) } else { Ok(r) }) {
            Ok(r) => r,
            Err(reason) => {
                let e = rejects.entry(reason).or_default();
                e.0 += 1;
                if e.1.len() < 5 {
                    e.1.push(line);
                }
                continue;
            }
        };
        let artist_id = ids.artist(&raw.artist, raw.artist_mbid.as_deref())?;
        let track_id = ids.titled("track", artist_id, &raw.title, title_key(&raw.title), raw.track_mbid.as_deref())?;
        let album_id = match &raw.album {
            Some(a) => Some(ids.titled("album", artist_id, a, album_key(a), raw.album_mbid.as_deref())?),
            None => None,
        };
        // Titre ou album jamais vu d'un artiste fusionné, arrivé sous l'ancien nom : rattaché
        // à la fiche de même clé chez l'artiste cible (créée au besoin), par la même fusion
        // (annulée avec elle). Sinon il resterait à part, compté en double avec la même
        // écoute venue d'une autre source sous le bon nom.
        let target_artist = resolver.get(crate::stats::Kind::Artist, artist_id);
        if target_artist != artist_id {
            let op: Option<i64> =
                tx.query_row("SELECT op_id FROM alias WHERE kind = 'artist' AND from_id = ?", [artist_id], |r| r.get(0)).optional()?;
            let mut follow = |table: &str, kind: crate::stats::Kind, id: i64, title: &str, key: String| -> rusqlite::Result<()> {
                if resolver.get(kind, id) != id {
                    return Ok(());
                }
                let Some(op) = op else { return Ok(()) };
                tx.execute(&format!("INSERT OR IGNORE INTO {table} (artist_id, title, key) VALUES (?, ?, ?)"), params![target_artist, title, key])?;
                let to: i64 = tx.query_row(&format!("SELECT id FROM {table} WHERE artist_id = ? AND key = ?"), params![target_artist, key], |r| r.get(0))?;
                if to != id {
                    tx.execute("INSERT OR REPLACE INTO alias (kind, from_id, to_id, op_id) VALUES (?, ?, ?, ?)", params![table, id, to, op])?;
                    resolver.insert(kind, id, to);
                }
                Ok(())
            };
            follow("track", crate::stats::Kind::Track, track_id, &raw.title, title_key(&raw.title))?;
            if let (Some(al), Some(a)) = (album_id, &raw.album) {
                follow("album", crate::stats::Kind::Album, al, a, album_key(a))?;
            }
        }
        // Identifiants corrigés ; les bruts sont gardés à côté.
        let c_track = resolver.get(crate::stats::Kind::Track, track_id);
        let c_artist = if c_track == track_id { artist_id } else { artist_of.query_row([c_track], |r| r.get(0))? };
        let c_artist = resolver.get(crate::stats::Kind::Artist, c_artist);
        let c_album = album_id.map(|a| resolver.get(crate::stats::Kind::Album, a));

        let candidates: Vec<(i64, i64, bool, Option<i64>, bool, Option<i64>)> = find
            // Plus proche d'abord ; une heure à la minute est comparée au milieu de sa minute.
            .query_map(params![source_id, c_track, raw.ts - TOLERANCE_MINUTE, raw.ts + TOLERANCE_MINUTE, raw.ts + if raw.precise { 0 } else { 30 }], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        // Une écoute déjà liée à cette source n'est la même que si la source donnait
        // la même heure (ligne relue : fichier relu, page de compte recouverte), même
        // si l'écoute a été recalée par une autre source. Une autre heure de la même
        // source est une autre écoute (titre court réécouté). Lien ancien sans heure :
        // dans la tolérance, sauf si une autre ligne de cette lecture l'a déjà pris.
        let same = candidates.into_iter().find(|&(id, ts, precise, _, from_this_source, link_ts)| {
            let close = same_play(ts, precise, raw.ts, raw.precise);
            match (from_this_source, link_ts) {
                (true, Some(t)) => t == raw.ts,
                (true, None) => close && (ts == raw.ts || !claimed.contains(&id)),
                (false, _) => close,
            }
        });

        match same {
            Some((id, _, precise, existing_album, _, _)) => {
                claimed.insert(id);
                link.execute(params![id, source_id, raw.ts])?;
                // On garde la meilleure information disponible.
                if !precise && raw.precise {
                    let (ym, ymd, dow, hour) = local_fields(raw.ts);
                    tx.execute(
                        "UPDATE scrobble SET ts = ?, precise = 1, ym = ?, ymd = ?, dow = ?, hour = ? WHERE id = ?",
                        params![raw.ts, ym, ymd, dow, hour, id],
                    )?;
                }
                if existing_album.is_none() && album_id.is_some() {
                    tx.execute("UPDATE scrobble SET album_id = ?, raw_album_id = ? WHERE id = ?", params![c_album, album_id, id])?;
                }
                rep.merged += 1;
            }
            // Double envoi déjà retiré (relecture de la source) : connu, pas une nouvelle écoute.
            None if dropped
                .query_map(params![raw.ts - TOLERANCE_MINUTE, raw.ts + TOLERANCE_MINUTE], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?)))?
                .filter_map(|r| r.ok())
                .any(|(t, ts, precise)| resolver.get(crate::stats::Kind::Track, t) == c_track && same_play(ts, precise, raw.ts, raw.precise)) =>
            {
                rep.merged += 1;
            }
            None => {
                let (ym, ymd, dow, hour) = local_fields(raw.ts);
                insert.execute(params![raw.ts, c_track, c_artist, c_album, track_id, album_id, raw.precise, ym, ymd, dow, hour])?;
                let id = tx.last_insert_rowid();
                claimed.insert(id);
                link.execute(params![id, source_id, raw.ts])?;
                rep.added += 1;
            }
        }
        rep.first_ts = Some(rep.first_ts.map_or(raw.ts, |t| t.min(raw.ts)));
        rep.last_ts = Some(rep.last_ts.map_or(raw.ts, |t| t.max(raw.ts)));
    }

    rep.rejects = rejects
        .into_iter()
        .map(|(reason, (count, sample_lines))| RejectCount { reason, count, sample_lines })
        .collect();
    rep.rejected = rep.rejects.iter().map(|r| r.count).sum();

    let mut ov = tx.prepare(
        "SELECT b.source_id, s.file_name, COUNT(*) FROM scrobble_source a
         JOIN scrobble_source b ON b.scrobble_id = a.scrobble_id AND b.source_id <> a.source_id
         JOIN source s ON s.id = b.source_id
         WHERE a.source_id = ? GROUP BY b.source_id ORDER BY 3 DESC",
    )?;
    rep.overlaps = ov
        .query_map([source_id], |r| Ok(Overlap { source_id: r.get(0)?, file_name: r.get(1)?, shared: r.get(2)? }))?
        .collect::<rusqlite::Result<_>>()?;

    tx.execute(
        "UPDATE source SET rows_read = rows_read + ?, rows_added = rows_added + ?, rows_merged = rows_merged + ?,
           rows_rejected = rows_rejected + ?, first_ts = COALESCE(MIN(first_ts, ?5), first_ts, ?5), last_ts = COALESCE(MAX(last_ts, ?6), last_ts, ?6),
           imported_at = ?8 WHERE id = ?7",
        params![rep.rows_read, rep.added, rep.merged, rep.rejected, rep.first_ts, rep.last_ts, source_id, now()],
    )?;
    Ok(rep)
}
