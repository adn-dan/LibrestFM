//! Recalcul interrompu, ligne en cours d'écriture, doubles envois rétablis, nom corrigé.
use librestfm_core::Library;
use librestfm_core::stats::Kind;

fn count(lib: &Library) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap()
}

/// Recalcul interrompu après le dépliage des écoutes réunies : il ne doit rien laisser
/// à moitié fait (l'écoute réunie revenait en double).
#[test]
fn rebuild_is_atomic() {
    let mut lib = Library::open_in_memory().unwrap();
    // Deux graphies d'une même écoute, réunies par une fusion manuelle.
    lib.import_bytes("a.csv", b"artist,title,album,timestamp\nDaft Punk,Get Lucky,,1709287200\n").unwrap();
    lib.import_bytes("b.csv", b"artist,title,album,timestamp\nDaft Punk,Get Lucky Alt,,1709287230\n").unwrap();
    let id = |t: &str| lib.conn.query_row("SELECT id FROM track WHERE title = ?", [t], |r| r.get::<_, i64>(0)).unwrap();
    lib.merge(Kind::Track, id("Get Lucky Alt"), id("Get Lucky"), "manual").unwrap();
    assert_eq!(count(&lib), 1);
    // Panne simulée pendant le repliage.
    lib.conn.execute_batch("CREATE TEMP TRIGGER boom BEFORE INSERT ON scrobble_fold BEGIN SELECT RAISE(ABORT, 'panne'); END;").unwrap();
    assert!(lib.rebuild_canonical().is_err());
    lib.conn.execute_batch("DROP TRIGGER boom;").unwrap();
    assert_eq!(count(&lib), 1);
}

/// Fichier du dossier d'import lu pendant qu'on y écrit : la dernière ligne, coupée,
/// ne doit ni entrer tronquée ni être perdue quand le fichier est complété.
#[test]
fn inbox_partial_last_line_is_read_later() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("journal.csv");
    let mut lib = Library::open_in_memory().unwrap();
    std::fs::write(&f, "uts,artist,track\n1709287200,Daft Punk,Get Lucky\n1709290800,Daft Punk,Instant Cr").unwrap();
    lib.scan_input(dir.path()).unwrap();
    assert_eq!(count(&lib), 1);
    let title: i64 = lib.conn.query_row("SELECT COUNT(*) FROM track WHERE title LIKE 'Instant Cr%'", [], |r| r.get(0)).unwrap();
    assert_eq!(title, 0, "ligne coupée importée");
    std::fs::write(&f, "uts,artist,track\n1709287200,Daft Punk,Get Lucky\n1709290800,Daft Punk,Instant Crush\n1709294400,Daft Punk,Contact\n").unwrap();
    let file = std::fs::File::options().write(true).open(&f).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10)).unwrap();
    lib.scan_input(dir.path()).unwrap();
    assert_eq!(count(&lib), 3);
}

/// Doubles envois : retrait gardé (listé, rétablissable, plus retiré ensuite), et une
/// écoute confirmée par une autre source qui n'a pas la première reste.
#[test]
fn double_sends_are_traced_and_restorable() {
    let csv = |rows: &[i64]| {
        let mut s = String::from("artist,title,album,timestamp\n");
        for ts in rows {
            s.push_str(&format!("Daft Punk,Get Lucky,,{ts}\n"));
        }
        s
    };
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", csv(&[1_700_000_000, 1_700_000_010]).as_bytes()).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 180000", []).unwrap();
    assert_eq!(lib.drop_double_sends().unwrap(), 1);
    assert_eq!(lib.double_sends().unwrap().len(), 1);
    assert_eq!(lib.restore_double_sends().unwrap(), 1);
    assert_eq!(count(&lib), 2);
    // Rétabli : plus retiré, ni listé.
    assert_eq!(lib.drop_double_sends().unwrap(), 0);
    assert!(lib.double_sends().unwrap().is_empty());

    // L'autre appareil n'a vu que la seconde écoute : elle reste.
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", csv(&[1_700_000_000, 1_700_000_010]).as_bytes()).unwrap();
    lib.import_bytes("b.csv", csv(&[1_700_000_011]).as_bytes()).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 180000", []).unwrap();
    assert_eq!(lib.drop_double_sends().unwrap(), 0);
    assert_eq!(count(&lib), 2);
}

/// Nom mal écrit corrigé : la fiche rejoint une fiche créée au bon nom (annulable), et une
/// écoute future sous l'ancien nom suit.
#[test]
fn rename_merges_into_corrected_name() {
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", b"artist,title,album,timestamp\nNova,Le chant de Rivieres,,1709287200\n").unwrap();
    let bad: i64 = lib.conn.query_row("SELECT id FROM track", [], |r| r.get(0)).unwrap();
    let good = lib.rename(Kind::Track, bad, "Le Chant des Rivières").unwrap();
    assert_ne!(good, bad);
    let title = |l: &Library| l.conn.query_row("SELECT t.title FROM scrobble s JOIN track t ON t.id = s.track_id", [], |r| r.get::<_, String>(0)).unwrap();
    assert_eq!(title(&lib), "Le Chant des Rivières");
    lib.import_bytes("b.csv", b"artist,title,album,timestamp\nNova,Le chant de Rivieres,,1709300000\n").unwrap();
    let n: i64 = lib.conn.query_row("SELECT COUNT(*) FROM scrobble WHERE track_id = ?", [good], |r| r.get(0)).unwrap();
    assert_eq!(n, 2);
    // Simple changement de casse : réécrit sur place.
    assert_eq!(lib.rename(Kind::Track, good, "Le chant des rivières").unwrap(), good);
}
