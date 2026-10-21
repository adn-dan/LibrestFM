//! Scrobbleur Android : le service Kotlin (ScrobbleService) ajoute chaque écoute
//! détectée au journal `scrobbles/scrobbles.csv` du dossier de l'app. Ce journal
//! est lu comme un fichier du dossier d'import (seules les nouvelles lignes entrent).

use crate::{with_lib, AppState, CmdResult};
use librestfm_core::inbox::InboxReport;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime, State};

#[cfg(target_os = "android")]
struct Handle<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("scrobbler")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            _app.manage(Handle(_api.register_android_plugin("io.github.adn_dan.librestfm", "ScrobblerPlugin")?));
            Ok(())
        })
        .build()
}

/// Dossier du journal.
pub fn journal_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("scrobbles"))
}

fn call<R: Runtime>(app: &AppHandle<R>, cmd: &str, payload: Value) -> Result<Value, String> {
    #[cfg(target_os = "android")]
    {
        let h = app.try_state::<Handle<R>>().ok_or("scrobbler")?;
        h.0.run_mobile_plugin(cmd, payload).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, cmd, payload);
        Err("android".into())
    }
}

/// Indique au service où écrire le journal.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn configure<R: Runtime>(app: &AppHandle<R>) {
    if let Some(dir) = journal_dir(app) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = call(app, "configure", json!({ "dir": dir.display().to_string() }));
    }
}

#[tauri::command]
pub async fn scrobbler_status(app: AppHandle) -> Result<Value, String> {
    call(&app, "status", json!({}))
}

#[tauri::command]
pub async fn scrobbler_set_app(app: AppHandle, pkg: String, enabled: bool) -> Result<Value, String> {
    call(&app, "setApp", json!({ "pkg": pkg, "enabled": enabled }))
}

/// `what` : "access" (accès aux notifications) ou "app" (fiche de l'app).
#[tauri::command]
pub async fn scrobbler_open(app: AppHandle, what: String) -> Result<Value, String> {
    call(&app, if what == "app" { "openAppInfo" } else { "openAccess" }, json!({}))
}

/// Autres commandes du service (comptes, enregistrement local), transmises telles quelles.
#[tauri::command]
pub async fn scrobbler_call(app: AppHandle, cmd: String, args: Value) -> Result<Value, String> {
    const ALLOWED: [&str; 15] = ["mediaControl", "keepAwake", "localCopy", "tempPath", "copyToUri", "nowPlaying", "takeOpen", "setNotifs", "progress", "setBars", "setLocal", "addDestination", "setDestination", "removeDestination", "flush"];
    if !ALLOWED.contains(&cmd.as_str()) {
        return Err(format!("unknown command {cmd}"));
    }
    // Connexion à un compte : appel réseau, pas sur un fil de l'exécuteur asynchrone.
    tauri::async_runtime::spawn_blocking(move || call(&app, &cmd, args)).await.map_err(|e| e.to_string())?
}

/// Nouvelles lignes du journal importées.
#[tauri::command]
pub async fn scrobbler_scan(app: AppHandle, state: State<'_, AppState>) -> CmdResult<InboxReport> {
    let dir = journal_dir(&app).unwrap_or_default();
    with_lib(&state, move |lib| lib.scan_input(&dir)).await
}
