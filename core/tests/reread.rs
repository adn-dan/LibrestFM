//! Relectures de sources, réunions après fusion, dossier d'import.

use librestfm_core::Library;
use librestfm_core::stats::Kind;

const T: i64 = 1_709_287_200; // 2024-03-01 10:00:00 UTC

fn count(lib: &Library) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap()
}

fn csv(rows: &[(&str, i64)]) -> String {
    let mut s = String::from("artist,title,timestamp\n");
    for (t, ts) in rows {
        s.push_str(&format!("Daft Punk,{t},{ts}\n"));
    }
    s
}

fn bump_mtime(p: &std::path::Path) {
    let f = std::fs::File::options().write(true).open(p).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10)).unwrap();
}

/// H1 : un fichier du dossier d'import relu (même contenu, date changée)
/// recompte une écoute fusionnée avec une autre source à une heure différente.
#[test]
fn inbox_reread_duplicates_merged_play() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", csv(&[("Get Lucky", T)]).as_bytes()).unwrap();
    let b = dir.path().join("b.csv");
    std::fs::write(&b, csv(&[("Get Lucky", T + 30)])).unwrap();
    lib.scan_input(dir.path()).unwrap();
    assert_eq!(count(&lib), 1, "fusion initiale");
    bump_mtime(&b); // même contenu, touché (copie, synchro…)
    let rep = lib.scan_input(dir.path()).unwrap();
    println!("relecture : {:?}", rep.files.iter().map(|f| (f.added, f.merged)).collect::<Vec<_>>());
    assert_eq!(count(&lib), 1, "écoute comptée deux fois après relecture");
}

/// H1 bis : source à la minute (lastfm-to-csv) dans le dossier, heure corrigée
/// par une source précise, puis fichier complété : l'écoute revient en double.
#[test]
fn inbox_grown_minute_file_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    let a = dir.path().join("lastfm.csv");
    std::fs::write(&a, "Daft Punk,RAM,Get Lucky,01 Mar 2024 10:00\n").unwrap();
    lib.scan_input(dir.path()).unwrap();
    lib.import_bytes("lb.csv", csv(&[("Get Lucky", T + 37)]).as_bytes()).unwrap();
    assert_eq!(count(&lib), 1);
    std::fs::write(&a, "Daft Punk,RAM,One More Time,01 Mar 2024 11:00\nDaft Punk,RAM,Get Lucky,01 Mar 2024 10:00\n").unwrap();
    bump_mtime(&a);
    lib.scan_input(dir.path()).unwrap();
    assert_eq!(count(&lib), 2, "attendu : Get Lucky + One More Time");
}

/// H2 : réunion après fusion de titres : une écoute d'une source absorbe deux
/// écoutes distinctes d'une autre source (que l'import, lui, garde séparées).
#[test]
fn fold_swallows_two_plays_of_same_source() {
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", csv(&[("Get Lucky", T + 30)]).as_bytes()).unwrap();
    lib.import_bytes("b.csv", csv(&[("Get Lucy", T), ("Get Lucy", T + 60)]).as_bytes()).unwrap();
    // title_key ignore peut-être la ponctuation : on vérifie qu'il y a bien deux titres.
    let tracks: i64 = lib.conn.query_row("SELECT COUNT(*) FROM track", [], |r| r.get(0)).unwrap();
    let typo: (i64, i64) = if tracks == 2 {
        let a = lib.conn.query_row("SELECT id FROM track WHERE title = 'Get Lucky'", [], |r| r.get(0)).unwrap();
        let b = lib.conn.query_row("SELECT id FROM track WHERE title = 'Get Lucy'", [], |r| r.get(0)).unwrap();
        (b, a)
    } else {
        panic!("titres confondus à l'import, changer la faute de frappe");
    };
    assert_eq!(count(&lib), 3);
    lib.merge(Kind::Track, typo.0, typo.1, "manual").unwrap();
    // Même chose importée après la fusion : 2 écoutes (b.csv en a deux, a.csv recoupe l'une).
    let mut lib2 = Library::open_in_memory().unwrap();
    lib2.import_bytes("a.csv", csv(&[("Get Lucky", T + 30)]).as_bytes()).unwrap();
    lib2.import_bytes("x.csv", csv(&[("Get Lucy", T - 9999)]).as_bytes()).unwrap();
    let ids: Vec<i64> = { let mut st = lib2.conn.prepare("SELECT id FROM track ORDER BY id").unwrap(); st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
    lib2.merge(Kind::Track, ids[1], ids[0], "manual").unwrap();
    lib2.import_bytes("b.csv", csv(&[("Get Lucy", T), ("Get Lucy", T + 60)]).as_bytes()).unwrap();
    let after_import = count(&lib2) - 1;
    println!("import après fusion : {after_import}, réunion après coup : {}", count(&lib));
    assert_eq!(count(&lib), after_import, "la réunion perd une écoute");
}

fn plays_of(lib: &Library, title: &str) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM scrobble s JOIN track t ON t.id = s.raw_track_id WHERE t.title = ?", [title], |r| r.get(0)).unwrap()
}

