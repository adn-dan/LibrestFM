//! Base SQLite locale. Les données brutes importées ne sont jamais modifiées :
//! chaque écoute garde la liste des fichiers sources qui la contiennent.

use crate::Result;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

const SCHEMA_V1: &str = r#"
CREATE TABLE source (
  id            INTEGER PRIMARY KEY,
  file_name     TEXT NOT NULL,
  sha256        TEXT NOT NULL UNIQUE,
  format        TEXT NOT NULL,
  imported_at   INTEGER NOT NULL,
  rows_read     INTEGER NOT NULL,
  rows_added    INTEGER NOT NULL,
  rows_merged   INTEGER NOT NULL,
  rows_rejected INTEGER NOT NULL,
  first_ts      INTEGER,
  last_ts       INTEGER
);
CREATE TABLE artist (
  id    INTEGER PRIMARY KEY,
  name  TEXT NOT NULL,
  key   TEXT NOT NULL UNIQUE,
  mbid  TEXT
);
CREATE TABLE album (
  id        INTEGER PRIMARY KEY,
  artist_id INTEGER NOT NULL REFERENCES artist(id),
  title     TEXT NOT NULL,
  key       TEXT NOT NULL,
  mbid      TEXT,
  UNIQUE (artist_id, key)
);
CREATE TABLE track (
  id        INTEGER PRIMARY KEY,
  artist_id INTEGER NOT NULL REFERENCES artist(id),
  title     TEXT NOT NULL,
  key       TEXT NOT NULL,
  mbid      TEXT,
  UNIQUE (artist_id, key)
);
-- ts : début de l'écoute, secondes Unix UTC. precise = 0 si la source n'a que la minute.
-- artist_id, ym (202403), dow (0 = lundi) et hour (heure locale) sont dérivés à l'import
-- pour que les agrégats n'aient ni jointure ni conversion de fuseau à faire.
CREATE TABLE scrobble (
  id        INTEGER PRIMARY KEY,
  ts        INTEGER NOT NULL,
  track_id  INTEGER NOT NULL REFERENCES track(id),
  artist_id INTEGER NOT NULL REFERENCES artist(id),
  album_id  INTEGER REFERENCES album(id),
  precise   INTEGER NOT NULL,
  ym        INTEGER NOT NULL,
  dow       INTEGER NOT NULL,
  hour      INTEGER NOT NULL
);
CREATE INDEX scrobble_cover ON scrobble(ts, artist_id, track_id, album_id, ym, dow, hour);
CREATE INDEX scrobble_track_ts ON scrobble(track_id, ts);
CREATE INDEX scrobble_artist_ts ON scrobble(artist_id, ts);
CREATE INDEX scrobble_album_ts ON scrobble(album_id, ts);
CREATE TABLE scrobble_source (
  scrobble_id INTEGER NOT NULL REFERENCES scrobble(id) ON DELETE CASCADE,
  source_id   INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  PRIMARY KEY (scrobble_id, source_id)
) WITHOUT ROWID;
CREATE INDEX scrobble_source_src ON scrobble_source(source_id);

-- Caractéristiques audio. status : ok, not_found, error. Valeurs nulles si absentes.
CREATE TABLE track_features (
  track_id         INTEGER PRIMARY KEY REFERENCES track(id),
  status           TEXT NOT NULL,
  provider         TEXT NOT NULL,
  external_id      TEXT,
  matched_title    TEXT,
  matched_artist   TEXT,
  isrc             TEXT,
  danceability     REAL,
  energy           REAL,
  valence          REAL,
  acousticness     REAL,
  instrumentalness REAL,
  speechiness      REAL,
  liveness         REAL,
  tempo            REAL,
  loudness         REAL,
  key              INTEGER,
  mode             INTEGER,
  fetched_at       INTEGER NOT NULL
);
-- Catalogue ReccoBeats d'un artiste, mis en cache pour ne pas le retélécharger par titre.
CREATE TABLE reccobeats_artist_cache (
  artist_id  INTEGER PRIMARY KEY REFERENCES artist(id),
  fetched_at INTEGER NOT NULL,
  tracks     TEXT NOT NULL
);
"#;

