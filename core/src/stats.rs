//! Requêtes d'analyse. Tout est calculé à la volée en SQL sur la table des
//! écoutes ; les index (ts, track_id) suffisent pour quelques centaines de
//! milliers d'écoutes.

use crate::normalize::fold;
use crate::{Error, Library, Result};
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

/// Écart maximal entre deux débuts d'écoute pour les considérer « à la suite ».
pub const SESSION_GAP: i64 = 30 * 60;

/// Durée supposée d'un titre dont on ne connaît pas la durée (secondes).
const DEFAULT_TRACK_S: i64 = 210;

/// Temps d'écoute estimé d'une écoute, en SQL, à partir de `dur` (durée du
/// titre) et `gap` (secondes jusqu'à l'écoute suivante, NULL s'il n'y en a pas).
/// Un scrobble n'est envoyé qu'après la moitié du titre (ou 4 min) : c'est le
/// minimum `lo`. Si l'écoute suivante démarre avant la fin, on s'arrête là ;
/// sinon le titre est compté en entier.
const LO: &str = "MIN(dur / 2, 240)";
fn estimate() -> String {
    format!("MIN(dur, MAX({LO}, COALESCE(gap, dur)))")
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Period {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl Period {
    fn bounds(&self) -> (i64, i64) {
        (self.from.unwrap_or(i64::MIN), self.to.unwrap_or(i64::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Artist,
    Album,
    Track,
}

impl Kind {
    /// Expression SQL de l'identifiant de l'entité pour une écoute `s`.
    fn col(self) -> &'static str {
        match self {
            Kind::Artist => "s.artist_id",
            Kind::Album => "s.album_id",
            Kind::Track => "s.track_id",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Ranked {
    pub kind: Kind,
    pub id: i64,
    pub name: String,
    /// Artiste, pour un album ou un titre.
    pub artist: Option<String>,
    pub artist_id: Option<i64>,
    pub plays: i64,
    /// Temps d'écoute estimé, quand le classement est fait au temps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub total: i64,
    pub artists: i64,
    pub albums: i64,
    pub tracks: i64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    /// Temps d'écoute estimé et son minimum certain (secondes), part des écoutes de durée connue.
    pub listened_s: i64,
    pub listened_min_s: i64,
    pub listened_known: f64,
    /// ("2024-03", écoutes), mois sans écoute inclus.
    pub per_month: Vec<(String, i64)>,
    /// ("2024-03-10", écoutes), jours sans écoute inclus.
    pub per_day: Vec<(String, i64)>,
    pub top_artists: Vec<Ranked>,
    pub top_albums: Vec<Ranked>,
    pub top_tracks: Vec<Ranked>,
    /// [jour (0 = lundi), heure, écoutes], heure locale.
    pub heatmap: Vec<[i64; 3]>,
    /// (année de sortie de l'album, écoutes).
    pub release_years: Vec<(i64, i64)>,
    /// Part des écoutes dont l'année de sortie est connue.
    pub release_covered: f64,
    /// (code pays ISO, écoutes, artistes).
    pub countries: Vec<(String, i64, i64)>,
    pub countries_covered: f64,
    /// (langue ISO 639-3, écoutes) ; un titre multilingue compte pour chacune de ses langues.
    pub languages: Vec<(String, i64)>,
    pub languages_covered: f64,
}

/// Une session : des écoutes qui s'enchaînent avec moins de `SESSION_GAP` d'écart.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SessionStats {
    pub count: i64,
    pub avg_tracks: f64,
    /// Durée médiane estimée (début de la première écoute → fin de la dernière).
    pub median_minutes: f64,
    pub longest: Option<LongestSession>,
    /// (tranche de nombre de titres : « 1 », « 2-3 », … « 21+ », sessions).
    pub lengths: Vec<(String, i64)>,
    /// Sessions commencées à chaque heure locale (0 à 23).
    pub start_hours: Vec<i64>,
    /// Artistes qui ouvrent / ferment le plus souvent une session (sessions d'au moins 3 titres).
    pub openers: Vec<Ranked>,
    pub closers: Vec<Ranked>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LongestSession {
    pub start_ts: i64,
    pub tracks: i64,
    pub minutes: i64,
}

/// Un artiste découvert (ou lâché) : premières (ou dernières) écoutes.
#[derive(Debug, Clone, Serialize)]
pub struct Turn {
    pub artist: Ranked,
    pub first_ts: i64,
    pub last_ts: i64,
    /// Écoutes dans les 3 mois après la découverte, ou dans les 6 mois avant l'abandon.
    pub burst: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Discoveries {
    /// ("2024-03", nouveaux artistes écoutés au moins 5 fois en tout).
    pub per_month: Vec<(String, i64)>,
    /// Artistes découverts dans la période et beaucoup écoutés dans les 3 mois suivants.
    pub found: Vec<Turn>,
    /// Artistes beaucoup écoutés puis plus du tout depuis 6 mois, arrêtés dans la période.
    pub dropped: Vec<Turn>,
}

/// Une période dans une comparaison.
#[derive(Debug, Clone, Serialize)]
pub struct Side {
    pub plays: i64,
    pub listened_s: i64,
    pub artists: i64,
    /// Artistes écoutés pour la toute première fois dans la période.
    pub new_artists: i64,
    /// Artistes les plus écoutés, avec leur rang dans l'autre période (None : absent).
    pub top_artists: Vec<(Ranked, Option<usize>)>,
    /// Part des écoutes à chaque heure locale (0 à 23), en %.
    pub hours: Vec<f64>,
    /// Caractéristiques audio moyennes (dans l'ordre de `AXES`), pondérées par les écoutes.
    pub sound: Option<[Option<f64>; 7]>,
    /// Écoutes ayant des caractéristiques audio.
    pub sound_basis: i64,
}

/// Un artiste dont la part d'écoute a le plus changé d'une période à l'autre.
#[derive(Debug, Clone, Serialize)]
pub struct Shift {
    pub artist: Ranked,
    pub plays_a: i64,
    /// Parts des écoutes de chaque période, en %.
    pub share_a: f64,
    pub share_b: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub a: Side,
    pub b: Side,
    pub gained: Vec<Shift>,
    pub lost: Vec<Shift>,
}

/// Filtre du panneau de détail : toutes les conditions présentes s'appliquent ensemble.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Filter {
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Restreint à un artiste, un album ou un titre.
    pub kind: Option<Kind>,
    pub id: Option<i64>,
    /// Pays d'origine de l'artiste (code ISO, « FR »).
    pub country: Option<String>,
    /// Langue des paroles (ISO 639-3, « fra »).
    pub language: Option<String>,
    /// Années de sortie de l'album, bornes incluses.
    pub release_from: Option<i64>,
    pub release_to: Option<i64>,
    /// Jour de la semaine (0 = lundi) et heure locale.
    pub dow: Option<i64>,
    pub hour: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detail {
    pub total: i64,
    pub artists: i64,
    pub albums: i64,
    pub tracks: i64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub top_artists: Vec<Ranked>,
    pub top_albums: Vec<Ranked>,
    pub top_tracks: Vec<Ranked>,
    pub per_month: Vec<(String, i64)>,
    pub per_day: Vec<(String, i64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumRelease {
    pub album: Ranked,
    pub release_date: String,
}

pub const AXES: [&str; 7] = ["danceability", "energy", "valence", "acousticness", "instrumentalness", "speechiness", "liveness"];

#[derive(Debug, Clone, Default, Serialize)]
pub struct Features {
    pub status: String,
    pub provider: String,
    pub matched_title: Option<String>,
    pub matched_artist: Option<String>,
    pub isrc: Option<String>,
    /// Dans l'ordre de `AXES`.
    pub axes: [Option<f64>; 7],
    pub tempo: Option<f64>,
    pub loudness: Option<f64>,
    pub key: Option<i64>,
    pub mode: Option<i64>,
}

/// Ce que représente un profil du radar ; l'interface en fait un libellé traduit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    /// L'entité affichée elle-même (titre, album ou artiste).
    #[serde(rename = "self")]
    Itself,
    /// Moyenne de l'album du titre.
    Album,
    /// Moyenne de l'artiste.
    Artist,
    /// Toute la bibliothèque, pondérée par les écoutes.
    Library,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub kind: ProfileKind,
    /// Nom de l'album ou de l'artiste moyenné.
    pub name: Option<String>,
    /// Nombre d'écoutes (ou de titres) ayant des caractéristiques.
    pub basis: i64,
    pub axes: [Option<f64>; 7],
    /// Percentile de chaque valeur parmi les titres de la bibliothèque.
    pub percentiles: [Option<f64>; 7],
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityDetail {
    pub kind: Kind,
    pub id: i64,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<i64>,
    pub total: i64,
    /// Temps d'écoute estimé (secondes).
    pub listened_s: i64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub per_month: Vec<(String, i64)>,
    pub per_day: Vec<(String, i64)>,
    /// Titres d'un artiste ou d'un album ; albums d'un titre.
    pub top_tracks: Vec<Ranked>,
    pub top_albums: Vec<Ranked>,
    pub before: Vec<Ranked>,
    pub after: Vec<Ranked>,
    /// Pour un titre seulement.
    pub features: Option<Features>,
    /// Durée du titre, ou de l'album en entier.
    pub duration_ms: Option<i64>,
    /// Album affiché, ou album principal du titre.
    pub album_info: Option<crate::info::AlbumInfo>,
    /// Artiste affiché, ou artiste du titre ou de l'album.
    pub artist_info: Option<crate::info::ArtistInfo>,
    /// Langues des paroles (ISO 639-3) d'un titre.
    pub languages: Vec<String>,
    /// Autres versions d'un titre (live, remix…) chez le même artiste.
    pub versions: Vec<Ranked>,
    /// Titre : artistes crédités par MusicBrainz, dans l'ordre (vide si un seul).
    pub artist_credit: Vec<crate::credits::CreditedArtist>,
    /// Fusions dont cette entité est la cible.
    pub merged: Vec<crate::aliases::MergeOp>,
    /// Le titre lui-même, la moyenne de l'artiste (pondérée par tes écoutes), ton profil global.
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub artists: Vec<Ranked>,
    pub albums: Vec<Ranked>,
    pub tracks: Vec<Ranked>,
}

/// Année de sortie d'une écoute : la plus ancienne entre la première sortie de l'œuvre
/// (MusicBrainz, tous enregistrements) et la sortie de l'album écouté. Un titre écouté sur
/// un best-of ou un remaster récent garde son année d'origine. 9999 : inconnue.
const RELEASE_YEAR: &str = "CAST(substr(min(COALESCE(NULLIF(rc.first_release, ''), '9999'), COALESCE(ri.release_date, '9999')), 1, 4) AS INTEGER)";
const RELEASE_JOIN: &str = "LEFT JOIN track_credits rc ON rc.track_id = s.track_id LEFT JOIN album_info ri ON ri.album_id = s.album_id";

impl Library {
    pub fn overview(&self, period: Period, top: usize) -> Result<Overview> {
        self.overview_by(period, top, false)
    }

    /// Recalcule le temps d'écoute de chaque écoute dans une table temporaire
    /// (une passe sur toutes les écoutes, quelques dizaines de ms).
    fn refresh_listened(&self) -> Result<()> {
        // Aucune écriture depuis le dernier calcul : rien à refaire.
        // Empreinte de ce dont dépend le calcul (écoutes, leurs titres et heures, durées des
        // titres) : une écriture sans rapport (caractéristiques, caches) ne le refait pas.
        let print: u64 = self.conn.query_row(
            "SELECT (SELECT COUNT(*) || ':' || COALESCE(MAX(id), 0) || ':' || COALESCE(SUM(track_id), 0) || ':' || COALESCE(SUM(ts), 0) FROM scrobble)
                    || ':' || (SELECT COALESCE(SUM(duration_ms), 0) FROM track)",
            [],
            |r| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                r.get::<_, String>(0)?.hash(&mut h);
                Ok(h.finish())
            },
        )?;
        if self.listened_at.get() == Some(print) {
            return Ok(());
        }
        self.conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS temp.listened;
             CREATE TEMP TABLE listened (id INTEGER PRIMARY KEY, sec INTEGER NOT NULL, lo INTEGER NOT NULL, known INTEGER NOT NULL);
             INSERT INTO temp.listened
               SELECT id, {est}, MIN({LO}, {est}), known FROM (
                 SELECT s.id, COALESCE(t.duration_ms / 1000, {DEFAULT_TRACK_S}) AS dur, t.duration_ms IS NOT NULL AS known,
                        LEAD(s.ts) OVER (ORDER BY s.ts, s.id) - s.ts AS gap
                 FROM scrobble s JOIN track t ON t.id = s.track_id);",
            est = estimate()
        ))?;
        self.listened_at.set(Some(print));
        Ok(())
    }

    /// Vue d'ensemble ; `by_time` : classements au temps d'écoute plutôt qu'au nombre d'écoutes.
    pub fn overview_by(&self, period: Period, top: usize, by_time: bool) -> Result<Overview> {
        let (from, to) = period.bounds();
        let c = &self.conn;
        self.refresh_listened()?;
        let (listened_s, listened_min_s, listened_known) = c.query_row(
            "SELECT COALESCE(SUM(l.sec), 0), COALESCE(SUM(l.lo), 0), COALESCE(AVG(l.known), 0)
             FROM scrobble s JOIN temp.listened l ON l.id = s.id WHERE s.ts BETWEEN ?1 AND ?2",
            params![from, to],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let top_of = |kind: Kind, filter: &str| if by_time { self.top_time(kind, filter, period, top) } else { self.top(kind, filter, &[], period, top) };
        let (total, artists, albums, tracks, first_ts, last_ts) = c.query_row(
            "SELECT COUNT(*), COUNT(DISTINCT s.artist_id), COUNT(DISTINCT s.album_id), COUNT(DISTINCT s.track_id), MIN(s.ts), MAX(s.ts)
             FROM scrobble s WHERE s.ts BETWEEN ?1 AND ?2",
            params![from, to],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        let mut st = c.prepare(
            "SELECT dow, hour, COUNT(*) FROM scrobble WHERE ts BETWEEN ?1 AND ?2 GROUP BY 1, 2",
        )?;
        let heatmap = st
            .query_map(params![from, to], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?]))?
            .collect::<rusqlite::Result<_>>()?;
        let (release_years, release_covered) = self.release_years(period)?;
        let (countries, countries_covered) = self.countries(period)?;
        let (languages, languages_covered) = self.languages(period)?;
        Ok(Overview {
            total,
            artists,
            albums,
            tracks,
            first_ts,
            last_ts,
            listened_s,
            listened_min_s,
            listened_known,
            per_month: self.per_month("1 = 1", &[], period)?,
            per_day: self.per_day("1 = 1", &[], period)?,
            top_artists: top_of(Kind::Artist, "1 = 1")?,
            top_albums: top_of(Kind::Album, "s.album_id IS NOT NULL")?,
            top_tracks: top_of(Kind::Track, "1 = 1")?,
            heatmap,
            release_years,
            release_covered,
            countries,
            countries_covered,
            languages,
            languages_covered,
        })
    }

    /// Part des écoutes de la période qui vérifient une condition.
    fn share(&self, join: &str, cond: &str, period: Period) -> Result<f64> {
        let (from, to) = period.bounds();
        Ok(self.conn.query_row(
            &format!("SELECT COALESCE(AVG({cond}), 0) FROM scrobble s {join} WHERE s.ts BETWEEN ?1 AND ?2"),
            params![from, to],
            |r| r.get(0),
        )?)
    }

    fn release_years(&self, period: Period) -> Result<(Vec<(i64, i64)>, f64)> {
        let (from, to) = period.bounds();
        // Une seule passe : les années connues, et l'inconnue (9999) pour la couverture.
        let mut st = self.conn.prepare(&format!(
            "SELECT {RELEASE_YEAR} AS y, COUNT(*) FROM scrobble s {RELEASE_JOIN}
             WHERE s.ts BETWEEN ?1 AND ?2 GROUP BY 1 ORDER BY 1"
        ))?;
        let mut rows: Vec<(i64, i64)> = st.query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let total: i64 = rows.iter().map(|r| r.1).sum();
        rows.retain(|r| r.0 < 9999);
        let known: i64 = rows.iter().map(|r| r.1).sum();
        Ok((rows, if total > 0 { known as f64 / total as f64 } else { 0.0 }))
    }

    fn countries(&self, period: Period) -> Result<(Vec<(String, i64, i64)>, f64)> {
        let (from, to) = period.bounds();
        let mut st = self.conn.prepare(
            "SELECT i.country, COUNT(*), COUNT(DISTINCT s.artist_id) FROM scrobble s JOIN artist_info i ON i.artist_id = s.artist_id
             WHERE i.country IS NOT NULL AND s.ts BETWEEN ?1 AND ?2 GROUP BY 1 ORDER BY 2 DESC",
        )?;
        let rows = st.query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        let covered = self.share("LEFT JOIN artist_info i ON i.artist_id = s.artist_id", "i.country IS NOT NULL", period)?;
        Ok((rows, covered))
    }

    fn languages(&self, period: Period) -> Result<(Vec<(String, i64)>, f64)> {
        let (from, to) = period.bounds();
        let mut st = self.conn.prepare(
            "SELECT c.work_language, COUNT(*) FROM scrobble s JOIN track_credits c ON c.track_id = s.track_id
             WHERE c.work_language IS NOT NULL AND s.ts BETWEEN ?1 AND ?2 GROUP BY 1",
        )?;
        let mut acc: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for row in st.query_map(params![from, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (langs, n) = row?;
            for l in langs.split(',').filter(|l| !l.is_empty() && *l != "mul") {
                *acc.entry(l.to_owned()).or_default() += n;
            }
        }
        let mut rows: Vec<(String, i64)> = acc.into_iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        let covered = self.share("LEFT JOIN track_credits c ON c.track_id = s.track_id", "c.work_language IS NOT NULL", period)?;
        Ok((rows, covered))
    }

    /// Compare deux périodes : volumes, artistes, heures d'écoute, son.
    pub fn compare(&self, a: Period, b: Period, top: usize) -> Result<Comparison> {
        use std::collections::HashMap;
        self.refresh_listened()?;
        let plays_by_artist = |p: Period| -> Result<Vec<(i64, i64)>> {
            let (from, to) = p.bounds();
            let mut st = self.conn.prepare("SELECT artist_id, COUNT(*) FROM scrobble WHERE ts BETWEEN ?1 AND ?2 GROUP BY 1 ORDER BY 2 DESC, 1")?;
            let r = st.query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            Ok(r)
        };
        let (pa, pb) = (plays_by_artist(a)?, plays_by_artist(b)?);
        let rank = |v: &[(i64, i64)]| -> HashMap<i64, (usize, i64)> { v.iter().enumerate().map(|(i, &(id, n))| (id, (i + 1, n))).collect() };
        let (ra, rb) = (rank(&pa), rank(&pb));
        let side = |p: Period, own: &[(i64, i64)], other: &HashMap<i64, (usize, i64)>| -> Result<Side> {
            let (from, to) = p.bounds();
            let (plays, listened_s): (i64, i64) = self.conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(l.sec), 0) FROM scrobble s JOIN temp.listened l ON l.id = s.id WHERE s.ts BETWEEN ?1 AND ?2",
                params![from, to],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let new_artists = self.conn.query_row(
                "SELECT COUNT(*) FROM (SELECT MIN(ts) AS f FROM scrobble GROUP BY artist_id) WHERE f BETWEEN ?1 AND ?2",
                params![from, to],
                |r| r.get(0),
            )?;
            let mut hours = vec![0.0; 24];
            let mut st = self.conn.prepare("SELECT hour, COUNT(*) FROM scrobble WHERE ts BETWEEN ?1 AND ?2 GROUP BY 1")?;
            for row in st.query_map(params![from, to], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
                let (h, n) = row?;
                hours[h.clamp(0, 23) as usize] = 100.0 * n as f64 / plays.max(1) as f64;
            }
            let profile = self.profile_where(ProfileKind::Itself, None, "s.ts BETWEEN ? AND ?", &[from, to])?;
            let top_artists = own
                .iter()
                .take(top)
                .map(|&(id, n)| Ok((self.ranked(Kind::Artist, id, n)?, other.get(&id).map(|x| x.0))))
                .collect::<Result<_>>()?;
            Ok(Side {
                plays,
                listened_s,
                artists: own.len() as i64,
                new_artists,
                top_artists,
                hours,
                sound_basis: profile.as_ref().map_or(0, |p| p.basis),
                sound: profile.map(|p| p.axes),
            })
        };
        let (sa, sb) = (side(a, &pa, &rb)?, side(b, &pb, &ra)?);
        // Écarts de part d'écoute, pour les artistes qui comptent dans l'une des deux périodes.
        let (ta, tb) = (sa.plays.max(1) as f64, sb.plays.max(1) as f64);
        let mut shifts: Vec<(i64, i64, i64, f64)> = ra
            .keys()
            .chain(rb.keys())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .map(|&id| {
                let (na, nb) = (ra.get(&id).map_or(0, |x| x.1), rb.get(&id).map_or(0, |x| x.1));
                (id, na, nb, 100.0 * (nb as f64 / tb - na as f64 / ta))
            })
            .filter(|x| x.1 + x.2 >= 10)
            .collect();
        shifts.sort_by(|x, y| y.3.total_cmp(&x.3).then(x.0.cmp(&y.0)));
        let shift = |&(id, na, nb, _): &(i64, i64, i64, f64)| -> Result<Shift> {
            Ok(Shift { artist: self.ranked(Kind::Artist, id, nb)?, plays_a: na, share_a: 100.0 * na as f64 / ta, share_b: 100.0 * nb as f64 / tb })
        };
        let gained = shifts.iter().take(10).filter(|x| x.3 > 0.0).map(shift).collect::<Result<_>>()?;
        let lost = shifts.iter().rev().take(10).filter(|x| x.3 < 0.0).map(shift).collect::<Result<_>>()?;
        Ok(Comparison { a: sa, b: sb, gained, lost })
    }

    /// Découvertes et abandons. Les 3 premiers mois de l'historique ne comptent
    /// pas comme des découvertes (tout y est nouveau).
    pub fn discoveries(&self, period: Period) -> Result<Discoveries> {
        const DAY: i64 = 86_400;
        let (from, to) = period.bounds();
        let Some((g0, g1)) = self.conn.query_row("SELECT MIN(ts), MAX(ts) FROM scrobble", [], |r| Ok(r.get::<_, Option<i64>>(0)?.zip(r.get::<_, Option<i64>>(1)?)))? else {
            return Ok(Discoveries::default());
        };
        let from = from.max(g0 + 90 * DAY);
        let artists = "SELECT artist_id, MIN(ts) AS f, MAX(ts) AS l, COUNT(*) AS n FROM scrobble GROUP BY 1";
        let mut st = self.conn.prepare(&format!(
            "SELECT (f / 86400) * 86400, COUNT(*) FROM ({artists}) WHERE n >= 5 AND f BETWEEN ?1 AND ?2 GROUP BY 1"
        ))?;
        let mut months: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for row in st.query_map(params![from, to], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (day, n) = row?;
            let (ym, ..) = crate::import::local_fields(day);
            *months.entry(format!("{:04}-{:02}", ym / 100, ym % 100)).or_default() += n;
        }
        let turns = |sql: &str, args: [i64; 3]| -> Result<Vec<Turn>> {
            let mut st = self.conn.prepare(sql)?;
            let rows: Vec<(i64, i64, i64, i64, i64)> =
                st.query_map(args, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
            rows.into_iter().map(|(id, f, l, n, burst)| Ok(Turn { artist: self.ranked(Kind::Artist, id, n)?, first_ts: f, last_ts: l, burst })).collect()
        };
        let found = turns(
            &format!(
                "SELECT * FROM (SELECT a.*, (SELECT COUNT(*) FROM scrobble s WHERE s.artist_id = a.artist_id AND s.ts < a.f + 90 * 86400) AS b
                 FROM ({artists}) a WHERE a.f BETWEEN ?1 AND ?2) WHERE b >= 10 ORDER BY b DESC LIMIT ?3"
            ),
            [from, to, 12],
        )?;
        let dropped = turns(
            &format!(
                "SELECT * FROM (SELECT a.*, (SELECT COUNT(*) FROM scrobble s WHERE s.artist_id = a.artist_id AND s.ts > a.l - 180 * 86400) AS b
                 FROM ({artists}) a WHERE a.l BETWEEN ?1 AND ?2) WHERE b >= 10 ORDER BY b DESC LIMIT ?3"
            ),
            [period.bounds().0, to.min(g1 - 180 * DAY), 12],
        )?;
        Ok(Discoveries { per_month: fill_months(months.into_iter().collect()), found, dropped })
    }

    /// Découpe les écoutes en sessions et en tire quelques mesures.
    pub fn sessions(&self, period: Period) -> Result<SessionStats> {
        let (from, to) = period.bounds();
        // Durées lues une fois (quelques milliers de titres) plutôt que jointes à chaque écoute.
        let mut st = self.conn.prepare("SELECT id, duration_ms / 1000 FROM track WHERE duration_ms IS NOT NULL")?;
        let durations_s: std::collections::HashMap<i64, i64> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut st = self.conn.prepare("SELECT ts, artist_id, hour, track_id FROM scrobble WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts")?;
        let rows: Vec<(i64, i64, i64, i64)> = st
            .query_map(params![from, to], |r| {
                let track: i64 = r.get(3)?;
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, durations_s.get(&track).copied().unwrap_or(DEFAULT_TRACK_S)))
            })?
            .collect::<rusqlite::Result<_>>()?;
        // (indice de début, indice de fin inclus)
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for i in 0..rows.len() {
            match spans.last_mut() {
                Some(s) if rows[i].0 - rows[i - 1].0 <= SESSION_GAP => s.1 = i,
                _ => spans.push((i, i)),
            }
        }
        if spans.is_empty() {
            return Ok(SessionStats { start_hours: vec![0; 24], ..Default::default() });
        }
        let minutes = |&(a, b): &(usize, usize)| (rows[b].0 - rows[a].0 + rows[b].3) / 60;
        let mut durations: Vec<i64> = spans.iter().map(minutes).collect();
        durations.sort_unstable();
        let longest = spans.iter().max_by_key(|s| (s.1 - s.0, minutes(s))).map(|s| LongestSession {
            start_ts: rows[s.0].0,
            tracks: (s.1 - s.0 + 1) as i64,
            minutes: minutes(s),
        });
        const BUCKETS: [(&str, usize, usize); 6] = [("1", 1, 1), ("2-3", 2, 3), ("4-6", 4, 6), ("7-10", 7, 10), ("11-20", 11, 20), ("21+", 21, usize::MAX)];
        let lengths = BUCKETS
            .iter()
            .map(|&(label, lo, hi)| (label.to_owned(), spans.iter().filter(|s| (lo..=hi).contains(&(s.1 - s.0 + 1))).count() as i64))
            .collect();
        let mut start_hours = vec![0i64; 24];
        for s in &spans {
            start_hours[rows[s.0].2.clamp(0, 23) as usize] += 1;
        }
        let rank = |pick: &dyn Fn(&(usize, usize)) -> usize| -> Result<Vec<Ranked>> {
            let mut counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
            for s in spans.iter().filter(|s| s.1 - s.0 >= 2) {
                *counts.entry(rows[pick(s)].1).or_default() += 1;
            }
            let mut v: Vec<(i64, i64)> = counts.into_iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            v.into_iter().take(8).map(|(id, n)| self.ranked(Kind::Artist, id, n)).collect()
        };
        Ok(SessionStats {
            count: spans.len() as i64,
            avg_tracks: rows.len() as f64 / spans.len() as f64,
            median_minutes: durations[durations.len() / 2] as f64,
            longest,
            lengths,
            start_hours,
            openers: rank(&|s| s.0)?,
            closers: rank(&|s| s.1)?,
        })
    }

    /// Détail d'une sélection (un mois, un pays, une heure…) : volumes et tops.
    pub fn detail(&self, f: &Filter, top: usize) -> Result<Detail> {
        // Conditions SQL : nombres formatés depuis des i64, codes vérifiés caractère par caractère.
        let code = |s: &str, max: usize| (!s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_alphabetic())).then(|| s.to_owned());
        let mut conds = vec!["1 = 1".to_owned()];
        if let (Some(k), Some(id)) = (f.kind, f.id) {
            conds.push(format!("{} = {id}", k.col()));
        }
        if let Some(c) = f.country.as_deref().and_then(|c| code(c, 2)) {
            conds.push(format!("s.artist_id IN (SELECT artist_id FROM artist_info WHERE country = '{c}')"));
        }
        if let Some(l) = f.language.as_deref().and_then(|l| code(l, 3)) {
            conds.push(format!("s.track_id IN (SELECT track_id FROM track_credits WHERE ',' || work_language || ',' LIKE '%,{l},%')"));
        }
        if f.release_from.is_some() || f.release_to.is_some() {
            let (a, b) = (f.release_from.unwrap_or(0), f.release_to.unwrap_or(9999));
            conds.push(format!(
                "s.id IN (SELECT s.id FROM scrobble s {RELEASE_JOIN} WHERE {RELEASE_YEAR} BETWEEN {a} AND {b})"
            ));
        }
        if let Some(d) = f.dow {
            conds.push(format!("s.dow = {d}"));
        }
        if let Some(h) = f.hour {
            conds.push(format!("s.hour = {h}"));
        }
        let filter = conds.join(" AND ");
        let period = Period { from: f.from, to: f.to };
        let (from, to) = period.bounds();
        let (total, artists, albums, tracks, first_ts, last_ts) = self.conn.query_row(
            &format!(
                "SELECT COUNT(*), COUNT(DISTINCT s.artist_id), COUNT(DISTINCT s.album_id), COUNT(DISTINCT s.track_id), MIN(s.ts), MAX(s.ts)
                 FROM scrobble s WHERE {filter} AND s.ts BETWEEN ?1 AND ?2"
            ),
            params![from, to],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        Ok(Detail {
            total,
            artists,
            albums,
            tracks,
            first_ts,
            last_ts,
            top_artists: self.top(Kind::Artist, &filter, &[], period, top)?,
            top_albums: self.top(Kind::Album, &format!("{filter} AND s.album_id IS NOT NULL"), &[], period, top)?,
            top_tracks: self.top(Kind::Track, &filter, &[], period, top)?,
            per_month: self.per_month(&filter, &[], period)?,
            per_day: self.per_day(&filter, &[], period)?,
        })
    }

    /// Albums sortis entre deux années (incluses), les plus écoutés d'abord.
    pub fn albums_by_release(&self, from_year: i64, to_year: i64, period: Period) -> Result<Vec<AlbumRelease>> {
        let (from, to) = period.bounds();
        let mut st = self.conn.prepare(
            "SELECT s.album_id, COUNT(*), i.release_date FROM scrobble s JOIN album_info i ON i.album_id = s.album_id
             WHERE CAST(substr(i.release_date, 1, 4) AS INTEGER) BETWEEN ?1 AND ?2 AND s.ts BETWEEN ?3 AND ?4
             GROUP BY s.album_id ORDER BY 2 DESC LIMIT 200",
        )?;
        let rows: Vec<(i64, i64, String)> =
            st.query_map(params![from_year, to_year, from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        rows.into_iter().map(|(id, n, date)| Ok(AlbumRelease { album: self.ranked(Kind::Album, id, n)?, release_date: date })).collect()
    }

    /// Écoutes par mois pour les écoutes qui vérifient `filter` (SQL sur s et t).
    fn per_month(&self, filter: &str, args: &[i64], period: Period) -> Result<Vec<(String, i64)>> {
        let (from, to) = period.bounds();
        let sql = format!(
            "SELECT s.ym, COUNT(*) FROM scrobble s WHERE {filter} AND s.ts BETWEEN ? AND ? GROUP BY 1 ORDER BY 1"
        );
        let mut st = self.conn.prepare(&sql)?;
        let all: Vec<i64> = args.iter().copied().chain([from, to]).collect();
        let rows: Vec<(String, i64)> = st
            .query_map(params_from_iter(all), |r| {
                let ym: i64 = r.get(0)?;
                Ok((format!("{:04}-{:02}", ym / 100, ym % 100), r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(fill_months(rows))
    }

    /// Écoutes par jour local pour les écoutes qui vérifient `filter`.
    fn per_day(&self, filter: &str, args: &[i64], period: Period) -> Result<Vec<(String, i64)>> {
        let (from, to) = period.bounds();
        let sql = format!("SELECT s.ymd, COUNT(*) FROM scrobble s WHERE {filter} AND s.ts BETWEEN ? AND ? GROUP BY 1 ORDER BY 1");
        let mut st = self.conn.prepare(&sql)?;
        let all: Vec<i64> = args.iter().copied().chain([from, to]).collect();
        let rows: Vec<(i64, i64)> =
            st.query_map(params_from_iter(all), |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(fill_days(rows))
    }

    fn top(&self, kind: Kind, filter: &str, args: &[i64], period: Period, limit: usize) -> Result<Vec<Ranked>> {
        let (from, to) = period.bounds();
        let sql = format!(
            "SELECT {col}, COUNT(*) FROM scrobble s
             WHERE {filter} AND s.ts BETWEEN ? AND ? GROUP BY 1 ORDER BY 2 DESC LIMIT ?",
            col = kind.col()
        );
        let mut st = self.conn.prepare(&sql)?;
        let all: Vec<i64> = args.iter().copied().chain([from, to, limit as i64]).collect();
        let pairs: Vec<(i64, i64)> =
            st.query_map(params_from_iter(all), |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        pairs.into_iter().map(|(id, plays)| self.ranked(kind, id, plays)).collect()
    }

    /// Classement au temps d'écoute (après `refresh_listened`).
    fn top_time(&self, kind: Kind, filter: &str, period: Period, limit: usize) -> Result<Vec<Ranked>> {
        let (from, to) = period.bounds();
        let mut st = self.conn.prepare(&format!(
            "SELECT {col}, COUNT(*), SUM(l.sec) FROM scrobble s JOIN temp.listened l ON l.id = s.id
             WHERE {filter} AND s.ts BETWEEN ? AND ? GROUP BY 1 ORDER BY 3 DESC LIMIT ?",
            col = kind.col()
        ))?;
        let rows: Vec<(i64, i64, i64)> =
            st.query_map(params![from, to, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        rows.into_iter().map(|(id, plays, sec)| Ok(Ranked { seconds: Some(sec), ..self.ranked(kind, id, plays)? })).collect()
    }

    pub(crate) fn ranked(&self, kind: Kind, id: i64, plays: i64) -> Result<Ranked> {
        let (name, artist, artist_id) = match kind {
            Kind::Artist => (self.conn.query_row("SELECT name FROM artist WHERE id = ?", [id], |r| r.get(0))?, None, None),
            Kind::Album | Kind::Track => {
                let table = if kind == Kind::Album { "album" } else { "track" };
                self.conn.query_row(
                    // Album regroupant plusieurs éditions : son titre de base.
                    &format!(
                        "SELECT {}, a.name, a.id FROM {table} x JOIN artist a ON a.id = x.artist_id WHERE x.id = ?",
                        if table == "album" { "COALESCE(x.display_title, x.title)" } else { "x.title" }
                    ),
                    [id],
                    |r| Ok((r.get(0)?, Some(r.get(1)?), Some(r.get(2)?))),
                )?
            }
        };
        Ok(Ranked { kind, id, name, artist, artist_id, plays, seconds: None })
    }

    /// Ce qui est écouté juste avant (`before = true`) ou juste après l'entité,
    /// au même niveau (titre, album ou artiste), en ignorant l'entité elle-même.
    pub fn neighbours(&self, kind: Kind, id: i64, before: bool, limit: usize) -> Result<Vec<Ranked>> {
        let (f, g) = if before { ("LAG", "ts - nts") } else { ("LEAD", "nts - ts") };
        let col = kind.col();
        // On ne garde que les écoutes à ±SESSION_GAP de l'entité avant de fenêtrer :
        // inutile de trier toute la base.
        let sql = format!(
            "WITH anchor AS (SELECT s.ts FROM scrobble s WHERE {col} = ?1),
             near AS (SELECT DISTINCT s.id, s.ts, {col} AS e FROM scrobble s
                      JOIN anchor a ON s.ts BETWEEN a.ts - {gap} AND a.ts + {gap}),
             seq AS (SELECT ts, e, {f}(e) OVER w AS ne, {f}(ts) OVER w AS nts FROM near WINDOW w AS (ORDER BY ts, id))
             SELECT ne, COUNT(*) FROM seq
             WHERE e = ?1 AND ne IS NOT NULL AND ne <> ?1 AND {g} <= {gap}
             GROUP BY ne ORDER BY 2 DESC LIMIT ?2",
            gap = SESSION_GAP
        );
        let mut st = self.conn.prepare(&sql)?;
        let pairs: Vec<(i64, i64)> =
            st.query_map(params![id, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        pairs.into_iter().map(|(n, plays)| self.ranked(kind, n, plays)).collect()
    }

    pub fn entity(&self, kind: Kind, id: i64) -> Result<EntityDetail> {
        let head = self.ranked(kind, id, 0).optional_not_found(kind, id)?;
        let filter = format!("{} = ?", kind.col());
        let all = Period::default();
        let (total, first_ts, last_ts) = self.conn.query_row(
            &format!("SELECT COUNT(*), MIN(s.ts), MAX(s.ts) FROM scrobble s WHERE {filter}"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        // Temps d'écoute : l'écoute suivante est cherchée dans tout l'historique.
        let listened_s: i64 = self.conn.query_row(
            &format!(
                "SELECT COALESCE(SUM({est}), 0) FROM (
                   SELECT COALESCE(t.duration_ms / 1000, {DEFAULT_TRACK_S}) AS dur,
                          (SELECT MIN(n.ts) FROM scrobble n WHERE n.ts > s.ts) - s.ts AS gap
                   FROM scrobble s JOIN track t ON t.id = s.track_id WHERE {filter})",
                est = estimate()
            ),
            [id],
            |r| r.get(0),
        )?;
        let top_tracks = if kind == Kind::Track { vec![] } else { self.top(Kind::Track, &filter, &[id], all, 50)? };
        let album_filter = format!("{filter} AND s.album_id IS NOT NULL");
        let top_albums = if kind == Kind::Album { vec![] } else { self.top(Kind::Album, &album_filter, &[id], all, 20)? };

        let features = if kind == Kind::Track { self.features(id)? } else { None };
        let mut profiles = Vec::new();
        if let Some(f) = features.as_ref().filter(|f| f.status == "ok") {
            profiles.push(self.with_percentiles(ProfileKind::Itself, Some(head.name.clone()), 1, f.axes)?);
        }
        if kind == Kind::Album {
            profiles.extend(self.profile_where(ProfileKind::Itself, Some(&head.name), "s.album_id = ?", &[id])?);
        }
        // Pour un titre : la moyenne de l'album sur lequel tu l'écoutes le plus.
        if let (Kind::Track, Some(album)) = (kind, top_albums.first()) {
            profiles.extend(self.profile_where(ProfileKind::Album, Some(&album.name), "s.album_id = ?", &[album.id])?);
        }
        if let Some(artist_id) = head.artist_id.or((kind == Kind::Artist).then_some(id)) {
            let (pk, name) = if kind == Kind::Artist { (ProfileKind::Itself, &head.name) } else { (ProfileKind::Artist, head.artist.as_ref().unwrap_or(&head.name)) };
            profiles.extend(self.profile_where(pk, Some(name), "s.artist_id = ?", &[artist_id])?);
        }
        profiles.extend(self.profile_where(ProfileKind::Library, None, "1 = 1", &[])?);

        let album_info = match kind {
            Kind::Album => self.album_info(id)?,
            Kind::Track => match top_albums.first() {
                Some(a) => self.album_info(a.id)?,
                None => None,
            },
            Kind::Artist => None,
        };
        let head_artist = head.artist_id.unwrap_or(id);
        Ok(EntityDetail {
            kind,
            id,
            name: head.name,
            artist: head.artist,
            artist_id: head.artist_id,
            total,
            listened_s,
            first_ts,
            last_ts,
            per_month: self.per_month(&filter, &[id], all)?,
            per_day: self.per_day(&filter, &[id], all)?,
            top_tracks,
            top_albums,
            before: self.neighbours(kind, id, true, 10)?,
            after: self.neighbours(kind, id, false, 10)?,
            features,
            duration_ms: match kind {
                Kind::Track => self.conn.query_row("SELECT duration_ms FROM track WHERE id = ?", [id], |r| r.get(0))?,
                Kind::Album => album_info.as_ref().and_then(|i| i.duration_s).map(|s| s * 1000),
                Kind::Artist => None,
            },
            album_info,
            artist_info: self.artist_info(head_artist)?,
            languages: if kind == Kind::Track {
                self.conn
                    .query_row("SELECT work_language FROM track_credits WHERE track_id = ?", [id], |r| r.get::<_, Option<String>>(0))
                    .optional()?
                    .flatten()
                    .map(|l| l.split(',').filter(|x| !x.is_empty()).map(str::to_owned).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            },
            versions: if kind == Kind::Track { self.versions(id)? } else { vec![] },
            artist_credit: if kind == Kind::Track { self.artist_credit(id)? } else { vec![] },
            merged: self.merged_into(kind, id)?,
            profiles,
        })
    }

    pub fn features(&self, track_id: i64) -> Result<Option<Features>> {
        Ok(self
            .conn
            .query_row(
                "SELECT status, provider, matched_title, matched_artist, isrc, danceability, energy, valence, acousticness,
                        instrumentalness, speechiness, liveness, tempo, loudness, key, mode
                 FROM track_features WHERE track_id = ?",
                [track_id],
                |r| {
                    let mut axes = [None; 7];
                    for (i, a) in axes.iter_mut().enumerate() {
                        *a = r.get(5 + i)?;
                    }
                    Ok(Features {
                        status: r.get(0)?, provider: r.get(1)?, matched_title: r.get(2)?, matched_artist: r.get(3)?,
                        isrc: r.get(4)?, axes, tempo: r.get(12)?, loudness: r.get(13)?, key: r.get(14)?, mode: r.get(15)?,
                    })
                },
            )
            .optional()?)
    }

    /// Moyenne des caractéristiques sur les écoutes qui vérifient `filter`, donc
    /// pondérée par le nombre d'écoutes. None si aucune écoute n'a de caractéristiques.
    fn profile_where(&self, kind: ProfileKind, name: Option<&str>, filter: &str, args: &[i64]) -> Result<Option<Profile>> {
        let avgs = AXES.map(|a| format!("AVG(f.{a})")).join(", ");
        let sql = format!(
            "SELECT COUNT(*), {avgs} FROM scrobble s
             JOIN track_features f ON f.track_id = s.track_id AND f.status = 'ok' WHERE {filter}"
        );
        let (basis, axes) = self.conn.query_row(&sql, params_from_iter(args), |r| {
            let mut axes = [None; 7];
            for (i, a) in axes.iter_mut().enumerate() {
                *a = r.get(1 + i)?;
            }
            Ok((r.get::<_, i64>(0)?, axes))
        })?;
        if basis == 0 {
            return Ok(None);
        }
        Ok(Some(self.with_percentiles(kind, name.map(str::to_owned), basis, axes)?))
    }

    fn with_percentiles(&self, kind: ProfileKind, name: Option<String>, basis: i64, axes: [Option<f64>; 7]) -> Result<Profile> {
        let mut percentiles = [None; 7];
        for (i, (axis, v)) in AXES.iter().zip(axes).enumerate() {
            let Some(v) = v else { continue };
            let (below, equal, n): (f64, f64, f64) = self.conn.query_row(
                &format!(
                    "SELECT COALESCE(SUM({axis} < ?1), 0), COALESCE(SUM({axis} = ?1), 0), COUNT({axis})
                     FROM track_features WHERE status = 'ok' AND track_id IN (SELECT DISTINCT track_id FROM scrobble)"
                ),
                [v],
                |r| Ok((r.get::<_, i64>(0)? as f64, r.get::<_, i64>(1)? as f64, r.get::<_, i64>(2)? as f64)),
            )?;
            if n > 0.0 {
                percentiles[i] = Some(100.0 * (below + equal / 2.0) / n);
            }
        }
        Ok(Profile { kind, name, basis, axes, percentiles })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<SearchResults> {
        let pattern = format!("%{}%", fold(query));
        let find = |kind: Kind, table: &str, col: &str| -> Result<Vec<Ranked>> {
            let sql = format!(
                "SELECT x.id, COUNT(*) FROM {table} x JOIN scrobble s ON s.{col} = x.id
                 WHERE x.key LIKE ?1 GROUP BY x.id ORDER BY 2 DESC LIMIT ?2"
            );
            let mut st = self.conn.prepare(&sql)?;
            let pairs: Vec<(i64, i64)> = st
                .query_map(params![pattern, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            pairs.into_iter().map(|(id, n)| self.ranked(kind, id, n)).collect()
        };
        Ok(SearchResults {
            artists: find(Kind::Artist, "artist", "artist_id")?,
            albums: find(Kind::Album, "album", "album_id")?,
            tracks: find(Kind::Track, "track", "track_id")?,
        })
    }
}

/// Tri de la bibliothèque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowseSort {
    Plays,
    Time,
    First,
    Last,
    Name,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowseRow {
    pub item: Ranked,
    pub seconds: i64,
    pub first_ts: i64,
    pub last_ts: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowsePage {
    /// Nombre total d'entrées qui correspondent au filtre.
    pub total: i64,
    pub rows: Vec<BrowseRow>,
}

impl Library {
    /// Bibliothèque : tous les artistes, albums ou titres écoutés, triés et
    /// filtrés (nom ou artiste), par pages.
    pub fn browse(&self, kind: Kind, sort: BrowseSort, desc: bool, filter: &str, offset: usize, limit: usize) -> Result<BrowsePage> {
        self.refresh_listened()?;
        let (table, name, artist_join, artist_cols) = match kind {
            Kind::Artist => ("artist", "x.name", "", "NULL, NULL"),
            Kind::Album => ("album", "COALESCE(x.display_title, x.title)", "JOIN artist a ON a.id = x.artist_id", "a.name, a.id"),
            Kind::Track => ("track", "x.title", "JOIN artist a ON a.id = x.artist_id", "a.name, a.id"),
        };
        let artist_match = if kind == Kind::Artist { "" } else { " OR a.key LIKE ?1" };
        let order = match sort {
            BrowseSort::Plays => "g.n",
            BrowseSort::Time => "g.sec",
            BrowseSort::First => "g.f",
            BrowseSort::Last => "g.l",
            BrowseSort::Name => "x.key",
        };
        let dir = if desc { "DESC" } else { "ASC" };
        let from = format!(
            "FROM (SELECT {col} AS id, COUNT(*) AS n, SUM(l.sec) AS sec, MIN(s.ts) AS f, MAX(s.ts) AS l
                   FROM scrobble s JOIN temp.listened l ON l.id = s.id WHERE {col} IS NOT NULL GROUP BY 1) g
             JOIN {table} x ON x.id = g.id {artist_join}
             WHERE ?1 = '%%' OR x.key LIKE ?1{artist_match}",
            col = kind.col()
        );
        let pattern = format!("%{}%", fold(filter));
        let total = self.conn.query_row(&format!("SELECT COUNT(*) {from}"), [&pattern], |r| r.get(0))?;
        let mut st = self.conn.prepare(&format!(
            "SELECT g.id, {name}, {artist_cols}, g.n, g.sec, g.f, g.l {from} ORDER BY {order} {dir}, g.id LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = st
            .query_map(params![pattern, limit as i64, offset as i64], |r| {
                Ok(BrowseRow {
                    item: Ranked { kind, id: r.get(0)?, name: r.get(1)?, artist: r.get(2)?, artist_id: r.get(3)?, plays: r.get(4)?, seconds: None },
                    seconds: r.get(5)?,
                    first_ts: r.get(6)?,
                    last_ts: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(BrowsePage { total, rows })
    }
}

trait NotFound<T> {
    fn optional_not_found(self, kind: Kind, id: i64) -> Result<T>;
}

impl<T> NotFound<T> for Result<T> {
    fn optional_not_found(self, kind: Kind, id: i64) -> Result<T> {
        self.map_err(|e| match e {
            Error::Db(rusqlite::Error::QueryReturnedNoRows) => Error::NotFound(format!("{kind:?} {id}")),
            e => e,
        })
    }
}

/// Ajoute les jours sans écoute. Entrée : (20240310, n) triés.
fn fill_days(rows: Vec<(i64, i64)>) -> Vec<(String, i64)> {
    use chrono::NaiveDate;
    let date = |d: i64| NaiveDate::from_ymd_opt((d / 10000) as i32, (d / 100 % 100) as u32, (d % 100) as u32);
    let (Some(first), Some(last)) = (rows.first().and_then(|r| date(r.0)), rows.last().and_then(|r| date(r.0))) else {
        return vec![];
    };
    let counts: std::collections::HashMap<NaiveDate, i64> = rows.iter().filter_map(|&(d, n)| Some((date(d)?, n))).collect();
    first.iter_days().take_while(|d| *d <= last).map(|d| (d.format("%Y-%m-%d").to_string(), counts.get(&d).copied().unwrap_or(0))).collect()
}

/// Ajoute les mois sans écoute, pour que les graphes montrent les trous.
fn fill_months(rows: Vec<(String, i64)>) -> Vec<(String, i64)> {
    let parse = |m: &str| -> Option<(i32, u32)> { Some((m.get(0..4)?.parse().ok()?, m.get(5..7)?.parse().ok()?)) };
    let (Some(first), Some(last)) = (rows.first().and_then(|r| parse(&r.0)), rows.last().and_then(|r| parse(&r.0))) else {
        return rows;
    };
    let mut out = Vec::new();
    let mut it = rows.into_iter().peekable();
    let (mut y, mut m) = first;
    while (y, m) <= last {
        let key = format!("{y:04}-{m:02}");
        let n = if it.peek().is_some_and(|r| r.0 == key) { it.next().map_or(0, |r| r.1) } else { 0 };
        out.push((key, n));
        (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    }
    out
}