fn setup_fold() -> (Library, i64, i64, i64) {
    let mut lib = Library::open_in_memory().unwrap();
    let a = lib.import_bytes("a.csv", csv(&[("Get Lucky", T)]).as_bytes()).unwrap().source_id.unwrap();
    let b = lib.import_bytes("b.csv", csv(&[("Get Lucy", T + 30)]).as_bytes()).unwrap().source_id.unwrap();
    let ids: Vec<i64> = { let mut st = lib.conn.prepare("SELECT id FROM track ORDER BY id").unwrap(); st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
    let op = lib.merge(Kind::Track, ids[1], ids[0], "manual").unwrap();
    assert_eq!(count(&lib), 1);
    (lib, a, b, op)
}

#[test]
fn fold_then_remove_keep_source_then_undo() {
    let (mut lib, a, _, op) = setup_fold();
    lib.remove_source(a).unwrap();
    assert_eq!(count(&lib), 1);
    lib.undo_merge(op).unwrap();
    assert_eq!((plays_of(&lib, "Get Lucky"), plays_of(&lib, "Get Lucy")), (0, 1));
}

#[test]
fn fold_then_remove_dup_source_then_undo() {
    let (mut lib, _, b, op) = setup_fold();
    lib.remove_source(b).unwrap();
    assert_eq!(count(&lib), 1);
    lib.undo_merge(op).unwrap();
    assert_eq!((plays_of(&lib, "Get Lucky"), plays_of(&lib, "Get Lucy")), (1, 0));
}

#[test]
fn fold_then_third_source_then_undo() {
    let (mut lib, _, _, op) = setup_fold();
    lib.import_bytes("c.csv", csv(&[("Get Lucky", T + 10)]).as_bytes()).unwrap();
    assert_eq!(count(&lib), 1);
    lib.undo_merge(op).unwrap();
    assert_eq!((plays_of(&lib, "Get Lucky"), plays_of(&lib, "Get Lucy")), (1, 1));
    let srcs: i64 = lib.conn.query_row("SELECT COUNT(*) FROM scrobble_source", [], |r| r.get(0)).unwrap();
    assert_eq!(srcs, 3);
}

/// Écoute de la source B réunie, puis B relue (dossier d'import) : reste-t-elle unique ?
#[test]
fn fold_then_reread_dup_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", csv(&[("Get Lucky", T)]).as_bytes()).unwrap();
    let b = dir.path().join("b.csv");
    std::fs::write(&b, csv(&[("Get Lucy", T + 30)])).unwrap();
    lib.scan_input(dir.path()).unwrap();
    let ids: Vec<i64> = { let mut st = lib.conn.prepare("SELECT id FROM track ORDER BY id").unwrap(); st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
    let op = lib.merge(Kind::Track, ids[1], ids[0], "manual").unwrap();
    bump_mtime(&b);
    lib.scan_input(dir.path()).unwrap();
    println!("après relecture : {} écoutes", count(&lib));
    lib.undo_merge(op).unwrap();
    assert_eq!((plays_of(&lib, "Get Lucky"), plays_of(&lib, "Get Lucy")), (1, 1), "après annulation");
}

/// Fichier du dossier remplacé par un export plus complet (historique plus ancien en plus).
#[test]
fn inbox_backfilled_file_loses_older_plays() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    let f = dir.path().join("export.csv");
    std::fs::write(&f, csv(&[("Get Lucky", T)])).unwrap();
    lib.scan_input(dir.path()).unwrap();
    let full = csv(&[("Get Lucky", T), ("One More Time", T - 100 * 86_400), ("Da Funk", T - 200 * 86_400)]);
    std::fs::write(&f, &full).unwrap();
    bump_mtime(&f);
    lib.scan_input(dir.path()).unwrap();
    let manual = lib.import_bytes("export.csv", full.as_bytes()).unwrap();
    println!("import manuel du même fichier : {:?}", manual.status);
    assert_eq!(count(&lib), 3, "écoutes anciennes ignorées");
}

/// Les caches inutiles sont vidés et la place rendue.
#[test]
fn caches_are_purged_and_space_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open(&dir.path().join("db.sqlite")).unwrap();
    lib.conn.execute("INSERT INTO artist (name, key) VALUES ('A', 'a')", []).unwrap();
    let big = "x".repeat(4 << 20);
    // Artiste sans titre en attente : son catalogue ne sert plus.
    lib.conn.execute("INSERT INTO reccobeats_artist_cache VALUES (1, strftime('%s','now'), ?)", [&big]).unwrap();
    lib.conn.execute("INSERT INTO disc_cache VALUES ('old', ?, 0)", [&big]).unwrap();
    lib.conn.execute("INSERT INTO disc_cache VALUES ('fresh', '{}', strftime('%s','now'))", []).unwrap();
    assert_eq!(lib.purge_caches().unwrap(), 2);
    let left: i64 = lib.conn.query_row("SELECT COUNT(*) FROM disc_cache", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 1);
    assert!(lib.compact_if_worth().unwrap(), "8 Mo libres sur ~8 Mo : compactée");
    assert!(!lib.compact_if_worth().unwrap(), "plus rien à rendre");
    // Toujours utilisable après le remplacement du fichier.
    assert_eq!(lib.conn.query_row("SELECT COUNT(*) FROM artist", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert!(std::fs::metadata(dir.path().join("db.sqlite")).unwrap().len() < 1 << 20);
}
