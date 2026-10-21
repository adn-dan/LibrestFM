//! Export de l'historique : toutes les sources, ou une sélection, fusionnées
//! (une écoute présente dans plusieurs sources n'est écrite qu'une fois).
//!
//! Les fichiers produits se réimportent tels quels, dans LibrestFM ou ailleurs :
//! - CSV à en-tête (`utc_time,artist,album,title,…`), heure UTC à la seconde, ou
//!   à la minute quand la source n'avait que la minute ;
//! - JSON Lines au format d'export ListenBrainz.

use crate::{Library, Result};
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryFormat {
    Csv,
    Listenbrainz,
}

struct Play {
    ts: i64,
    precise: bool,
    artist: String,
    title: String,
    album: Option<String>,
    track_mbid: Option<String>,
    artist_mbid: Option<String>,
    album_mbid: Option<String>,
}

impl Library {
    /// Écrit les écoutes des `sources` (toutes si vide), dans l'ordre chronologique.
    /// `corrected` : noms après nettoyage (éditions regroupées, noms de fichiers
    /// corrigés…) ; sinon tels qu'importés. Renvoie le nombre d'écoutes écrites.
    pub fn export_history(&self, sources: &[i64], corrected: bool, format: HistoryFormat, out: impl Write) -> Result<usize> {
        let (track, artist, album, album_title) = if corrected {
            ("s.track_id", "s.artist_id", "s.album_id", "COALESCE(al.display_title, al.title)")
        } else {
            ("COALESCE(s.raw_track_id, s.track_id)", "t.artist_id", "COALESCE(s.raw_album_id, s.album_id)", "al.title")
        };
        let filter = if sources.is_empty() {
            String::new()
        } else {
            let ids: Vec<String> = sources.iter().map(i64::to_string).collect();
            format!("WHERE EXISTS (SELECT 1 FROM scrobble_source x WHERE x.scrobble_id = s.id AND x.source_id IN ({}))", ids.join(","))
        };
        let mut st = self.conn.prepare(&format!(
            "SELECT s.ts, s.precise, a.name, t.title, {album_title}, t.mbid, a.mbid, al.mbid
             FROM scrobble s JOIN track t ON t.id = {track} JOIN artist a ON a.id = {artist}
             LEFT JOIN album al ON al.id = {album}
             {filter} ORDER BY s.ts, s.id"
        ))?;
        let rows = st.query_map([], |r| {
            Ok(Play {
                ts: r.get(0)?,
                precise: r.get(1)?,
                artist: r.get(2)?,
                title: r.get(3)?,
                album: r.get(4)?,
                track_mbid: r.get(5)?,
                artist_mbid: r.get(6)?,
                album_mbid: r.get(7)?,
            })
        })?;
        let mut out = std::io::BufWriter::new(out);
        let mut n = 0;
        match format {
            HistoryFormat::Csv => {
                let mut w = csv::Writer::from_writer(&mut out);
                w.write_record(["utc_time", "artist", "album", "title", "track_mbid", "artist_mbid", "album_mbid"]).map_err(csv_err)?;
                for p in rows {
                    let p = p?;
                    let when = Utc.timestamp_opt(p.ts, 0).single().unwrap_or_default();
                    let when = when.format(if p.precise { "%Y-%m-%d %H:%M:%S" } else { "%Y-%m-%d %H:%M" }).to_string();
                    let o = |x: &Option<String>| x.clone().unwrap_or_default();
                    w.write_record([when, p.artist, o(&p.album), p.title, o(&p.track_mbid), o(&p.artist_mbid), o(&p.album_mbid)]).map_err(csv_err)?;
                    n += 1;
                }
                w.flush()?;
            }
            HistoryFormat::Listenbrainz => {
                for p in rows {
                    let p = p?;
                    let mut info = serde_json::Map::new();
                    if let Some(m) = p.track_mbid {
                        info.insert("recording_mbid".into(), m.into());
                    }
                    if let Some(m) = p.artist_mbid {
                        info.insert("artist_mbids".into(), vec![m].into());
                    }
                    if let Some(m) = p.album_mbid {
                        info.insert("release_mbid".into(), m.into());
                    }
                    info.insert("submission_client".into(), "LibrestFM".into());
                    let mut meta = serde_json::json!({ "artist_name": p.artist, "track_name": p.title, "additional_info": info });
                    if let Some(a) = p.album {
                        meta["release_name"] = a.into();
                    }
                    serde_json::to_writer(&mut out, &serde_json::json!({ "listened_at": p.ts, "track_metadata": meta })).map_err(std::io::Error::other)?;
                    out.write_all(b"\n")?;
                    n += 1;
                }
            }
        }
        out.flush()?;
        Ok(n)
    }
}

fn csv_err(e: csv::Error) -> crate::Error {
    crate::Error::Io(std::io::Error::other(e))
}
