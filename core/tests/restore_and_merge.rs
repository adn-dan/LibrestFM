//! Restauration d'une base étrangère ou avec journal, dates aberrantes, fusion d'un bloc.
use librestfm_core::Library;
use librestfm_core::stats::{Kind, Period};
use std::time::Instant;

fn csv(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("Artist {},Album {},Title {},{:02} Jan 2024 {:02}:{:02}\n", i % 7, i % 3, i % 11, 1 + i % 28, i % 24, i % 60));
    }
    s
}

/// Une base SQLite étrangère qui a une table `scrobble` passe la vérification, remplace la
/// base, puis ne s'ouvre pas : la bibliothèque reste sur une base en mémoire vide, et la
/// base sur disque ne s'ouvrira plus au lancement suivant.
#[test]
fn restore_foreign_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("librestfm.sqlite");
    let mut lib = Library::open(&db).unwrap();
    lib.import_bytes("a.csv", csv(50).as_bytes()).unwrap();
    let foreign = dir.path().join("autre.sqlite");
    {
        let c = rusqlite::Connection::open(&foreign).unwrap();
        c.execute_batch("CREATE TABLE scrobble (ts INTEGER); INSERT INTO scrobble VALUES (1700000000);").unwrap();
    }
    let info = Library::check_backup(&foreign);
    println!("check_backup(étrangère) = {:?}", info.as_ref().map(|i| i.plays).map_err(|e| e.to_string()));
    let r = lib.restore_from(&foreign, &db, &dir.path().join("safety.sqlite"));
    println!("restore_from = {:?}", r.as_ref().map(|i| i.plays).map_err(|e| e.to_string()));
    let after = lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get::<_, i64>(0));
    println!("après : COUNT(*) sur la connexion gardée = {:?} ; chemin = {:?}", after.as_ref().map_err(|e| e.to_string()), lib.conn.path());
    let reopen = Library::open(&db);
    println!("relance : Library::open = {:?}", reopen.as_ref().err().map(|e| e.to_string()));
    assert!(info.is_err() || (r.is_ok() && reopen.is_ok()), "base étrangère acceptée puis base inutilisable");
}

/// Sauvegarde choisie avec son journal WAL non intégré : check_backup compte les écoutes
/// du journal, mais seule la base principale est copiée.
#[test]
fn restore_ignores_wal_of_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.sqlite");
    let src_lib = Library::open(&src).unwrap();
    src_lib.conn.execute_batch("PRAGMA wal_autocheckpoint = 0;").unwrap();
    let mut src_lib = src_lib;
    src_lib.import_bytes("a.csv", csv(200).as_bytes()).unwrap();
    // Application arrêtée brutalement : le journal n'est pas intégré.
    std::mem::forget(src_lib);
    let announced = Library::check_backup(&src).unwrap().plays;
    let db = dir.path().join("librestfm.sqlite");
    let mut lib = Library::open(&db).unwrap();
    let r = lib.restore_from(&src, &db, &dir.path().join("safety.sqlite")).map(|i| i.plays);
    let got: i64 = lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap();
    println!("annoncé {announced}, restore_from {:?}, présent après {got}", r.map_err(|e| e.to_string()));
    assert_eq!(announced, got, "écoutes annoncées perdues à la restauration");
}

/// Dates aberrantes acceptées (1970 des exports Last.fm sans date, an 5138) : la vue
/// d'ensemble remplit chaque jour entre les deux.
#[test]
fn absurd_dates_blow_up_overview() {
    let mut lib = Library::open_in_memory().unwrap();
    let mut s = csv(100);
    s.push_str("Zed,Alb,Old,01 Jan 1970 00:00\n");
    let r = lib.import_bytes("a.csv", s.as_bytes()).unwrap();
    println!("import 1970 : ajoutées {}", r.added);
    let t = Instant::now();
    let o = lib.overview_by(Period::default(), 15, false).unwrap();
    println!("1970 : per_day {} entrées, per_month {}, {:.1?}, json {} o", o.per_day.len(), o.per_month.len(), t.elapsed(), serde_json::to_string(&o).unwrap().len());
    lib.import_bytes("b.csv", b"artist,title,timestamp\nFuture,Far,99999999999\n").unwrap();
    let t = Instant::now();
    let o = lib.overview_by(Period::default(), 15, false).unwrap();
    println!("an 5138 : per_day {} entrées, per_month {}, {:.1?}, json {} o", o.per_day.len(), o.per_month.len(), t.elapsed(), serde_json::to_string(&o).unwrap().len());
    assert!(o.per_day.len() < 100_000, "vue d'ensemble démesurée pour une seule date aberrante");
}

/// Fusion : merge_op et alias écrits hors transaction, puis recalcul dans la sienne.
/// Si le recalcul échoue, la fusion reste enregistrée alors que l'appel renvoie une erreur.
#[test]
fn merge_not_atomic() {
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", b"Daft Punk,Discovery,One More Time,10 Mar 2024 20:00\nDaft-Punk,Discovery,Aerodynamic,10 Mar 2024 21:00\nOther,X,Y,10 Mar 2024 22:00\n").unwrap();
    let ids: Vec<i64> = lib.conn.prepare("SELECT id FROM artist ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    let ops0: i64 = lib.conn.query_row("SELECT COUNT(*) FROM merge_op", [], |r| r.get(0)).unwrap();
    // Échec simulé du recalcul (disque plein, verrou…).
    lib.conn.execute_batch("CREATE TEMP TRIGGER boom BEFORE UPDATE ON scrobble BEGIN SELECT RAISE(ABORT, 'disque plein'); END;").unwrap();
    let r = lib.merge(Kind::Artist, ids[ids.len() - 1], ids[0], "manual");
    let ops: i64 = lib.conn.query_row("SELECT COUNT(*) FROM merge_op", [], |r| r.get(0)).unwrap();
    let aliases: i64 = lib.conn.query_row("SELECT COUNT(*) FROM alias", [], |r| r.get(0)).unwrap();
    let moved: i64 = lib.conn.query_row("SELECT COUNT(*) FROM scrobble WHERE artist_id = ?", [ids[0]], |r| r.get(0)).unwrap();
    println!("merge = {:?} ; merge_op {ops0} -> {ops}, alias {aliases}, écoutes sous la cible {moved}", r.as_ref().map_err(|e| e.to_string()));
    assert!(r.is_ok() || ops == ops0, "fusion en erreur mais enregistrée (à moitié appliquée)");
}

