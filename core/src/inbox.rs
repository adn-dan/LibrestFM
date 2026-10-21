//! Dossier d'import : chaque fichier d'écoutes déposé (ou complété) dans le
//! dossier est importé au démarrage. Seuls les fichiers nouveaux ou modifiés
//! sont lus (taille et date retenues) ; un fichier qui grossit (journal
//! d'écoutes complété au fil de l'eau) reste une seule source et n'apporte que
//! ses nouvelles lignes.

use crate::import::{ImportReport, ImportStatus, decode, formats, ingest_into};
use crate::{Library, Result};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Extensions lues ; le contenu décide ensuite du format.
const EXTENSIONS: [&str; 7] = ["csv", "tsv", "txt", "json", "jsonl", "ndjson", "log"];
/// Fichier modifié depuis moins longtemps : peut-être encore en cours d'écriture.
const WRITING: std::time::Duration = std::time::Duration::from_secs(60);
/// Taille maximale d'un fichier lu (un export de plusieurs centaines de milliers d'écoutes
/// tient en quelques dizaines de Mo).
const MAX_FILE: u64 = 512 * 1024 * 1024;
/// Profondeur de sous-dossiers parcourue.
const DEPTH: usize = 3;

#[derive(Debug, Clone, Default, Serialize)]
pub struct InboxReport {
    pub dir: String,
    /// Fichiers nouveaux ou modifiés, et ce qu'ils ont apporté.
    pub files: Vec<ImportReport>,
    /// Fichiers déjà lus et inchangés.
    pub unchanged: usize,
    /// Corrections automatiques appliquées ensuite.
    pub cleaned: usize,
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| EXTENSIONS.contains(&x.to_string_lossy().to_lowercase().as_str()))
            // Fichiers ordinaires de taille raisonnable seulement : un tube nommé bloquerait la
            // lecture, un lien vers /dev/zero remplirait la mémoire.
            && std::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE)
        {
            out.push(p);
        }
    }
}

impl Library {
    /// Dossier d'import choisi (None : celui par défaut).
    pub fn input_dir(&self) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM config WHERE key = 'input_dir'", [], |r| r.get(0)).optional()?)
    }

    pub fn set_input_dir(&self, dir: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO config (key, value) VALUES ('input_dir', ?)", [dir])?;
        Ok(())
    }

    /// Importe les fichiers nouveaux ou modifiés du dossier, puis nettoie.
    pub fn scan_input(&mut self, dir: &Path) -> Result<InboxReport> {
        let mut files = Vec::new();
        walk(dir, DEPTH, &mut files);
        files.sort();
        let mut rep = InboxReport { dir: dir.display().to_string(), ..Default::default() };
        for path in files {
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let size = meta.len() as i64;
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            let key = path.display().to_string();
            let known: Option<(i64, i64, Option<i64>, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT f.size, f.mtime, s.id, f.sha FROM input_file f LEFT JOIN source s ON s.id = f.source_id WHERE f.path = ?",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            // Inchangé : rien à relire. Y compris quand sa source a été retirée : le
            // fichier ne revient que s'il change.
            if known.as_ref().is_some_and(|k| k.0 == size && k.1 == mtime) {
                rep.unchanged += 1;
                continue;
            }
            let Ok(mut bytes) = std::fs::read(&path) else { continue };
            // Modifié à l'instant, peut-être en cours d'écriture : la dernière ligne, sans saut
            // de ligne, attend le prochain passage (lue tronquée, elle entrerait avec un titre
            // coupé, puis serait sautée une fois complétée). Plus ancien : fichier complet,
            // même sans saut de ligne final (cas courant des exports).
            let fresh = meta.modified().ok().and_then(|t| t.elapsed().ok()).is_none_or(|age| age < WRITING);
            if fresh && let Some(end) = bytes.iter().rposition(|&b| b == b'\n') {
                bytes.truncate(end + 1);
            }
            let size = bytes.len() as i64;
            let name = path.file_name().map_or_else(|| key.clone(), |n| n.to_string_lossy().into_owned());
            let report = match known.as_ref().and_then(|k| k.2.map(|s| (k.0, s, k.3.as_deref()))) {
                // Déjà lu, depuis modifié : dans la même source. Complété à la fin
                // seulement si l'ancien contenu en est exactement le début.
                Some((old_size, source, old_sha)) => {
                    let old = &bytes[..(old_size as usize).min(bytes.len())];
                    let appended = old.len() == old_size as usize && old_sha.is_some_and(|sha| format!("{:x}", Sha256::digest(old)) == sha);
                    self.reingest(&name, &bytes, source, appended.then_some(old))?
                }
                None => self.import_bytes(&name, &bytes)?,
            };
            self.conn.execute(
                "INSERT OR REPLACE INTO input_file (path, size, mtime, source_id, sha) VALUES (?, ?, ?, ?, ?)",
                params![key, size, mtime, report.source_id, format!("{:x}", Sha256::digest(&bytes))],
            )?;
            if !matches!(report.status, ImportStatus::AlreadyImported) {
                rep.files.push(report);
            }
        }
        if rep.files.iter().any(|f| f.added > 0) {
            rep.cleaned = self.auto_clean()?;
        }
        Ok(rep)
    }

    /// Relit un fichier déjà importé dans sa source. `old` : l'ancien contenu,
    /// quand il est exactement le début du nouveau : seules les lignes ajoutées
    /// après lui sont lues. Sinon tout le fichier (les écoutes déjà liées à cette
    /// source sont reconnues).
    fn reingest(&mut self, name: &str, bytes: &[u8], source: i64, old: Option<&[u8]>) -> Result<ImportReport> {
        let text = decode(bytes);
        let Some((format, rows)) = formats::parse(&text) else {
            return Ok(ImportReport::empty(name, ImportStatus::Unrecognized));
        };
        let already = old.and_then(|o| formats::parse(&decode(o))).map_or(0, |(_, r)| r.len());
        let rows: Vec<formats::Row> = rows.into_iter().skip(already).collect();
        let sha = format!("{:x}", Sha256::digest(bytes));
        let tx = self.conn.transaction()?;
        let rep = ingest_into(&tx, source, name, format.code(), rows)?;
        // Empreinte à jour : réimporter ce même contenu à la main sera reconnu.
        tx.execute("UPDATE OR IGNORE source SET sha256 = ? WHERE id = ?", params![sha, source])?;
        tx.commit()?;
        Ok(rep)
    }
}
