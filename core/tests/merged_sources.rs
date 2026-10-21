//! Artiste fusionné qui revient, double envoi réuni, relecture, sauvegarde par-dessus.
use librestfm_core::Library;
use librestfm_core::stats::Kind;

fn count(lib: &Library) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap()
}

fn id(lib: &Library, table: &str, col: &str, v: &str) -> i64 {
    lib.conn.query_row(&format!("SELECT id FROM {table} WHERE {col} = ?"), [v], |r| r.get(0)).unwrap()
}

/// artiste fusionné, puis un titre jamais vu de cet artiste arrive sous l'ancien nom.
/// La même écoute, présente dans une autre source sous le bon nom, est comptée deux fois.
#[test]
fn new_track_of_merged_artist() {
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", b"artist,title,album,timestamp\nThe Beatles,Yesterday,,1709287200\nThe Beatles,Help!,,1709300000\nBeatles,Yesterday,,1709400000\n").unwrap();
    // « Beatles » rejoint « The Beatles » (règle article, ou fusion manuelle).
    lib.merge(Kind::Artist, id(&lib, "artist", "name", "Beatles"), id(&lib, "artist", "name", "The Beatles"), "manual").unwrap();
    assert_eq!(count(&lib), 3);
    // Un autre export (autre service) a la même écoute de Help! sous « Beatles ».
    let rep = lib.import_bytes("b.csv", b"artist,title,album,timestamp\nBeatles,Help!,,1709300010\n").unwrap();
    lib.auto_clean().unwrap();
    let incoherent: i64 = lib.conn.query_row("SELECT COUNT(*) FROM scrobble s JOIN track t ON t.id = s.track_id WHERE s.artist_id <> t.artist_id", [], |r| r.get(0)).unwrap();
    let help: i64 = lib.conn.query_row("SELECT COUNT(DISTINCT track_id) FROM scrobble s JOIN track t ON t.id = s.track_id WHERE t.title = 'Help!'", [], |r| r.get(0)).unwrap();
    println!("ajoutées {} fusionnées {} ; écoutes {} ; incohérentes {incoherent} ; fiches Help! {help}", rep.added, rep.merged, count(&lib));
    assert_eq!(count(&lib), 3, "écoute de Help! comptée deux fois");
}

/// un double envoi retiré qui portait une écoute réunie la fait revenir au recalcul suivant.
#[test]
fn double_send_drops_fold_keeper() {
    let mut lib = Library::open_in_memory().unwrap();
    let t0 = 1_700_000_000;
    // Source A : envoi double (t0, t0+60). Source B : t0 (même titre) et t0+60 sous une autre graphie.
    lib.import_bytes("a.csv", format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,,{t0}\nDaft Punk,Get Lucky,,{}\n", t0 + 60).as_bytes()).unwrap();
    lib.import_bytes("b.csv", format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,,{t0}\nDaft Punk,Get Lucky Alt,,{}\n", t0 + 60).as_bytes()).unwrap();
    lib.merge(Kind::Track, id(&lib, "track", "title", "Get Lucky Alt"), id(&lib, "track", "title", "Get Lucky"), "manual").unwrap();
    let folds: i64 = lib.conn.query_row("SELECT COUNT(*) FROM scrobble_fold", [], |r| r.get(0)).unwrap();
    println!("après fusion : {} écoutes, {folds} réunion(s)", count(&lib));
    lib.conn.execute("UPDATE track SET duration_ms = 240000", []).unwrap();
    let dropped = lib.drop_double_sends().unwrap();
    println!("doubles retirés : {dropped} ; écoutes {}", count(&lib));
    let before = count(&lib);
    // Un recalcul quelconque (fusion ou annulation d'une autre fiche).
    lib.rebuild_canonical().unwrap();
    println!("après recalcul : {}", count(&lib));
    assert_eq!(count(&lib), before, "l'écoute absorbée est revenue");
}

/// rétablir les doubles envois après H1 donne plus d'écoutes qu'au départ.
#[test]
fn restore_after_fold() {
    let mut lib = Library::open_in_memory().unwrap();
    let t0 = 1_700_000_000;
    lib.import_bytes("a.csv", format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,,{t0}\nDaft Punk,Get Lucky,,{}\n", t0 + 60).as_bytes()).unwrap();
    lib.import_bytes("b.csv", format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,,{t0}\nDaft Punk,Get Lucky Alt,,{}\n", t0 + 60).as_bytes()).unwrap();
    lib.merge(Kind::Track, id(&lib, "track", "title", "Get Lucky Alt"), id(&lib, "track", "title", "Get Lucky"), "manual").unwrap();
    let start = count(&lib);
    lib.conn.execute("UPDATE track SET duration_ms = 240000", []).unwrap();
    lib.auto_clean().unwrap();
    lib.rebuild_canonical().unwrap();
    lib.auto_clean().unwrap();
    let n = lib.restore_double_sends().unwrap();
    println!("départ {start}, rétablis {n}, à la fin {}", count(&lib));
    assert_eq!(count(&lib), start, "rétablir doit revenir à l'état de départ");
}

/// relecture d'une source (synchro, 14 jours) : les doubles envois retirés reviennent
/// comptés comme « ajoutés » à chaque passage.
#[test]
fn resync_readds_double_sends() {
    let mut lib = Library::open_in_memory().unwrap();
    let t0 = 1_700_000_000;
    let body = format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,,{t0}\nDaft Punk,Get Lucky,,{}\n", t0 + 10);
    lib.import_bytes("a.csv", body.as_bytes()).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 240000", []).unwrap();
    lib.auto_clean().unwrap();
    assert_eq!(count(&lib), 1);
    // Même source relue (comme une page de compte recouverte).
    let src: i64 = lib.conn.query_row("SELECT id FROM source", [], |r| r.get(0)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.csv");
    std::fs::write(&f, &body).unwrap();
    lib.conn.execute("INSERT INTO input_file (path, size, mtime, source_id, sha) VALUES (?, 1, 1, ?, NULL)", rusqlite::params![f.display().to_string(), src]).unwrap();
    let rep = lib.scan_input(dir.path()).unwrap();
    println!("relecture : ajoutées {:?}, nettoyées {}", rep.files.iter().map(|r| r.added).collect::<Vec<_>>(), rep.cleaned);
    assert_eq!(count(&lib), 1);
    assert_eq!(rep.files[0].added, 0, "double envoi déjà retiré compté comme nouvelle écoute");
}

/// sauvegarde par-dessus une sauvegarde existante : l'ancienne est effacée avant
/// que la nouvelle soit écrite.
#[test]
fn backup_overwrite_not_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lib.sqlite");
    let mut lib = Library::open(&db).unwrap();
    lib.import_bytes("a.csv", b"artist,title,album,timestamp\nDaft Punk,Get Lucky,,1709287200\n").unwrap();
    let dest = dir.path().join("save.sqlite");
    lib.backup_to(&dest).unwrap();
    assert!(Library::check_backup(&dest).is_ok());
    // Écriture impossible (simule disque plein / clé retirée) : une transaction ouverte fait échouer VACUUM.
    lib.conn.execute_batch("BEGIN; INSERT INTO config VALUES ('x','y');").unwrap();
    let r = lib.backup_to(&dest);
    println!("sauvegarde : {:?} ; ancienne présente : {}", r.as_ref().err().map(|e| e.to_string()), dest.exists());
    assert!(dest.exists(), "l'ancienne sauvegarde a été effacée alors que la nouvelle a échoué");
}
