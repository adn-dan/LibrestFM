//! Scénario d'import désordonné : Last.fm (minute près), Libre.fm (seconde près,
//! même période), Spotify (date de fin), ListenBrainz, CSV Windows à en-tête,
//! réimport, fichier invalide, puis retrait d'une source.

use librestfm_core::Library;
use librestfm_core::import::{ImportStatus, Reject};
use librestfm_core::stats::{Kind, Period};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn count(lib: &Library, sql: &str) -> i64 {
    lib.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn reasons(rep: &librestfm_core::import::ImportReport) -> Vec<(Reject, usize)> {
    rep.rejects.iter().map(|r| (r.reason, r.count)).collect()
}

fn track_id(lib: &Library, artist: &str, title: &str) -> i64 {
    lib.conn
        .query_row(
            "SELECT t.id FROM track t JOIN artist a ON a.id = t.artist_id WHERE a.name = ? AND t.title = ?",
            [artist, title],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn messy_imports_merge_cleanly() {
    let mut lib = Library::open_in_memory().unwrap();

    let lastfm = lib.import_file(&fixture("lastfm.csv")).unwrap();
    assert!(matches!(lastfm.status, ImportStatus::Imported));
    assert_eq!(lastfm.format.as_deref(), Some("lastfm_csv"));
    assert_eq!((lastfm.rows_read, lastfm.added, lastfm.merged), (8, 6, 0));
    assert_eq!(reasons(&lastfm), vec![(Reject::MissingTitle, 1), (Reject::DateOutOfRange, 1)]);

    // Même période scrobblée sur Libre.fm, avec quelques secondes d'écart et d'autres graphies.
    let libre = lib.import_file(&fixture("librefm.tsv")).unwrap();
    assert_eq!(libre.format.as_deref(), Some("last_export_tsv"));
    // C, D, E fusionnés ; F ajouté puis sa ligne en double fusionnée ; Ramones réécouté 80 s après = 2 écoutes.
    assert_eq!((libre.rows_read, libre.added, libre.merged), (7, 3, 4));
    assert_eq!(libre.overlaps.len(), 1);
    assert_eq!(libre.overlaps[0].shared, 3);
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM artist WHERE key = 'bjork'"), 1);
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM artist WHERE name = 'Björk'"), 1, "graphie accentuée retenue");
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM track WHERE title = 'Jóga'"), 1);

    // Spotify date la fin : Get Lucky fini à 20:04:13 après 248 s = commencé à 20:00:05.
    let spotify = lib.import_file(&fixture("spotify_extended.json")).unwrap();
    assert_eq!((spotify.rows_read, spotify.added, spotify.merged), (4, 1, 1));
    assert_eq!(reasons(&spotify), vec![(Reject::TooShort, 1), (Reject::NotMusic, 1)]);
    // L'écoute Last.fm (minute près) prend l'heure précise de Spotify.
    let get_lucky = track_id(&lib, "Daft Punk", "Get Lucky");
    assert_eq!(count(&lib, &format!("SELECT COUNT(*) FROM scrobble WHERE track_id = {get_lucky} AND ts = 1710100805 AND precise = 1")), 1);

    let lb = lib.import_file(&fixture("listenbrainz.jsonl")).unwrap();
    assert_eq!((lb.rows_read, lb.added), (2, 1));
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM track WHERE mbid = '5c3f0d2e-1111-4000-8000-00000000000b'"), 1);

    let header = lib.import_file(&fixture("header_cp1252.csv")).unwrap();
    assert_eq!(header.format.as_deref(), Some("header_csv"));
    assert_eq!((header.added, header.rejected), (1, 1));
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM track WHERE title = 'L''Aigle noir'"), 1, "accents Windows-1252 décodés");

    // Réimport du même fichier : rien ne bouge.
    let again = lib.import_file(&fixture("lastfm.csv")).unwrap();
    assert!(matches!(again.status, ImportStatus::AlreadyImported));
    let garbage = lib.import_file(&fixture("garbage.txt")).unwrap();
    assert!(matches!(garbage.status, ImportStatus::Unrecognized));

    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 12);

    // Statistiques
    let ov = lib.overview(Period::default(), 10).unwrap();
    assert_eq!(ov.total, 12);
    assert_eq!(ov.top_artists[0].name, "Daft Punk");
    assert_eq!(ov.top_artists[0].plays, 4);
    assert_eq!(ov.per_month.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["2024-03", "2024-04", "2024-05", "2024-06"]);

    let before = lib.neighbours(Kind::Track, get_lucky, true, 10).unwrap();
    let after = lib.neighbours(Kind::Track, get_lucky, false, 10).unwrap();
    assert_eq!(before.iter().map(|r| (r.name.as_str(), r.plays)).collect::<Vec<_>>(), [("Reptilia", 1)]);
    assert_eq!(after.iter().map(|r| (r.name.as_str(), r.plays)).collect::<Vec<_>>(), [("Instant Crush", 1)]);

    let dp = ov.top_artists[0].id;
    let after_dp = lib.neighbours(Kind::Artist, dp, false, 10).unwrap();
    assert_eq!(after_dp.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["The Strokes"]);

    let detail = lib.entity(Kind::Track, get_lucky).unwrap();
    assert_eq!(detail.total, 3);
    assert!(detail.profiles.is_empty(), "pas encore de caractéristiques");

    let found = lib.search("bjork", 5).unwrap();
    assert_eq!(found.artists.len(), 1);

    // Retirer Libre.fm : seules ses écoutes exclusives disparaissent (F et les deux Ramones).
    let sources = lib.sources().unwrap();
    let libre_src = sources.iter().find(|s| s.file_name == "librefm.tsv").unwrap();
    assert_eq!(libre_src.exclusive, 3);
    assert_eq!(lib.remove_source(libre_src.id).unwrap(), 3);
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 9);
    // Et on peut le réimporter ensuite.
    let back = lib.import_file(&fixture("librefm.tsv")).unwrap();
    assert!(matches!(back.status, ImportStatus::Imported));
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 12);
}

