//! « Tes mixes » : playlists générées à partir de toutes les données, en local,
//! sans modèle de langage.
//!
//! Ingrédients par titre :
//! - le son (caractéristiques audio, tempo, tonalité) ;
//! - la logique d'écoute : titres écoutés l'un près de l'autre dans une session,
//!   corrigés du biais des albums (une écoute d'album dans l'ordre ne crée
//!   presque pas de lien entre ses titres) et de la popularité (PMI) ;
//! - le contexte : moments de la semaine, saisons, ancienneté de la dernière écoute.
//!
//! Familles : ambiances, moments, saisons, redécouvertes, « autour de », pépites.
//! Chaque mix privilégie ce qui est distinctif (et non simplement le plus écouté),
//! impose de la diversité, mêle titres familiers et oubliés, change chaque semaine,
//! et ordonne ses titres pour qu'ils s'enchaînent bien.

use crate::stats::SESSION_GAP;
use crate::{Library, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const DAY: i64 = 86_400;
/// Demi-vie d'une écoute dans les préférences : une écoute d'il y a 3 mois compte moitié,
/// d'il y a un an 1/16. Plancher minime : un plancher de 5 % laissait des milliers
/// d'écoutes anciennes l'emporter sur ce qui est écouté en ce moment.
const HALF_LIFE: f64 = 90.0 * DAY as f64;
const FLOOR: f64 = 0.002;

/// Poids d'une écoute d'il y a `age` secondes (1 pour une écoute du jour).
pub(crate) fn recency(age: i64) -> f64 {
    FLOOR + (1.0 - FLOOR) * 0.5f64.powf(age.max(0) as f64 / HALF_LIFE)
}
/// Taille visée d'un mix, et minimum pour le proposer.
/// Taille par défaut d'un mix (titres) ; l'utilisateur peut la changer.
pub const MIX_SIZE: usize = 30;
const MIX_MIN: usize = 12;
/// Un titre écouté moins souvent n'est jamais proposé (écoute de passage, zapping).
const MIN_PLAYS: f64 = 3.0;
/// Écoute d'album : au moins ce nombre de titres consécutifs du même album.
const ALBUM_RUN: usize = 3;
/// Poids d'un lien entre deux titres d'une même écoute d'album.
const ALBUM_RUN_WEIGHT: f64 = 0.05;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixTrack {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub artist_id: i64,
    pub album_id: Option<i64>,
    pub album: Option<String>,
    pub plays: i64,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mix {
    /// Identifiant stable d'une semaine à l'autre (pour les retours de l'utilisateur).
    pub key: String,
    /// ambiance, moment, season, rediscover, around, gems
    pub kind: String,
    /// Paramètres du nom et de la raison, traduits par l'interface
    /// (traits d'ambiance, créneau, saison, artiste…).
    pub params: HashMap<String, String>,
    pub tracks: Vec<MixTrack>,
    /// Albums pour la mosaïque de pochettes.
    pub covers: Vec<i64>,
    pub duration_ms: i64,
}

// ---------- hasard déterministe (change chaque semaine) ----------

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Nombre entre 0 et 1, stable pour une graine et un titre donnés.
fn noise(seed: u64, id: i64) -> f64 {
    (splitmix(seed ^ (id as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 11) as f64 / (1u64 << 53) as f64
}

// ---------- données ----------

struct Track {
    id: i64,
    title: String,
    artist: String,
    artist_id: i64,
    album_id: Option<i64>,
    album: Option<String>,
    plays: f64,
    /// Écoutes pondérées par leur ancienneté (voir `recency`) : ce qui sert de préférence.
    score: f64,
    last_ts: i64,
    duration_ms: Option<i64>,
    /// Son normalisé (z-scores), si connu.
    sound: Option<[f64; 8]>,
    energy: Option<f64>,
    tempo: Option<f64>,
    key: Option<i64>,
    mode: Option<i64>,
    languages: Vec<String>,
}

struct Play {
    ts: i64,
    t: usize,
    album_id: Option<i64>,
    dow: i64,
    hour: i64,
}

struct Data {
    tracks: Vec<Track>,
    plays: Vec<Play>,
    /// Liens d'écoute (PMI positive), voisins triés par force décroissante.
    links: Vec<Vec<(usize, f64)>>,
    /// Enchaînements directs a → b (hors écoutes d'album), pour l'ordre des titres.
    transitions: HashMap<(usize, usize), f64>,
    end_ts: i64,
    seed: u64,
    /// Nombre de titres visé par mix.
    size: usize,
}

impl Data {
    /// En dessous, un mix n'est pas proposé.
    fn min(&self) -> usize {
        MIX_MIN.min(self.size)
    }

    fn link(&self, a: usize, b: usize) -> f64 {
        self.links[a].iter().find(|(x, _)| *x == b).map_or(0.0, |(_, w)| *w)
    }

    /// Distance de son (z-scores), None si l'un des deux n'a pas de caractéristiques.
    fn sound_dist(&self, a: usize, b: usize) -> Option<f64> {
        let (x, y) = (self.tracks[a].sound?, self.tracks[b].sound?);
        Some(x.iter().zip(y).map(|(p, q)| (p - q) * (p - q)).sum::<f64>().sqrt())
    }
}

impl Library {
    fn load_mix_data(&self, size: usize) -> Result<Data> {
        let mut st = self.conn.prepare(
            "SELECT t.id, t.title, a.name, a.id, COUNT(*), MAX(s.ts), t.duration_ms,
                    (SELECT s2.album_id FROM scrobble s2 WHERE s2.track_id = t.id AND s2.album_id IS NOT NULL
                     GROUP BY s2.album_id ORDER BY COUNT(*) DESC LIMIT 1),
                    f.danceability, f.energy, f.valence, f.acousticness, f.instrumentalness, f.speechiness, f.liveness, f.tempo,
                    f.key, f.mode, c.work_language
             FROM scrobble s JOIN track t ON t.id = s.track_id JOIN artist a ON a.id = s.artist_id
             LEFT JOIN track_features f ON f.track_id = t.id AND f.status = 'ok'
             LEFT JOIN track_credits c ON c.track_id = t.id
             GROUP BY t.id",
        )?;
        let mut raw_sound: Vec<Option<[f64; 8]>> = Vec::new();
        let mut tracks: Vec<Track> = st
            .query_map([], |r| {
                let s: Vec<Option<f64>> = (8..16).map(|i| r.get(i)).collect::<rusqlite::Result<_>>()?;
                let sound = s.iter().all(Option::is_some).then(|| {
                    let mut v = [0.0; 8];
                    for (i, x) in s.iter().enumerate() {
                        v[i] = x.unwrap_or(0.0);
                    }
                    v
                });
                let langs: Option<String> = r.get(18)?;
                Ok((
                    Track {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        artist: r.get(2)?,
                        artist_id: r.get(3)?,
                        plays: r.get::<_, i64>(4)? as f64,
                        score: 0.0,
                        last_ts: r.get(5)?,
                        duration_ms: r.get(6)?,
                        album_id: r.get(7)?,
                        album: None,
                        sound: None,
                        energy: s[1],
                        tempo: s[7],
                        key: r.get(16)?,
                        mode: r.get(17)?,
                        languages: langs.map(|l| l.split(',').filter(|x| !x.is_empty() && *x != "mul").map(str::to_owned).collect()).unwrap_or_default(),
                    },
                    sound,
                ))
            })?
            .map(|r| {
                r.map(|(t, s)| {
                    raw_sound.push(s);
                    t
                })
            })
            .collect::<rusqlite::Result<_>>()?;

        // Titres d'albums (titre de base si l'album regroupe plusieurs éditions).
        let mut st = self.conn.prepare("SELECT id, COALESCE(display_title, title) FROM album")?;
        let albums: HashMap<i64, String> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for t in &mut tracks {
            t.album = t.album_id.and_then(|a| albums.get(&a).cloned());
        }

        // Son en z-scores (moyenne et écart-type de la bibliothèque).
        let known: Vec<&[f64; 8]> = raw_sound.iter().flatten().collect();
        if !known.is_empty() {
            let n = known.len() as f64;
            let mut mean = [0.0; 8];
            let mut sd = [0.0; 8];
            for v in &known {
                for i in 0..8 {
                    mean[i] += v[i] / n;
                }
            }
            for v in &known {
                for i in 0..8 {
                    sd[i] += (v[i] - mean[i]).powi(2) / n;
                }
            }
            for (t, s) in tracks.iter_mut().zip(&raw_sound) {
                t.sound = s.map(|v| {
                    let mut z = [0.0; 8];
                    for i in 0..8 {
                        z[i] = (v[i] - mean[i]) / sd[i].sqrt().max(1e-6);
                    }
                    z
                });
            }
        }

        let index: HashMap<i64, usize> = tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let mut st = self.conn.prepare("SELECT ts, track_id, album_id, dow, hour FROM scrobble ORDER BY ts")?;
        let plays: Vec<Play> = st
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .filter_map(|r| r.ok())
            .filter_map(|(ts, id, album_id, dow, hour)| Some(Play { ts, t: *index.get(&id)?, album_id, dow, hour }))
            .collect();
        let end_ts = plays.last().map_or(0, |p| p.ts);
        for p in &plays {
            tracks[p.t].score += recency(end_ts - p.ts);
        }

        let (links, transitions) = listening_links(&plays, tracks.len());
        Ok(Data { tracks, plays, links, transitions, end_ts, seed: (chrono::Utc::now().timestamp() / (7 * DAY)) as u64, size })
    }
}

/// Liens d'écoute entre titres, corrigés du biais des albums et de la popularité.
fn listening_links(plays: &[Play], n: usize) -> (Vec<Vec<(usize, f64)>>, HashMap<(usize, usize), f64>) {
    // Sessions.
    let mut sessions: Vec<std::ops::Range<usize>> = Vec::new();
    let mut start = 0;
    for i in 1..=plays.len() {
        if i == plays.len() || plays[i].ts - plays[i - 1].ts > SESSION_GAP {
            sessions.push(start..i);
            start = i;
        }
    }
    let mut pair: HashMap<(usize, usize), f64> = HashMap::new();
    let mut transitions: HashMap<(usize, usize), f64> = HashMap::new();
    for s in sessions {
        let seq = &plays[s];
        // Écoutes d'album : blocs d'au moins ALBUM_RUN titres consécutifs du même album.
        let mut run = vec![usize::MAX; seq.len()];
        let mut i = 0;
        while i < seq.len() {
            let mut j = i + 1;
            while j < seq.len() && seq[j].album_id.is_some() && seq[j].album_id == seq[i].album_id {
                j += 1;
            }
            if j - i >= ALBUM_RUN {
                for r in run.iter_mut().take(j).skip(i) {
                    *r = i;
                }
            }
            i = j;
        }
        for i in 0..seq.len() {
            for d in 1..=2 {
                let j = i + d;
                if j >= seq.len() || seq[i].t == seq[j].t {
                    continue;
                }
                let in_album = run[i] != usize::MAX && run[i] == run[j];
                let w = (1.0 / d as f64) * if in_album { ALBUM_RUN_WEIGHT } else { 1.0 };
                let (a, b) = (seq[i].t.min(seq[j].t), seq[i].t.max(seq[j].t));
                *pair.entry((a, b)).or_default() += w;
                if d == 1 && !in_album {
                    *transitions.entry((seq[i].t, seq[j].t)).or_default() += 1.0;
                }
            }
        }
    }
    // PMI positive : lien plus fréquent que ne le voudrait la popularité des deux titres.
    let mut marginal = vec![0.0; n];
    let mut total = 0.0;
    for (&(a, b), &w) in &pair {
        marginal[a] += w;
        marginal[b] += w;
        total += 2.0 * w;
    }
    let mut links: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for (&(a, b), &w) in &pair {
        if w < 1.5 {
            continue; // au moins deux rapprochements (ou un très proche répété)
        }
        let pmi = (w * total / (marginal[a] * marginal[b])).ln();
        if pmi > 0.0 {
            links[a].push((b, pmi));
            links[b].push((a, pmi));
        }
    }
    for l in &mut links {
        l.sort_by(|x, y| y.1.total_cmp(&x.1));
        l.truncate(60);
    }
    (links, transitions)
}

// ---------- sélection ----------

/// Choisit jusqu'à `size` titres parmi des candidats notés, avec diversité :
/// au plus `per_artist` titres par artiste et 2 par album, un peu de hasard
/// hebdomadaire, et une pénalité pour les titres déjà pris par d'autres mixes.
fn pick(data: &Data, mut candidates: Vec<(usize, f64)>, size: usize, per_artist: usize, used: &HashSet<usize>, banned: &HashSet<i64>) -> Vec<usize> {
    for c in &mut candidates {
        c.1 *= 1.0 + 0.35 * noise(data.seed, data.tracks[c.0].id);
        if used.contains(&c.0) {
            c.1 *= 0.35;
        }
    }
    candidates.retain(|c| c.1 > 0.0 && data.tracks[c.0].plays >= MIN_PLAYS && !banned.contains(&data.tracks[c.0].id));
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut per_a: HashMap<i64, usize> = HashMap::new();
    let mut per_al: HashMap<i64, usize> = HashMap::new();
    let mut out = Vec::new();
    for (t, _) in candidates {
        let tr = &data.tracks[t];
        if per_a.get(&tr.artist_id).copied().unwrap_or(0) >= per_artist {
            continue;
        }
        if let Some(al) = tr.album_id
            && per_al.get(&al).copied().unwrap_or(0) >= 2
        {
            continue;
        }
        *per_a.entry(tr.artist_id).or_default() += 1;
        if let Some(al) = tr.album_id {
            *per_al.entry(al).or_default() += 1;
        }
        out.push(t);
        if out.len() == size {
            break;
        }
    }
    out
}

/// Environ 30 % de titres pas écoutés depuis longtemps, le reste familier.
fn familiar_and_forgotten(data: &Data, candidates: Vec<(usize, f64)>, used: &HashSet<usize>, banned: &HashSet<i64>) -> Vec<usize> {
    let cutoff = data.end_ts - 180 * DAY;
    let (old, recent): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|c| data.tracks[c.0].last_ts < cutoff);
    let n_old = data.size * 3 / 10;
    let mut out = pick(data, old.clone(), n_old, 3, used, banned);
    let taken: HashSet<usize> = out.iter().copied().collect();
    out.extend(pick(data, recent, data.size - out.len(), 3, used, banned));
    // Pas assez de récents : on complète avec d'autres titres anciens.
    if out.len() < data.size {
        let more: Vec<(usize, f64)> = old.into_iter().filter(|c| !taken.contains(&c.0)).collect();
        out.extend(pick(data, more, data.size - out.len(), 2, used, banned));
    }
    out
}

// ---------- ordre des titres ----------

/// Position sur la roue harmonique (roue de Camelot) : (numéro 1-12, majeur ?).
fn camelot(key: i64, mode: i64) -> (i64, bool) {
    let major = mode == 1;
    let k = if major { key } else { (key + 3) % 12 }; // relatif majeur
    ((k * 7 % 12 + 7) % 12 + 1, major)
}

fn key_cost(a: &Track, b: &Track) -> f64 {
    let (Some(ka), Some(ma), Some(kb), Some(mb)) = (a.key, a.mode, b.key, b.mode) else { return 0.5 };
    let ((na, la), (nb, lb)) = (camelot(ka, ma), camelot(kb, mb));
    let step = ((na - nb).rem_euclid(12)).min((nb - na).rem_euclid(12));
    match (step, la == lb) {
        (0, _) | (1, true) => 0.0, // même tonalité, relative, ou voisine sur la roue
        (1, false) | (2, true) => 0.5,
        _ => 1.0,
    }
}

/// Ordonne les titres pour qu'ils s'enchaînent : transitions déjà faites par
/// l'utilisateur, tempo et tonalité compatibles, courbe d'énergie qui monte puis
/// redescend, pas deux fois le même artiste de suite.
/// Ordonne les titres ; `first` : titre imposé en ouverture (mix à partir d'un titre).
fn order(data: &Data, set: Vec<usize>, first: Option<usize>) -> Vec<usize> {
    if set.len() < 3 {
        return set;
    }
    let energies: Vec<f64> = set.iter().filter_map(|&t| data.tracks[t].energy).collect();
    let (lo, hi) = if energies.is_empty() {
        (0.5, 0.5)
    } else {
        let mut e = energies.clone();
        e.sort_by(f64::total_cmp);
        (e[e.len() / 5], e[e.len() * 9 / 10])
    };
    let n = set.len();
    // Énergie visée : monte jusqu'aux deux tiers, puis redescend un peu.
    let target = |pos: usize| {
        let x = pos as f64 / (n - 1) as f64;
        let k = if x < 0.66 { x / 0.66 } else { 1.0 - (x - 0.66) / 0.34 * 0.5 };
        lo + (hi - lo) * k
    };
    let cost = |prev: usize, next: usize, pos: usize| {
        let (a, b) = (&data.tracks[prev], &data.tracks[next]);
        let tempo = match (a.tempo, b.tempo) {
            (Some(x), Some(y)) => ((x - y).abs() / x.max(y)).min(1.0) * 2.0,
            _ => 0.5,
        };
        let energy = b.energy.map_or(0.3, |e| (e - target(pos)).abs() * 2.0);
        let habit = data.transitions.get(&(prev, next)).copied().unwrap_or(0.0).min(3.0);
        let same_artist = if a.artist_id == b.artist_id { 1.5 } else { 0.0 };
        tempo + key_cost(a, b) + energy + same_artist - 0.6 * habit - 0.15 * data.link(prev, next)
    };
    // Départ : le titre imposé, sinon le plus proche de l'énergie de début.
    let mut rest = set;
    let first = first.and_then(|f| rest.iter().position(|&t| t == f)).unwrap_or_else(|| {
        (0..rest.len())
            .min_by(|&i, &j| {
                let d = |t: usize| data.tracks[rest[t]].energy.map_or(1.0, |e| (e - lo).abs());
                d(i).total_cmp(&d(j))
            })
            .unwrap()
    });
    let mut out = vec![rest.swap_remove(first)];
    while !rest.is_empty() {
        let prev = *out.last().unwrap();
        let pos = out.len();
        let best = (0..rest.len()).min_by(|&i, &j| cost(prev, rest[i], pos).total_cmp(&cost(prev, rest[j], pos))).unwrap();
        out.push(rest.swap_remove(best));
    }
    out
}

// ---------- familles de mixes ----------

/// Traits d'ambiance : axes du son nettement au-dessus ou au-dessous de la moyenne.
fn traits(centroid: &[f64; 8]) -> Vec<String> {
    let names = ["danceable", "energetic", "positive", "acoustic", "instrumental", "spoken"];
    let mut t: Vec<(f64, String)> = centroid
        .iter()
        .take(6)
        .enumerate()
        .filter(|(_, z)| z.abs() > 0.35)
        // « Peu parlé » ou « peu instrumental » ne disent rien d'une ambiance.
        .filter(|(i, z)| !((*i == 4 || *i == 5) && **z < 0.0))
        .map(|(i, z)| (z.abs(), format!("{}_{}", names[i], if *z > 0.0 { "high" } else { "low" })))
        .collect();
    t.sort_by(|a, b| b.0.total_cmp(&a.0));
    t.into_iter().take(2).map(|x| x.1).collect()
}

fn kmeans(points: &[(usize, [f64; 8], f64)], k: usize, seed: u64) -> Vec<[f64; 8]> {
    // Initialisation k-means++ déterministe.
    let mut centers: Vec<[f64; 8]> = vec![points[(splitmix(seed) as usize) % points.len()].1];
    let d2 = |a: &[f64; 8], b: &[f64; 8]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>();
    while centers.len() < k {
        let dists: Vec<f64> = points.iter().map(|p| centers.iter().map(|c| d2(&p.1, c)).fold(f64::MAX, f64::min) * p.2).collect();
        let total: f64 = dists.iter().sum();
        let mut r = noise(seed + centers.len() as u64, 7) * total;
        let mut chosen = 0;
        for (i, d) in dists.iter().enumerate() {
            r -= d;
            if r <= 0.0 {
                chosen = i;
                break;
            }
        }
        centers.push(points[chosen].1);
    }
    for _ in 0..40 {
        let mut sums = vec![([0.0; 8], 0.0); k];
        for p in points {
            let c = (0..k).min_by(|&a, &b| d2(&p.1, &centers[a]).total_cmp(&d2(&p.1, &centers[b]))).unwrap();
            for i in 0..8 {
                sums[c].0[i] += p.1[i] * p.2;
            }
            sums[c].1 += p.2;
        }
        for (c, (s, w)) in sums.iter().enumerate() {
            if *w > 0.0 {
                for i in 0..8 {
                    centers[c][i] = s[i] / w;
                }
            }
        }
    }
    centers
}

fn ambiances(data: &Data, used: &mut HashSet<usize>, banned: &HashSet<i64>, out: &mut Vec<(String, HashMap<String, String>, Vec<usize>)>) {
    // Les ambiances sont stables d'une semaine à l'autre : graine fixe pour le regroupement.
    let points: Vec<(usize, [f64; 8], f64)> =
        data.tracks.iter().enumerate().filter_map(|(i, t)| t.sound.map(|s| (i, s, t.score.sqrt()))).collect();
    if points.len() < 40 {
        return;
    }
    let k = (points.len() / 60).clamp(3, 8);
    let centers = kmeans(&points, k, 42);
    let d = |a: &[f64; 8], b: &[f64; 8]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt();
    let mut members: Vec<Vec<(usize, f64)>> = vec![Vec::new(); k];
    let mut cluster_of: HashMap<usize, usize> = HashMap::new();
    for p in &points {
        let c = (0..k).min_by(|&a, &b| d(&p.1, &centers[a]).total_cmp(&d(&p.1, &centers[b]))).unwrap();
        cluster_of.insert(p.0, c);
        // Le cœur de l'ambiance d'abord : la proximité compte plus que la popularité.
        members[c].push((p.0, data.tracks[p.0].score.sqrt() * (-1.5 * d(&p.1, &centers[c])).exp()));
    }
    // Titres sans caractéristiques : rattachés à l'ambiance de ce avec quoi on les écoute.
    for (i, t) in data.tracks.iter().enumerate() {
        if t.sound.is_some() {
            continue;
        }
        let mut votes = vec![0.0; k];
        for (j, w) in &data.links[i] {
            if let Some(&c) = cluster_of.get(j) {
                votes[c] += w;
            }
        }
        if let Some((c, &v)) = votes.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))
            && v > 1.0
        {
            members[c].push((i, t.score.sqrt() * 0.5));
        }
    }
    for (c, m) in members.into_iter().enumerate() {
        let tr = traits(&centers[c]);
        if tr.is_empty() {
            continue; // ambiance « moyenne » : rien de distinctif à proposer
        }
        let picked = familiar_and_forgotten(data, m.clone(), used, banned);
        let picked = if picked.len() >= data.min() { picked } else { pick(data, m, data.size, 3, used, banned) };
        if picked.len() < data.min() {
            continue;
        }
        let mut params = HashMap::from([("traits".to_owned(), tr.join(","))]);
        // Langue dominante (au moins 60 % des titres dont la langue est connue).
        let langs: Vec<&String> = picked.iter().flat_map(|&t| &data.tracks[t].languages).collect();
        if langs.len() >= 8 {
            let mut count: HashMap<&String, usize> = HashMap::new();
            for l in &langs {
                *count.entry(l).or_default() += 1;
            }
            if let Some((l, n)) = count.into_iter().max_by_key(|x| x.1)
                && n * 10 >= langs.len() * 6
                && l != "zxx"
            {
                params.insert("language".into(), l.clone());
            }
        }
        used.extend(&picked);
        out.push((format!("ambiance:{}", tr.join("+")), params, picked));
    }
}

/// Créneaux de la semaine : (code, jours, heures de début incluses).
const MOMENTS: [(&str, &[i64], std::ops::Range<i64>); 5] = [
    ("weekday_morning", &[0, 1, 2, 3, 4], 6..11),
    ("weekday_evening", &[0, 1, 2, 3, 4], 18..24),
    ("weekend_day", &[5, 6], 10..18),
    ("weekend_evening", &[5, 6], 18..24),
    ("night", &[0, 1, 2, 3, 4, 5, 6], 0..5),
];

/// Titres nettement sur-représentés dans un sous-ensemble d'écoutes : écart à
/// l'attendu (écoutes du titre × part du sous-ensemble) en écarts-types de Poisson.
/// Un simple ratio ferait ressortir le hasard des titres peu écoutés.
fn distinctive(data: &Data, subset: &[usize]) -> Vec<(usize, f64)> {
    let share = subset.len() as f64 / data.plays.len() as f64;
    let mut count: HashMap<usize, f64> = HashMap::new();
    for &t in subset {
        *count.entry(t).or_default() += 1.0;
    }
    count
        .into_iter()
        .filter(|&(_, c)| c >= 3.0)
        .map(|(t, c)| {
            let expected = data.tracks[t].plays * share;
            (t, (c - expected) / expected.max(0.5).sqrt())
        })
        .filter(|x| x.1 >= 2.5)
        .collect()
}

/// Garde les `keep` sous-ensembles les plus marqués (somme des écarts de leurs titres).
fn strongest<K>(data: &Data, subsets: Vec<(K, Vec<usize>)>, keep: usize) -> Vec<(K, Vec<(usize, f64)>)> {
    let mut scored: Vec<(f64, K, Vec<(usize, f64)>)> = subsets
        .into_iter()
        .map(|(k, s)| {
            let d = distinctive(data, &s);
            (d.iter().map(|x| x.1).sum(), k, d)
        })
        .filter(|x| x.2.len() >= data.min())
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(keep).map(|(_, k, d)| (k, d)).collect()
}

fn moments(data: &Data, used: &mut HashSet<usize>, banned: &HashSet<i64>, out: &mut Vec<(String, HashMap<String, String>, Vec<usize>)>) {
    let subsets = MOMENTS
        .iter()
        .map(|(code, days, hours)| (*code, data.plays.iter().filter(|p| days.contains(&p.dow) && hours.contains(&p.hour)).map(|p| p.t).collect()))
        .collect();
    for (code, cand) in strongest(data, subsets, 2) {
        let picked = pick(data, cand, data.size, 3, used, banned);
        if picked.len() >= data.min() {
            used.extend(&picked);
            out.push((format!("moment:{code}"), HashMap::from([("moment".to_owned(), code.to_owned())]), picked));
        }
    }
}

fn seasons(data: &Data, used: &mut HashSet<usize>, banned: &HashSet<i64>, out: &mut Vec<(String, HashMap<String, String>, Vec<usize>)>) {
    use chrono::{Datelike, TimeZone, Utc};
    // (année de la saison, saison) : l'hiver de décembre 2024 compte pour l'hiver 2025.
    let season_of = |ts: i64| {
        let d = Utc.timestamp_opt(ts, 0).single().unwrap_or_default();
        let (m, y) = (d.month(), d.year());
        match m {
            12 => (y + 1, "winter"),
            1 | 2 => (y, "winter"),
            3..=5 => (y, "spring"),
            6..=8 => (y, "summer"),
            _ => (y, "autumn"),
        }
    };
    let mut by: HashMap<(i32, &str), Vec<usize>> = HashMap::new();
    for p in &data.plays {
        by.entry(season_of(p.ts)).or_default().push(p.t);
    }
    let rank = |s: &str| ["winter", "spring", "summer", "autumn"].iter().position(|x| *x == s).unwrap_or(0);
    let mut keys: Vec<(i32, &str)> = by.keys().copied().collect();
    keys.sort_by_key(|k| std::cmp::Reverse((k.0, rank(k.1))));
    // La saison en cours n'est pas finie : on part de la précédente, sur deux ans.
    let subsets = keys.into_iter().skip(1).take(8).map(|k| (k, by.remove(&k).unwrap_or_default())).collect();
    for (key, cand) in strongest(data, subsets, 2) {
        let picked = pick(data, cand, data.size, 3, used, banned);
        if picked.len() >= data.min() {
            used.extend(&picked);
            out.push((
                format!("season:{}:{}", key.0, key.1),
                HashMap::from([("season".to_owned(), key.1.to_owned()), ("year".to_owned(), key.0.to_string())]),
                picked,
            ));
        }
    }
}

fn rediscover(data: &Data, used: &mut HashSet<usize>, banned: &HashSet<i64>, out: &mut Vec<(String, HashMap<String, String>, Vec<usize>)>) {
    let first = data.plays.first().map_or(0, |p| p.ts);
    // Un an sans écoute, ou six mois si l'historique est court.
    let gap = if data.end_ts - first > 540 * DAY { 365 * DAY } else { 180 * DAY };
    let cutoff = data.end_ts - gap;
    let forgotten: HashSet<usize> = data.tracks.iter().enumerate().filter(|(_, t)| t.last_ts < cutoff).map(|(i, _)| i).collect();
    // Titres beaucoup écoutés autrefois, puis leur entourage, oublié lui aussi.
    let mut cand: HashMap<usize, f64> = HashMap::new();
    for &t in &forgotten {
        let p = data.tracks[t].plays;
        if p >= 4.0 {
            *cand.entry(t).or_default() += p;
            for (n, w) in &data.links[t] {
                if forgotten.contains(n) {
                    *cand.entry(*n).or_default() += w * 0.5;
                }
            }
        }
    }
    let picked = pick(data, cand.into_iter().collect(), data.size, 3, used, banned);
    if picked.len() >= data.min() {
        used.extend(&picked);
        out.push(("rediscover".to_owned(), HashMap::from([("days".to_owned(), (gap / DAY).to_string())]), picked));
    }
}

fn around(data: &Data, used: &mut HashSet<usize>, banned: &HashSet<i64>, out: &mut Vec<(String, HashMap<String, String>, Vec<usize>)>) {
    // Artistes les plus écoutés.
    let mut by_artist: HashMap<i64, (f64, Vec<usize>)> = HashMap::new();
    for (i, t) in data.tracks.iter().enumerate() {
        let e = by_artist.entry(t.artist_id).or_default();
        e.0 += t.score;
        e.1.push(i);
    }
    let mut artists: Vec<(i64, (f64, Vec<usize>))> = by_artist.into_iter().collect();
    artists.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    for (artist_id, (_, own)) in artists.into_iter().take(3) {
        // Son moyen de l'artiste.
        let sounds: Vec<[f64; 8]> = own.iter().filter_map(|&t| data.tracks[t].sound).collect();
        let centroid = (!sounds.is_empty()).then(|| {
            let mut c = [0.0; 8];
            for s in &sounds {
                for i in 0..8 {
                    c[i] += s[i] / sounds.len() as f64;
                }
            }
            c
        });
        let mut cand: HashMap<usize, f64> = HashMap::new();
        for &t in &own {
            for (n, w) in &data.links[t] {
                if data.tracks[*n].artist_id != artist_id {
                    *cand.entry(*n).or_default() += w;
                }
            }
        }
        if let Some(c) = centroid {
            for (i, t) in data.tracks.iter().enumerate() {
                if t.artist_id == artist_id {
                    continue;
                }
                if let Some(s) = t.sound {
                    let dist = s.iter().zip(&c).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt();
                    let close = (1.5 - dist).max(0.0);
                    if close > 0.0 {
                        *cand.entry(i).or_default() += close * t.score.sqrt() * 0.3;
                    }
                }
            }
        }
        // Le son tranche : un lien d'écoute vers un titre très éloigné reste hors du mix.
        if let Some(c) = centroid {
            cand.retain(|&t, _| data.tracks[t].sound.is_none_or(|s| s.iter().zip(&c).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt() < 2.2));
        }
        let mut picked: Vec<usize> = {
            let mut o: Vec<(usize, f64)> = own.iter().map(|&t| (t, data.tracks[t].score)).collect();
            o.sort_by(|a, b| b.1.total_cmp(&a.1));
            o.into_iter().take(4).map(|x| x.0).filter(|t| data.tracks[*t].plays >= MIN_PLAYS && !banned.contains(&data.tracks[*t].id)).collect()
        };
        picked.extend(pick(data, cand.into_iter().collect(), data.size - picked.len(), 2, used, banned));
        if picked.len() >= data.min() {
            used.extend(&picked);
            out.push((
                format!("around:{artist_id}"),
                HashMap::from([("artist".to_owned(), data.tracks[own[0]].artist.clone()), ("artist_id".to_owned(), artist_id.to_string())]),
                picked,
            ));
        }
    }
}

fn gems(data: &Data, used: &mut HashSet<usize>, banned: &HashSet<i64>, out: &mut Vec<(String, HashMap<String, String>, Vec<usize>)>) {
    let mut by_plays: Vec<usize> = (0..data.tracks.len()).collect();
    by_plays.sort_by(|&a, &b| data.tracks[b].plays.total_cmp(&data.tracks[a].plays));
    let favorites: Vec<usize> = by_plays.iter().take(30).copied().collect();
    // Pépite : dans la moitié la moins écoutée des titres proposables.
    let mut plays: Vec<f64> = data.tracks.iter().map(|t| t.plays).filter(|&p| p >= MIN_PLAYS).collect();
    plays.sort_by(f64::total_cmp);
    let low = plays.get(plays.len() / 2).copied().unwrap_or(0.0).max(5.0);
    let fav: HashSet<usize> = favorites.iter().copied().collect();
    let mut cand: Vec<(usize, f64)> = Vec::new();
    for (i, t) in data.tracks.iter().enumerate() {
        if t.plays > low || fav.contains(&i) {
            continue;
        }
        let listen: f64 = favorites.iter().map(|&f| data.link(i, f)).sum();
        let sound = favorites.iter().filter_map(|&f| data.sound_dist(i, f)).fold(f64::MAX, f64::min);
        let sound = if sound == f64::MAX { 0.0 } else { (1.2 - sound).max(0.0) };
        let score = listen + sound * 2.0;
        if score > 0.0 {
            cand.push((i, score));
        }
    }
    let picked = pick(data, cand, data.size * 5 / 6, 2, used, banned);
    if picked.len() >= data.min() {
        used.extend(&picked);
        out.push(("gems".to_owned(), HashMap::new(), picked));
    }
}

/// Portée d'un retrait : le mix lui-même, sauf pour les ambiances dont le nom
/// change quand l'enrichissement progresse. Un titre n'appartient qu'à une
/// ambiance : retiré d'une ambiance, il l'est de toutes.
fn feedback_scope(key: &str) -> &str {
    if key.starts_with("ambiance:") { "ambiance" } else { key }
}

/// Mix final : retraits de l'utilisateur appliqués, titres ordonnés, pochettes, durée.
fn finish(data: &Data, removed: &HashSet<(String, i64)>, key: String, params: HashMap<String, String>, set: Vec<usize>, first: Option<usize>) -> Mix {
    let scope = feedback_scope(&key);
    let set: Vec<usize> = set.into_iter().filter(|t| !removed.contains(&(scope.to_owned(), data.tracks[*t].id))).collect();
    let tracks: Vec<MixTrack> = order(data, set, first)
        .into_iter()
        .map(|t| {
            let x = &data.tracks[t];
            MixTrack {
                id: x.id,
                title: x.title.clone(),
                artist: x.artist.clone(),
                artist_id: x.artist_id,
                album_id: x.album_id,
                album: x.album.clone(),
                plays: x.plays as i64,
                duration_ms: x.duration_ms,
            }
        })
        .collect();
    let mut covers: Vec<i64> = Vec::new();
    for a in tracks.iter().filter_map(|t| t.album_id) {
        if !covers.contains(&a) {
            covers.push(a);
        }
        if covers.len() == 4 {
            break;
        }
    }
    let duration_ms = tracks.iter().map(|t| t.duration_ms.unwrap_or(210_000)).sum();
    let kind = key.split(':').next().unwrap_or_default().to_owned();
    Mix { key, kind, params, tracks, covers, duration_ms }
}

/// Point de départ d'un mix à la demande.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedKind {
    Track,
    Album,
    Artist,
}

