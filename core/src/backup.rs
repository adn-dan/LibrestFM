//! Sauvegarde et restauration de la base : tout l'historique, les corrections
//! du nettoyage, l'enrichissement et les pochettes tiennent dans un seul fichier.

use crate::db::SCHEMA_VERSION;
use crate::{Error, Library, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub plays: i64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub bytes: u64,
}

/// Nom de la copie faite juste avant une restauration.
pub fn safety_name() -> String {
    format!("avant-restauration-{}.sqlite", chrono::Local::now().format("%Y-%m-%d-%H%M%S"))
}

/// Préfixe des sauvegardes automatiques : librestfm-AAAA-MM-JJ.sqlite.
const AUTO_PREFIX: &str = "librestfm-";

fn invalid(code: &'static str) -> Error {
    Error::Invalid(code)
}

/// Chemin absolu sans liens, même pour un fichier qui n'existe pas encore.
fn canonical(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    match (p.parent().and_then(|d| d.canonicalize().ok()), p.file_name()) {
        (Some(d), Some(f)) => d.join(f),
        _ => p.to_path_buf(),
    }
}

/// Vrai si `dest` est le fichier de la base `db` ou l'un de ses journaux :
/// l'écraser effacerait l'historique.
pub fn is_database_file(dest: &Path, db: &Path) -> bool {
    let dest = canonical(dest);
    let db = canonical(db);
    let name = db.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    [db.clone(), db.with_file_name(format!("{name}-wal")), db.with_file_name(format!("{name}-shm"))].contains(&dest)
}

impl Library {
    /// Refuse d'écrire par-dessus la base ouverte (sauvegarde, exports).
    pub fn check_destination(&self, dest: &Path) -> Result<()> {
        match self.conn.path().filter(|p| !p.is_empty()) {
            Some(db) if is_database_file(dest, Path::new(db)) => Err(Error::Invalid("own_database")),
            _ => Ok(()),
        }
    }

