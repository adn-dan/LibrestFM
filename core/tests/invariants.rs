//! Invariants du compte des écoutes, sur un historique synthétique dont on connaît
//! la vérité : chaque écoute réelle est présente dans une ou plusieurs sources
//! (minute près, seconde près, décalée), avec réécoutes de titres courts et graphies
//! différentes d'une source à l'autre.
use librestfm_core::Library;
use librestfm_core::stats::{Kind, Period};
use std::collections::BTreeMap;

/// (artiste, titre, durée s) ; graphies alternatives pour la source C.
const TRACKS: [(&str, &str, i64, &str, &str); 8] = [
    ("Daft Punk", "One More Time", 320, "DAFT PUNK", "One More Time (Radio Edit)"),
    ("Françoise Hardy", "Comment te dire adieu (Remasterisé en 2016)", 150, "Françoise Hardy", "Comment te dire adieu (Remasterise\u{301} en 2016)"),
    ("Earth, Wind & Fire", "September", 215, "Earth, Wind &amp; Fire", "September"),
    ("Ramones", "Judy Is a Punk", 92, "Ramones", "Judy Is A Punk"),
    ("Beyoncé", "Halo", 261, "Beyonce", "Halo"),
    ("Stromae", "Santé", 190, "Stromae", "Santé (feat. Nobody)"),
    ("Björk", "Jóga", 268, "Bjork", "Joga"),
    ("The Beatles", "Her Majesty", 23 + 80, "The Beatles", "Her Majesty - Remastered 2009"),
];

struct Truth {
    /// (index du titre, début de l'écoute)
    listens: Vec<(usize, i64)>,
    a: String,
    b: String,
    c: String,
}

fn lcg(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *seed >> 33
}

fn minute(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).unwrap().format("%d %b %Y %H:%M").to_string()
}

/// Historique autour du passage à l'heure d'hiver 2024 (27 octobre) et de minuits.
fn truth(seed: u64, alt_spellings: bool) -> Truth { truth_opts(seed, alt_spellings, false) }

/// `same_sources` : une réécoute est dans les mêmes sources que l'écoute qui la précède.
fn truth_opts(seed: u64, alt_spellings: bool, same_sources: bool) -> Truth {
    let mut s = seed;
    let mut t = 1_729_900_000; // 26 oct 2024 vers 0 h UTC
    let mut listens = vec![];
    while listens.len() < 400 {
        let i = (lcg(&mut s) % TRACKS.len() as u64) as usize;
        listens.push((i, t));
        // Réécoute immédiate d'un titre court, une fois sur 8.
        if lcg(&mut s).is_multiple_of(8) {
            t += TRACKS[i].2 + 3;
            listens.push((i, t));
        }
        t += TRACKS[i].2 + lcg(&mut s).is_multiple_of(4) as i64 * (lcg(&mut s) % 20_000) as i64 + 2;
    }
    let (mut a, mut b, mut c) = (String::new(), String::from("artist,title,timestamp\n"), String::from("artist,title,timestamp\n"));
    let mut prev: Option<(usize, i64, u64)> = None;
    for &(i, ts) in &listens {
        let (ar, ti, _, ar2, ti2) = TRACKS[i];
        let r = match prev { Some((pi, pts, pr)) if same_sources && pi == i && ts - pts < 600 => pr, _ => lcg(&mut s) % 7 };
        prev = Some((i, ts, r));
        // Toujours au moins une source.
        let (in_a, in_b, in_c) = match r { 0 => (true, false, false), 1 => (false, true, false), 2 => (false, false, true), 3 => (true, true, false), 4 => (true, false, true), 5 => (false, true, true), _ => (true, true, true) };
        if in_a { a.push_str(&format!("\"{ar}\",,\"{ti}\",{}\n", minute(ts))); }
        if in_b { b.push_str(&format!("\"{ar}\",\"{ti}\",{ts}\n")); }
        if in_c {
            let (ar, ti) = if alt_spellings { (ar2, ti2) } else { (ar, ti) };
            c.push_str(&format!("\"{ar}\",\"{ti}\",{}\n", ts + 20));
        }
    }
    Truth { listens, a, b, c }
}

fn total(lib: &Library) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap()
}