/// v2 : jour local (ymd = 20240310) pour les courbes au jour près.
const MIGRATION_V2: &str = r#"
ALTER TABLE scrobble ADD COLUMN ymd INTEGER NOT NULL DEFAULT 0;
UPDATE scrobble SET ymd = CAST(strftime('%Y%m%d', ts, 'unixepoch', 'localtime') AS INTEGER);
DROP INDEX scrobble_cover;
CREATE INDEX scrobble_cover ON scrobble(ts, artist_id, track_id, album_id, ym, ymd, dow, hour);
"#;

/// v3 : photos d'artiste et pochettes (une ligne par artiste ou album).
const MIGRATION_V3: &str = r#"
CREATE TABLE image (
  kind       TEXT NOT NULL,
  entity_id  INTEGER NOT NULL,
  status     TEXT NOT NULL,
  mime       TEXT,
  data       BLOB,
  source_url TEXT,
  fetched_at INTEGER NOT NULL,
  PRIMARY KEY (kind, entity_id)
);
"#;

/// v4 : formats de source enregistrés en codes (traduits par l'interface).
const MIGRATION_V4: &str = r#"
UPDATE source SET format = CASE format
  WHEN 'CSV Last.fm (lastfm-to-csv)' THEN 'lastfm_csv'
  WHEN 'TSV lastexport / Libre.fm' THEN 'last_export_tsv'
  WHEN 'CSV avec en-tête' THEN 'header_csv'
  WHEN 'ListenBrainz' THEN 'listen_brainz'
  WHEN 'Spotify (historique étendu)' THEN 'spotify_extended'
  WHEN 'Spotify (historique de compte)' THEN 'spotify_account'
  ELSE format END;
"#;

/// v5 : crédits détaillés (MusicBrainz) et personnes créditées.
const MIGRATION_V5: &str = r#"
CREATE TABLE person (
  id   INTEGER PRIMARY KEY,
  mbid TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL
);
CREATE TABLE credit (
  track_id  INTEGER NOT NULL REFERENCES track(id),
  person_id INTEGER NOT NULL REFERENCES person(id),
  role      TEXT NOT NULL,
  detail    TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (track_id, person_id, role, detail)
) WITHOUT ROWID;
CREATE INDEX credit_person ON credit(person_id);
-- status : ok, not_found, error. length_ms et work_language serviront aux stats.
CREATE TABLE track_credits (
  track_id       INTEGER PRIMARY KEY REFERENCES track(id),
  status         TEXT NOT NULL,
  recording_mbid TEXT,
  work_language  TEXT,
  length_ms      INTEGER,
  fetched_at     INTEGER NOT NULL
);
"#;

/// v6 : durées des titres, informations d'album (sortie, durée) et d'artiste (origine).
const MIGRATION_V6: &str = r#"
ALTER TABLE track ADD COLUMN duration_ms INTEGER;
CREATE TABLE album_info (
  album_id     INTEGER PRIMARY KEY REFERENCES album(id),
  status       TEXT NOT NULL,
  release_date TEXT,
  edition_date TEXT,
  duration_s   INTEGER,
  nb_tracks    INTEGER,
  label        TEXT,
  record_type  TEXT,
  fetched_at   INTEGER NOT NULL
);
CREATE TABLE artist_info (
  artist_id  INTEGER PRIMARY KEY REFERENCES artist(id),
  status     TEXT NOT NULL,
  mbid       TEXT,
  kind       TEXT,
  country    TEXT,
  area       TEXT,
  begin      TEXT,
  end        TEXT,
  fetched_at INTEGER NOT NULL
);
-- Durées déjà connues par les crédits MusicBrainz.
UPDATE track SET duration_ms = (SELECT length_ms FROM track_credits c WHERE c.track_id = track.id) WHERE duration_ms IS NULL;
"#;