    /// Copie compacte et cohérente de la base (même pendant son utilisation).
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        self.check_destination(dest)?;
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Écrite à côté puis renommée : si l'écriture échoue (disque plein, clé retirée),
        // l'ancienne sauvegarde du même nom reste intacte.
        let mut part = dest.as_os_str().to_owned();
        part.push(".part");
        let part = std::path::PathBuf::from(part);
        let _ = std::fs::remove_file(&part);
        if let Err(e) = self.conn.execute("VACUUM INTO ?", [part.to_string_lossy()]) {
            let _ = std::fs::remove_file(&part);
            return Err(e.into());
        }
        std::fs::rename(&part, dest)?;
        Ok(())
    }

    /// Vérifie qu'un fichier est une base LibrestFM lisible, et la résume.
    pub fn check_backup(path: &Path) -> Result<BackupInfo> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|_| invalid("unreadable"))?;
        let ok: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(|_| invalid("not_sqlite"))?;
        if ok != "ok" {
            return Err(invalid("damaged"));
        }
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(invalid("newer"));
        }
        // Base SQLite qui n'a jamais été une base LibrestFM.
        if version < 1 {
            return Err(invalid("not_librestfm"));
        }
        let (plays, first_ts, last_ts) = conn
            .query_row("SELECT COUNT(*), MIN(ts), MAX(ts) FROM scrobble", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|_| invalid("not_librestfm"))?;
        Ok(BackupInfo { plays, first_ts, last_ts, bytes: std::fs::metadata(path)?.len() })
    }

    /// Remplace la base `db_path` (celle ouverte) par la sauvegarde `src`. La base
    /// actuelle est d'abord copiée dans `safety`, pour pouvoir revenir en arrière.
    ///
    /// La sauvegarde n'est jamais lue en place (aucun fichier créé à côté d'elle) : elle est
    /// copiée avec son journal WAL éventuel, ouverte comme une base LibrestFM (une base SQLite
    /// étrangère échoue ici, avant tout échange), puis réécrite d'un bloc (journal intégré).
    pub fn restore_from(&mut self, src: &Path, db_path: &Path, safety: &Path) -> Result<BackupInfo> {
        let dir = db_path.parent().unwrap_or(Path::new("."));
        let staged = dir.join("restore-src.sqlite");
        let ready = dir.join("restore-ready.sqlite");
        let cleanup = || {
            for p in [&staged, &ready] {
                for suffix in ["", "-wal", "-shm"] {
                    let mut f = p.as_os_str().to_owned();
                    f.push(suffix);
                    let _ = std::fs::remove_file(PathBuf::from(f));
                }
            }
        };
        cleanup();
        let prepared = (|| -> Result<BackupInfo> {
            std::fs::copy(src, &staged)?;
            let mut wal = src.as_os_str().to_owned();
            wal.push("-wal");
            if Path::new(&wal).exists() {
                let mut to = staged.as_os_str().to_owned();
                to.push("-wal");
                std::fs::copy(&wal, PathBuf::from(to))?;
            }
            Self::check_backup(&staged)?;
            let candidate = Library::open(&staged).map_err(|_| invalid("not_librestfm"))?;
            candidate.conn.execute("VACUUM INTO ?", [ready.to_string_lossy()])?;
            drop(candidate);
            Self::check_backup(&ready)
        })();
        let info = match prepared {
            Ok(i) => i,
            Err(e) => {
                cleanup();
                return Err(e);
            }
        };
        self.backup_to(safety)?;
        // Fermer la base (le journal WAL est alors intégré au fichier), puis la remplacer.
        drop(std::mem::replace(&mut self.conn, Connection::open_in_memory()?));
        for ext in ["sqlite-wal", "sqlite-shm"] {
            let _ = std::fs::remove_file(db_path.with_extension(ext));
        }
        let swap = std::fs::rename(&ready, db_path);
        cleanup();
        match Library::open(db_path) {
            Ok(lib) => *self = lib,
            // Ne devrait pas arriver (déjà ouverte une fois) : la base d'avant est remise.
            Err(e) => {
                std::fs::copy(safety, db_path)?;
                *self = Library::open(db_path)?;
                return Err(e);
            }
        }
        swap?;
        Ok(info)
    }

    /// Sauvegarde automatique dans `dir` si la dernière a plus de `days` jours ;
    /// garde les `keep` plus récentes. Renvoie le fichier écrit.
    pub fn auto_backup(&self, dir: &Path, days: i64, keep: usize) -> Result<Option<PathBuf>> {
        let mut existing: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(AUTO_PREFIX))).collect())
            .unwrap_or_default();
        existing.sort();
        let today = chrono::Local::now().date_naive();
        let newest = existing.last().and_then(|p| {
            let name = p.file_stem()?.to_string_lossy().strip_prefix(AUTO_PREFIX)?.to_owned();
            chrono::NaiveDate::parse_from_str(&name, "%Y-%m-%d").ok()
        });
        if newest.is_some_and(|d| (today - d).num_days() < days) {
            return Ok(None);
        }
        let dest = dir.join(format!("{AUTO_PREFIX}{}.sqlite", today.format("%Y-%m-%d")));
        self.backup_to(&dest)?;
        existing.push(dest.clone());
        existing.sort();
        existing.dedup();
        for old in existing.iter().rev().skip(keep) {
            let _ = std::fs::remove_file(old);
        }
        Ok(Some(dest))
    }

    /// Sauvegardes automatiques présentes (la plus récente d'abord).
    pub fn auto_backups(dir: &Path) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = std::fs::read_dir(dir)
            .map(|r| {
                r.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(AUTO_PREFIX))
                    .map(|e| (e.path().display().to_string(), e.metadata().map_or(0, |m| m.len())))
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| b.0.cmp(&a.0));
        v
    }
}