#[test]
fn import_order_does_not_matter() {
    let files = ["lastfm.csv", "librefm.tsv", "spotify_extended.json", "listenbrainz.jsonl", "header_cp1252.csv"];
    let total = |order: &[&str]| {
        let mut lib = Library::open_in_memory().unwrap();
        for f in order {
            lib.import_file(&fixture(f)).unwrap();
        }
        count(&lib, "SELECT COUNT(*) FROM scrobble")
    };
    let mut reversed = files;
    reversed.reverse();
    assert_eq!(total(&files), 12);
    assert_eq!(total(&reversed), 12);
}

#[test]
fn detail_filters() {
    use librestfm_core::stats::Filter;
    let mut lib = Library::open_in_memory().unwrap();
    for f in ["lastfm.csv", "librefm.tsv", "spotify_extended.json", "listenbrainz.jsonl", "header_cp1252.csv"] {
        lib.import_file(&fixture(f)).unwrap();
    }
    let all = lib.detail(&Filter::default(), 10).unwrap();
    assert_eq!(all.total, 12);

    // Un artiste : ses écoutes et son titre le plus écouté.
    let dp = lib.overview(Period::default(), 1).unwrap().top_artists[0].id;
    let d = lib.detail(&Filter { kind: Some(Kind::Artist), id: Some(dp), ..Default::default() }, 10).unwrap();
    assert_eq!((d.total, d.top_tracks[0].name.as_str()), (4, "Get Lucky"));

    // Une période : avril 2024 (Air seulement).
    let (from, to) = (1711929600, 1714521599); // 2024-04-01 → 2024-04-30, UTC
    let d = lib.detail(&Filter { from: Some(from), to: Some(to), ..Default::default() }, 10).unwrap();
    assert_eq!((d.total, d.top_artists[0].name.as_str()), (1, "Air"));

    // Valeur piégée : ignorée, pas interprétée comme du SQL.
    let d = lib.detail(&Filter { country: Some("FR' OR 1=1 --".into()), ..Default::default() }, 10).unwrap();
    assert_eq!(d.total, 12);
    // Pays inconnu (aucune info d'artiste) : rien.
    assert_eq!(lib.detail(&Filter { country: Some("FR".into()), ..Default::default() }, 10).unwrap().total, 0);
}