/// v7 : correspondances (alias). Les écoutes gardent leurs identifiants bruts ;
/// track_id / album_id / artist_id deviennent les identifiants corrigés.
const MIGRATION_V7: &str = r#"
ALTER TABLE scrobble ADD COLUMN raw_track_id INTEGER;
ALTER TABLE scrobble ADD COLUMN raw_album_id INTEGER;
UPDATE scrobble SET raw_track_id = track_id, raw_album_id = album_id;
CREATE INDEX scrobble_raw_track ON scrobble(raw_track_id);
ALTER TABLE album ADD COLUMN display_title TEXT;
CREATE TABLE merge_op (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL,
  from_id    INTEGER NOT NULL,
  to_id      INTEGER NOT NULL,
  rule       TEXT NOT NULL,
  auto       INTEGER NOT NULL,
  label      TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE alias (
  kind    TEXT NOT NULL,
  from_id INTEGER NOT NULL,
  to_id   INTEGER NOT NULL,
  op_id   INTEGER NOT NULL REFERENCES merge_op(id) ON DELETE CASCADE,
  PRIMARY KEY (kind, from_id)
) WITHOUT ROWID;
CREATE INDEX alias_to ON alias(kind, to_id);
CREATE TABLE alias_dismissed (key TEXT PRIMARY KEY) WITHOUT ROWID;
"#;

/// v8 : une fusion garde la suggestion d'origine, pour l'écarter si on l'annule.
const MIGRATION_V8: &str = "ALTER TABLE merge_op ADD COLUMN suggestion_key TEXT;";

/// Comptes en ligne synchronisés ; chacun a sa source (supprimée avec lui).
/// run_* : fenêtre de synchronisation en cours, pour reprendre après une interruption.
const MIGRATION_V10: &str = "CREATE TABLE account (
  id           INTEGER PRIMARY KEY,
  service      TEXT NOT NULL,
  username     TEXT NOT NULL,
  api_key      TEXT,
  source_id    INTEGER NOT NULL UNIQUE REFERENCES source(id) ON DELETE CASCADE,
  synced_until INTEGER,
  run_from     INTEGER,
  run_to       INTEGER,
  run_top      INTEGER,
  synced_at    INTEGER,
  error        TEXT
);";

/// Écoutes réunies parce qu'elles sont devenues identiques après une fusion
/// (même titre corrigé, même moment, sources différentes). Gardées ici pour
/// être rendues si la fusion est annulée.
const MIGRATION_V11: &str = "CREATE TABLE scrobble_fold (
  id           INTEGER PRIMARY KEY,
  keep_id      INTEGER NOT NULL,
  ts           INTEGER NOT NULL,
  precise      INTEGER NOT NULL,
  raw_track_id INTEGER NOT NULL,
  raw_album_id INTEGER,
  sources      TEXT NOT NULL
);";

/// Tâches de fond lancées et ni finies ni arrêtées : reprises au démarrage.
const MIGRATION_V12: &str = "CREATE TABLE job_wanted (name TEXT PRIMARY KEY) WITHOUT ROWID;";

/// Réglages gardés en base (dossier d'import…) et fichiers du dossier déjà lus.
const MIGRATION_V13: &str = "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE input_file (
  path      TEXT PRIMARY KEY,
  size      INTEGER NOT NULL,
  mtime     INTEGER NOT NULL,
  source_id INTEGER REFERENCES source(id) ON DELETE SET NULL
);";

/// Découvertes : réponses des services gardées un mois, avis sur les titres
/// proposés, dernières sélections calculées.
const MIGRATION_V14: &str = "CREATE TABLE disc_cache (url TEXT PRIMARY KEY, body TEXT NOT NULL, fetched_at INTEGER NOT NULL) WITHOUT ROWID;
CREATE TABLE disc_feedback (
  deezer_id INTEGER PRIMARY KEY,
  artist    TEXT NOT NULL,
  verdict   INTEGER NOT NULL,
  at        INTEGER NOT NULL
);
CREATE TABLE disc_result (scope TEXT PRIMARY KEY, body TEXT NOT NULL, computed_at INTEGER NOT NULL) WITHOUT ROWID;";

/// Empreinte du dernier contenu lu de chaque fichier du dossier d'import : un
/// fichier qui commence par ce contenu a seulement été complété.
const MIGRATION_V15: &str = "ALTER TABLE input_file ADD COLUMN sha TEXT;";