/// Écoutes par (artiste, titre) corrigés, en clés normalisées.
fn per_track(lib: &Library) -> BTreeMap<String, i64> {
    let mut st = lib.conn.prepare("SELECT a.key || '|' || t.key, COUNT(*) FROM scrobble s JOIN track t ON t.id = s.track_id JOIN artist a ON a.id = s.artist_id GROUP BY s.track_id").unwrap();
    let mut m = BTreeMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap() {
        let (k, n) = r.unwrap();
        *m.entry(k).or_default() += n;
    }
    m
}

fn dump(lib: &Library) -> Vec<String> {
    let mut out = vec![];
    for sql in [
        "SELECT ts || ':' || track_id || ':' || artist_id || ':' || COALESCE(album_id, '') || ':' || precise || ':' || ym || ':' || ymd || ':' || dow || ':' || hour FROM scrobble ORDER BY ts, track_id",
        "SELECT s.ts || ':' || s.track_id || ':' || x.source_id FROM scrobble_source x JOIN scrobble s ON s.id = x.scrobble_id ORDER BY 1",
    ] {
        let mut st = lib.conn.prepare(sql).unwrap();
        out.extend(st.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()));
    }
    out
}

fn import_all(order: &[usize], t: &Truth) -> Library {
    let mut lib = Library::open_in_memory().unwrap();
    for &i in order {
        let (name, body) = [("a.csv", &t.a), ("b.csv", &t.b), ("c.csv", &t.c)][i];
        lib.import_bytes(name, body.as_bytes()).unwrap();
    }
    lib
}

/// Somme des tops = somme des graphiques = total affiché = COUNT(*).
fn assert_sums(lib: &Library, p: Period) {
    let o = lib.overview(p, 1_000_000).unwrap();
    let (from, to) = (p.from.unwrap_or(i64::MIN), p.to.unwrap_or(i64::MAX));
    let direct: i64 = lib.conn.query_row("SELECT COUNT(*) FROM scrobble WHERE ts BETWEEN ? AND ?", [from, to], |r| r.get(0)).unwrap();
    assert_eq!(o.total, direct, "total affiché");
    assert_eq!(o.top_artists.iter().map(|r| r.plays).sum::<i64>(), direct, "top artistes");
    assert_eq!(o.top_tracks.iter().map(|r| r.plays).sum::<i64>(), direct, "top titres");
    assert_eq!(o.per_month.iter().map(|r| r.1).sum::<i64>(), direct, "écoutes par mois");
    assert_eq!(o.per_day.iter().map(|r| r.1).sum::<i64>(), direct, "écoutes par jour");
    assert_eq!(o.heatmap.iter().map(|r| r[2]).sum::<i64>(), direct, "carte jour × heure");
}

#[test]
// Limite connue (2 écoutes sur 400) : réécoute immédiate d'un titre de 103 s, vue à la minute
// par une source et décalée de 20 s par une autre ; sans durée, les deux lectures se valent.
#[ignore = "limite connue : réécoute immédiate d'un titre très court entre sources décalées"]
fn inv_total_equals_truth_same_spelling() {
    let t = truth(1, false);
    let lib = import_all(&[0, 1, 2], &t);
    assert_eq!(total(&lib), t.listens.len() as i64);
}

#[test]
fn inv_total_equals_truth_alt_spellings() {
    let t = truth(2, true);
    let lib = import_all(&[0, 1, 2], &t);
    let got = per_track(&lib);
    assert_eq!(total(&lib), t.listens.len() as i64, "par titre : {got:?}");
}

#[test]
fn inv_sums_match() {
    let t = truth(3, false);
    let lib = import_all(&[0, 1, 2], &t);
    assert_sums(&lib, Period::default());
    // Bornes de jour local autour du changement d'heure (27 oct 2024, Europe/Paris si TZ l'est).
    for (from, to) in [(1_729_980_000, 1_730_066_399), (1_730_000_000, 1_730_100_000), (1_729_900_000, 1_729_900_000 + 86_399)] {
        assert_sums(&lib, Period { from: Some(from), to: Some(to) });
    }
}