#[test]
fn mp4_titles_are_recording_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mp4.csv");
    std::fs::write(
        &path,
        "Daft Punk,Discovery,One More Time,10 Mar 2024 20:00\nDaft Punk,Discovery,One More Time.mp4,10 Mar 2024 20:06\nDaft Punk,Discovery,Aerodynamic.MP4,10 Mar 2024 20:12\n",
    )
    .unwrap();
    let mut lib = Library::open_in_memory().unwrap();
    let rep = lib.import_file(&path).unwrap();
    assert_eq!(rep.added, 1);
    assert_eq!(reasons(&rep), vec![(Reject::RecordingError, 2)]);

    // Écoutes en .mp4 importées avant la règle : retirées au nettoyage automatique.
    let artist = count(&lib, "SELECT artist_id FROM scrobble");
    lib.conn.execute("INSERT INTO track (artist_id, title, key) VALUES (?, 'Veridis Quo.mp4', 'veridis quo mp4')", [artist]).unwrap();
    let t = lib.conn.last_insert_rowid();
    lib.conn
        .execute("INSERT INTO scrobble (ts, track_id, artist_id, precise, ym, dow, hour, raw_track_id) VALUES (1710100000, ?1, ?2, 1, 0, 0, 0, ?1)", [t, artist])
        .unwrap();
    lib.auto_clean().unwrap();
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 1);
}

#[test]
fn export_reimports_identically() {
    use librestfm_core::export::HistoryFormat;
    let mut lib = Library::open_in_memory().unwrap();
    for f in ["lastfm.csv", "librefm.tsv", "listenbrainz.jsonl", "spotify_extended.json"] {
        lib.import_file(&fixture(f)).unwrap();
    }
    let total = count(&lib, "SELECT COUNT(*) FROM scrobble");
    let lastfm = count(&lib, "SELECT id FROM source WHERE file_name = 'lastfm.csv'");
    let only_lastfm = count(&lib, &format!("SELECT COUNT(*) FROM scrobble_source WHERE source_id = {lastfm}"));
    for format in [HistoryFormat::Csv, HistoryFormat::Listenbrainz] {
        let mut buf = Vec::new();
        assert_eq!(lib.export_history(&[], true, format, &mut buf).unwrap() as i64, total);
        // Réimport dans une bibliothèque vide : mêmes écoutes, même précision.
        let mut copy = Library::open_in_memory().unwrap();
        let rep = copy.import_bytes("export", &buf).unwrap();
        assert_eq!((rep.added as i64, rep.rejected), (total, 0), "{format:?}");
        let sum = "SELECT SUM(ts), COUNT(DISTINCT track_id) FROM scrobble";
        let q = |l: &Library| l.conn.query_row(sum, [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap();
        assert_eq!(q(&copy), q(&lib));
        if format == HistoryFormat::Csv {
            assert_eq!(count(&copy, "SELECT SUM(precise) FROM scrobble"), count(&lib, "SELECT SUM(precise) FROM scrobble"));
        }
    }
    // Une seule source : seulement ses écoutes, chacune une fois.
    let mut buf = Vec::new();
    assert_eq!(lib.export_history(&[lastfm], false, HistoryFormat::Csv, &mut buf).unwrap() as i64, only_lastfm);
}

#[test]
fn merging_tracks_merges_their_shared_plays() {
    // La même écoute dans deux sources, orthographiée différemment : deux titres
    // distincts à l'import, donc deux écoutes ; une seule après la fusion des titres.
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("a.csv", b"Daft Punk,Random Access Memories,Get Lucky,10 Mar 2024 20:00\nDaft Punk,Random Access Memories,Get Lucky,11 Mar 2024 20:00\n").unwrap();
    lib.import_bytes("b.csv", b"Daft Punk,Random Access Memories,Get Lukcy,10 Mar 2024 20:01\n").unwrap();
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 3);
    let from = track_id(&lib, "Daft Punk", "Get Lukcy");
    let to = track_id(&lib, "Daft Punk", "Get Lucky");
    let op = lib.merge(Kind::Track, from, to, "manual").unwrap();
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 2);
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble_source"), 3);
    // Annuler la fusion rend l'écoute à sa source.
    lib.undo_merge(op).unwrap();
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 3);

    // Fusion refaite, puis retrait de la source qui portait l'écoute gardée :
    // celle de l'autre source survit, et revient seule si la fusion est annulée.
    let op = lib.merge(Kind::Track, from, to, "manual").unwrap();
    let a = count(&lib, "SELECT id FROM source WHERE file_name = 'a.csv'");
    lib.remove_source(a).unwrap();
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 1);
    lib.undo_merge(op).unwrap();
    assert_eq!(count(&lib, &format!("SELECT COUNT(*) FROM scrobble WHERE raw_track_id = {from}")), 1);
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 1);
}