/// Mix à partir d'un titre, d'un album ou d'un artiste : ce qu'on écoute avec,
/// filtré et complété par le son. `own` : titres du point de départ gardés.
/// `kin` : artistes liés par les crédits (personnes en commun) → force du lien.
fn seeded(data: &Data, kind: SeedKind, id: i64, kin: &HashMap<i64, f64>, banned: &HashSet<i64>) -> Option<(Vec<usize>, Option<usize>)> {
    let of_seed = |t: &Track| match kind {
        SeedKind::Track => t.id == id,
        SeedKind::Album => t.album_id == Some(id),
        SeedKind::Artist => t.artist_id == id,
    };
    let mut seed: Vec<usize> = (0..data.tracks.len()).filter(|&i| of_seed(&data.tracks[i])).collect();
    if seed.is_empty() {
        return None;
    }
    // Les titres du départ les plus écoutés (un peu de hasard pour « Autre tirage »).
    seed.sort_by(|&a, &b| {
        let w = |t: usize| data.tracks[t].score * (1.0 + 0.5 * noise(data.seed, data.tracks[t].id));
        w(b).total_cmp(&w(a))
    });
    seed.truncate(30);
    let in_seed: HashSet<usize> = seed.iter().copied().collect();
    // Hors du départ : pour un artiste ou un album, ses autres titres n'entrent que par `own`.
    let outside = |t: usize| match kind {
        SeedKind::Track => t != seed[0],
        SeedKind::Album => !in_seed.contains(&t),
        SeedKind::Artist => data.tracks[t].artist_id != id,
    };
    let mut cand: HashMap<usize, f64> = HashMap::new();
    for &s in &seed {
        let w = data.tracks[s].score.sqrt();
        for &(n, l) in &data.links[s] {
            if outside(n) {
                *cand.entry(n).or_default() += l * w;
            }
        }
    }
    // Deuxième cercle, plus faible : utile quand le départ a peu de liens.
    let mut first_ring: Vec<(usize, f64)> = cand.iter().map(|(&k, &v)| (k, v)).collect();
    first_ring.sort_by(|a, b| b.1.total_cmp(&a.1));
    for &(c, v) in first_ring.iter().take(15) {
        for &(n, l) in &data.links[c] {
            if outside(n) {
                *cand.entry(n).or_default() += 0.2 * l * v.sqrt();
            }
        }
    }
    // Son moyen du départ : rapproche les titres voisins, écarte ceux qui détonnent.
    let sounds: Vec<[f64; 8]> = seed.iter().filter_map(|&t| data.tracks[t].sound).collect();
    if !sounds.is_empty() {
        let mut c = [0.0; 8];
        for s in &sounds {
            for i in 0..8 {
                c[i] += s[i] / sounds.len() as f64;
            }
        }
        let dist = |s: &[f64; 8]| s.iter().zip(&c).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt();
        for (i, t) in data.tracks.iter().enumerate() {
            if let Some(s) = t.sound.filter(|_| outside(i)) {
                let close = (1.5 - dist(&s)).max(0.0);
                if close > 0.0 {
                    *cand.entry(i).or_default() += close * t.score.sqrt() * 0.3;
                }
            }
        }
        cand.retain(|&t, _| data.tracks[t].sound.is_none_or(|s| dist(&s) < 2.2));
    }
    // Affinité par artiste : la somme des liens de ses titres, rapportée à sa
    // popularité. Un artiste écouté à côté de tout ne l'emporte plus sur un
    // artiste écouté surtout avec le départ.
    let mut artist_plays: HashMap<i64, f64> = HashMap::new();
    for t in &data.tracks {
        *artist_plays.entry(t.artist_id).or_default() += t.score;
    }
    let mut affinity: HashMap<i64, f64> = HashMap::new();
    for (&t, &v) in &cand {
        *affinity.entry(data.tracks[t].artist_id).or_default() += v;
    }
    for (a, v) in affinity.iter_mut() {
        *v /= artist_plays[a].sqrt();
    }
    let top = affinity.values().copied().fold(0.0, f64::max).max(1e-9);
    // Artistes liés par les crédits : présents même sans écoute commune.
    for (i, t) in data.tracks.iter().enumerate() {
        if let Some(k) = kin.get(&t.artist_id).filter(|_| outside(i)) {
            *cand.entry(i).or_default() += k.min(3.0) * t.score.sqrt() * 0.4;
        }
    }
    for (t, v) in cand.iter_mut() {
        let a = data.tracks[*t].artist_id;
        let aff = affinity.get(&a).copied().unwrap_or(0.0) / top;
        *v *= 0.3 + aff.sqrt() + if kin.contains_key(&a) { 0.5 } else { 0.0 };
    }
    let keep = |t: &usize| data.tracks[*t].plays >= MIN_PLAYS && !banned.contains(&data.tracks[*t].id);
    let (own, first): (Vec<usize>, Option<usize>) = match kind {
        SeedKind::Track => (seed.iter().take(1).copied().collect(), Some(seed[0])),
        SeedKind::Album => (seed.iter().copied().filter(keep).take(4).collect(), None),
        SeedKind::Artist => (seed.iter().copied().filter(keep).take(5).collect(), None),
    };
    let mut set = own;
    set.extend(pick(data, cand.into_iter().collect(), data.size.saturating_sub(set.len()), 3, &HashSet::new(), banned));
    (set.len() >= data.min()).then_some((set, first))
}