/// Heure de l'écoute telle que chaque source la donne : relire une source ne
/// reconnaît que ses propres lignes (les liens antérieurs restent sans heure).
const MIGRATION_V16: &str = "ALTER TABLE scrobble_source ADD COLUMN ts INTEGER;";

/// Artistes crédités dans l'ordre MusicBrainz, avec leurs liaisons (« feat. »,
/// « & »…), en JSON. NULL : fiche lue avant que l'ordre soit gardé.
const MIGRATION_V17: &str = "ALTER TABLE track_credits ADD COLUMN artist_credit TEXT;";

/// Paroles (LRCLIB) par artiste et titre normalisés ; synced et plain NULL : rien trouvé.
const MIGRATION_V18: &str = "CREATE TABLE lyrics (
  key          TEXT PRIMARY KEY,
  synced       TEXT,
  plain        TEXT,
  instrumental INTEGER NOT NULL DEFAULT 0,
  fetched_at   INTEGER NOT NULL
) WITHOUT ROWID;";

/// Doubles envois retirés (même source, même titre relancé avant la moitié de sa durée) :
/// gardés pour être listés et rétablis. restored = 1 : rétabli, plus jamais retiré.
const MIGRATION_V20: &str = "CREATE TABLE double_send (
  id           INTEGER PRIMARY KEY,
  ts           INTEGER NOT NULL,
  precise      INTEGER NOT NULL,
  raw_track_id INTEGER NOT NULL,
  raw_album_id INTEGER,
  sources      TEXT NOT NULL,
  dropped_at   INTEGER NOT NULL,
  restored     INTEGER NOT NULL DEFAULT 0,
  UNIQUE (raw_track_id, ts)
);";

/// Première sortie de l'œuvre (tous enregistrements de ce titre par cet artiste, hors live
/// et remix) : « Jusqu'à demain » écouté sur un best-of de 2022 date de 1981.
/// '' : rien trouvé ; NULL : pas encore cherché.
const MIGRATION_V21: &str = "ALTER TABLE track_credits ADD COLUMN first_release TEXT;";

/// Annuler la fusion d'un album relit les écoutes par album d'origine.
const MIGRATION_V22: &str = "CREATE INDEX IF NOT EXISTS scrobble_raw_album ON scrobble(raw_album_id);";

/// Retours sur les mixes : titre retiré d'un mix, ou jamais proposé (mix_key = '*').
const MIGRATION_V9: &str = "CREATE TABLE mix_feedback (
  mix_key  TEXT NOT NULL,
  track_id INTEGER NOT NULL,
  PRIMARY KEY (mix_key, track_id)
) WITHOUT ROWID;";

pub struct Library {
    pub conn: Connection,
    /// Empreinte des écoutes et des durées lors du dernier calcul des temps
    /// d'écoute : inchangée, la table temporaire est encore juste.
    pub(crate) listened_at: std::cell::Cell<Option<u64>>,
}

/// Version du schéma après toutes les migrations ci-dessous.
pub const SCHEMA_VERSION: i64 = 22;

/// Bloc d'écritures annulé s'il n'est pas validé (point de sauvegarde SQLite : imbricable,
/// contrairement à une transaction, pour qu'une fusion et son recalcul forment un seul bloc).
pub(crate) struct Atomic<'a> {
    conn: &'a Connection,
    done: bool,
}

impl<'a> Atomic<'a> {
    pub(crate) fn commit(mut self) -> rusqlite::Result<()> {
        self.done = true;
        self.conn.execute_batch("RELEASE lfm")
    }
}

impl std::ops::Deref for Atomic<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl Drop for Atomic<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.conn.execute_batch("ROLLBACK TO lfm; RELEASE lfm");
        }
    }
}