#[test]
fn listening_time_follows_the_scrobble_rules() {
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes(
        "t.csv",
        b"utc_time,artist,album,title\n2024-03-10 20:00:00,A,X,Long\n2024-03-10 20:01:40,A,X,Long\n2024-03-10 20:10:00,A,X,Court\n2024-03-10 20:12:00,A,X,Inconnu\n",
    )
    .unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 300000 WHERE title = 'Long'", []).unwrap();
    lib.conn.execute("UPDATE track SET duration_ms = 600000 WHERE title = 'Court'", []).unwrap();
    let ov = lib.overview_by(Period::default(), 10, true).unwrap();
    // Long : coupé à 100 s mais au moins la moitié (150 s), puis en entier (300 s).
    // Court (10 min) : l'écoute suivante arrive après 120 s, en dessous du seuil de 4 min.
    // Inconnu : durée supposée de 210 s.
    assert_eq!(ov.listened_s, 150 + 300 + 240 + 210);
    assert_eq!(ov.listened_min_s, 150 + 150 + 240 + 105);
    assert_eq!(ov.top_tracks[0].name, "Long");
    assert_eq!(ov.top_tracks[0].seconds, Some(450));
    let long = track_id(&lib, "A", "Long");
    assert_eq!(lib.entity(Kind::Track, long).unwrap().listened_s, 450);
}

#[test]
fn discoveries_and_drops() {
    // A dès le début (pas une découverte), B découvert en juin et adopté, C lâché.
    let mut rows = String::from("utc_time,artist,album,title\n");
    let day = |d: i64| chrono::DateTime::from_timestamp(1_704_067_200 + d * 86_400, 0).unwrap().format("%Y-%m-%d %H:%M:%S").to_string();
    for d in 0..400 {
        rows += &format!("{},A,X,a{}\n", day(d), d % 7);
    }
    for d in 150..170 {
        rows += &format!("{},B,Y,b\n", day(d).replace("00:00:00", "12:00:00"));
    }
    for d in (180..400).step_by(30) {
        rows += &format!("{},B,Y,b\n", day(d).replace("00:00:00", "12:00:00"));
    }
    for d in 10..30 {
        rows += &format!("{},C,Z,c\n", day(d).replace("00:00:00", "18:00:00"));
    }
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("d.csv", rows.as_bytes()).unwrap();
    let d = lib.discoveries(Period::default()).unwrap();
    assert_eq!(d.found.iter().map(|t| t.artist.name.as_str()).collect::<Vec<_>>(), ["B"]);
    assert_eq!(d.found[0].burst, 22);
    assert_eq!(d.dropped.iter().map(|t| t.artist.name.as_str()).collect::<Vec<_>>(), ["C"]);
    assert_eq!(d.per_month.iter().map(|m| m.1).sum::<i64>(), 1);
}

