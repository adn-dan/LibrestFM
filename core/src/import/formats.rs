//! Reconnaissance et lecture des formats d'export. Chaque ligne donne soit une
//! écoute brute, soit un motif de rejet : rien n'est perdu silencieusement.

use super::{RawScrobble, Reject};
use crate::normalize::clean;
use chrono::{DateTime, NaiveDateTime};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// lastfm-to-csv (benjaminbenben) : artiste,album,titre,date sans en-tête.
    LastfmCsv,
    /// lastexport.py / last2libre (Last.fm, Libre.fm) : epoch\ttitre\tartiste\talbum\tmbids.
    LastExportTsv,
    /// CSV ou TSV avec en-tête : colonnes reconnues par leur nom.
    HeaderCsv,
    ListenBrainz,
    /// Historique étendu Spotify (Streaming_History_Audio_*.json).
    SpotifyExtended,
    /// Historique de compte Spotify (StreamingHistory*.json, sans album).
    SpotifyAccount,
}

impl Format {
    /// Code stable, enregistré en base et traduit par l'interface.
    pub fn code(self) -> &'static str {
        match self {
            Format::LastfmCsv => "lastfm_csv",
            Format::LastExportTsv => "last_export_tsv",
            Format::HeaderCsv => "header_csv",
            Format::ListenBrainz => "listen_brainz",
            Format::SpotifyExtended => "spotify_extended",
            Format::SpotifyAccount => "spotify_account",
        }
    }
}

pub type Row = (usize, Result<RawScrobble, Reject>);

/// En dessous, Spotify compte un passage, pas une écoute (même seuil que Last.fm).
const MIN_PLAY_MS: i64 = 30_000;

pub fn parse(text: &str) -> Option<(Format, Vec<Row>)> {
    let head = text.trim_start();
    if head.starts_with('[') || head.starts_with('{') {
        return parse_json(head);
    }
    parse_delimited(text)
}

// ---------- dates ----------

/// Renvoie (secondes Unix UTC, précision à la seconde).
pub fn parse_date(s: &str) -> Option<(i64, bool)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        let n: i64 = s.parse().ok()?;
        return Some(if s.len() >= 12 { (n / 1000, true) } else { (n, true) });
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some((d.timestamp(), true));
    }
    for f in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y/%m/%d %H:%M:%S"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Some((d.and_utc().timestamp(), true));
        }
    }
    // Sans fuseau : les exports Last.fm sont en UTC.
    for f in ["%d %b %Y %H:%M", "%d %b %Y, %H:%M", "%d %B %Y %H:%M", "%d %B %Y, %H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Some((d.and_utc().timestamp(), false));
        }
    }
    None
}

pub(crate) fn build(
    ts: Option<(i64, bool)>,
    artist: Option<&str>,
    title: Option<&str>,
    album: Option<&str>,
    mbids: [Option<&str>; 3],
) -> Result<RawScrobble, Reject> {
    let artist = artist.and_then(clean).ok_or(Reject::MissingArtist)?;
    let title = title.and_then(clean).ok_or(Reject::MissingTitle)?;
    if crate::normalize::is_recording_error(&title) {
        return Err(Reject::RecordingError);
    }
    let (ts, precise) = ts.ok_or(Reject::BadDate)?;
    let mbid = |m: Option<&str>| m.and_then(clean).filter(|m| m.len() == 36);
    Ok(RawScrobble {
        ts,
        precise,
        artist,
        title,
        album: album.and_then(clean),
        track_mbid: mbid(mbids[0]),
        artist_mbid: mbid(mbids[1]),
        album_mbid: mbid(mbids[2]),
    })
}

// ---------- CSV / TSV ----------

fn sniff_delimiter(first_line: &str) -> u8 {
    if first_line.contains('\t') {
        return b'\t';
    }
    let commas = first_line.matches(',').count();
    let semis = first_line.matches(';').count();
    if semis > commas { b';' } else { b',' }
}

#[derive(Default)]
struct Columns {
    artist: Option<usize>,
    title: Option<usize>,
    album: Option<usize>,
    date: Option<usize>,
    date_rank: u8,
    track_mbid: Option<usize>,
    artist_mbid: Option<usize>,
    album_mbid: Option<usize>,
}