impl Library {
    /// Génère les mixes de la semaine, de `size` titres chacun.
    pub fn mixes(&self, size: usize) -> Result<Vec<Mix>> {
        let data = self.load_mix_data(size.clamp(10, 100))?;
        if data.tracks.iter().filter(|t| t.plays >= MIN_PLAYS).count() < 30 {
            return Ok(vec![]);
        }
        let banned = self.banned_tracks()?;
        let mut used = HashSet::new();
        let mut raw = Vec::new();
        moments(&data, &mut used, &banned, &mut raw);
        ambiances(&data, &mut used, &banned, &mut raw);
        rediscover(&data, &mut used, &banned, &mut raw);
        around(&data, &mut used, &banned, &mut raw);
        seasons(&data, &mut used, &banned, &mut raw);
        gems(&data, &mut used, &banned, &mut raw);
        let removed = self.removed_from_mixes()?;
        Ok(raw.into_iter().map(|(key, params, set)| finish(&data, &removed, key, params, set, None)).collect())
    }

    /// Un mix de la semaine recalculé à une autre taille (None s'il n'existe plus).
    pub fn mix(&self, key: &str, size: usize) -> Result<Option<Mix>> {
        Ok(self.mixes(size)?.into_iter().find(|m| m.key == key))
    }

    /// Mix à la demande à partir d'un titre, d'un album ou d'un artiste. `variant` :
    /// numéro de tirage (0 pour le premier). None s'il y a trop peu de titres liés.
    pub fn seeded_mix(&self, kind: SeedKind, id: i64, variant: u64, size: usize) -> Result<Option<Mix>> {
        let mut data = self.load_mix_data(size.clamp(10, 100))?;
        data.seed = splitmix(data.seed ^ splitmix(variant.wrapping_add(1)));
        let banned = self.banned_tracks()?;
        let (code, k) = match kind {
            SeedKind::Track => ("track", crate::stats::Kind::Track),
            SeedKind::Album => ("album", crate::stats::Kind::Album),
            SeedKind::Artist => ("artist", crate::stats::Kind::Artist),
        };
        let head = self.ranked(k, id, 0)?;
        let artist = head.artist_id.unwrap_or(id);
        let kin: HashMap<i64, f64> = self.artist_links(artist, 30)?.into_iter().map(|l| (l.artist.id, l.people.len() as f64)).collect();
        let Some((set, first)) = seeded(&data, kind, id, &kin, &banned) else { return Ok(None) };
        let name = head.name;
        let params = HashMap::from([("seed".to_owned(), code.to_owned()), ("name".to_owned(), name), ("id".to_owned(), id.to_string())]);
        let removed = self.removed_from_mixes()?;
        Ok(Some(finish(&data, &removed, format!("seed:{code}:{id}"), params, set, first)))
    }

