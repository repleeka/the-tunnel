//! Desktop shell: starts the engine, exposes it to the UI as commands,
//! and forwards engine events to the window.

use std::path::PathBuf;

use tauri::{Emitter, Manager, State, WebviewWindow};
use tokio::sync::broadcast::error::RecvError;
use tunnel_core::{DeviceView, Engine, EngineConfig, Listing, Settings, SettingsPatch, Snapshot};

type CmdResult<T> = Result<T, String>;

fn msg(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
fn get_state(engine: State<Engine>) -> Snapshot {
    engine.snapshot()
}

#[tauri::command]
fn update_settings(engine: State<Engine>, patch: SettingsPatch) -> CmdResult<Settings> {
    engine.update_settings(patch).map_err(msg)
}

#[tauri::command]
async fn pair_device(engine: State<'_, Engine>, device_id: String) -> CmdResult<String> {
    Ok(engine.pair(&device_id))
}

#[tauri::command]
fn respond_pairing(engine: State<Engine>, session_id: String, accept: bool) {
    engine.respond_pairing(&session_id, accept);
}

#[tauri::command]
fn unpair_device(engine: State<Engine>, device_id: String) {
    engine.unpair(&device_id);
}

#[tauri::command]
async fn connect_address(engine: State<'_, Engine>, address: String) -> CmdResult<DeviceView> {
    engine.connect_address(&address).await.map_err(msg)
}

#[tauri::command]
async fn send_paths(engine: State<'_, Engine>, device_id: String, paths: Vec<String>) -> CmdResult<String> {
    let paths = paths.into_iter().map(PathBuf::from).collect();
    engine.send_paths(&device_id, paths).await.map_err(msg)
}

#[tauri::command]
fn respond_offer(engine: State<Engine>, transfer_id: String, accept: bool) {
    engine.respond_offer(&transfer_id, accept);
}

#[tauri::command]
async fn list_remote(engine: State<'_, Engine>, device_id: String, path: String) -> CmdResult<Listing> {
    engine.list_remote(&device_id, &path).await.map_err(msg)
}

#[tauri::command]
async fn pull_remote(engine: State<'_, Engine>, device_id: String, paths: Vec<String>) -> CmdResult<String> {
    engine.pull(&device_id, paths).await.map_err(msg)
}

#[tauri::command]
async fn resolve_deletions(engine: State<'_, Engine>, device_id: String, paths: Vec<String>, delete: bool) -> CmdResult<()> {
    engine.resolve_deletions(&device_id, paths, delete).await.map_err(msg)
}

#[tauri::command]
async fn sync_now(engine: State<'_, Engine>) -> CmdResult<()> {
    engine.sync_now().await.map_err(msg)
}

#[tauri::command]
fn clear_transfers(engine: State<Engine>) {
    engine.clear_finished_transfers();
}

/// Whether the native window material (macOS vibrancy, Windows 11 Mica) is active,
/// so the UI knows whether to let it show through.
struct NativeLook {
    effects: bool,
}

#[tauri::command]
fn native_effects(look: State<NativeLook>) -> bool {
    look.effects
}

fn apply_native_effects(window: &WebviewWindow) -> bool {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy};
        return apply_vibrancy(window, NSVisualEffectMaterial::Sidebar, Some(NSVisualEffectState::FollowsWindowActiveState), None).is_ok();
    }
    #[cfg(target_os = "windows")]
    {
        // Mica needs Windows 11; on Windows 10 this fails and the UI stays opaque.
        return window_vibrancy::apply_mica(window, None).is_ok();
    }
    #[allow(unreachable_code)]
    {
        let _ = window;
        false
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // TUNNEL_DATA_DIR / TUNNEL_PORT let two copies run side by side during development.
            let data_dir = match std::env::var_os("TUNNEL_DATA_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            let port = std::env::var("TUNNEL_PORT").ok().and_then(|p| p.parse().ok());
            let engine = tauri::async_runtime::block_on(Engine::start(EngineConfig { data_dir, port, discovery: true, use_trash: true }))?;

            let mut events = engine.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let _ = handle.emit("tunnel", event);
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            app.manage(engine);

            let effects = app.get_webview_window("main").is_some_and(|w| apply_native_effects(&w));
            app.manage(NativeLook { effects });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            update_settings,
            pair_device,
            respond_pairing,
            unpair_device,
            connect_address,
            send_paths,
            respond_offer,
            list_remote,
            pull_remote,
            clear_transfers,
            resolve_deletions,
            sync_now,
            native_effects,
        ])
        .run(tauri::generate_context!())
        .expect("error while running The Tunnel");
}
