//! Corrections : éditions d'albums, noms de fichiers, artiste dans le titre,
//! variantes d'artiste, versions, annulation et imports suivants.

use librestfm_core::Library;
use librestfm_core::stats::{Kind, Period};

const CSV: &str = "\
Daft Punk,Random Access Memories,Get Lucky,01 Mar 2024 10:00
Daft Punk,Random Access Memories (10th Anniversary Edition),Instant Crush,01 Mar 2024 10:06
Daft Punk,Random Access Memories (10th Anniversary Edition),Get Lucky,02 Mar 2024 10:00
Unknown Artist,,03 - Daft Punk - Get Lucky.flac,03 Mar 2024 10:00
Daft Punk,,Daft Punk - Instant Crush,04 Mar 2024 10:00
Daft Punk,Alive 2007,Around the World,05 Mar 2024 10:00
Daft Punk,Discovery,One More Time,06 Mar 2024 10:00
Daft Punk,Discovery,One More Time - Live,06 Mar 2024 11:00
The Beatles,Abbey Road,Come Together,07 Mar 2024 10:00
Beatles,Abbey Road,Something,07 Mar 2024 10:05
";

fn plays(lib: &Library, kind: Kind, name: &str) -> i64 {
    let ov = lib.overview(Period::default(), 100).unwrap();
    let list = match kind {
        Kind::Artist => ov.top_artists,
        Kind::Album => ov.top_albums,
        Kind::Track => ov.top_tracks,
    };
    list.iter().filter(|r| r.name == name).map(|r| r.plays).sum()
}

#[test]
fn cleanup_merges_and_undo() {
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("messy.csv", CSV.as_bytes()).unwrap();

    // Avant : deux éditions, un nom de fichier, un artiste inconnu.
    assert_eq!(plays(&lib, Kind::Track, "Get Lucky"), 2);
    let sugg = lib.suggestions().unwrap();
    let sure: Vec<_> = sugg.iter().filter(|s| s.sure).map(|s| (s.rule.as_str(), s.from_name.as_str())).collect();
    assert!(sure.contains(&("edition", "Random Access Memories (10th Anniversary Edition)")), "{sure:?}");
    assert!(sure.contains(&("filename", "03 - Daft Punk - Get Lucky.flac")) || sure.contains(&("artist_in_title", "03 - Daft Punk - Get Lucky.flac")), "{sure:?}");
    assert!(sure.contains(&("artist_in_title", "Daft Punk - Instant Crush")), "{sure:?}");
    // « The Beatles » / « Beatles » : proposé, pas automatique.
    assert!(sugg.iter().any(|s| s.rule == "article" && !s.sure));
    // Une version live n'est jamais proposée à la fusion.
    assert!(!sugg.iter().any(|s| s.from_name.contains("Live")));

    let applied = lib.auto_clean().unwrap();
    assert_eq!(applied, 3);
    assert_eq!(plays(&lib, Kind::Track, "Get Lucky"), 3);
    assert_eq!(plays(&lib, Kind::Track, "Instant Crush"), 2);
    assert_eq!(plays(&lib, Kind::Album, "Random Access Memories"), 3, "éditions regroupées sous le titre de base");
    assert_eq!(plays(&lib, Kind::Artist, "Unknown Artist"), 0);
    assert_eq!(plays(&lib, Kind::Artist, "Daft Punk"), 8);

    // Versions d'un titre : séparées, mais reliées.
    let omt = lib.search("one more time", 5).unwrap().tracks.into_iter().find(|t| t.name == "One More Time").unwrap();
    let versions = lib.versions(omt.id).unwrap();
    assert_eq!(versions.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["One More Time - Live"]);

    // Un nouvel import atterrit directement dans l'album regroupé.
    lib.import_bytes("later.csv", b"Daft Punk,Random Access Memories (10th Anniversary Edition),Touch,08 Mar 2024 10:00\n").unwrap();
    assert_eq!(plays(&lib, Kind::Album, "Random Access Memories"), 4);

    // Fusion manuelle proposée : The Beatles.
    let beatles = lib.suggestions().unwrap().into_iter().find(|s| s.rule == "article").unwrap();
    let op = lib.apply_suggestion(&beatles).unwrap();
    assert_eq!(lib.overview(Period::default(), 100).unwrap().top_artists.iter().filter(|a| a.name.contains("Beatles")).count(), 1);
    // Annulation : retour à deux artistes.
    lib.undo_merge(op).unwrap();
    assert_eq!(lib.overview(Period::default(), 100).unwrap().top_artists.iter().filter(|a| a.name.contains("Beatles")).count(), 2);

    // Annuler la fusion des éditions rend son nom d'origine à l'album.
    let ram_op = lib.merge_ops().unwrap().into_iter().find(|o| o.rule == "edition").unwrap();
    lib.undo_merge(ram_op.id).unwrap();
    assert_eq!(plays(&lib, Kind::Album, "Random Access Memories"), 1);
    assert_eq!(plays(&lib, Kind::Album, "Random Access Memories (10th Anniversary Edition)"), 3);

    // Une fusion annulée ne revient pas toute seule…
    assert_eq!(lib.auto_clean().unwrap(), 0);
    assert_eq!(plays(&lib, Kind::Album, "Random Access Memories"), 1);
    // …sauf si on réaffiche les suggestions écartées.
    lib.clear_dismissed().unwrap();
    lib.auto_clean().unwrap();
    assert_eq!(plays(&lib, Kind::Album, "Random Access Memories"), 4);
}