impl Library {
    pub(crate) fn atomic(&self) -> rusqlite::Result<Atomic<'_>> {
        self.conn.execute_batch("SAVEPOINT lfm")?;
        Ok(Atomic { conn: &self.conn, done: false })
    }

    /// Note qu'une tâche de fond doit tourner (ou plus) d'un lancement à l'autre.
    pub fn set_job_wanted(&self, name: &str, wanted: bool) -> Result<()> {
        let sql = if wanted { "INSERT OR IGNORE INTO job_wanted (name) VALUES (?)" } else { "DELETE FROM job_wanted WHERE name = ?" };
        self.conn.execute(sql, [name])?;
        Ok(())
    }

    /// Tâches interrompues par la fermeture de l'application.
    pub fn wanted_jobs(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT name FROM job_wanted")?;
        let r = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(r)
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(&format!("BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"))?;
        }
        if version < 2 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V2} PRAGMA user_version = 2; COMMIT;"))?;
        }
        if version < 3 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V3} PRAGMA user_version = 3; COMMIT;"))?;
        }
        if version < 4 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V4} PRAGMA user_version = 4; COMMIT;"))?;
        }
        if version < 5 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V5} PRAGMA user_version = 5; COMMIT;"))?;
        }
        if version < 6 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V6} PRAGMA user_version = 6; COMMIT;"))?;
        }
        if version < 7 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V7} PRAGMA user_version = 7; COMMIT;"))?;
        }
        if version < 8 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V8} PRAGMA user_version = 8; COMMIT;"))?;
        }
        if version < 9 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V9} PRAGMA user_version = 9; COMMIT;"))?;
        }
        if version < 10 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V10} PRAGMA user_version = 10; COMMIT;"))?;
        }
        if version < 11 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V11} PRAGMA user_version = 11; COMMIT;"))?;
        }
        if version < 12 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V12} PRAGMA user_version = 12; COMMIT;"))?;
        }
        if version < 13 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V13} PRAGMA user_version = 13; COMMIT;"))?;
        }
        if version < 14 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V14} PRAGMA user_version = 14; COMMIT;"))?;
        }
        if version < 15 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V15} PRAGMA user_version = 15; COMMIT;"))?;
        }
        if version < 16 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V16} PRAGMA user_version = 16; COMMIT;"))?;
        }
        if version < 17 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V17} PRAGMA user_version = 17; COMMIT;"))?;
        }
        if version < 18 {
            conn.execute_batch(&format!("BEGIN; {MIGRATION_V18} PRAGMA user_version = 18; COMMIT;"))?;
        }
        let lib = Self { conn, listened_at: Default::default() };
        if version < 19 {
            lib.conn.execute_batch("PRAGMA user_version = 19;")?;
        }
        if version < 20 {
            lib.conn.execute_batch(&format!("BEGIN; {MIGRATION_V20} PRAGMA user_version = 20; COMMIT;"))?;
        }
        if version < 21 {
            lib.conn.execute_batch(&format!("BEGIN; {MIGRATION_V21} PRAGMA user_version = 21; COMMIT;"))?;
        }
        if version < 22 {
            lib.conn.execute_batch(&format!("BEGIN; {MIGRATION_V22} PRAGMA user_version = 22; COMMIT;"))?;
        }
        // Règles de rapprochement changées depuis la dernière ouverture : clés recalculées, et
        // entités devenues identiques réunies (annulable, règle « spelling »). Base neuve : rien.
        let rules: Option<i64> =
            lib.conn.query_row("SELECT CAST(value AS INTEGER) FROM config WHERE key = 'rules_version'", [], |r| r.get(0)).optional()?;
        // Fuseau horaire changé depuis le dernier lancement : champs locaux des écoutes recalculés
        // (sinon un mois affiché et son détail ne compteraient pas les mêmes écoutes).
        let tz: Option<String> = lib.conn.query_row("SELECT value FROM config WHERE key = 'timezone'", [], |r| r.get(0)).optional()?;
        let here = crate::import::timezone_print();
        if tz.as_deref() != Some(here.as_str()) {
            if tz.is_some() {
                lib.refresh_local_fields()?;
            }
            lib.conn.execute("INSERT OR REPLACE INTO config (key, value) VALUES ('timezone', ?)", [&here])?;
        }
        if rules != Some(crate::normalize::RULES_VERSION) {
            if version >= 1 {
                lib.rekey()?;
                lib.rebuild_canonical()?;
            }
            lib.conn.execute(
                "INSERT OR REPLACE INTO config (key, value) VALUES ('rules_version', ?)",
                [crate::normalize::RULES_VERSION.to_string()],
            )?;
        }
        Ok(lib)
    }
}