    fn banned_tracks(&self) -> Result<HashSet<i64>> {
        let mut st = self.conn.prepare("SELECT track_id FROM mix_feedback WHERE mix_key = '*'")?;
        let r = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(r)
    }

    fn removed_from_mixes(&self) -> Result<HashSet<(String, i64)>> {
        let mut st = self.conn.prepare("SELECT mix_key, track_id FROM mix_feedback WHERE mix_key <> '*'")?;
        let r = st
            .query_map([], |r| Ok((feedback_scope(&r.get::<_, String>(0)?).to_owned(), r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(r)
    }

    /// Retire un titre d'un mix ; `mix_key = "*"` : ne plus jamais le proposer.
    pub fn mix_feedback(&self, mix_key: &str, track_id: i64) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO mix_feedback (mix_key, track_id) VALUES (?, ?)", rusqlite::params![mix_key, track_id])?;
        Ok(())
    }

    pub fn clear_mix_feedback(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM mix_feedback", [])?)
    }
}

// ---------- exports ----------

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// XSPF : lu par VLC, Strawberry, foobar2000… qui retrouvent les titres par artiste et titre.
pub fn to_xspf(title: &str, mix: &Mix) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<playlist version=\"1\" xmlns=\"http://xspf.org/ns/0/\">\n  <title>{}</title>\n  <creator>LibrestFM</creator>\n  <trackList>\n",
        xml(title)
    );
    for t in &mix.tracks {
        s.push_str("    <track>\n");
        s.push_str(&format!("      <title>{}</title>\n      <creator>{}</creator>\n", xml(&t.title), xml(&t.artist)));
        if let Some(a) = &t.album {
            s.push_str(&format!("      <album>{}</album>\n", xml(a)));
        }
        if let Some(d) = t.duration_ms {
            s.push_str(&format!("      <duration>{d}</duration>\n"));
        }
        s.push_str("    </track>\n");
    }
    s.push_str("  </trackList>\n</playlist>\n");
    s
}

