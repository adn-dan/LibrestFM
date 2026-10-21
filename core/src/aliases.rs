//! Correspondances (alias) : « cette entité est en réalité celle-là ».
//!
//! Les écoutes gardent leurs identifiants bruts (`raw_track_id`, `raw_album_id`) ;
//! `track_id`, `album_id` et `artist_id` sont les identifiants corrigés, recalculés
//! à partir des alias. Rien n'est jamais effacé : chaque fusion est une opération
//! (`merge_op`) qu'on peut annuler.
//!
//! Détection automatique :
//! - éditions d'un même album (Remaster, Deluxe, Anniversary…) : fusion sûre ;
//! - noms de fichiers (« 03 - Get Lucky.flac ») et artiste collé dans le titre
//!   (« Daft Punk - Get Lucky ») : sûr si le titre corrigé existe déjà ;
//! - même identifiant MusicBrainz : sûr ; « The Beatles » / « Beatles » : proposé ;
//! - titre écouté sous le nom de l'artiste invité (crédits MusicBrainz) : rendu à
//!   l'artiste principal, sûr s'il est déjà dans l'historique.

use crate::normalize::{
    album_base, album_key, artist_key, artist_key_no_article, clean_filename, fold, is_live, is_placeholder_artist, main_artist,
    split_artist_title, title_key, unescape_html, without_nickname,
};
use crate::stats::{Kind, Ranked};
use crate::{Error, Library, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Artist => "artist",
        Kind::Album => "album",
        Kind::Track => "track",
    }
}

/// Résolution des alias, chargée en mémoire.
pub struct Resolver {
    maps: HashMap<&'static str, HashMap<i64, i64>>,
}

impl Resolver {
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut maps: HashMap<&'static str, HashMap<i64, i64>> = HashMap::new();
        let mut st = conn.prepare("SELECT kind, from_id, to_id FROM alias")?;
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
            let (k, from, to) = row?;
            let k = match k.as_str() {
                "artist" => "artist",
                "album" => "album",
                _ => "track",
            };
            maps.entry(k).or_default().insert(from, to);
        }
        Ok(Self { maps })
    }

    /// Alias ajouté en cours de route (import) : pris en compte aussitôt.
    pub fn insert(&mut self, kind: Kind, from: i64, to: i64) {
        self.maps.entry(kind_str(kind)).or_default().insert(from, to);
    }

    /// Entité finale (les alias peuvent s'enchaîner : A → B → C).
    pub fn get(&self, kind: Kind, id: i64) -> i64 {
        let Some(m) = self.maps.get(kind_str(kind)) else { return id };
        let mut cur = id;
        for _ in 0..32 {
            match m.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    }
}

/// Titre de la bibliothèque, après corrections.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnownTrack {
    pub id: i64,
    pub title: String,
    pub artist: String,
}