#[test]
fn comparing_two_periods() {
    let mut lib = Library::open_in_memory().unwrap();
    // Janvier : A ×9, B ×2. Février : A ×1, B ×3, C ×4 (nouveau).
    let mut rows = String::from("utc_time,artist,album,title\n");
    let mut add = |day: u32, artist: &str, n: u32| {
        for i in 0..n {
            rows += &format!("2024-{}-{:02} 1{}:00:00,{artist},X,t{i}\n", if day > 31 { "02" } else { "01" }, if day > 31 { day - 31 } else { day }, i);
        }
    };
    add(5, "A", 9);
    add(6, "B", 2);
    add(36, "A", 1);
    add(37, "B", 3);
    add(38, "C", 4);
    lib.import_bytes("c.csv", rows.as_bytes()).unwrap();
    let ts = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap().and_utc().timestamp();
    let jan = Period { from: Some(ts("2024-01-01 00:00:00")), to: Some(ts("2024-01-31 23:59:59")) };
    let feb = Period { from: Some(ts("2024-02-01 00:00:00")), to: Some(ts("2024-02-29 23:59:59")) };
    let c = lib.compare(jan, feb, 10).unwrap();
    assert_eq!((c.a.plays, c.b.plays), (11, 8));
    assert_eq!((c.a.new_artists, c.b.new_artists), (2, 1));
    let top_b: Vec<(&str, Option<usize>)> = c.b.top_artists.iter().map(|(r, o)| (r.name.as_str(), *o)).collect();
    assert_eq!(top_b, [("C", None), ("B", Some(2)), ("A", Some(1))]);
    // Seul A a au moins 10 écoutes sur les deux périodes : 9/11 → 1/8.
    assert_eq!(c.lost.iter().map(|s| s.artist.name.as_str()).collect::<Vec<_>>(), ["A"]);
    assert!((c.lost[0].share_a - 900.0 / 11.0).abs() < 1e-9 && (c.lost[0].share_b - 12.5).abs() < 1e-9);
}

#[test]
fn a_person_who_is_an_artist_has_one_page() {
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("p.csv", b"Nile Rodgers,X,Le Freak,10 Mar 2024 20:00\nDavid Bowie,Let's Dance,Let's Dance,10 Mar 2024 21:00\n").unwrap();
    let nile = count(&lib, "SELECT id FROM artist WHERE name = 'Nile Rodgers'");
    let dance = track_id(&lib, "David Bowie", "Let's Dance");
    let mbid = "6e7a8e1c-0000-4000-8000-000000000001";
    lib.conn.execute("INSERT INTO artist_info (artist_id, status, mbid, fetched_at) VALUES (?, 'ok', ?, 0)", rusqlite::params![nile, mbid]).unwrap();
    lib.conn.execute("INSERT INTO person (mbid, name) VALUES (?, 'Nile Rodgers'), ('6e7a8e1c-0000-4000-8000-000000000002', 'Nathan East')", [mbid]).unwrap();
    let person = count(&lib, &format!("SELECT id FROM person WHERE mbid = '{mbid}'"));
    let other = count(&lib, "SELECT id FROM person WHERE name = 'Nathan East'");
    lib.conn.execute("INSERT INTO credit VALUES (?, ?, 'producer', '')", [dance, person]).unwrap();
    lib.conn.execute("INSERT INTO credit VALUES (?, ?, 'instrument', 'bass')", [dance, other]).unwrap();

    // Rapprochés par identifiant MusicBrainz ; Nathan East n'est pas un artiste écouté.
    let map = lib.person_artists().unwrap();
    assert_eq!(map.get(&person), Some(&nile));
    assert_eq!(map.get(&other), None);
    assert_eq!(lib.person(person).unwrap().artist_id, Some(nile));
    let credited = lib.artist_credits(nile).unwrap();
    assert_eq!(credited.len(), 1);
    assert_eq!(credited[0].artist.name, "David Bowie");
    assert_eq!(credited[0].roles, ["producer"]);
}