/// JSPF : la version JSON de XSPF (format de ListenBrainz).
pub fn to_jspf(title: &str, mix: &Mix) -> String {
    let tracks: Vec<serde_json::Value> = mix
        .tracks
        .iter()
        .map(|t| {
            let mut o = serde_json::json!({ "title": t.title, "creator": t.artist });
            if let Some(a) = &t.album {
                o["album"] = a.clone().into();
            }
            if let Some(d) = t.duration_ms {
                o["duration"] = d.into();
            }
            o
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "playlist": { "title": title, "creator": "LibrestFM", "track": tracks } })).unwrap_or_default()
}

/// CSV : artiste, titre, album, durée (s).
pub fn to_csv(mix: &Mix) -> String {
    let mut w = csv::Writer::from_writer(Vec::new());
    let _ = w.write_record(["artist", "title", "album", "duration_s"]);
    for t in &mix.tracks {
        let _ = w.write_record([
            t.artist.as_str(),
            t.title.as_str(),
            t.album.as_deref().unwrap_or(""),
            &t.duration_ms.map(|d| (d / 1000).to_string()).unwrap_or_default(),
        ]);
    }
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_plays_weigh_more() {
        assert!((recency(0) - 1.0).abs() < 1e-9);
        assert!((recency(90 * DAY) - (FLOOR + (1.0 - FLOOR) * 0.5)).abs() < 1e-9);
        // 20 écoutes ce mois-ci pèsent plus que 1 000 écoutes d'il y a cinq ans.
        assert!(20.0 * recency(15 * DAY) > 1000.0 * recency(5 * 365 * DAY));
    }

    #[test]
    fn camelot_wheel() {
        assert_eq!(camelot(0, 1), (8, true)); // do majeur = 8B
        assert_eq!(camelot(9, 0), (8, false)); // la mineur = 8A
        assert_eq!(camelot(7, 1), (9, true)); // sol majeur = 9B
        assert_eq!(camelot(6, 0), (11, false)); // fa♯ mineur = 11A
    }

    #[test]
    fn album_runs_barely_link_their_tracks() {
        // Session 1 : un album écouté en entier (titres 0, 1, 2, 3).
        // Sessions 2-4 : 4 et 5 écoutés ensemble trois fois, sans album.
        let mut plays = Vec::new();
        let mut ts = 0;
        for t in 0..4 {
            plays.push(Play { ts, t, album_id: Some(1), dow: 0, hour: 0 });
            ts += 200;
        }
        for _ in 0..3 {
            ts += 10_000;
            for t in [4, 5] {
                plays.push(Play { ts, t, album_id: None, dow: 0, hour: 0 });
                ts += 200;
            }
        }
        // Une deuxième écoute du même album.
        ts += 10_000;
        for t in 0..4 {
            plays.push(Play { ts, t, album_id: Some(1), dow: 0, hour: 0 });
            ts += 200;
        }
        let (links, transitions) = listening_links(&plays, 6);
        assert!(links[0].is_empty() && links[1].is_empty(), "écoutes d'album : pas de lien");
        assert!(links[4].iter().any(|(b, _)| *b == 5), "écoutes ensemble hors album : lien");
        assert!(!transitions.contains_key(&(0, 1)) && transitions.contains_key(&(4, 5)));
    }

    #[test]
    fn ambiance_removals_survive_a_rename() {
        assert_eq!(feedback_scope("ambiance:calm+instrumental"), feedback_scope("ambiance:acoustic+calm"));
        assert_eq!(feedback_scope("moment:night"), "moment:night");
    }

    #[test]
    fn exports_escape_and_list_tracks() {
        let mix = Mix {
            key: "k".into(),
            kind: "gems".into(),
            params: HashMap::new(),
            tracks: vec![MixTrack {
                id: 1,
                title: "Rock & Roll <live>".into(),
                artist: "AC/DC".into(),
                artist_id: 1,
                album_id: None,
                album: Some("Back in Black".into()),
                plays: 3,
                duration_ms: Some(255_000),
            }],
            covers: vec![],
            duration_ms: 255_000,
        };
        let x = to_xspf("Mix", &mix);
        assert!(x.contains("<title>Rock &amp; Roll &lt;live&gt;</title>") && x.contains("<duration>255000</duration>"));
        assert!(to_jspf("Mix", &mix).contains("\"creator\": \"AC/DC\""));
        assert_eq!(to_csv(&mix).lines().nth(1), Some("AC/DC,Rock & Roll <live>,Back in Black,255"));
    }
}