impl Library {
    /// Titre de la bibliothèque pour un titre joué ailleurs (lecteur du téléphone), avec
    /// les corrections appliquées : « Zoé – Une nuit de plus » donne « Nova –
    /// Une nuit de plus » si ce titre a été rendu à l'artiste principal. Essaie l'artiste
    /// annoncé, puis l'artiste de l'album.
    pub fn find_track(&self, artist: &str, title: &str, album_artist: Option<&str>) -> Result<Option<KnownTrack>> {
        let resolver = Resolver::load(&self.conn)?;
        let key = crate::normalize::title_key(title);
        for a in std::iter::once(artist).chain(album_artist).filter(|a| !a.trim().is_empty()) {
            let raw: Option<i64> = self
                .conn
                .query_row(
                    "SELECT t.id FROM track t JOIN artist a ON a.id = t.artist_id WHERE a.key = ? AND t.key = ?",
                    params![crate::normalize::artist_key(a), key],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(raw) = raw else { continue };
            let id = resolver.get(Kind::Track, raw);
            let (title, artist_id): (String, i64) = self.conn.query_row("SELECT title, artist_id FROM track WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let artist: String = self.conn.query_row("SELECT name FROM artist WHERE id = ?", [resolver.get(Kind::Artist, artist_id)], |r| r.get(0))?;
            return Ok(Some(KnownTrack { id, title, artist }));
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    /// Identifiant stable (pour « Ignorer »).
    pub key: String,
    /// edition, filename, artist_in_title, same_mbid, article, album_variant
    pub rule: String,
    pub kind: Kind,
    pub from_id: i64,
    pub from_name: String,
    pub from_artist: Option<String>,
    /// Cible existante, ou None : elle sera créée avec `to_artist` / `to_name`.
    pub to_id: Option<i64>,
    pub to_name: String,
    pub to_artist: Option<String>,
    /// Écoutes concernées.
    pub plays: i64,
    /// Appliquée automatiquement.
    pub sure: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeOp {
    pub id: i64,
    pub kind: Kind,
    pub rule: String,
    pub auto: bool,
    pub label: String,
    pub created_at: i64,
}

/// Liaison d'« artist-credit » qui annonce un invité : « feat. », « featuring », « with », « vs. »…
fn is_featuring(join: &str) -> bool {
    let j = join.trim().to_lowercase();
    ["feat", "ft.", "featuring", "with", "avec", "vs"].iter().any(|m| j.starts_with(m))
}

/// Nom d'artiste qui en contient plusieurs (« A feat. B »).
fn is_compound(name: &str) -> bool {
    let n = name.to_lowercase();
    [" feat", " ft.", " featuring "].iter().any(|m| n.contains(m))
}

fn parse_kind(s: &str) -> Kind {
    match s {
        "artist" => Kind::Artist,
        "album" => Kind::Album,
        _ => Kind::Track,
    }
}

/// Réunit les écoutes devenues identiques après correction : même titre, même moment
/// (`same_play`), sans source commune. Ce sont les règles de l'import, appliquées après coup.
fn fold_duplicates(conn: &Connection) -> Result<usize> {
    let mut st = conn.prepare(
        "SELECT a.id, a.precise, a.album_id IS NOT NULL, b.id, b.precise, b.album_id IS NOT NULL, a.ts, b.ts
         FROM scrobble a JOIN scrobble b ON b.track_id = a.track_id AND b.ts BETWEEN a.ts - 120 AND a.ts + 120 AND b.id > a.id
         WHERE NOT EXISTS (SELECT 1 FROM scrobble_source x JOIN scrobble_source y ON y.source_id = x.source_id
                           WHERE x.scrobble_id = a.id AND y.scrobble_id = b.id)
         ORDER BY a.ts, a.id",
    )?;
    let pairs: Vec<((i64, bool, bool), (i64, bool, bool))> = st
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), (r.get(3)?, r.get(4)?, r.get(5)?), r.get::<_, i64>(6)?, r.get::<_, i64>(7)?)))?
        .filter_map(|r| r.ok())
        .filter(|(a, b, ta, tb)| crate::import::same_play(*ta, a.1, *tb, b.1))
        .map(|(a, b, _, _)| (a, b))
        .collect();
    let mut gone = std::collections::HashSet::new();
    for (a, b) in pairs {
        if gone.contains(&a.0) || gone.contains(&b.0) {
            continue;
        }
        // Sources revérifiées maintenant : une réunion précédente a pu donner à l'une
        // les sources d'une autre (deux écoutes d'un même fichier restent deux).
        let shared: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM scrobble_source x JOIN scrobble_source y ON y.source_id = x.source_id
                            WHERE x.scrobble_id = ?1 AND y.scrobble_id = ?2)",
            [a.0, b.0],
            |r| r.get(0),
        )?;
        if shared {
            continue;
        }
        // On garde l'écoute la plus précise, puis celle qui a un album.
        let (keep, dup) = if (b.1, b.2) > (a.1, a.2) { (b.0, a.0) } else { (a.0, b.0) };
        // Sources propres de l'écoute absorbée : sans celles qu'elle tenait d'écoutes
        // qu'elle avait elle-même absorbées (rendues à celles-ci à l'annulation).
        let inherited: std::collections::HashSet<i64> = {
            let mut st = conn.prepare("SELECT sources FROM scrobble_fold WHERE keep_id = ?")?;
            let rows: Vec<String> = st.query_map([dup], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            rows.iter().flat_map(|s| s.split(',').filter_map(|x| x.split(':').next()?.parse().ok())).collect()
        };
        // « source:heure » (heure vide pour un lien ancien).
        let own: Vec<String> = {
            let mut st = conn.prepare("SELECT source_id, ts FROM scrobble_source WHERE scrobble_id = ?")?;
            let rows: Vec<(i64, Option<i64>)> = st.query_map([dup], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            rows.into_iter().filter(|s| !inherited.contains(&s.0)).map(|(s, t)| format!("{s}:{}", t.map_or(String::new(), |t| t.to_string()))).collect()
        };
        conn.execute(
            "INSERT INTO scrobble_fold (keep_id, ts, precise, raw_track_id, raw_album_id, sources)
             SELECT ?1, ts, precise, raw_track_id, raw_album_id, ?3 FROM scrobble s WHERE id = ?2",
            params![keep, dup, own.join(",")],
        )?;
        conn.execute("INSERT OR IGNORE INTO scrobble_source (scrobble_id, source_id, ts) SELECT ?1, source_id, ts FROM scrobble_source WHERE scrobble_id = ?2", [keep, dup])?;
        // L'écoute absorbée en avait peut-être absorbé d'autres : leur trace suit l'écoute gardée.
        conn.execute("UPDATE scrobble_fold SET keep_id = ?1 WHERE keep_id = ?2", [keep, dup])?;
        conn.execute("DELETE FROM scrobble_source WHERE scrobble_id = ?", [dup])?;
        conn.execute("DELETE FROM scrobble WHERE id = ?", [dup])?;
        gone.insert(dup);
    }
    Ok(gone.len())
}

/// Rend les écoutes réunies par `fold_duplicates` (identifiants corrigés à recalculer).
fn unfold_duplicates(conn: &Connection) -> Result<()> {
    let mut st = conn.prepare("SELECT keep_id, ts, precise, raw_track_id, raw_album_id, sources FROM scrobble_fold")?;
    let folds: Vec<(i64, i64, bool, i64, Option<i64>, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<rusqlite::Result<_>>()?;
    for (keep, ts, precise, track, album, sources) in folds {
        // Sources retirées entre-temps : leurs écoutes ne reviennent pas.
        let mut alive = Vec::new();
        // « source:heure » ; les traces d'avant l'heure par lien n'ont que la source.
        for item in sources.split(',') {
            let mut parts = item.split(':');
            let Some(s) = parts.next().and_then(|x| x.parse::<i64>().ok()) else { continue };
            let t: Option<i64> = parts.next().and_then(|x| x.parse().ok());
            conn.execute("DELETE FROM scrobble_source WHERE scrobble_id = ? AND source_id = ?", [keep, s])?;
            if conn.query_row("SELECT 1 FROM source WHERE id = ?", [s], |_| Ok(())).optional()?.is_some() {
                alive.push((s, t));
            }
        }
        conn.execute("DELETE FROM scrobble WHERE id = ? AND NOT EXISTS (SELECT 1 FROM scrobble_source WHERE scrobble_id = ?1)", [keep])?;
        if alive.is_empty() {
            continue;
        }
        let (ym, ymd, dow, hour) = crate::import::local_fields(ts);
        conn.execute(
            "INSERT INTO scrobble (ts, track_id, artist_id, album_id, raw_track_id, raw_album_id, precise, ym, ymd, dow, hour)
             SELECT ?1, ?2, artist_id, ?3, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM track WHERE id = ?2",
            params![ts, track, album, precise, ym, ymd, dow, hour],
        )?;
        let id = conn.last_insert_rowid();
        for (s, t) in alive {
            conn.execute("INSERT INTO scrobble_source (scrobble_id, source_id, ts) VALUES (?, ?, ?)", params![id, s, t])?;
        }
    }
    conn.execute("DELETE FROM scrobble_fold", [])?;
    Ok(())
}

impl Library {
    /// Recalcule les identifiants corrigés de toutes les écoutes ; renvoie le nombre d'écoutes modifiées.
    pub fn rebuild_canonical(&self) -> Result<usize> {
        // Les écoutes réunies lors d'un recalcul précédent sont rendues, puis
        // réunies à nouveau si elles le sont toujours (une fusion a pu être annulée).
        // Une seule transaction : interrompu, le recalcul ne laisse rien à moitié fait.
        let tx = self.atomic()?;
        unfold_duplicates(&tx)?;
        let r = Resolver::load(&self.conn)?;
        let mut st = self.conn.prepare("SELECT id, artist_id FROM track")?;
        let track_artist: HashMap<i64, i64> = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut st = self.conn.prepare("SELECT id, raw_track_id, raw_album_id, track_id, album_id, artist_id FROM scrobble")?;
        let mut changes = Vec::new();
        for row in st.query_map([], |x| {
            Ok((x.get::<_, i64>(0)?, x.get::<_, i64>(1)?, x.get::<_, Option<i64>>(2)?, x.get::<_, i64>(3)?, x.get::<_, Option<i64>>(4)?, x.get::<_, i64>(5)?))
        })? {
            let (id, raw_t, raw_al, t, al, a) = row?;
            let nt = r.get(Kind::Track, raw_t);
            let na = r.get(Kind::Artist, track_artist.get(&nt).copied().unwrap_or(a));
            let nal = raw_al.map(|x| r.get(Kind::Album, x));
            if (nt, na, nal) != (t, a, al) {
                changes.push((id, nt, na, nal));
            }
        }
        {
            let mut up = tx.prepare("UPDATE scrobble SET track_id = ?, artist_id = ?, album_id = ? WHERE id = ?")?;
            for (id, t, a, al) in &changes {
                up.execute(params![t, a, al, id])?;
            }
        }
        fold_duplicates(&tx)?;
        tx.commit()?;
        Ok(changes.len())
    }

    fn new_op(&self, kind: Kind, from: i64, to: i64, rule: &str, auto: bool, label: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO merge_op (kind, from_id, to_id, rule, auto, label, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![kind_str(kind), from, to, rule, auto, label, chrono::Utc::now().timestamp()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn add_alias(&self, kind: Kind, from: i64, to: i64, op: i64) -> Result<()> {
        if from != to {
            self.conn.execute(
                "INSERT OR REPLACE INTO alias (kind, from_id, to_id, op_id) VALUES (?, ?, ?, ?)",
                params![kind_str(kind), from, to, op],
            )?;
        }
        Ok(())
    }

    /// Les données récupérées pour la source servent à la cible si elle n'en a pas.
    fn copy_enrichment(&self, kind: Kind, from: i64, to: i64) -> Result<()> {
        let c = &self.conn;
        match kind {
            Kind::Track => {
                c.execute(
                    "INSERT OR IGNORE INTO track_features SELECT ?2, status, provider, external_id, matched_title, matched_artist, isrc,
                       danceability, energy, valence, acousticness, instrumentalness, speechiness, liveness, tempo, loudness, key, mode, fetched_at
                     FROM track_features WHERE track_id = ?1",
                    params![from, to],
                )?;
                let copied = c.execute(
                    "INSERT OR IGNORE INTO track_credits (track_id, status, recording_mbid, work_language, length_ms, fetched_at, artist_credit) SELECT ?2, status, recording_mbid, work_language, length_ms, fetched_at, artist_credit FROM track_credits WHERE track_id = ?1",
                    params![from, to],
                )?;
                if copied > 0 {
                    c.execute("INSERT OR IGNORE INTO credit SELECT ?2, person_id, role, detail FROM credit WHERE track_id = ?1", params![from, to])?;
                }
                c.execute("UPDATE track SET duration_ms = (SELECT duration_ms FROM track WHERE id = ?1) WHERE id = ?2 AND duration_ms IS NULL", params![from, to])?;
            }
            Kind::Album => {
                c.execute(
                    "INSERT OR IGNORE INTO album_info SELECT ?2, status, release_date, edition_date, duration_s, nb_tracks, label, record_type, fetched_at
                     FROM album_info WHERE album_id = ?1",
                    params![from, to],
                )?;
            }
            Kind::Artist => {
                c.execute(
                    "INSERT OR IGNORE INTO artist_info SELECT ?2, status, mbid, kind, country, area, begin, end, fetched_at FROM artist_info WHERE artist_id = ?1",
                    params![from, to],
                )?;
            }
        }
        c.execute(
            "INSERT OR IGNORE INTO image SELECT kind, ?3, status, mime, data, source_url, fetched_at FROM image WHERE kind = ?1 AND entity_id = ?2",
            params![kind_str(kind), from, to],
        )?;
        Ok(())
    }

    fn name_of(&self, kind: Kind, id: i64) -> Result<String> {
        let sql = match kind {
            Kind::Artist => "SELECT name FROM artist WHERE id = ?",
            Kind::Album => "SELECT COALESCE(display_title, title) FROM album WHERE id = ?",
            Kind::Track => "SELECT title FROM track WHERE id = ?",
        };
        self.conn.query_row(sql, [id], |r| r.get(0)).optional()?.ok_or_else(|| Error::NotFound(format!("{kind:?} {id}")))
    }

    /// Titre ou album de même clé chez un autre artiste, créé s'il n'existe pas.
    fn counterpart(&self, table: &str, artist_id: i64, title: &str, key: &str) -> Result<i64> {
        self.conn.execute(&format!("INSERT OR IGNORE INTO {table} (artist_id, title, key) VALUES (?, ?, ?)"), params![artist_id, title, key])?;
        Ok(self.conn.query_row(&format!("SELECT id FROM {table} WHERE artist_id = ? AND key = ?"), params![artist_id, key], |r| r.get(0))?)
    }

    fn artist_or_create(&self, name: &str) -> Result<i64> {
        let key = artist_key(name);
        self.conn.execute("INSERT OR IGNORE INTO artist (name, key) VALUES (?, ?)", params![name, key])?;
        Ok(self.conn.query_row("SELECT id FROM artist WHERE key = ?", [&key], |r| r.get(0))?)
    }

    /// Fusionne `from` dans `to` sans recalculer les écoutes (voir `merge`).
    fn merge_no_rebuild(&self, kind: Kind, from: i64, to: i64, rule: &str, auto: bool, display: Option<&str>) -> Result<i64> {
        let to = Resolver::load(&self.conn)?.get(kind, to);
        if from == to {
            return Err(Error::NotFound("fusion d'une entité avec elle-même".into()));
        }
        let label = format!("{} → {}", self.name_of(kind, from)?, display.map(str::to_owned).unwrap_or(self.name_of(kind, to)?));
        let op = self.new_op(kind, from, to, rule, auto, &label)?;
        self.add_alias(kind, from, to, op)?;
        self.copy_enrichment(kind, from, to)?;
        match kind {
            Kind::Album => {
                if let Some(d) = display {
                    self.conn.execute("UPDATE album SET display_title = ? WHERE id = ? AND display_title IS NULL", params![d, to])?;
                }
            }
            // Un artiste fusionné : ses titres et albums rejoignent leurs équivalents chez la cible.
            Kind::Artist => {
                for table in ["track", "album"] {
                    let k = if table == "track" { Kind::Track } else { Kind::Album };
                    let mut st = self.conn.prepare(&format!(
                        "SELECT id, title, key FROM {table} x WHERE artist_id = ? AND NOT EXISTS (SELECT 1 FROM alias a WHERE a.kind = ? AND a.from_id = x.id)"
                    ))?;
                    let items: Vec<(i64, String, String)> =
                        st.query_map(params![from, table], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
                    for (id, title, key) in items {
                        let target = self.counterpart(table, to, &title, &key)?;
                        self.add_alias(k, id, target, op)?;
                        self.copy_enrichment(k, id, target)?;
                    }
                }
            }
            Kind::Track => {}
        }
        Ok(op)
    }

    /// Corrige le nom d'une fiche (« Le chant de Rivieres » → « Le Chant des Rivières ») :
    /// elle rejoint la fiche qui porte déjà ce nom chez le même artiste, sinon une fiche
    /// créée à ce nom. Fusion annulable ; les écoutes futures sous l'ancien nom suivent.
    /// Même clé (casse, accents) : le nom est simplement réécrit. Renvoie la fiche cible.
    pub fn rename(&self, kind: Kind, id: i64, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("empty_name"));
        }
        let to = match kind {
            Kind::Artist => self.artist_or_create(name)?,
            Kind::Album | Kind::Track => {
                let (table, key) = if kind == Kind::Album { ("album", album_key(name)) } else { ("track", title_key(name)) };
                let artist: i64 = self.conn.query_row(&format!("SELECT artist_id FROM {table} WHERE id = ?"), [id], |r| r.get(0))?;
                self.counterpart(table, artist, name, &key)?
            }
        };
        if to == id {
            let (table, col) = match kind {
                Kind::Artist => ("artist", "name"),
                Kind::Album => ("album", "title"),
                Kind::Track => ("track", "title"),
            };
            self.conn.execute(&format!("UPDATE {table} SET {col} = ? WHERE id = ?"), params![name, id])?;
            return Ok(id);
        }
        let to = Resolver::load(&self.conn)?.get(kind, to);
        self.merge(kind, id, to, "manual")?;
        Ok(to)
    }

    /// Fusionne `from` dans `to` (même type d'entité). Renvoie l'opération (annulable).
    pub fn merge(&self, kind: Kind, from: i64, to: i64, rule: &str) -> Result<i64> {
        // D'un bloc : un recalcul en échec ne laisse pas la fusion à moitié enregistrée.
        let block = self.atomic()?;
        let op = self.merge_no_rebuild(kind, from, to, rule, false, None)?;
        self.rebuild_canonical()?;
        block.commit()?;
        Ok(op)
    }

    /// Recalcule les clés de rapprochement après un changement de leurs règles. Une entité
    /// dont la nouvelle clé est déjà prise rejoint celle qui la porte (fusion annulable,
    /// règle « spelling ») ; sinon sa clé est simplement mise à jour. Renvoie le nombre de
    /// fusions ; les écoutes sont à recalculer ensuite.
    pub fn rekey(&self) -> Result<usize> {
        let tx = self.atomic()?;
        let mut merged = 0;
        let aliased = |kind: &str, id: i64| -> Result<bool> {
            Ok(self.conn.query_row("SELECT 1 FROM alias WHERE kind = ? AND from_id = ?", params![kind, id], |_| Ok(())).optional()?.is_some())
        };
        let rows: Vec<(i64, String, String)> =
            self.conn.prepare("SELECT id, name, key FROM artist")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        for (id, name, key) in rows {
            let nk = artist_key(&name);
            if nk == key {
                continue;
            }
            let taken = self.conn.query_row("SELECT id FROM artist WHERE key = ?", [&nk], |r| r.get::<_, i64>(0)).optional()?;
            // Fiche déjà fusionnée : sa clé suit aussi les règles (sinon la même graphie, relue,
            // créerait une nouvelle fiche), sauf si une autre fiche la porte déjà.
            if aliased("artist", id)? {
                if taken.is_none() {
                    self.conn.execute("UPDATE artist SET key = ? WHERE id = ?", params![nk, id])?;
                }
                continue;
            }
            match taken {
                Some(other) if other != id => merged += self.merge_no_rebuild(Kind::Artist, id, other, "spelling", true, None).is_ok() as usize,
                _ => {
                    self.conn.execute("UPDATE artist SET key = ? WHERE id = ?", params![nk, id])?;
                }
            }
        }
        for (table, kind) in [("album", Kind::Album), ("track", Kind::Track)] {
            let rows: Vec<(i64, i64, String, String)> = self
                .conn
                .prepare(&format!("SELECT id, artist_id, title, key FROM {table}"))?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (id, artist, title, key) in rows {
                let nk = if kind == Kind::Album { album_key(&title) } else { title_key(&title) };
                if nk == key {
                    continue;
                }
                let other: Option<i64> =
                    self.conn.query_row(&format!("SELECT id FROM {table} WHERE artist_id = ? AND key = ?"), params![artist, nk], |r| r.get(0)).optional()?;
                if aliased(table, id)? {
                    if other.is_none() {
                        self.conn.execute(&format!("UPDATE {table} SET key = ? WHERE id = ?"), params![nk, id])?;
                    }
                    continue;
                }
                match other {
                    Some(o) if o != id => merged += self.merge_no_rebuild(kind, id, o, "spelling", true, None).is_ok() as usize,
                    _ => {
                        self.conn.execute(&format!("UPDATE {table} SET key = ? WHERE id = ?"), params![nk, id])?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(merged)
    }

    /// Annule une fusion (et les alias qu'elle avait créés).
    pub fn undo_merge(&self, op: i64) -> Result<()> {
        let block = self.atomic()?;
        self.undo_merge_in(op)?;
        block.commit()?;
        Ok(())
    }

    fn undo_merge_in(&self, op: i64) -> Result<()> {
        let row: Option<(String, i64, Option<String>)> = self
            .conn
            .query_row("SELECT kind, to_id, suggestion_key FROM merge_op WHERE id = ?", [op], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        // Annulée = refusée : la correction ne doit pas revenir au prochain import.
        if let Some((_, _, Some(key))) = &row {
            self.dismiss_suggestion(key)?;
        }
        self.conn.execute("DELETE FROM alias WHERE op_id = ?", [op])?;
        self.conn.execute("DELETE FROM merge_op WHERE id = ?", [op])?;
        // Plus aucune édition regroupée sous ce titre : on rend son nom d'origine à l'album.
        if let Some((k, to, _)) = row
            && k == "album"
        {
            let left: i64 = self.conn.query_row("SELECT COUNT(*) FROM alias WHERE kind = 'album' AND to_id = ?", [to], |r| r.get(0))?;
            if left == 0 {
                self.conn.execute("UPDATE album SET display_title = NULL WHERE id = ?", [to])?;
            }
        }
        self.rebuild_canonical()?;
        // Envois retirés comme doubles à cause de cette fusion (deux graphies d'une même
        // source) : ils redeviennent deux écoutes.
        self.revalidate_double_sends()?;
        // Titre rendu à l'invité : l'album qui avait suivi l'artiste principal revient aussi
        // (sans être écarté : il repartira si le titre y retourne).
        let stale = self.conn.execute(
            "DELETE FROM merge_op WHERE rule = 'featured' AND kind = 'album' AND id IN (
               SELECT x.op_id FROM alias x JOIN album t ON t.id = x.to_id
               WHERE x.kind = 'album' AND EXISTS (SELECT 1 FROM scrobble s WHERE s.raw_album_id = x.from_id AND s.artist_id <> t.artist_id))",
            [],
        )?;
        if stale > 0 {
            self.rebuild_canonical()?;
        }
        Ok(())
    }

    pub fn merge_ops(&self) -> Result<Vec<MergeOp>> {
        let mut st = self.conn.prepare("SELECT id, kind, rule, auto, label, created_at FROM merge_op ORDER BY created_at DESC, id DESC")?;
        let ops = st
            .query_map([], |r| {
                Ok(MergeOp { id: r.get(0)?, kind: parse_kind(&r.get::<_, String>(1)?), rule: r.get(2)?, auto: r.get(3)?, label: r.get(4)?, created_at: r.get(5)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ops)
    }

    /// Fusions dont cette entité est la cible (pour l'afficher sur sa fiche).
    pub fn merged_into(&self, kind: Kind, id: i64) -> Result<Vec<MergeOp>> {
        let mut st = self.conn.prepare(
            "SELECT id, kind, rule, auto, label, created_at FROM merge_op WHERE kind = ? AND to_id = ? ORDER BY created_at",
        )?;
        let ops = st
            .query_map(params![kind_str(kind), id], |r| {
                Ok(MergeOp { id: r.get(0)?, kind: parse_kind(&r.get::<_, String>(1)?), rule: r.get(2)?, auto: r.get(3)?, label: r.get(4)?, created_at: r.get(5)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ops)
    }

    /// Toutes les corrections possibles, sauf celles déjà ignorées.
    pub fn suggestions(&self) -> Result<Vec<Suggestion>> {
        let mut out = Vec::new();
        self.album_suggestions(&mut out)?;
        self.track_suggestions(&mut out)?;
        self.artist_suggestions(&mut out)?;
        self.featured_suggestions(&mut out)?;
        let mut st = self.conn.prepare("SELECT key FROM alias_dismissed")?;
        let dismissed: std::collections::HashSet<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        out.retain(|s| !dismissed.contains(&s.key));
        out.sort_by_key(|s| (!s.sure, std::cmp::Reverse(s.plays)));
        Ok(out)
    }

    fn album_suggestions(&self, out: &mut Vec<Suggestion>) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT al.id, al.title, al.artist_id, a.name, COUNT(*) FROM scrobble s JOIN album al ON al.id = s.album_id
             JOIN artist a ON a.id = al.artist_id GROUP BY al.id",
        )?;
        let albums: Vec<(i64, String, i64, String, i64)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
        // Entités HTML (« Simon &amp; Garfunkel ») : le titre décodé, chez le même artiste.
        for a in &albums {
            let decoded = unescape_html(&a.1);
            if decoded == a.1 {
                continue;
            }
            let to_id: Option<i64> = self
                .conn
                .query_row("SELECT id FROM album WHERE artist_id = ? AND key = ? AND id <> ?", params![a.2, album_key(&decoded), a.0], |r| r.get(0))
                .optional()?;
            // Même clé (l'encodage était dans une mention d'édition) : un simple renommage.
            if to_id.is_none() && album_key(&decoded) == album_key(&a.1) {
                self.conn.execute("UPDATE album SET title = ? WHERE id = ?", params![decoded, a.0])?;
                continue;
            }
            out.push(Suggestion {
                key: format!("html:album:{}", a.0),
                rule: "html".into(),
                kind: Kind::Album,
                from_id: a.0,
                from_name: a.1.clone(),
                from_artist: Some(a.3.clone()),
                to_id,
                to_name: decoded,
                to_artist: Some(a.3.clone()),
                plays: a.4,
                sure: true,
            });
        }
        // Deux regroupements : sans mentions d'édition (sûr), sans aucune parenthèse (proposé).
        let loose = |t: &str| fold(t.split(['(', '[']).next().unwrap_or(t).split(" - ").next().unwrap_or(t));
        for (rule, sure) in [("edition", true), ("album_variant", false)] {
            let mut groups: HashMap<(i64, String, bool), Vec<&(i64, String, i64, String, i64)>> = HashMap::new();
            for a in &albums {
                let base = if sure { fold(&album_base(&a.1)) } else { loose(&a.1) };
                groups.entry((a.2, base, is_live(&a.1))).or_default().push(a);
            }
            for ((_, _, _), g) in groups.into_iter().filter(|(_, g)| g.len() > 1) {
                // Cible : l'édition sans mention s'il y en a une, sinon la plus écoutée.
                let target = g
                    .iter()
                    .find(|a| album_base(&a.1) == a.1 && (sure || !a.1.contains('(')))
                    .or_else(|| g.iter().max_by_key(|a| a.4))
                    .unwrap();
                let display = if sure { album_base(&target.1) } else { target.1.clone() };
                for a in g.iter().filter(|a| a.0 != target.0) {
                    // Déjà couvert par une suggestion sûre : pas de doublon.
                    if !sure && out.iter().any(|s| s.kind == Kind::Album && s.from_id == a.0) {
                        continue;
                    }
                    out.push(Suggestion {
                        key: format!("{rule}:album:{}:{}", a.0, target.0),
                        rule: rule.into(),
                        kind: Kind::Album,
                        from_id: a.0,
                        from_name: a.1.clone(),
                        from_artist: Some(a.3.clone()),
                        to_id: Some(target.0),
                        to_name: display.clone(),
                        to_artist: Some(target.3.clone()),
                        plays: a.4,
                        sure,
                    });
                }
            }
        }
        Ok(())
    }

    fn track_suggestions(&self, out: &mut Vec<Suggestion>) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT t.id, t.title, a.id, a.name, COUNT(*) FROM scrobble s JOIN track t ON t.id = s.track_id
             JOIN artist a ON a.id = t.artist_id GROUP BY t.id",
        )?;
        let tracks: Vec<(i64, String, i64, String, i64)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut st = self.conn.prepare("SELECT DISTINCT a.key, a.name FROM scrobble s JOIN artist a ON a.id = s.artist_id")?;
        let known: HashMap<String, String> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let existing = |artist: &str, title: &str| -> Result<Option<i64>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT t.id FROM track t JOIN artist a ON a.id = t.artist_id WHERE a.key = ? AND t.key = ?",
                    params![artist_key(artist), title_key(title)],
                    |r| r.get(0),
                )
                .optional()?)
        };
        for (id, title, _artist_id, artist, plays) in tracks {
            let decoded = unescape_html(&title);
            let cleaned = clean_filename(&decoded);
            let mut new_title = cleaned.clone().unwrap_or_else(|| decoded.clone());
            let mut new_artist = artist.clone();
            let mut rule = if cleaned.is_some() { "filename" } else if decoded != title { "html" } else { "" };
            if let Some((left, right)) = split_artist_title(&new_title) {
                if artist_key(&left) == artist_key(&artist) {
                    new_title = right;
                    rule = "artist_in_title";
                } else if is_placeholder_artist(&artist) {
                    new_artist = known.get(&artist_key(&left)).cloned().unwrap_or(left);
                    new_title = right;
                    rule = "artist_in_title";
                }
            }
            if rule.is_empty() || (artist_key(&new_artist) == artist_key(&artist) && new_title == title) {
                continue;
            }
            let to_id = existing(&new_artist, &new_title)?.filter(|t| *t != id);
            // Même clé que l'original (seul l'encodage diffère) : renommer plutôt que fusionner avec soi-même.
            if to_id.is_none() && rule == "html" && title_key(&new_title) == title_key(&title) && artist_key(&new_artist) == artist_key(&artist) {
                self.conn.execute("UPDATE track SET title = ? WHERE id = ?", params![new_title, id])?;
                continue;
            }
            out.push(Suggestion {
                key: format!("{rule}:track:{id}:{}:{}", artist_key(&new_artist), title_key(&new_title)),
                rule: rule.into(),
                kind: Kind::Track,
                from_id: id,
                from_name: title,
                from_artist: Some(artist),
                sure: to_id.is_some() || rule == "html",
                to_id,
                to_name: new_title,
                to_artist: Some(new_artist),
                plays,
            });
        }
        Ok(())
    }

    /// Titre écouté sous le nom d'un artiste invité : MusicBrainz crédite un autre
    /// artiste en premier (« Nova feat. Zoé » écouté sous « Zoé »). Les
    /// écoutes rejoignent l'artiste principal, seulement s'il est déjà dans
    /// l'historique et que l'artiste écouté est marqué comme invité (« feat. »,
    /// « with »…). Une simple liste (« A & B », « A, B ») est un duo ou un
    /// générique de bande originale : l'artiste écouté reste. Vérifié à la main
    /// sur 62 cas : sans ces deux conditions, une sur deux est fausse
    /// (reprise, mauvais enregistrement trouvé).
    fn featured_suggestions(&self, out: &mut Vec<Suggestion>) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT t.id, t.title, a.id, a.name, c.artist_credit, COUNT(*) FROM scrobble s JOIN track t ON t.id = s.track_id
             JOIN artist a ON a.id = t.artist_id JOIN track_credits c ON c.track_id = t.id
             WHERE c.status = 'ok' AND c.artist_credit LIKE '[{%},{%' GROUP BY t.id",
        )?;
        let tracks: Vec<(i64, String, i64, String, String, i64)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<rusqlite::Result<_>>()?;
        if tracks.is_empty() {
            return Ok(());
        }
        let persons = self.person_artists()?;
        let mut person = self.conn.prepare_cached("SELECT id FROM person WHERE mbid = ?")?;
        let mut st = self.conn.prepare("SELECT a.key, a.id, a.name FROM scrobble s JOIN artist a ON a.id = s.artist_id GROUP BY a.id ORDER BY COUNT(*)")?;
        // Artistes de l'historique par nom (le plus écouté l'emporte) et par identifiant.
        let mut by_key: HashMap<String, (i64, String)> = HashMap::new();
        let mut names: HashMap<i64, String> = HashMap::new();
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))? {
            let (k, id, name) = row?;
            names.insert(id, name.clone());
            by_key.insert(k, (id, name));
        }
        for (id, title, artist_id, artist, json, plays) in tracks {
            let credit: Vec<crate::credits::CreditName> = serde_json::from_str(&json).unwrap_or_default();
            // Artiste de l'historique pour chaque nom crédité : même nom d'abord, sinon
            // même identifiant MusicBrainz (« Cat Stevens » → « Yusuf / Cat Stevens »).
            // Un nom composé (« Danger Mouse feat. Run the Jewels ») n'est pas une
            // cible : le nom crédité sera créé.
            let mut known = Vec::new();
            for c in &credit {
                let p: Option<i64> = person.query_row([&c.mbid], |r| r.get(0)).optional()?;
                let by_mbid = p.and_then(|p| persons.get(&p)).and_then(|a| names.get(a).map(|n| (*a, n.clone())));
                known.push(by_key.get(&artist_key(&c.name)).cloned().map(Some).or(by_mbid.map(|(a, n)| if is_compound(&n) { None } else { Some((a, n)) })));
            }
            let ours = |i: usize| known[i].as_ref().and_then(Option::as_ref).is_some_and(|k| k.0 == artist_id) || artist_key_no_article(&credit[i].name) == artist_key_no_article(&artist);
            // L'artiste écouté doit être crédité comme invité, pas en premier.
            let guest = (1..credit.len()).any(|i| ours(i) && is_featuring(&credit[i - 1].join));
            if credit.len() < 2 || ours(0) || !guest {
                continue;
            }
            // Artiste principal absent de l'historique : on s'abstient.
            let Some(main) = &known[0] else { continue };
            let (target, to_artist) = match main {
                Some((id, name)) => (Some(*id), name.clone()),
                None => (None, credit[0].name.clone()),
            };
            let to_id: Option<i64> = match target {
                Some(a) => self.conn.query_row("SELECT id FROM track WHERE artist_id = ? AND key = ?", params![a, title_key(&title)], |r| r.get(0)).optional()?,
                None => None,
            };
            out.push(Suggestion {
                key: format!("featured:track:{id}:{}", artist_key(&to_artist)),
                rule: "featured".into(),
                kind: Kind::Track,
                from_id: id,
                from_name: title.clone(),
                from_artist: Some(artist),
                sure: true,
                to_id,
                to_name: title,
                to_artist: Some(to_artist),
                plays,
            });
        }
        // Album du même nom chez l'invité (« La route du nord » de Zoé) dont toutes les
        // écoutes sont désormais chez l'artiste principal : rejoint l'album de celui-ci.
        let mut st = self.conn.prepare(
            "SELECT al.id, COALESCE(al.display_title, al.title), al.key, g.name, MAX(s.artist_id), a.name, COUNT(*)
             FROM scrobble s JOIN album al ON al.id = s.album_id JOIN artist g ON g.id = al.artist_id JOIN artist a ON a.id = s.artist_id
             GROUP BY al.id
             HAVING COUNT(DISTINCT s.artist_id) = 1 AND MAX(s.artist_id) <> al.artist_id
               AND SUM(EXISTS (SELECT 1 FROM alias x JOIN merge_op m ON m.id = x.op_id
                               WHERE x.kind = 'track' AND x.from_id = s.raw_track_id AND m.rule = 'featured')) > 0",
        )?;
        let albums: Vec<(i64, String, String, String, i64, String, i64)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, title, key, guest, main_id, main, plays) in albums {
            let to_id: Option<i64> = self.conn.query_row("SELECT id FROM album WHERE artist_id = ? AND key = ?", params![main_id, key], |r| r.get(0)).optional()?;
            out.push(Suggestion {
                key: format!("featured:album:{id}:{}", artist_key(&main)),
                rule: "featured".into(),
                kind: Kind::Album,
                from_id: id,
                from_name: title.clone(),
                from_artist: Some(guest),
                sure: true,
                to_id,
                to_name: title,
                to_artist: Some(main),
                plays,
            });
        }
        Ok(())
    }

    fn artist_suggestions(&self, out: &mut Vec<Suggestion>) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT a.id, a.name, COALESCE(a.mbid, i.mbid), COUNT(*) FROM scrobble s JOIN artist a ON a.id = s.artist_id
             LEFT JOIN artist_info i ON i.artist_id = a.id GROUP BY a.id",
        )?;
        let artists: Vec<(i64, String, Option<String>, i64)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        // Noms abîmés : entités HTML, artistes multiples « A; B », surnom entre guillemets.
        let mut by_key: HashMap<String, &(i64, String, Option<String>, i64)> = HashMap::new();
        for a in &artists {
            let e = by_key.entry(artist_key(&a.1)).or_insert(a);
            if a.3 > e.3 {
                *e = a;
            }
        }
        for a in &artists {
            let decoded = unescape_html(&a.1);
            let (rule, name) = if decoded != a.1 {
                ("html", decoded)
            } else if let Some(main) = main_artist(&a.1) {
                ("multi_artist", main)
            } else if let Some(n) = without_nickname(&a.1).filter(|n| by_key.contains_key(&artist_key(n))) {
                ("nickname", n)
            } else {
                continue;
            };
            let target = by_key.get(&artist_key(&name)).filter(|t| t.0 != a.0);
            // « A; A » ou artiste principal déjà connu : sûr. Sinon proposé (l'artiste sera créé).
            let same_parts = a.1.split(';').map(artist_key).filter(|k| !k.is_empty()).collect::<std::collections::HashSet<_>>().len() == 1;
            let sure = rule != "multi_artist" || target.is_some() || same_parts;
            if target.is_none() && artist_key(&name) == artist_key(&a.1) {
                if rule == "html" {
                    self.conn.execute("UPDATE artist SET name = ? WHERE id = ?", params![name, a.0])?;
                }
                continue;
            }
            out.push(Suggestion {
                key: format!("{rule}:artist:{}", a.0),
                rule: rule.into(),
                kind: Kind::Artist,
                from_id: a.0,
                from_name: a.1.clone(),
                from_artist: None,
                to_id: target.map(|t| t.0),
                to_name: target.map_or(name, |t| t.1.clone()),
                to_artist: None,
                plays: a.3,
                sure,
            });
        }
        for (rule, sure) in [("same_mbid", true), ("article", false)] {
            let mut groups: HashMap<String, Vec<&(i64, String, Option<String>, i64)>> = HashMap::new();
            for a in artists.iter().filter(|a| !is_placeholder_artist(&a.1) && !out.iter().any(|s| s.kind == Kind::Artist && s.from_id == a.0)) {
                let k = if sure { a.2.clone() } else { Some(artist_key_no_article(&a.1)) };
                if let Some(k) = k {
                    groups.entry(k).or_default().push(a);
                }
            }
            for g in groups.into_values().filter(|g| g.len() > 1) {
                let target = g.iter().max_by_key(|a| a.3).unwrap();
                for a in g.iter().filter(|a| a.0 != target.0) {
                    if !sure && out.iter().any(|s| s.kind == Kind::Artist && s.from_id == a.0) {
                        continue;
                    }
                    out.push(Suggestion {
                        key: format!("{rule}:artist:{}:{}", a.0, target.0),
                        rule: rule.into(),
                        kind: Kind::Artist,
                        from_id: a.0,
                        from_name: a.1.clone(),
                        from_artist: None,
                        to_id: Some(target.0),
                        to_name: target.1.clone(),
                        to_artist: None,
                        plays: a.3,
                        sure,
                    });
                }
            }
        }
        Ok(())
    }

