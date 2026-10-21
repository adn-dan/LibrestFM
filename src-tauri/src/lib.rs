//! Couche Tauri : expose le cœur (`librestfm-core`) à l'interface. Les commandes
//! tournent hors du thread principal ; l'enrichissement tourne dans un thread
//! dédié et publie sa progression par l'événement `enrich-progress`.

use librestfm_core::accounts::{self, Account, Service, SyncStatus};
use librestfm_core::enrich::{self, EnrichStatus};
use librestfm_core::credits::{self, AlbumCredits, ArtistLink, CreditedFor, CreditsStatus, PersonDetail, TrackCredits};
use librestfm_core::aliases::{MergeOp, Suggestion};
use librestfm_core::images;
use librestfm_core::info;
use librestfm_core::mixes::{self, Mix, SeedKind};
use librestfm_core::stats::{AlbumRelease, BrowsePage, BrowseSort, Comparison, Detail, Discoveries, Filter, SessionStats};
use std::collections::HashMap;
use librestfm_core::import::{ImportReport, SourceInfo};
use librestfm_core::inbox::InboxReport;
use librestfm_core::discover::{self, Scope, Selection};
use librestfm_core::backup::BackupInfo;
use librestfm_core::stats::{EntityDetail, Kind, Overview, Period, SearchResults};
use librestfm_core::Library;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

mod scrobbler;

struct AppState {
    lib: Arc<Mutex<Library>>,
    /// Dossier d'import par défaut (~/Musique/LibrestFM).
    default_input: PathBuf,
    /// Fichier de la base, et dossier des sauvegardes automatiques.
    db_path: PathBuf,
    backups: PathBuf,
    enrich_running: Arc<AtomicBool>,
    enrich_cancel: Arc<AtomicBool>,
    credits_running: Arc<AtomicBool>,
    credits_cancel: Arc<AtomicBool>,
    info_running: Arc<AtomicBool>,
    info_cancel: Arc<AtomicBool>,
    discover_running: Arc<AtomicBool>,
    discover_cancel: Arc<AtomicBool>,
    sync_running: Arc<AtomicBool>,
    sync_cancel: Arc<AtomicBool>,
    /// Dernier état de synchronisation, pour l'interface qui arrive en cours de route.
    sync_status: Arc<Mutex<SyncStatus>>,
    /// Récupérations lancées à l'ouverture d'une fiche (image, crédits, infos) en cours :
    /// une restauration les attend, et n'en laisse pas partir de nouvelles pendant ce temps
    /// (elles écriraient dans la base restaurée sur un identifiant qui y désigne autre chose).
    fetching: Arc<AtomicUsize>,
    restoring: Arc<AtomicBool>,
}

/// Une récupération à la demande en cours ; None si une restauration est en cours.
struct Fetching(Arc<AtomicUsize>);

impl Fetching {
    fn start(state: &AppState) -> Option<Fetching> {
        state.fetching.fetch_add(1, Ordering::SeqCst);
        let guard = Fetching(state.fetching.clone());
        (!state.restoring.load(Ordering::SeqCst)).then_some(guard)
    }
}

impl Drop for Fetching {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

type CmdResult<T> = Result<T, String>;

/// Exécute `f` sur la bibliothèque dans un thread bloquant.
/// Erreur transmise à l'interface : « code␟détail », le code étant traduit là-bas.
fn err(e: librestfm_core::Error) -> String {
    format!("{}\u{1f}{e}", e.code())
}

async fn with_lib<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&mut Library) -> librestfm_core::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let lib = state.lib.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = lib.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
struct ImportResult {
    reports: Vec<ImportReport>,
    /// Corrections sûres appliquées juste après l'import.
    cleaned: usize,
}