fn map_header(cells: &[String]) -> Option<Columns> {
    let mut c = Columns::default();
    for (i, raw) in cells.iter().enumerate() {
        let h: String = raw.trim().trim_start_matches('\u{feff}').to_lowercase().replace([' ', '-', '.'], "_");
        match h.as_str() {
            "artist" | "artist_name" | "artistname" | "artiste" | "interprete" | "master_metadata_album_artist_name" => {
                c.artist.get_or_insert(i);
            }
            "track" | "title" | "track_name" | "trackname" | "song" | "titre" | "name" | "recording_name" => {
                c.title.get_or_insert(i);
            }
            "album" | "album_name" | "release" | "release_name" | "albumname" => {
                c.album.get_or_insert(i);
            }
            "track_mbid" | "recording_mbid" | "mbid" => {
                c.track_mbid.get_or_insert(i);
            }
            "artist_mbid" | "artist_mbids" => {
                c.artist_mbid.get_or_insert(i);
            }
            "album_mbid" | "release_mbid" => {
                c.album_mbid.get_or_insert(i);
            }
            _ => {
                // Plusieurs colonnes de date possibles : on garde la plus fiable (epoch d'abord).
                let rank = match h.as_str() {
                    "uts" | "date_uts" | "timestamp" | "listened_at" | "unix" | "unix_timestamp" => 3,
                    "utc_time" | "played_at" | "scrobble_time" | "datetime" | "ts" => 2,
                    "date" | "time" | "date_time" | "date_text" | "heure" => 1,
                    _ => 0,
                };
                if rank > c.date_rank {
                    c.date = Some(i);
                    c.date_rank = rank;
                }
            }
        }
    }
    (c.artist.is_some() && c.title.is_some() && c.date.is_some()).then_some(c)
}

fn parse_delimited(text: &str) -> Option<(Format, Vec<Row>)> {
    let first = text.lines().find(|l| !l.trim().is_empty())?;
    let delim = sniff_delimiter(first);
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(false)
        .flexible(true)
        .quoting(delim != b'\t')
        .from_reader(text.as_bytes());

    let mut records = rdr.records();
    let mut first_rec = None;
    let mut first_line = 0;
    for r in records.by_ref() {
        let r = r.ok()?;
        first_line = r.position().map_or(1, |p| p.line() as usize);
        if r.iter().any(|c| !c.trim().is_empty()) {
            first_rec = Some(r);
            break;
        }
    }
    let first_rec = first_rec?;
    let cells: Vec<String> = first_rec.iter().map(str::to_string).collect();

    let (format, cols, header) = if let Some(cols) = map_header(&cells) {
        (Format::HeaderCsv, cols, true)
    } else if delim == b'\t' && cells.len() >= 4 && cells[0].trim().bytes().all(|b| b.is_ascii_digit()) {
        let cols = Columns {
            date: Some(0), title: Some(1), artist: Some(2), album: Some(3),
            track_mbid: Some(4), artist_mbid: Some(5), album_mbid: Some(6), ..Default::default()
        };
        (Format::LastExportTsv, cols, false)
    } else if cells.len() >= 4 && parse_date(&cells[3]).is_some() {
        let cols = Columns { artist: Some(0), album: Some(1), title: Some(2), date: Some(3), ..Default::default() };
        (Format::LastfmCsv, cols, false)
    } else {
        return None;
    };

    fn get(r: &csv::StringRecord, i: Option<usize>) -> Option<&str> {
        i.and_then(|i| r.get(i)).map(str::trim)
    }
    let convert = |line: usize, r: &csv::StringRecord| -> Row {
        let ts = get(r, cols.date).and_then(parse_date);
        let raw = build(
            ts,
            get(r, cols.artist),
            get(r, cols.title),
            get(r, cols.album),
            [get(r, cols.track_mbid), get(r, cols.artist_mbid), get(r, cols.album_mbid)],
        );
        (line, raw)
    };

    let mut rows = Vec::new();
    if !header {
        rows.push(convert(first_line, &first_rec));
    }
    for r in records {
        match r {
            Ok(r) => {
                if r.iter().all(|c| c.trim().is_empty()) {
                    continue;
                }
                let line = r.position().map_or(0, |p| p.line() as usize);
                rows.push(convert(line, &r));
            }
            Err(e) => rows.push((e.position().map_or(0, |p| p.line() as usize), Err(Reject::Malformed))),
        }
    }
    Some((format, rows))
}

// ---------- JSON ----------

fn json_format(probe: &Value) -> Option<Format> {
    if probe.get("master_metadata_track_name").is_some() || probe.get("ms_played").is_some() {
        Some(Format::SpotifyExtended)
    } else if probe.get("endTime").is_some() && probe.get("trackName").is_some() {
        Some(Format::SpotifyAccount)
    } else if probe.get("listened_at").is_some() || probe.get("track_metadata").is_some() {
        Some(Format::ListenBrainz)
    } else {
        None
    }
}

/// Lignes d'un tableau JSON lu élément par élément : chaque écoute est convertie puis
/// oubliée, au lieu de garder tout le document en arbre (des centaines de Mo pour un gros
/// export Spotify, de quoi faire tomber l'appli sur un téléphone).
struct StreamedRows(Option<(Format, Vec<Row>)>);