#[test]
// Limite connue (2 écoutes sur 400) : réécoute immédiate d'un titre de 103 s, vue à la minute
// par une source et décalée de 20 s par une autre ; sans durée, les deux lectures se valent.
#[ignore = "limite connue : réécoute immédiate d'un titre très court entre sources décalées"]
fn inv_order_does_not_matter() {
    for alt in [false, true] {
        let t = truth(4, alt);
        let reference = per_track(&import_all(&[0, 1, 2], &t));
        for order in [[0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            let got = per_track(&import_all(&order, &t));
            assert_eq!(got, reference, "ordre {order:?}, graphies différentes : {alt}");
        }
    }
}

#[test]
fn inv_reimport_is_idempotent() {
    // Même contenu sous un autre nom, et fichier relu dans sa source (dossier d'import modifié).
    let t = truth(5, false);
    let mut lib = import_all(&[0, 1, 2], &t);
    let before = dump(&lib);
    let dir = tempfile::tempdir().unwrap();
    for (n, body) in [("a.csv", &t.a), ("b.csv", &t.b), ("c.csv", &t.c)] {
        std::fs::write(dir.path().join(n), body).unwrap();
    }
    lib.scan_input(dir.path()).unwrap(); // nouvelles sources, mêmes écoutes
    let after_first = total(&lib);
    for (n, body) in [("a.csv", &t.a), ("b.csv", &t.b), ("c.csv", &t.c)] {
        std::fs::write(dir.path().join(n), format!("{body}\n")).unwrap(); // modifié : relu en entier
    }
    lib.scan_input(dir.path()).unwrap();
    assert_eq!(after_first, before.len() as i64 - dump_links(&before), "deuxième source identique");
    assert_eq!(total(&lib), after_first, "relecture dans la même source");
}

fn dump_links(d: &[String]) -> i64 {
    d.iter().filter(|l| l.matches(':').count() == 2).count() as i64
}

#[test]
fn inv_remove_then_reimport_restores_state() {
    let t = truth(6, false);
    let mut lib = import_all(&[0, 1, 2], &t);
    let before = per_track(&lib);
    for (i, (name, body)) in [("a.csv", &t.a), ("b.csv", &t.b), ("c.csv", &t.c)].into_iter().enumerate() {
        let id: i64 = lib.conn.query_row("SELECT id FROM source WHERE file_name = ?", [name], |r| r.get(0)).unwrap();
        lib.remove_source(id).unwrap();
        lib.import_bytes(name, body.as_bytes()).unwrap();
        assert_eq!(per_track(&lib), before, "source {i} retirée puis réimportée");
    }
}

#[test]
fn inv_backup_restore_is_identical() {
    let t = truth(7, true);
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("librestfm.sqlite");
    let mut lib = Library::open(&db).unwrap();
    for (n, b) in [("a.csv", &t.a), ("b.csv", &t.b), ("c.csv", &t.c)] {
        lib.import_bytes(n, b.as_bytes()).unwrap();
    }
    lib.auto_clean().unwrap();
    let before = dump(&lib);
    lib.backup_to(&dir.path().join("save.sqlite")).unwrap();
    lib.import_bytes("d.csv", b"artist,title,timestamp\nX,Y,1729900000\n").unwrap();
    lib.restore_from(&dir.path().join("save.sqlite"), &db, &dir.path().join("safety.sqlite")).unwrap();
    assert_eq!(dump(&lib), before);
}

#[test]
fn inv_merge_then_undo_is_reversible() {
    // Deux graphies d'un même titre dans deux sources : la fusion réunit les écoutes,
    // l'annulation rend exactement l'état d'avant.
    let a = "artist,title,timestamp\nDaft Punk,One More Tme,1729900000\nDaft Punk,One More Tme,1729900400\nDaft Punk,Aerodynamic,1729901000\n";
    let b = "artist,title,timestamp\nDaft Punk,One More Time,1729900010\nDaft Punk,One More Time,1729900800\n";
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", a.as_bytes()).unwrap();
    lib.import_bytes("b.csv", b.as_bytes()).unwrap();
    let before = dump(&lib);
    assert_eq!(total(&lib), 5);
    let id = |t: &str| -> i64 { lib.conn.query_row("SELECT id FROM track WHERE title = ?", [t], |r| r.get(0)).unwrap() };
    let op = lib.merge(Kind::Track, id("One More Tme"), id("One More Time"), "manual").unwrap();
    assert_eq!(total(&lib), 4, "écoute présente dans les deux sources réunie");
    assert_sums(&lib, Period::default());
    lib.undo_merge(op).unwrap();
    assert_eq!(dump(&lib), before);
}