#[tauri::command]
async fn import_files(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<ImportResult> {
    with_lib(&state, move |lib| {
        let mut reports = Vec::new();
        for p in paths {
            let report = lib.import_file(&PathBuf::from(&p))?;
            let _ = app.emit("import-progress", &report);
            reports.push(report);
        }
        let cleaned = lib.auto_clean()?;
        Ok(ImportResult { reports, cleaned })
    })
    .await
}

/// Dossier d'import en vigueur : celui choisi, sinon celui par défaut.
fn input_dir_of(lib: &Library, default: &std::path::Path) -> PathBuf {
    lib.input_dir().ok().flatten().map(PathBuf::from).unwrap_or_else(|| default.to_path_buf())
}

#[tauri::command]
async fn input_dir(state: State<'_, AppState>) -> CmdResult<String> {
    let default = state.default_input.clone();
    with_lib(&state, move |lib| Ok(input_dir_of(lib, &default).display().to_string())).await
}

#[tauri::command]
async fn set_input_dir(state: State<'_, AppState>, path: String) -> CmdResult<InboxReport> {
    with_lib(&state, move |lib| {
        lib.set_input_dir(&path)?;
        lib.scan_input(std::path::Path::new(&path))
    })
    .await
}

#[tauri::command]
async fn scan_input(state: State<'_, AppState>) -> CmdResult<InboxReport> {
    let default = state.default_input.clone();
    with_lib(&state, move |lib| {
        let dir = input_dir_of(lib, &default);
        let _ = std::fs::create_dir_all(&dir);
        lib.scan_input(&dir)
    })
    .await
}

#[tauri::command]
async fn backup_db(state: State<'_, AppState>, path: String) -> CmdResult<BackupInfo> {
    with_lib(&state, move |lib| {
        lib.backup_to(std::path::Path::new(&path))?;
        Library::check_backup(std::path::Path::new(&path))
    })
    .await
}

#[tauri::command]
async fn check_backup(path: String) -> CmdResult<BackupInfo> {
    tauri::async_runtime::spawn_blocking(move || Library::check_backup(std::path::Path::new(&path)).map_err(err))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn restore_db(state: State<'_, AppState>, path: String) -> CmdResult<BackupInfo> {
    // Tâches de fond arrêtées, et attendues : une écriture en cours (lot
    // d'enrichissement…) irait sinon dans la base remplacée, avec des identifiants
    // qui n'y correspondent plus.
    for c in [&state.enrich_cancel, &state.credits_cancel, &state.info_cancel, &state.sync_cancel, &state.discover_cancel] {
        c.store(true, Ordering::SeqCst);
    }
    let running = [&state.enrich_running, &state.credits_running, &state.info_running, &state.sync_running, &state.discover_running].map(|r| r.clone());
    // Récupérations ouvertes depuis une fiche : plus de nouvelles, celles en cours attendues.
    state.restoring.store(true, Ordering::SeqCst);
    let fetching = state.fetching.clone();
    let idle = tauri::async_runtime::spawn_blocking(move || {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let busy = || running.iter().any(|r| r.load(Ordering::SeqCst)) || fetching.load(Ordering::SeqCst) > 0;
        while busy() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if busy() { Err(err(librestfm_core::Error::Invalid("busy"))) } else { Ok(()) }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    let result = match idle {
        Ok(()) => {
            let (db, safety) = (state.db_path.clone(), state.backups.join(librestfm_core::backup::safety_name()));
            with_lib(&state, move |lib| lib.restore_from(std::path::Path::new(&path), &db, &safety)).await
        }
        Err(e) => Err(e),
    };
    state.restoring.store(false, Ordering::SeqCst);
    result
}

#[derive(serde::Serialize)]
struct Backups {
    dir: String,
    /// (fichier, octets), la plus récente d'abord.
    auto: Vec<(String, u64)>,
}

#[tauri::command]
fn backups(state: State<'_, AppState>) -> Backups {
    Backups { dir: state.backups.display().to_string(), auto: Library::auto_backups(&state.backups) }
}

#[tauri::command]
async fn discover_selection(state: State<'_, AppState>, scope: Scope) -> CmdResult<Selection> {
    with_lib(&state, move |lib| lib.selection(scope)).await
}

#[tauri::command]
fn start_discover(app: AppHandle, state: State<'_, AppState>, scope: Scope) {
    // Pas pendant une restauration : la tâche écrirait dans la base remplacée.
    if state.restoring.load(Ordering::SeqCst) {
        return;
    }
    spawn_discover(app, state.lib.clone(), state.discover_running.clone(), state.discover_cancel.clone(), scope);
}

fn spawn_discover(app: AppHandle, lib: Arc<Mutex<Library>>, running: Arc<AtomicBool>, cancel: Arc<AtomicBool>, scope: Scope) {
    // Seule la sélection de la semaine reprend au lancement si elle a été interrompue.
    let name = if scope == Scope::Weekly { "discover" } else { "discover_artist" };
    spawn_job(app, lib, running, cancel, name, move |l, c, p| discover::run(l, scope, c, p), |s| s.running = false);
}

#[tauri::command]
fn stop_discover(state: State<'_, AppState>) {
    state.discover_cancel.store(true, Ordering::SeqCst);
    forget_jobs(&state.lib, &["discover", "discover_artist"]);
}

#[tauri::command]
fn discover_running(state: State<'_, AppState>) -> bool {
    state.discover_running.load(Ordering::Relaxed)
}

#[tauri::command]
async fn discover_verdict(state: State<'_, AppState>, deezer_id: i64, artist: String, verdict: i64) -> CmdResult<()> {
    with_lib(&state, move |lib| lib.discover_verdict(deezer_id, &artist, verdict)).await
}

#[tauri::command]
async fn discover_preview(deezer_id: i64) -> CmdResult<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || discover::preview(deezer_id).map_err(err)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn suggestions(state: State<'_, AppState>) -> CmdResult<Vec<Suggestion>> {
    with_lib(&state, |lib| lib.suggestions()).await
}

/// Doubles envois retirés (artiste, titre, heure).
#[tauri::command]
async fn double_sends(state: State<'_, AppState>) -> CmdResult<Vec<(String, String, i64)>> {
    with_lib(&state, |lib| lib.double_sends()).await
}

#[tauri::command]
async fn restore_double_sends(state: State<'_, AppState>) -> CmdResult<usize> {
    with_lib(&state, |lib| lib.restore_double_sends()).await
}

#[tauri::command]
async fn apply_suggestions(state: State<'_, AppState>, list: Vec<Suggestion>) -> CmdResult<usize> {
    with_lib(&state, move |lib| {
        let mut n = 0;
        for s in &list {
            if lib.apply_suggestion(s).is_ok() {
                n += 1;
            }
        }
        Ok(n)
    })
    .await
}

#[tauri::command]
async fn dismiss_suggestions(state: State<'_, AppState>, keys: Vec<String>) -> CmdResult<()> {
    with_lib(&state, move |lib| keys.iter().try_for_each(|k| lib.dismiss_suggestion(k))).await
}

#[tauri::command]
async fn clear_dismissed(state: State<'_, AppState>) -> CmdResult<usize> {
    with_lib(&state, |lib| lib.clear_dismissed()).await
}

#[tauri::command]
async fn dismissed_count(state: State<'_, AppState>) -> CmdResult<i64> {
    with_lib(&state, |lib| lib.dismissed_count()).await
}

#[tauri::command]
async fn merge_entities(state: State<'_, AppState>, kind: Kind, from: i64, to: i64) -> CmdResult<i64> {
    with_lib(&state, move |lib| lib.merge(kind, from, to, "manual")).await
}

/// Corrige le nom d'une fiche ; renvoie la fiche qui porte désormais ses écoutes.
#[tauri::command]
async fn rename_entity(state: State<'_, AppState>, kind: Kind, id: i64, name: String) -> CmdResult<i64> {
    with_lib(&state, move |lib| lib.rename(kind, id, &name)).await
}

#[tauri::command]
async fn undo_merge(state: State<'_, AppState>, op: i64) -> CmdResult<()> {
    with_lib(&state, move |lib| lib.undo_merge(op)).await
}

#[tauri::command]
async fn merge_ops(state: State<'_, AppState>) -> CmdResult<Vec<MergeOp>> {
    with_lib(&state, |lib| lib.merge_ops()).await
}

#[tauri::command]
async fn list_sources(state: State<'_, AppState>) -> CmdResult<Vec<SourceInfo>> {
    with_lib(&state, |lib| lib.sources()).await
}

#[tauri::command]
async fn remove_source(state: State<'_, AppState>, id: i64) -> CmdResult<usize> {
    with_lib(&state, move |lib| lib.remove_source(id)).await
}

#[tauri::command]
async fn overview(state: State<'_, AppState>, period: Period, top: usize, by_time: Option<bool>) -> CmdResult<Overview> {
    with_lib(&state, move |lib| lib.overview_by(period, top, by_time.unwrap_or(false))).await
}

#[tauri::command]
async fn entity(state: State<'_, AppState>, kind: Kind, id: i64) -> CmdResult<EntityDetail> {
    with_lib(&state, move |lib| lib.entity(kind, id)).await
}

#[tauri::command]
async fn search(state: State<'_, AppState>, query: String) -> CmdResult<SearchResults> {
    with_lib(&state, move |lib| lib.search(&query, 8)).await
}

#[tauri::command]
async fn cached_images(state: State<'_, AppState>, kind: Kind, ids: Vec<i64>) -> CmdResult<HashMap<i64, String>> {
    with_lib(&state, move |lib| lib.cached_images(kind, &ids)).await
}

/// Image d'une entité, téléchargée si besoin (sans bloquer la base pendant le réseau).
#[tauri::command]
async fn image(state: State<'_, AppState>, kind: Kind, id: i64) -> CmdResult<Option<String>> {
    let lib = state.lib.clone();
    let Some(guard) = Fetching::start(&state) else { return Ok(None) };
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        images::image(&lib, kind, id).map_err(err)
    })
        .await
        .map_err(|e| e.to_string())?
}

/// Paroles d'un titre (LRCLIB), synchronisées si possible ; gardées dans la base.
#[tauri::command]
async fn lyrics(
    state: State<'_, AppState>,
    artist: String,
    title: String,
    album: Option<String>,
    duration_ms: Option<i64>,
    also: Option<Vec<String>>,
) -> CmdResult<librestfm_core::lyrics::Lyrics> {
    let lib = state.lib.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Autres noms à essayer si l'artiste ne donne rien (artiste de l'album, nom annoncé par le lecteur).
        let names: Vec<&str> = std::iter::once(artist.as_str()).chain(also.iter().flatten().map(String::as_str)).collect();
        librestfm_core::lyrics::get_any(&lib, &names, &title, album.as_deref(), duration_ms)
    })
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