    fn apply_no_rebuild(&self, s: &Suggestion, auto: bool) -> Result<i64> {
        let to = match s.to_id {
            Some(id) => id,
            // Cible à créer : artiste, ou titre / album corrigé chez le bon artiste.
            None => match s.kind {
                Kind::Artist => self.artist_or_create(&s.to_name)?,
                Kind::Album => {
                    let artist = self.artist_or_create(s.to_artist.as_deref().unwrap_or(&s.to_name))?;
                    self.counterpart("album", artist, &s.to_name, &album_key(&s.to_name))?
                }
                Kind::Track => {
                    let artist = self.artist_or_create(s.to_artist.as_deref().unwrap_or(&s.to_name))?;
                    self.counterpart("track", artist, &s.to_name, &title_key(&s.to_name))?
                }
            },
        };
        let display = (s.rule == "edition").then_some(s.to_name.as_str());
        let op = self.merge_no_rebuild(s.kind, s.from_id, to, &s.rule, auto, display)?;
        self.conn.execute("UPDATE merge_op SET suggestion_key = ? WHERE id = ?", params![s.key, op])?;
        Ok(op)
    }

    pub fn apply_suggestion(&self, s: &Suggestion) -> Result<i64> {
        let op = self.apply_no_rebuild(s, false)?;
        self.rebuild_canonical()?;
        Ok(op)
    }