impl<'de> serde::de::Visitor<'de> for &mut StreamedRows {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un tableau d'écoutes")
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        let mut format = None;
        let mut rows = Vec::new();
        let mut i = 0;
        while let Some(v) = seq.next_element::<Value>()? {
            i += 1;
            if format.is_none() && v.is_object() {
                match json_format(&v) {
                    Some(f) => format = Some(f),
                    None => return Err(serde::de::Error::custom("format")),
                }
            }
            match format {
                Some(f) => rows.push((i, json_row(f, &v))),
                None => rows.push((i, Err(Reject::Malformed))),
            }
        }
        self.0 = format.map(|f| (f, rows));
        Ok(())
    }
}

fn parse_json(text: &str) -> Option<(Format, Vec<Row>)> {
    if text.starts_with('[') {
        let mut out = StreamedRows(None);
        let mut de = serde_json::Deserializer::from_str(text);
        if serde::Deserializer::deserialize_seq(&mut de, &mut out).is_ok() && de.end().is_ok() {
            return out.0;
        }
        // Tableau illisible d'un bloc : peut-être du JSON Lines dont la 1re ligne est un tableau.
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(o)) => {
            // Réponse d'API ListenBrainz : {"payload": {"listens": [...]}}
            let listens = o.get("payload").and_then(|p| p.get("listens")).or_else(|| o.get("listens"));
            let items: Vec<Value> = match listens {
                Some(Value::Array(a)) => a.clone(),
                _ => vec![Value::Object(o)],
            };
            let format = json_format(items.iter().find(|v| v.is_object())?)?;
            Some((format, items.iter().enumerate().map(|(i, v)| (i + 1, json_row(format, v))).collect()))
        }
        Ok(_) => None,
        // Pas un document JSON unique : JSON Lines (export ListenBrainz), lu ligne par ligne.
        Err(_) => {
            let mut format = None;
            let mut rows = Vec::new();
            for (i, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
                let v: Value = serde_json::from_str(line).unwrap_or(Value::Null);
                if format.is_none() && v.is_object() {
                    format = Some(json_format(&v)?);
                }
                rows.push((i + 1, format.map_or(Err(Reject::Malformed), |f| json_row(f, &v))));
            }
            format.map(|f| (f, rows))
        }
    }
}

fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_str()
}

pub(crate) fn json_row(format: Format, v: &Value) -> Result<RawScrobble, Reject> {
    if !v.is_object() {
        return Err(Reject::Malformed);
    }
    match format {
        Format::SpotifyExtended => {
            let title = s(v, &["master_metadata_track_name"]);
            if title.is_none() {
                return Err(Reject::NotMusic); // podcast, livre audio
            }
            let ms = v.get("ms_played").and_then(Value::as_i64).unwrap_or(0);
            if ms < MIN_PLAY_MS {
                return Err(Reject::TooShort);
            }
            // Spotify date la fin de l'écoute ; Last.fm et ListenBrainz le début.
            let ts = s(v, &["ts"]).and_then(parse_date).map(|(t, p)| (t - ms / 1000, p));
            build(ts, s(v, &["master_metadata_album_artist_name"]), title, s(v, &["master_metadata_album_album_name"]), [None; 3])
        }
        Format::SpotifyAccount => {
            let ms = v.get("msPlayed").and_then(Value::as_i64).unwrap_or(0);
            if ms < MIN_PLAY_MS {
                return Err(Reject::TooShort);
            }
            let ts = s(v, &["endTime"]).and_then(parse_date).map(|(t, _)| (t - ms / 1000, false));
            build(ts, s(v, &["artistName"]), s(v, &["trackName"]), None, [None; 3])
        }
        Format::ListenBrainz => {
            let ts = v.get("listened_at").and_then(|t| match t {
                Value::Number(n) => n.as_i64().map(|n| (n, true)),
                Value::String(t) => parse_date(t),
                _ => None,
            });
            let m = v.get("track_metadata").unwrap_or(&Value::Null);
            let info = m.get("additional_info").unwrap_or(&Value::Null);
            let map = m.get("mbid_mapping").unwrap_or(&Value::Null);
            let first_artist_mbid = |x: &'_ Value| x.get("artist_mbids").and_then(|a| a.get(0)).and_then(Value::as_str).map(str::to_owned);
            let artist_mbid = first_artist_mbid(info).or_else(|| first_artist_mbid(map));
            build(
                ts,
                s(m, &["artist_name"]),
                s(m, &["track_name"]),
                s(m, &["release_name"]),
                [
                    s(info, &["recording_mbid"]).or(s(map, &["recording_mbid"])),
                    artist_mbid.as_deref(),
                    s(info, &["release_mbid"]).or(s(map, &["release_mbid"])),
                ],
            )
        }
        _ => Err(Reject::Malformed),
    }
}