/// Titre de la bibliothèque (corrections appliquées) pour un titre joué dans une autre appli.
#[tauri::command]
async fn find_track(state: State<'_, AppState>, artist: String, title: String, album_artist: Option<String>) -> CmdResult<Option<librestfm_core::aliases::KnownTrack>> {
    with_lib(&state, move |lib| lib.find_track(&artist, &title, album_artist.as_deref())).await
}

/// Crédits d'un titre ; s'ils n'ont jamais été cherchés, on les récupère maintenant.
#[tauri::command]
async fn track_credits(state: State<'_, AppState>, track_id: i64) -> CmdResult<TrackCredits> {
    let lib = state.lib.clone();
    let guard = Fetching::start(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
        let known = lock().track_credits(track_id).map_err(err)?;
        if guard.is_none() {
            return Ok(known);
        }
        if known.status == "none" {
            // Une erreur réseau est enregistrée comme telle : on renvoie l'état quoi qu'il arrive.
            let _ = credits::fetch_track(&lib, track_id, None);
        } else {
            // Fiche ancienne : ordre des artistes crédités relu (rien à faire sinon).
            let _ = credits::refresh_artist_credit(&lib, track_id, None);
        }
        drop(guard);
        lock().track_credits(track_id).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn album_credits(state: State<'_, AppState>, album_id: i64) -> CmdResult<AlbumCredits> {
    with_lib(&state, move |lib| lib.album_credits(album_id)).await
}

#[tauri::command]
async fn person(state: State<'_, AppState>, id: i64) -> CmdResult<PersonDetail> {
    with_lib(&state, move |lib| lib.person(id)).await
}

#[tauri::command]
async fn credits_status(state: State<'_, AppState>) -> CmdResult<CreditsStatus> {
    let running = state.credits_running.load(Ordering::Relaxed);
    let mut st = with_lib(&state, |lib| lib.credits_status()).await?;
    st.running = running;
    Ok(st)
}

#[tauri::command]
fn start_credits(app: AppHandle, state: State<'_, AppState>) {
    // Pas pendant une restauration : la tâche écrirait dans la base remplacée.
    if state.restoring.load(Ordering::SeqCst) {
        return;
    }
    spawn_credits(app, state.lib.clone(), state.credits_running.clone(), state.credits_cancel.clone());
}

fn spawn_credits(app: AppHandle, lib: Arc<Mutex<Library>>, running: Arc<AtomicBool>, cancel: Arc<AtomicBool>) {
    spawn_job(app, lib, running, cancel, "credits", |l, c, p| credits::run(l, c, p), |s| s.running = false);
}

#[tauri::command]
fn stop_credits(state: State<'_, AppState>) {
    state.credits_cancel.store(true, Ordering::SeqCst);
    forget_jobs(&state.lib, &["credits"]);
}

#[tauri::command]
async fn retry_credit_failures(state: State<'_, AppState>) -> CmdResult<usize> {
    with_lib(&state, |lib| lib.reset_credit_failures()).await
}

#[tauri::command]
async fn detail(state: State<'_, AppState>, filter: Filter, top: Option<usize>) -> CmdResult<Detail> {
    with_lib(&state, move |lib| lib.detail(&filter, top.unwrap_or(10).min(100))).await
}

#[tauri::command]
async fn artist_credits(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<CreditedFor>> {
    with_lib(&state, move |lib| lib.artist_credits(id)).await
}

#[tauri::command]
async fn artist_links(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<ArtistLink>> {
    with_lib(&state, move |lib| lib.artist_links(id, 15)).await
}

#[tauri::command]
async fn compare(state: State<'_, AppState>, a: Period, b: Period) -> CmdResult<Comparison> {
    with_lib(&state, move |lib| lib.compare(a, b, 15)).await
}

#[tauri::command]
async fn browse(state: State<'_, AppState>, kind: Kind, sort: BrowseSort, desc: bool, filter: String, offset: usize, limit: usize) -> CmdResult<BrowsePage> {
    with_lib(&state, move |lib| lib.browse(kind, sort, desc, &filter, offset, limit.min(500))).await
}

#[tauri::command]
async fn discoveries(state: State<'_, AppState>, period: Period) -> CmdResult<Discoveries> {
    with_lib(&state, move |lib| lib.discoveries(period)).await
}

#[tauri::command]
async fn sessions(state: State<'_, AppState>, period: Period) -> CmdResult<SessionStats> {
    with_lib(&state, move |lib| lib.sessions(period)).await
}

#[tauri::command]
async fn albums_by_release(state: State<'_, AppState>, from_year: i64, to_year: i64, period: Period) -> CmdResult<Vec<AlbumRelease>> {
    with_lib(&state, move |lib| lib.albums_by_release(from_year, to_year, period)).await
}

#[tauri::command]
async fn accounts(state: State<'_, AppState>) -> CmdResult<Vec<Account>> {
    with_lib(&state, |lib| lib.accounts()).await
}

/// Vérifie le compte auprès du service, l'ajoute, puis lance sa synchronisation.
#[tauri::command]
async fn add_account(app: AppHandle, state: State<'_, AppState>, service: Service, username: String, api_key: Option<String>) -> CmdResult<Account> {
    let lib = state.lib.clone();
    let acc = tauri::async_runtime::spawn_blocking(move || accounts::add_account(&lib, service, &username, api_key.as_deref()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    start_sync(app, state, None);
    Ok(acc)
}

#[tauri::command]
async fn remove_account(app: AppHandle, state: State<'_, AppState>, id: i64) -> CmdResult<usize> {
    // La synchro en cours (tous les comptes) est arrêtée, puis relancée pour les autres.
    let was_running = state.sync_running.load(Ordering::SeqCst);
    state.sync_cancel.store(true, Ordering::SeqCst);
    let removed = with_lib(&state, move |lib| lib.remove_account(id)).await?;
    if was_running {
        let running = state.sync_running.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while running.load(Ordering::SeqCst) && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .await
        .map_err(|e| e.to_string())?;
        let others = state.lib.lock().unwrap_or_else(|e| e.into_inner()).accounts().map(|a| !a.is_empty()).unwrap_or(false);
        if others {
            spawn_sync(app, state.lib.clone(), state.sync_running.clone(), state.sync_cancel.clone(), state.sync_status.clone(), None);
        }
    }
    Ok(removed)
}

#[tauri::command]
fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    let mut st = state.sync_status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    st.running = state.sync_running.load(Ordering::Relaxed);
    st
}

/// Synchronise un compte, ou tous (aussi lancé au démarrage).
#[tauri::command]
fn start_sync(app: AppHandle, state: State<'_, AppState>, id: Option<i64>) {
    // Pas pendant une restauration : la tâche écrirait dans la base remplacée.
    if state.restoring.load(Ordering::SeqCst) {
        return;
    }
    spawn_sync(app, state.lib.clone(), state.sync_running.clone(), state.sync_cancel.clone(), state.sync_status.clone(), id);
}

#[tauri::command]
fn stop_sync(state: State<'_, AppState>) {
    state.sync_cancel.store(true, Ordering::SeqCst);
}

/// Arrêt voulu : la tâche ne reprendra pas au prochain lancement. Écrit dans un
/// fil à part, pour ne pas bloquer l'interface si la base est occupée.
fn forget_jobs(lib: &Arc<Mutex<Library>>, names: &'static [&'static str]) {
    let lib = lib.clone();
    std::thread::spawn(move || {
        if let Ok(l) = lib.lock() {
            for n in names {
                let _ = l.set_job_wanted(n, false);
            }
        }
    });
}

/// Lance une tâche de fond (une seule à la fois par type). Elle est notée
/// voulue jusqu'à ce qu'elle se termine ou que l'utilisateur l'arrête : fermer
/// l'application en cours de route la fait reprendre au lancement suivant.
fn spawn_job<S, R>(app: AppHandle, lib: Arc<Mutex<Library>>, running: Arc<AtomicBool>, cancel: Arc<AtomicBool>, name: &'static str, run: R, finish: fn(&mut S))
where
    S: serde::Serialize + Clone + Send + 'static,
    R: FnOnce(&Mutex<Library>, Arc<AtomicBool>, &mut dyn FnMut(S)) -> librestfm_core::Result<S> + Send + 'static,
{
    if running.swap(true, Ordering::SeqCst) {
        return; // déjà en cours
    }
    cancel.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        // Dans le fil de la tâche : la commande rend la main sans attendre le verrou.
        let _ = lib.lock().map(|l| l.set_job_wanted(name, true));
        let result = run(&lib, cancel.clone(), &mut |st: S| {
            let _ = app.emit(&format!("{name}-progress"), &st);
        });
        running.store(false, Ordering::SeqCst);
        match result {
            Ok(mut st) => {
                if !cancel.load(Ordering::SeqCst) {
                    let _ = lib.lock().map(|l| l.set_job_wanted(name, false));
                }
                finish(&mut st);
                let _ = app.emit(&format!("{name}-progress"), &st);
            }
            // Erreur (réseau coupé…) : la tâche reste voulue et reprendra au prochain lancement.
            Err(e) => {
                let _ = app.emit(&format!("{name}-error"), e.to_string());
            }
        }
    });
}

fn spawn_sync(app: AppHandle, lib: Arc<Mutex<Library>>, running: Arc<AtomicBool>, cancel: Arc<AtomicBool>, status: Arc<Mutex<SyncStatus>>, id: Option<i64>) {
    if running.swap(true, Ordering::SeqCst) {
        return;
    }
    cancel.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let result = accounts::run(&lib, id, cancel, |st| {
            *status.lock().unwrap_or_else(|e| e.into_inner()) = st.clone();
            let _ = app.emit("sync-progress", st);
        });
        running.store(false, Ordering::SeqCst);
        let mut st = status.lock().unwrap_or_else(|e| e.into_inner()).clone();
        st.running = false;
        let _ = app.emit("sync-progress", &st);
        if let Err(e) = result {
            let _ = app.emit("sync-error", e.to_string());
        }
    });
}

#[tauri::command]
async fn mixes(state: State<'_, AppState>, size: Option<usize>) -> CmdResult<Vec<Mix>> {
    with_lib(&state, move |lib| lib.mixes(size.unwrap_or(mixes::MIX_SIZE))).await
}

#[tauri::command]
async fn mix_at(state: State<'_, AppState>, key: String, size: usize) -> CmdResult<Option<Mix>> {
    with_lib(&state, move |lib| lib.mix(&key, size)).await
}

#[tauri::command]
async fn seeded_mix(state: State<'_, AppState>, kind: SeedKind, id: i64, variant: u64, size: Option<usize>) -> CmdResult<Option<Mix>> {
    with_lib(&state, move |lib| lib.seeded_mix(kind, id, variant, size.unwrap_or(mixes::MIX_SIZE))).await
}

/// Retire un titre d'un mix ; `mix_key = "*"` : ne plus jamais le proposer.
#[tauri::command]
async fn mix_feedback(state: State<'_, AppState>, mix_key: String, track_id: i64) -> CmdResult<()> {
    with_lib(&state, move |lib| lib.mix_feedback(&mix_key, track_id)).await
}

#[tauri::command]
async fn clear_mix_feedback(state: State<'_, AppState>) -> CmdResult<usize> {
    with_lib(&state, |lib| lib.clear_mix_feedback()).await
}

/// Exporte l'historique des sources choisies (toutes si vide), fusionné, dans le fichier choisi.
#[tauri::command]
async fn export_history(state: State<'_, AppState>, sources: Vec<i64>, corrected: bool, format: librestfm_core::export::HistoryFormat, path: PathBuf) -> CmdResult<usize> {
    with_lib(&state, move |lib| {
        lib.check_destination(&path)?;
        lib.export_history(&sources, corrected, format, std::fs::File::create(&path)?)
    })
    .await
}

/// Écrit un mix dans le fichier choisi par l'utilisateur (xspf, jspf ou csv).
#[tauri::command]
async fn export_mix(state: State<'_, AppState>, mix: Mix, title: String, format: String, path: PathBuf) -> CmdResult<()> {
    if librestfm_core::backup::is_database_file(&path, &state.db_path) {
        return Err(err(librestfm_core::Error::Invalid("own_database")));
    }
    let text = match format.as_str() {
        "xspf" => mixes::to_xspf(&title, &mix),
        "jspf" => mixes::to_jspf(&title, &mix),
        "csv" => mixes::to_csv(&mix),
        _ => return Err(format!("format inconnu : {format}")),
    };
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Informations d'un album ou d'un artiste : récupérées maintenant si jamais cherchées.
/// Renvoie true si quelque chose a été récupéré (la fiche est alors à recharger).
#[tauri::command]
async fn ensure_info(state: State<'_, AppState>, kind: Kind, id: i64) -> CmdResult<bool> {
    let lib = state.lib.clone();
    let Some(guard) = Fetching::start(&state) else { return Ok(false) };
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let lock = || lib.lock().unwrap_or_else(|e| e.into_inner());
        let mut fetched = false;
        let (album, artist): (Option<i64>, i64) = match kind {
            Kind::Artist => (None, id),
            Kind::Album => (Some(id), lock().conn.query_row("SELECT artist_id FROM album WHERE id = ?", [id], |r| r.get(0)).map_err(|e| e.to_string())?),
            Kind::Track => return Ok(false),
        };
        if let Some(a) = album
            && lock().album_info(a).map_err(|e| e.to_string())?.is_none()
        {
            let _ = info::fetch_album(&lib, a, None);
            fetched = true;
        }
        if lock().artist_info(artist).map_err(|e| e.to_string())?.is_none() {
            let _ = info::fetch_artist(&lib, artist, None);
            fetched = true;
        }
        Ok(fetched)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn info_status(state: State<'_, AppState>) -> CmdResult<CreditsStatus> {
    let running = state.info_running.load(Ordering::Relaxed);
    let mut st = with_lib(&state, |lib| lib.info_status()).await?;
    st.running = running;
    Ok(st)
}

#[tauri::command]
fn start_info(app: AppHandle, state: State<'_, AppState>) {
    // Pas pendant une restauration : la tâche écrirait dans la base remplacée.
    if state.restoring.load(Ordering::SeqCst) {
        return;
    }
    spawn_info(app, state.lib.clone(), state.info_running.clone(), state.info_cancel.clone());
}

fn spawn_info(app: AppHandle, lib: Arc<Mutex<Library>>, running: Arc<AtomicBool>, cancel: Arc<AtomicBool>) {
    spawn_job(app, lib, running, cancel, "info", |l, c, p| info::run(l, c, p), |s| s.running = false);
}

#[tauri::command]
fn stop_info(state: State<'_, AppState>) {
    state.info_cancel.store(true, Ordering::SeqCst);
    forget_jobs(&state.lib, &["info"]);
}

#[tauri::command]
async fn retry_info_failures(state: State<'_, AppState>) -> CmdResult<usize> {
    with_lib(&state, |lib| lib.reset_info_failures()).await
}

#[tauri::command]
async fn enrich_status(state: State<'_, AppState>) -> CmdResult<EnrichStatus> {
    let running = state.enrich_running.load(Ordering::Relaxed);
    let mut st = with_lib(&state, |lib| lib.enrich_status()).await?;
    st.running = running;
    Ok(st)
}

#[tauri::command]
fn start_enrich(app: AppHandle, state: State<'_, AppState>) {
    // Pas pendant une restauration : la tâche écrirait dans la base remplacée.
    if state.restoring.load(Ordering::SeqCst) {
        return;
    }
    spawn_enrich(app, state.lib.clone(), state.enrich_running.clone(), state.enrich_cancel.clone());
}

fn spawn_enrich(app: AppHandle, lib: Arc<Mutex<Library>>, running: Arc<AtomicBool>, cancel: Arc<AtomicBool>) {
    spawn_job(app, lib, running, cancel, "enrich", |l, c, p| enrich::run(l, c, p), |s| s.running = false);
}

#[tauri::command]
fn stop_enrich(state: State<'_, AppState>) {
    state.enrich_cancel.store(true, Ordering::SeqCst);
    forget_jobs(&state.lib, &["enrich"]);
}

#[tauri::command]
async fn retry_enrich_failures(state: State<'_, AppState>) -> CmdResult<usize> {
    with_lib(&state, |lib| lib.reset_enrich_failures()).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init()).plugin(tauri_plugin_dialog::init());
    // Android : scrobbleur (écoutes des autres applis enregistrées dans un journal).
    #[cfg(target_os = "android")]
    let builder = builder.plugin(scrobbler::init());
    builder
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let lib = Arc::new(Mutex::new(Library::open(&dir.join("librestfm.sqlite"))?));
            // Lu avant de lancer la moindre tâche : rien n'attend le verrou de la base.
            let (has_accounts, wanted, discover_due) = {
                let l = lib.lock().unwrap_or_else(|e| e.into_inner());
                (l.accounts().map(|a| !a.is_empty()).unwrap_or(false), l.wanted_jobs().unwrap_or_default(), l.discover_due().unwrap_or(false))
            };
            // Dossier d'import (~/Musique/LibrestFM par défaut) lu, puis corrections sûres
            // (règles ajoutées depuis le dernier lancement), sans retarder l'ouverture.
            let default_input = app.path().audio_dir().or_else(|_| app.path().home_dir()).map(|d| d.join("LibrestFM")).unwrap_or_else(|_| dir.join("input"));
            let backups = dir.join("backups");
            let (lib_bg, input_bg, handle, backups_bg) = (lib.clone(), default_input.clone(), app.handle().clone(), backups.clone());
            std::thread::spawn(move || {
                let Ok(mut l) = lib_bg.lock() else { return };
                let dir = input_dir_of(&l, &input_bg);
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(rep) = l.scan_input(&dir)
                    && !rep.files.is_empty()
                {
                    let _ = handle.emit("inbox-done", &rep);
                }
                #[cfg(target_os = "android")]
                if let Some(journal) = scrobbler::journal_dir(&handle) {
                    scrobbler::configure(&handle);
                    let _ = l.scan_input(&journal);
                }
                let _ = l.auto_clean();
                // Caches inutiles vidés, place rendue (copie compacte, quelques dizaines de ms).
                let _ = l.purge_caches();
                let _ = l.compact_if_worth();
                // Sauvegarde automatique : une par semaine, les deux dernières gardées.
                let _ = l.auto_backup(&backups_bg, 7, 2);
            });
            // Comptes en ligne : nouvelles écoutes récupérées à chaque lancement.
            let (sync_running, sync_cancel, sync_status) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(SyncStatus::default())));
            if has_accounts {
                spawn_sync(app.handle().clone(), lib.clone(), sync_running.clone(), sync_cancel.clone(), sync_status.clone(), None);
            }
            // Tâches d'enrichissement interrompues par la fermeture : reprises.
            let flag = || Arc::new(AtomicBool::new(false));
            let (enrich_running, enrich_cancel, credits_running, credits_cancel, info_running, info_cancel) = (flag(), flag(), flag(), flag(), flag(), flag());
            let (discover_running, discover_cancel) = (flag(), flag());
            // Découvertes : nouvelle sélection chaque semaine, une fois la première demandée.
            if discover_due {
                spawn_discover(app.handle().clone(), lib.clone(), discover_running.clone(), discover_cancel.clone(), Scope::Weekly);
            }
            for job in wanted {
                let h = app.handle().clone();
                match job.as_str() {
                    "enrich" => spawn_enrich(h, lib.clone(), enrich_running.clone(), enrich_cancel.clone()),
                    "credits" => spawn_credits(h, lib.clone(), credits_running.clone(), credits_cancel.clone()),
                    "info" => spawn_info(h, lib.clone(), info_running.clone(), info_cancel.clone()),
                    "discover" => spawn_discover(h, lib.clone(), discover_running.clone(), discover_cancel.clone(), Scope::Weekly),
                    _ => {}
                }
            }
            app.manage(AppState {
                lib,
                default_input,
                db_path: dir.join("librestfm.sqlite"),
                backups,
                enrich_running,
                enrich_cancel,
                credits_running,
                credits_cancel,
                info_running,
                info_cancel,
                discover_running: discover_running.clone(),
                discover_cancel: discover_cancel.clone(),
                sync_running,
                sync_cancel,
                sync_status,
                fetching: Arc::new(AtomicUsize::new(0)),
                restoring: Arc::new(AtomicBool::new(false)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            lyrics,
            rename_entity,
            double_sends,
            restore_double_sends,
            find_track,
            scrobbler::scrobbler_status,
            scrobbler::scrobbler_set_app,
            scrobbler::scrobbler_open,
            scrobbler::scrobbler_scan,
            scrobbler::scrobbler_call,
            import_files,
            discover_selection,
            start_discover,
            stop_discover,
            discover_running,
            discover_verdict,
            discover_preview,
            backup_db,
            check_backup,
            restore_db,
            backups,
            input_dir,
            set_input_dir,
            scan_input,
            suggestions,
            apply_suggestions,
            dismiss_suggestions,
            clear_dismissed,
            dismissed_count,
            merge_entities,
            undo_merge,
            merge_ops,
            list_sources,
            remove_source,
            overview,
            entity,
            search,
            cached_images,
            image,
            sessions,
            discoveries,
            browse,
            compare,
            artist_links,
            artist_credits,
            detail,
            albums_by_release,
            mixes,
            seeded_mix,
            mix_at,
            accounts,
            add_account,
            remove_account,
            sync_status,
            start_sync,
            stop_sync,
            mix_feedback,
            clear_mix_feedback,
            export_mix,
            export_history,
            ensure_info,
            info_status,
            start_info,
            stop_info,
            retry_info_failures,
            track_credits,
            album_credits,
            person,
            credits_status,
            start_credits,
            stop_credits,
            retry_credit_failures,
            enrich_status,
            start_enrich,
            stop_enrich,
            retry_enrich_failures
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de LibrestFM");
}