    /// Réaffiche les suggestions écartées (ignorées ou annulées).
    pub fn clear_dismissed(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM alias_dismissed", [])?)
    }

    pub fn dismissed_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM alias_dismissed", [], |r| r.get(0))?)
    }

    pub fn dismiss_suggestion(&self, key: &str) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO alias_dismissed (key) VALUES (?)", [key])?;
        Ok(())
    }

    /// Retire les erreurs d'enregistrement et applique les corrections sûres ;
    /// renvoie leur nombre. Appelé après chaque import et au démarrage.
    pub fn auto_clean(&self) -> Result<usize> {
        let mut n = self.purge_recording_errors()? + self.drop_double_sends()?;
        // Une fusion peut en rendre une autre caduque (même source) : on refait la liste jusqu'à épuisement.
        for _ in 0..5 {
            let sure: Vec<Suggestion> = self.suggestions()?.into_iter().filter(|s| s.sure).collect();
            if sure.is_empty() {
                break;
            }
            // Une transaction par passe : sinon chaque écriture attend le disque (des secondes pour quelques centaines de corrections).
            let tx = self.atomic()?;
            for s in &sure {
                let already: bool = self
                    .conn
                    .query_row("SELECT 1 FROM alias WHERE kind = ? AND from_id = ?", params![kind_str(s.kind), s.from_id], |_| Ok(true))
                    .optional()?
                    .unwrap_or(false);
                if !already && self.apply_no_rebuild(s, true).is_ok() {
                    n += 1;
                }
            }
            tx.commit()?;
            self.rebuild_canonical()?;
        }
        Ok(n)
    }

    /// Autres versions d'un titre chez le même artiste (live, remix, version orchestrale…).
    pub fn versions(&self, track_id: i64) -> Result<Vec<Ranked>> {
        let (artist_id, title): (i64, String) = self.conn.query_row("SELECT artist_id, title FROM track WHERE id = ?", [track_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let loose = |t: &str| fold(t.split(['(', '[']).next().unwrap_or(t).split(" - ").next().unwrap_or(t));
        let base = loose(&title);
        let mut st = self.conn.prepare(
            "SELECT t.id, t.title, COUNT(*) FROM scrobble s JOIN track t ON t.id = s.track_id WHERE s.artist_id = ? AND t.id <> ? GROUP BY t.id ORDER BY 3 DESC",
        )?;
        let rows: Vec<(i64, String, i64)> = st.query_map(params![artist_id, track_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        let artist: String = self.conn.query_row("SELECT name FROM artist WHERE id = ?", [artist_id], |r| r.get(0))?;
        Ok(rows
            .into_iter()
            .filter(|(_, t, _)| loose(t) == base && !base.is_empty())
            .map(|(id, name, plays)| Ranked { kind: Kind::Track, id, name, artist: Some(artist.clone()), artist_id: Some(artist_id), plays, seconds: None })
            .collect())
    }
}
