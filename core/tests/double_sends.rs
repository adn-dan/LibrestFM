//! Doubles envois vus par plusieurs exports, relectures, annulation, source retirée.
use librestfm_core::Library;
use librestfm_core::stats::Kind;

fn count(lib: &Library) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get(0)).unwrap()
}
fn id(lib: &Library, table: &str, col: &str, v: &str) -> i64 {
    lib.conn.query_row(&format!("SELECT id FROM {table} WHERE {col} = ?"), [v], |r| r.get(0)).unwrap()
}
fn ds(lib: &Library) -> i64 {
    lib.conn.query_row("SELECT COUNT(*) FROM double_send", [], |r| r.get(0)).unwrap()
}
// 2023-11-14 22:13:00 UTC, aligné sur la minute
const T0: i64 = 1_699_999_980;
fn minute(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).unwrap().format("%d %b %Y %H:%M").to_string()
}

/// Export à la minute (A) + export précis (B) importés ensemble, double envoi retiré,
/// puis A relu (fichier du dossier d'import réécrit) : le double revient « ajouté ».
#[test]
fn reread_minute_source_after_recalage() {
    let mut lib = Library::open_in_memory().unwrap();
    let a = format!("Daft Punk,RAM,Get Lucky,{}\nDaft Punk,RAM,Get Lucky,{}\n", minute(T0 + 60), minute(T0));
    let b = format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,RAM,{}\nDaft Punk,Get Lucky,RAM,{}\n", T0 + 10, T0 + 80);
    let ra = lib.import_bytes("a.csv", a.as_bytes()).unwrap();
    lib.import_bytes("b.csv", b.as_bytes()).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 240000", []).unwrap();
    lib.auto_clean().unwrap();
    println!("après nettoyage : {} écoutes, {} double(s) retiré(s)", count(&lib), ds(&lib));
    assert_eq!(count(&lib), 1);
    // A est un fichier du dossier d'import, réexporté (lastfm-to-csv : nouvelle écoute en tête) : relu en entier.
    let _ = ra;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.csv");
    std::fs::write(&p, &a).unwrap();
    let r1 = lib.scan_input(dir.path()).unwrap();
    println!("dossier, 1re lecture : {:?}", r1.files.iter().map(|f| (f.status.clone(), f.added, f.merged)).collect::<Vec<_>>());
    let a2 = format!("Daft Punk,RAM,Instant Crush,{}\n{a}", minute(T0 + 3600));
    std::fs::write(&p, &a2).unwrap();
    let f = std::fs::File::options().write(true).open(&p).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(600)).unwrap();
    let r2 = lib.scan_input(dir.path()).unwrap();
    let rep = &r2.files[0];
    println!("relecture de A : ajoutées {} fusionnées {}", rep.added, rep.merged);
    let rep_added = rep.added;
    lib.auto_clean().unwrap();
    println!("après nettoyage : {} écoutes, {} double(s) retiré(s)", count(&lib), ds(&lib));
    let n = lib.restore_double_sends().unwrap();
    println!("rétablis {n} : {} écoutes (attendu 3)", count(&lib));
    assert_eq!(rep_added, 1, "relecture : double envoi compté comme ajouté");
    // Rétablir rend les deux envois de Get Lucky, plus Instant Crush ajouté par la relecture
    assert_eq!((n, count(&lib)), (1, 3), "rétablir donne une écoute de trop");
}

/// Export précis d'abord (double envoi retiré), puis un export à la minute des mêmes écoutes.
#[test]
fn minute_export_after_dropped_double() {
    let mut lib = Library::open_in_memory().unwrap();
    let b = format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,RAM,{}\nDaft Punk,Get Lucky,RAM,{}\n", T0 + 10, T0 + 80);
    lib.import_bytes("b.csv", b.as_bytes()).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 240000", []).unwrap();
    lib.auto_clean().unwrap();
    assert_eq!(count(&lib), 1);
    // lastfm-to-csv : plus récent d'abord.
    let a = format!("Daft Punk,RAM,Get Lucky,{}\nDaft Punk,RAM,Get Lucky,{}\n", minute(T0 + 60), minute(T0));
    let rep = lib.import_bytes("a.csv", a.as_bytes()).unwrap();
    lib.auto_clean().unwrap();
    println!("import minute : ajoutées {} fusionnées {} ; écoutes {}", rep.added, rep.merged, count(&lib));
    assert_eq!(count(&lib), 1, "même double envoi, présent dans deux exports, compté deux fois");
}

/// Double envoi né d'une fusion (deux graphies dans une même source), fusion annulée :
/// l'écoute retirée ne revient pas.
#[test]
fn undo_merge_after_double_send() {
    let mut lib = Library::open_in_memory().unwrap();
    let a = format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,,{}\nDaft Punk,Get Lucky - Single,,{}\n", T0, T0 + 30);
    lib.import_bytes("a.csv", a.as_bytes()).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 240000", []).unwrap();
    let op = lib.merge(Kind::Track, id(&lib, "track", "title", "Get Lucky - Single"), id(&lib, "track", "title", "Get Lucky"), "manual").unwrap();
    lib.auto_clean().unwrap();
    println!("après fusion + nettoyage : {} écoutes", count(&lib));
    lib.undo_merge(op).unwrap();
    lib.auto_clean().unwrap();
    println!("après annulation : {} écoutes (attendu 2), doubles listés {}", count(&lib), lib.double_sends().unwrap().len());
    assert_eq!(count(&lib), 2, "annuler la fusion ne rend pas l'écoute retirée comme double");
}

/// Source retirée : ses doubles envois restent listés au nettoyage.
#[test]
fn removed_source_double_sends_listed() {
    let mut lib = Library::open_in_memory().unwrap();
    let b = format!("artist,title,album,timestamp\nDaft Punk,Get Lucky,,{}\nDaft Punk,Get Lucky,,{}\n", T0, T0 + 30);
    let r = lib.import_bytes("b.csv", b.as_bytes()).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 240000", []).unwrap();
    lib.auto_clean().unwrap();
    lib.remove_source(r.source_id.unwrap()).unwrap();
    println!("écoutes {}, doubles listés {}", count(&lib), lib.double_sends().unwrap().len());
    assert_eq!(lib.double_sends().unwrap().len(), 0, "double envoi d'une source retirée encore listé");
}