#[test]
fn damaged_names_are_cleaned() {
    use librestfm_core::normalize::{main_artist, unescape_html, without_nickname};
    assert_eq!(unescape_html("Simon &amp; Garfunkel &#39;69&#x21;"), "Simon & Garfunkel '69!");
    assert_eq!(unescape_html("AT&T &nope;"), "AT&T &nope;");
    assert_eq!(main_artist("Band One; Singer Two;Anne-Sophie").as_deref(), Some("Band One"));
    assert_eq!(main_artist("Earth, Wind & Fire"), None);
    assert_eq!(without_nickname("Paul 'PJ' O'Neill").as_deref(), Some("Paul O'Neill"));
    assert_eq!(without_nickname("Rag'n'Bone Man"), None);

    // Une écoute dans deux sources : CSV propre et Libre.fm encodé ou multi-artistes.
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes(
        "csv.csv",
        b"Simon & Garfunkel,Bookends,Mrs. Robinson,10 Mar 2024 20:00\nBand One,What Is It?,Lullaby,10 Mar 2024 21:00\nPaul O'Neill,Far Away,Over the Hills,10 Mar 2024 22:00\n",
    )
    .unwrap();
    // Noms abîmés insérés tels quels (comme avant le décodage à l'import).
    let raw = |artist: &str, title: &str, ts: i64| {
        // Clé d'avant le décodage des entités dans les clés (« &amp; » distinct de « & »).
        let key = format!("legacy:{}", artist);
        lib.conn.execute("INSERT OR IGNORE INTO artist (name, key) VALUES (?1, ?2)", [artist, &key]).unwrap();
        let a: i64 = lib.conn.query_row("SELECT id FROM artist WHERE name = ?", [artist], |r| r.get(0)).unwrap();
        lib.conn
            .execute("INSERT OR IGNORE INTO track (artist_id, title, key) VALUES (?1, ?2, ?3)", rusqlite::params![a, title, librestfm_core::normalize::title_key(title)])
            .unwrap();
        let t: i64 = lib.conn.query_row("SELECT id FROM track WHERE artist_id = ? AND title = ?", rusqlite::params![a, title], |r| r.get(0)).unwrap();
        lib.conn
            .execute("INSERT INTO scrobble (ts, track_id, artist_id, precise, ym, dow, hour, raw_track_id) VALUES (?1, ?2, ?3, 1, 0, 0, 0, ?2)", [ts, t, a])
            .unwrap();
        lib.conn
            .execute(
                "INSERT OR IGNORE INTO source (id, file_name, sha256, format, imported_at, rows_read, rows_added, rows_merged, rows_rejected)
                 VALUES (99, 'lfm', 'lfm', 'librefm_account', 0, 0, 0, 0, 0)",
                [],
            )
            .unwrap();
        lib.conn.execute("INSERT INTO scrobble_source (scrobble_id, source_id, ts) VALUES ((SELECT MAX(id) FROM scrobble), 99, ?)", [ts]).unwrap();
    };
    let t0 = 1_710_100_800; // 10 mars 2024 20:00 UTC
    raw("Simon &amp; Garfunkel", "Mrs. Robinson", t0 + 30);
    raw("Band One; Singer Two; Anne-Sophie", "Lullaby", t0 + 3600 + 30);
    raw("Paul 'PJ' O'Neill", "Over the Hills", t0 + 7200 + 30);
    assert_eq!(lib.conn.query_row("SELECT COUNT(*) FROM scrobble", [], |r| r.get::<_, i64>(0)).unwrap(), 6);
    lib.auto_clean().unwrap();
    let ov = lib.overview(Period::default(), 10).unwrap();
    let mut names: Vec<&str> = ov.top_artists.iter().map(|a| a.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["Band One", "Paul O'Neill", "Simon & Garfunkel"]);
    assert_eq!(ov.total, 3, "les écoutes en double sont réunies");
}