#[test]
fn a_mix_from_an_artist_follows_what_is_played_with_it() {
    use librestfm_core::mixes::SeedKind;
    // Chaque soir : A au milieu des artistes B (même session) ; le matin : C seul, sans lien avec A.
    let mut rows = String::from("utc_time,artist,album,title\n");
    for day in 1..=6 {
        let mut ts = chrono::NaiveDate::from_ymd_opt(2024, 3, day).unwrap().and_hms_opt(20, 0, 0).unwrap();
        for i in 0..14 {
            let (artist, title) = if i % 4 == 0 { ("A".to_owned(), format!("a{}", i / 4)) } else { (format!("B{}", i % 5), format!("b{i}")) };
            rows += &format!("{},{artist},,{title}\n", ts.format("%Y-%m-%d %H:%M:%S"));
            ts += chrono::Duration::minutes(4);
        }
        let mut ts = chrono::NaiveDate::from_ymd_opt(2024, 3, day).unwrap().and_hms_opt(8, 0, 0).unwrap();
        for i in 0..14 {
            rows += &format!("{},C,,c{i}\n", ts.format("%Y-%m-%d %H:%M:%S"));
            ts += chrono::Duration::minutes(4);
        }
    }
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes("m.csv", rows.as_bytes()).unwrap();
    let a = count(&lib, "SELECT id FROM artist WHERE name = 'A'");
    let mix = lib.seeded_mix(SeedKind::Artist, a, 0, 30).unwrap().expect("assez de titres liés");
    let artists: std::collections::HashSet<&str> = mix.tracks.iter().map(|t| t.artist.as_str()).collect();
    assert!(artists.contains("A") && artists.iter().filter(|a| a.starts_with('B')).count() >= 3);
    assert!(!artists.contains("C"), "C n'est jamais écouté avec A");
    assert!(mix.tracks.len() >= 12);
    // Un autre tirage reste dans le même entourage.
    let other = lib.seeded_mix(SeedKind::Artist, a, 1, 30).unwrap().unwrap();
    assert!(other.tracks.iter().all(|t| t.artist != "C"));
    // Taille respectée.
    assert!(lib.seeded_mix(SeedKind::Artist, a, 0, 12).unwrap().unwrap().tracks.len() <= 12);
}

#[test]
fn input_folder_imports_new_and_grown_files() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("journal.csv");
    let mut text = String::from("utc_time,artist,album,title\n2024-03-10 20:00:00,A,X,a1\n2024-03-10 20:04:00,A,X,a2\n");
    std::fs::write(&log, &text).unwrap();
    std::fs::write(dir.path().join("notes.md"), "pas un export").unwrap();
    let mut lib = Library::open_in_memory().unwrap();

    let r = lib.scan_input(dir.path()).unwrap();
    assert_eq!((r.files.len(), r.files[0].added, r.unchanged), (1, 2, 0));
    // Rien de changé : rien de relu.
    let r = lib.scan_input(dir.path()).unwrap();
    assert_eq!((r.files.len(), r.unchanged), (0, 1));
    // Le journal grossit : une seule source, seules les nouvelles écoutes comptent.
    text += "2024-03-11 09:00:00,B,Y,b1\n";
    std::fs::write(&log, &text).unwrap();
    let r = lib.scan_input(dir.path()).unwrap();
    assert_eq!(r.files.len(), 1);
    assert_eq!(r.files[0].added, 1);
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 3);
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM source"), 1);
    // Le même contenu importé à la main est reconnu.
    let rep = lib.import_bytes("copie.csv", text.as_bytes()).unwrap();
    assert!(matches!(rep.status, ImportStatus::AlreadyImported));
}

