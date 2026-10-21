//! Réunions à trois sources, dossier d'import, sauvegarde, doubles envois.
use librestfm_core::Library;
use librestfm_core::stats::Kind;

const T: i64 = 1_709_287_200; // 2024-03-01 10:00:00 UTC

fn count(lib: &Library) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap()
}
fn csv(rows: &[(&str, i64)]) -> String {
    let mut s = String::from("artist,title,album,timestamp\n");
    for (t, ts) in rows { s.push_str(&format!("Daft Punk,{t},,{ts}\n")); }
    s
}
fn track_id(lib: &Library, title: &str) -> i64 {
    lib.conn.query_row("SELECT id FROM track WHERE title = ?", [title], |r| r.get(0)).unwrap()
}
fn bump_mtime(p: &std::path::Path) {
    let f = std::fs::File::options().write(true).open(p).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10)).unwrap();
}

/// trois sources, trois graphies d'une même écoute, réunies en une passe
/// (keep change en cours de route) ; au recalcul suivant, l'écoute revient en double.
#[test]
fn three_way_fold_duplicates_on_next_rebuild() {
    let mut lib = Library::open_in_memory().unwrap();
    // A : à la minute (lastfm-to-csv), B : précise sans album, C : précise avec album.
    lib.import_bytes("a.csv", b"Daft Punk,,Get Lucky,01 Mar 2024 10:00\n").unwrap();
    lib.import_bytes("b.csv", csv(&[("Get Lucy", T + 60)]).as_bytes()).unwrap();
    lib.import_bytes("c.csv", format!("artist,title,album,timestamp\nDaft Punk,Get Luky,RAM,{}\n", T + 100).as_bytes()).unwrap();
    assert_eq!(count(&lib), 3);
    let (a, b, c) = (track_id(&lib, "Get Lucky"), track_id(&lib, "Get Lucy"), track_id(&lib, "Get Luky"));
    lib.merge(Kind::Track, b, a, "manual").unwrap();
    lib.merge(Kind::Track, c, a, "manual").unwrap();
    println!("après les deux fusions : {}", count(&lib));
    assert_eq!(count(&lib), 1);
    lib.rebuild_canonical().unwrap(); // n'importe quelle fusion, annulation, import…
    println!("après un recalcul : {}", count(&lib));
    println!("après deux recalculs : {}", count(&lib));
    assert_eq!(count(&lib), 1, "l'écoute est comptée plusieurs fois");
}

/// retirer la source d'un fichier du dossier d'import ; au démarrage suivant il revient.
#[test]
fn removed_inbox_source_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    std::fs::write(dir.path().join("x.csv"), csv(&[("Get Lucky", T)])).unwrap();
    let rep = lib.scan_input(dir.path()).unwrap();
    let src = rep.files[0].source_id.unwrap();
    lib.remove_source(src).unwrap();
    assert_eq!(count(&lib), 0);
    let rep = lib.scan_input(dir.path()).unwrap(); // prochain démarrage, fichier inchangé
    println!("relu : {} fichier(s), {} inchangé(s)", rep.files.len(), rep.unchanged);
    assert_eq!(count(&lib), 0, "la source retirée est réimportée");
}

/// un fichier complété n'apporte qu'une ligne refusée : first_ts/last_ts de la source effacés.
#[test]
fn source_dates_erased_by_empty_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    let f = dir.path().join("x.csv");
    let base = csv(&[("Get Lucky", T)]);
    std::fs::write(&f, &base).unwrap();
    lib.scan_input(dir.path()).unwrap();
    std::fs::write(&f, format!("{base}Daft Punk,clip.mp4,,{}\n", T + 500)).unwrap();
    bump_mtime(&f);
    lib.scan_input(dir.path()).unwrap();
    let s = &lib.sources().unwrap()[0];
    println!("first_ts {:?} last_ts {:?}", s.first_ts, s.last_ts);
    assert_eq!((s.first_ts, s.last_ts), (Some(T), Some(T)));
}

/// fichier complété (journal) : deux écoutes d'un titre court à 60 s
/// d'intervalle, l'une avant, l'autre après l'ajout ; la seconde disparaît.
#[test]
fn grown_file_short_track_replay_lost() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    let f = dir.path().join("journal.csv");
    let first = csv(&[("Intro", T)]);
    std::fs::write(&f, &first).unwrap();
    lib.scan_input(dir.path()).unwrap();
    std::fs::write(&f, format!("{first}Daft Punk,Intro,,{}\n", T + 60)).unwrap();
    bump_mtime(&f);
    lib.scan_input(dir.path()).unwrap();
    // Le même contenu importé d'un coup donne 2 écoutes.
    let mut ref_lib = Library::open_in_memory().unwrap();
    ref_lib.import_bytes("j.csv", format!("{first}Daft Punk,Intro,,{}\n", T + 60).as_bytes()).unwrap();
    println!("complété : {}, d'un coup : {}", count(&lib), count(&ref_lib));
    assert_eq!(count(&lib), count(&ref_lib));
}

/// Sauvegarder ou exporter par-dessus la base est refusé.
#[test]
fn backup_over_the_database_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("librestfm.sqlite");
    let mut lib = Library::open(&db).unwrap();
    lib.import_bytes("a.csv", csv(&[("Get Lucky", T)]).as_bytes()).unwrap();
    assert!(lib.backup_to(&db).is_err());
    assert!(lib.backup_to(&dir.path().join("librestfm.sqlite-wal")).is_err());
    // Même chemin écrit autrement.
    assert!(lib.backup_to(&dir.path().join(".").join("librestfm.sqlite")).is_err());
    lib.import_bytes("b.csv", csv(&[("One More Time", T + 3600)]).as_bytes()).unwrap();
    drop(lib);
    assert_eq!(count(&Library::open(&db).unwrap()), 2, "rien de perdu");
    assert!(Library::open(&db).unwrap().backup_to(&dir.path().join("copie.sqlite")).is_ok());
}

#[test]
fn double_sends_are_dropped() {
    let mut lib = Library::open_in_memory().unwrap();
    // 3 min. Même source : +10 s (double envoi), +100 s (après la moitié du 1er : gardé),
    // puis +1000 s (réécoute).
    lib.import_bytes("a.csv", csv(&[("Get Lucky", 1_700_000_000), ("Get Lucky", 1_700_000_010), ("Get Lucky", 1_700_000_100), ("Get Lucky", 1_700_001_000)]).as_bytes()).unwrap();
    // Autre source, même moment : c'est la même écoute, pas un double envoi.
    lib.import_bytes("b.csv", csv(&[("Get Lucky", 1_700_000_000)]).as_bytes()).unwrap();
    let t = track_id(&lib, "Get Lucky");
    // Durée inconnue : seul l'envoi à +10 s (moins de 30 s) est un double.
    assert_eq!(lib.drop_double_sends().unwrap(), 1, "durée inconnue : moins de 30 s seulement");
    lib.conn.execute("UPDATE track SET duration_ms = 180000 WHERE id = ?", [t]).unwrap();
    assert_eq!(lib.auto_clean().unwrap(), 0);
    let ts: Vec<i64> = lib.conn.prepare("SELECT ts FROM scrobble ORDER BY ts").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(ts, [1_700_000_000, 1_700_000_100, 1_700_001_000]);
    // Relecture de la source : le double revient, le nettoyage le retire.
    lib.import_bytes("a2.csv", csv(&[("Get Lucky", 1_700_000_000), ("Get Lucky", 1_700_000_010)]).as_bytes()).unwrap();
    lib.auto_clean().unwrap();
    assert_eq!(count(&lib), 3);
}
