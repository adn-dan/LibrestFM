<p align="center"><img src="src-tauri/icons/source/logo.svg" width="112" alt=""></p>

# LibrestFM

Desktop and Android app that analyses your music listening history from Last.fm, Libre.fm, ListenBrainz or Spotify exports and accounts. Everything stays on your device: no LibrestFM account, no server. Independent project, not affiliated with Libre.fm or any of the services below.

- **Import and merge** several exports and accounts, even when they overlap or were already imported: a play is counted only once.
- **Overview**: plays and listening time over time, top artists, albums and tracks, day × hour heatmap, artist origins, lyrics languages, release years.
- **A page for every artist, album, track and person**: plays over time, what you listen to before and after, credits (writers, composers, musicians), audio features, lyrics.
- **Cleanup** of spellings, album editions, file names, featured artists and double sends, automatic when it is safe, always reversible.
- **Mixes and discoveries** based on what you are listening to lately.
- **Android**: built-in scrobbler (records what your other apps play, locally or to your accounts), "Now playing" screen with synced lyrics.

## Supported formats

The format is detected automatically.

| Source | File |
|---|---|
| Last.fm | CSV from [lastfm-to-csv](https://benjaminbenben.com/lastfm-to-csv/) (`artist,album,track,date`) |
| Last.fm, Libre.fm | TSV from `lastexport.py` / `last2libre` (`epoch	track	artist	album	mbids`) |
| ListenBrainz | JSON or JSON Lines export |
| Spotify | Extended history (`Streaming_History_Audio_*.json`) or account history (`StreamingHistory*.json`) |
| Other | Any CSV/TSV with a header containing at least artist, track and date (epoch, ISO 8601 or `31 Jan 2024 18:45`) |

Accepted encodings: UTF-8 (with or without BOM), UTF-16, Windows-1252.

### Online accounts

Sources page, "Online accounts": the whole history is fetched, then new plays at each launch. An interrupted sync resumes where it stopped.

| Service | What you need |
|---|---|
| Libre.fm | Username |
| ListenBrainz | Username |
| Last.fm | Username and a free personal API key ([create one](https://www.last.fm/api/account/create)), stored locally |

No password is needed to read: the history must be public. An account is a source like a file and follows the same merge rules.

### Import folder (desktop)

Exports dropped in `~/Music/LibrestFM` (your system's music folder, can be changed on the Sources page) are imported at each start. Only new or modified files are read; a file that grows over time stays a single source and only brings its new plays.

### Backup and export

- Settings, "Backup": copy of the whole database and restore from a file (the replaced database is copied first). Automatic weekly backup in the data folder (`backups/`), the last two kept.
- Sources page, "Export history": plays from the checked sources, merged and deduplicated, as CSV or as ListenBrainz-format JSON Lines. Both re-import identically.

### Mixes and discoveries

- **Mixes**: moments, ambiences, seasons, rediscover, around an artist, hidden gems; "Mix from this track / album / artist" from any page. Each play weighs less as it ages (3-month half-life): mixes follow what you are listening to now. XSPF, JSPF, CSV export.
- **Discoveries**: never-played tracks from artists close to yours (ListenBrainz, Deezer), with a 30 s preview. "Like" and "Not for me" weigh on the next selection, renewed every week.

### Android

- **Scrobbler** (Settings, Scrobbler): detects music played by the apps you choose through notification access, even with LibrestFM closed. A play counts at half the track or 4 min. Destinations: local file, Libre.fm, Last.fm, ListenBrainz or a compatible server (GNU FM, Maloja, Koito…), with an offline queue. The password is used only once, to get a session key. Plays recorded on the phone appear on the Sources page as "Phone scrobbler"; unticking "On this phone" stops recording new ones and keeps the others.
- **Now playing**: current track, playback controls, synced lyrics scrolling with the music (the screen stays on while they scroll).
- Optional, silent **notifications**: scrobbler active, download progress.
- No Google services required. Android 7 or later, phones and tablets.

### Interface

French, English, Spanish, German, Italian, Portuguese, Simplified Chinese, Japanese. Light or dark theme, accent colour and date format of your choice.

## Data sent

LibrestFM only sends artist, album and track names and public identifiers, to the services that enrich the library. Responses are stored locally.

| Service | For |
|---|---|
| [ReccoBeats](https://reccobeats.com) | Audio features |
| [MusicBrainz](https://musicbrainz.org) | Credits, release dates, artist origins |
| [Deezer](https://www.deezer.com) (public API) | Covers, photos, durations, related artists, previews |
| [ListenBrainz](https://listenbrainz.org) | Similar artists (discoveries), account sync |
| [LRCLIB](https://lrclib.net) | Lyrics (artist, track, album and duration) |
| Last.fm, Libre.fm | Account sync; on Android, sending plays if you chose to |

## How it is calculated

All computation happens locally, in a SQLite database. Every step below is reversible: nothing you imported is lost, the original identifiers of each play are kept next to the corrected ones.

### 1. Reading a line

A line becomes a play (artist, track, optional album, start time) or is rejected, with its reason shown in the import report:

| Rejection | Case |
|---|---|
| Missing artist or track | empty field after trimming spaces |
| Unreadable date | unrecognised format |
| Date out of range | before 2000 (almost always a "1970" date from faulty software) or more than one day in the future |
| Too short | Spotify: less than 30 s played (a scrobbling service would not have counted it) |
| Not music | Spotify: podcast or audiobook (no track name) |
| Recording error | track name ending in `.mp4` |
| Malformed line | CSV or JSON unreadable at that point |

Spotify timestamps the end of a play, other sources the start: Spotify times are moved back to the start (end time minus time played). A time without seconds (lastfm-to-csv export) is marked "to the minute": the play started between that minute and the next.

### 2. Who is who: artists, albums, tracks

Two spellings point to the same page when they share the same **key**, computed as follows:

- text recomposed (Unicode NFC), HTML entities decoded (`&amp;` → `&`);
- lowercase, accents removed from Latin letters only (in Japanese, removing a mark would change the word); `œ`, `æ`, `ß`, `ø`, `ł`, `đ` written `oe`, `ae`, `ss`, `o`, `l`, `d`;
- punctuation and spaces removed, `&` read as "and";
- for a track: mentions that name the same work are removed ("feat. …", "Remastered", "Remasterisé", "Radio Edit", "2003 Edit", "Mono", "Explicit", "Deluxe", "Official Video", web addresses…), unless the mention also marks another version ("Live", "Remix", "Acoustic", "Demo"…): a live version or a remix stays a separate track;
- for an artist: "feat. …" removed.

An album belongs to one artist: two artists' "Greatest Hits" are two albums. When these rules change between versions, keys are recomputed at startup and pages that became identical are joined (reversible merge, "Same name, different spelling").

### 3. Same play or two plays?

A play from one source is the same as a play already known from **another** source if the track is the same (after corrections) and the time is close:

| Times compared | Same play if |
|---|---|
| two precise times | ±90 s |
| one to-the-minute time and one precise | the precise one between 30 s before and 89 s after the minute |
| two to-the-minute times | ±60 s |

The window is not symmetric for a to-the-minute time: the time is truncated, so the play started within the following 59 seconds. A ±120 s window caught the next replay of a short track. When several plays match, the closest is taken (for a to-the-minute time, compared with the middle of its minute). The most precise time and the most complete album are kept.

Within **the same source**, another time is another play: two plays of the same track 3 minutes apart in one file are two plays. A line read again (file re-imported, account page fetched again) is recognised by its exact time and not added.

Consequences:
- a file already imported (same content) is skipped; a growing file only brings its new lines;
- importing sources in a different order gives the same total;
- removing a source only deletes the plays it alone contained.

### 4. Double sends

A player or scrobbling app sometimes sends the same play twice (two devices at once, repeated submission). Within one source, the same track restarted **before half its duration** counts once; unknown duration: less than 30 s apart between two precise times (a scrobble requires at least 30 s of listening). The first play stays. A play that another source confirms without having the first one is kept. Removed sends are listed in Cleanup, can be restored in one click, and are never removed again once restored.

### 5. Cleanup

Safe corrections apply on their own, after each import and at startup; the others are suggested in Cleanup. All can be undone.

| Rule | Example | Applied |
|---|---|---|
| Badly encoded characters | "Artist A &amp;amp; Artist B" | automatically |
| Editions of the same album | "Album (Remastered)", "Album (Deluxe)" → "Album" | automatically |
| Albums with a close title | same title outside brackets, unknown mention | suggested |
| File names | "03 - Track.mp3" | automatically if the right track already exists, suggested otherwise |
| Artist in the title | track "Artist - Track" | automatically if the right track already exists, suggested otherwise |
| Several artists in one | "Artist A; Artist B" → Artist A | automatically if the main artist already exists, suggested otherwise |
| Nickname in the name | "First 'Nick' Last" | automatically |
| Same MusicBrainz identifier | two names for the same artist | automatically |
| Name variants | "The Band" and "Band" | suggested |
| Featured artist | track played under "Guest", credited "Main feat. Guest" by MusicBrainz → Main, with the album of the same name | automatically, if the main artist is already in the history and the guest marked "feat.", "with"… |

A manual merge ("Merge into…") and a name correction ("Rename") are available from every page. After a correction, two plays that only differed by spelling and fall at the same time (rule of step 3, no common source) become one; undoing the correction splits them again. A future play under the old spelling follows the correction.

### 6. What is counted

The displayed total is the number of plays after merging, double-send removal and corrections. Tops, charts, details and exports all come from this same table: their sums always match the total.

Month, day, weekday and hour are those of the device's time zone; if it changes, they are recomputed at startup. A **session** is a run of plays less than 30 min apart.

### 7. Listening time

Each play counts the track's duration, unless the next play starts before the end: it then stops at the next play, without going below the minimum for a scrobble (half the track, at most 4 min). Track of unknown duration: 3:30. The overview also gives this minimum ("at least …").

### 8. Release years

The year of a play is the earliest of the work's first release (all MusicBrainz recordings of this track by this artist, excluding live and remix) and the release of the album listened to. A 1981 track played from a 2022 best-of stays in 1981.

### 9. Enrichment

- **Audio features** (ReccoBeats): the album listened to when several of its tracks are pending, otherwise the track, otherwise the artist's catalogue (25 pages at most). Collective name ("A & B"): the full name first, then each artist. The identical title is preferred to the title without mention; a live or remix version absent from the searched title is discarded; on a tie, the most popular.
- **Credits** (MusicBrainz): recording found by ISRC then by search, the album listened to and official releases first, the first one with credits kept (3 attempts at most). Writers and composers come from the work, producers and musicians from the recording.
- **Albums and artists**: first release date and origin (MusicBrainz); durations, label, covers, photos (Deezer).
- **Lyrics** (LRCLIB): synced version first, same artist and same title, duration within 4 s; otherwise plain text. A track without lyrics is searched again after 30 days.

### 10. Mixes and discoveries

Preferences weigh each play by its age: weight 1 today, ½ three months ago, 1/16 a year ago (3-month half-life, 0.2 % floor). Mixes therefore follow what is being played now.

- **Ambiences**: tracks grouped by sound (audio features), weighted by their recent plays.
- **Moments, seasons**: what is played more at this time of the week or in this season than the rest of the time.
- **Rediscover**: tracks played a lot, then not at all for 6 months or a year (raw count, on purpose).
- **Around an artist**: the current top 3 artists, their tracks and what is played in the same sessions.
- **Hidden gems**: rarely played tracks by favourite artists (raw count).
- **Mix from a track, album or artist**: what is played with it, artists sharing its musicians or producers, tracks with a close sound.
- Never a track played fewer than 3 times or removed with "Not for me".

**Discoveries**: the 20 most played artists lately are the starting points; ListenBrainz (artists played in the same sessions) and Deezer (related artists) give their neighbours. A neighbour close to several starting points comes first. Their top tracks (Deezer) are suggested, without anything already played (including under a featured artist's name), live versions or remixes. "Like" and "Not for me" weigh on the next selection.

### 11. Android scrobbler

A play is recorded when the track (longer than 30 s) has been played for half its duration or 4 min; unknown duration: 30 s. The time kept is the start time. The same track restarted from the beginning after being counted is a new play. Track and artist are those announced by the player; they are matched with the library (corrections included) for lyrics and the track page.

## Installation

Prebuilt packages: `.rpm` (Fedora), `.deb` (Debian, Ubuntu, Mint), `.AppImage` (any Linux distribution), `.exe` (Windows installer), `.apk` (Android). Data is stored in `~/.local/share/io.github.adn-dan.librestfm/` (Linux) or `%APPDATA%\io.github.adn-dan.librestfm\` (Windows); Settings, "Backup" moves it from one device to another.

| Platform | Status |
|---|---|
| Linux: Fedora (`.rpm`, `.AppImage`) | tested |
| Android (`.apk`) | tested on one phone |
| Linux: Debian, Ubuntu, Mint (`.deb`) | built, not tested yet |
| Windows (`.exe`) | built, not tested yet; unsigned, so SmartScreen warns on first launch ("More info", "Run anyway") |
| macOS | never built (needs a Mac) |
| iOS | not supported |

Android 13 and later: for an app installed outside a store, notification access first requires "Allow restricted settings" in the app info.

## Building from source

### 1. Common requirements

- [Rust](https://rustup.rs) (stable) and Node.js 20 or later.
- The [Tauri system dependencies](https://v2.tauri.app/start/prerequisites/) for your system:

```sh
# Fedora
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel
sudo dnf group install c-development

# Debian, Ubuntu, Mint
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Windows: "Microsoft C++ Build Tools" (workload "Desktop development with C++"); WebView2 is already present on Windows 10 and 11.

Then, in the project folder:

```sh
npm install
npm run tauri dev                  # run the app in development mode
cargo test -p librestfm-core       # core tests, no system dependency
```

### 2. Linux packages

```sh
npm run tauri build -- --bundles rpm,deb,appimage
```

Packages in `target/release/bundle/`. They depend on GTK 3 and WebKitGTK 4.1.

### 3. Windows installer

On Windows: `npm run tauri build` (`.msi` and `.exe` in `target\release\bundle\`).

From Linux, with [cargo-xwin](https://github.com/rust-cross/cargo-xwin) (it downloads the Microsoft CRT and Windows SDK, whose licence it accepts for you):

```sh
sudo dnf install mingw32-nsis clang lld llvm     # Debian, Ubuntu: nsis clang lld llvm
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
npm run tauri build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
```

Installer in `target/x86_64-pc-windows-msvc/release/bundle/nsis/`.

### 4. Android APK

Requirements: JDK 17 or 21 (a newer JDK makes Gradle fail), and the Android SDK with platform 37, build-tools and NDK, for example with `sdkmanager`:

```sh
sdkmanager "platform-tools" "platforms;android-37.0" "build-tools;36.0.0" "ndk;29.0.14206865"
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export JAVA_HOME=/path/to/jdk-21 ANDROID_HOME=/path/to/Android/Sdk NDK_HOME=$ANDROID_HOME/ndk/29.0.14206865
npm run tauri android build -- --apk --target aarch64 armv7 x86_64
```

APK in `src-tauri/gen/android/app/build/outputs/apk/universal/release/`. `--target aarch64` alone is enough for recent phones and builds faster.

Signing: create a key once and keep it safe (an APK signed with another key cannot update an installed one):

```sh
keytool -genkeypair -keystore librestfm.jks -alias librestfm -keyalg RSA -keysize 4096 -validity 10000
```

Then create `src-tauri/gen/android/keystore.properties` (ignored by git):

```properties
password=…
keyAlias=librestfm
storeFile=/path/to/librestfm.jks
```

Without this file, the release APK is not signed and cannot be installed.

### 5. macOS

`npm run tauri build` on a Mac produces a `.dmg`. Unsigned, it opens the first time with right click, Open.

## Architecture

```
core/                  Rust, without Tauri: import, merging, SQL statistics, enrichment
src-tauri/             Rust, Tauri commands on top of core
src-tauri/gen/android  Android project, Kotlin scrobbler
src/                   Svelte + ECharts interface (everything bundled)
```

Icons are generated from `src-tauri/icons/source/`: `npx tauri icon src-tauri/icons/source/icon.json`. The list of third-party components and their licences, shown in Settings, About, is regenerated with `npm run licenses`.

## Known limitations

- ReccoBeats relies on the Spotify catalogue: a track missing from Spotify has no audio features. Tempo may be doubled or halved.
- MusicBrainz allows one request per second: credits and original years arrive gradually (3 to 4 s per track).
- A private Last.fm profile cannot be read (no OAuth login). Libre.fm heavily limits the rate.
- Immediate replay of a very short track, seen to the minute by one source and shifted by another: both readings are equally valid, the play may be counted once too many or too few (step 3).
- The interface waits while a very large file is imported (about 14 s for 500,000 plays).
- iOS is not supported (no access to other apps' players).

## Trademarks

LibrestFM is an independent project. It is not affiliated with, endorsed or sponsored by Libre.fm, Last.fm, ListenBrainz, MusicBrainz (MetaBrainz Foundation), Spotify, Deezer, ReccoBeats or LRCLIB. Their names are used only to describe compatibility and belong to their respective owners.

## Licence

Copyright © 2026 adn-dan. LibrestFM is free software: you can redistribute it and modify it under the terms of the [GNU General Public License](LICENSE), version 3 or later. Third-party components keep their own licence (full list in the app, Settings, About).