#[test]
fn backup_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("librestfm.sqlite");
    let mut lib = Library::open(&db).unwrap();
    lib.import_bytes("a.csv", b"A,X,a1,10 Mar 2024 20:00\nA,X,a2,10 Mar 2024 20:05\n").unwrap();
    let saved = dir.path().join("sauvegarde.sqlite");
    lib.backup_to(&saved).unwrap();
    let info = Library::check_backup(&saved).unwrap();
    assert_eq!(info.plays, 2);
    assert_eq!(count(&lib, "PRAGMA user_version"), librestfm_core::db::SCHEMA_VERSION, "constante à jour avec les migrations");

    // Après la sauvegarde : une autre écoute, puis restauration.
    lib.import_bytes("b.csv", b"B,Y,b1,11 Mar 2024 20:00\n").unwrap();
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 3);
    let safety = dir.path().join("avant.sqlite");
    lib.restore_from(&saved, &db, &safety).unwrap();
    assert_eq!(count(&lib, "SELECT COUNT(*) FROM scrobble"), 2);
    assert_eq!(Library::check_backup(&safety).unwrap().plays, 3, "la base remplacée est gardée");
    // Toujours utilisable, et rouverte au prochain lancement.
    lib.import_bytes("b.csv", b"B,Y,b1,11 Mar 2024 20:00\n").unwrap();
    drop(lib);
    assert_eq!(count(&Library::open(&db).unwrap(), "SELECT COUNT(*) FROM scrobble"), 3);

    // Un fichier qui n'est pas une base est refusé.
    let junk = dir.path().join("x.sqlite");
    std::fs::write(&junk, "pas une base").unwrap();
    assert!(Library::check_backup(&junk).is_err());

    // Automatique : une par période, les plus anciennes supprimées.
    let auto = dir.path().join("auto");
    let lib = Library::open(&db).unwrap();
    assert!(lib.auto_backup(&auto, 7, 2).unwrap().is_some());
    assert!(lib.auto_backup(&auto, 7, 2).unwrap().is_none());
    for d in ["2020-01-01", "2020-01-08"] {
        std::fs::write(auto.join(format!("librestfm-{d}.sqlite")), "").unwrap();
    }
    assert_eq!(Library::auto_backups(&auto).len(), 3);
    assert!(lib.auto_backup(&auto, 0, 2).unwrap().is_some());
    assert_eq!(Library::auto_backups(&auto).len(), 2);
}

#[test]
fn browsing_the_library() {
    use librestfm_core::stats::BrowseSort;
    let mut lib = Library::open_in_memory().unwrap();
    lib.import_bytes(
        "b.csv",
        b"Daft Punk,Discovery,One More Time,10 Mar 2024 20:00\nDaft Punk,Discovery,Aerodynamic,10 Mar 2024 20:06\nDaft Punk,Discovery,One More Time,11 Mar 2024 20:00\nAir,Moon Safari,La Femme d'Argent,12 Mar 2024 20:00\n",
    )
    .unwrap();
    let p = lib.browse(Kind::Artist, BrowseSort::Plays, true, "", 0, 10).unwrap();
    assert_eq!(p.total, 2);
    assert_eq!(p.rows[0].item.name, "Daft Punk");
    assert_eq!(p.rows[0].item.plays, 3);
    let p = lib.browse(Kind::Artist, BrowseSort::Name, false, "", 0, 10).unwrap();
    assert_eq!(p.rows[0].item.name, "Air");
    // Filtre sur le titre ou l'artiste, sans accents ni casse ; pagination.
    let p = lib.browse(Kind::Track, BrowseSort::Plays, true, "daft", 0, 1).unwrap();
    assert_eq!((p.total, p.rows.len()), (2, 1));
    assert_eq!(p.rows[0].item.name, "One More Time");
    let p = lib.browse(Kind::Track, BrowseSort::Plays, true, "argent", 0, 10).unwrap();
    assert_eq!(p.rows[0].item.artist.as_deref(), Some("Air"));
    let p = lib.browse(Kind::Album, BrowseSort::First, true, "", 0, 10).unwrap();
    assert_eq!(p.rows[0].item.name, "Moon Safari");
}
