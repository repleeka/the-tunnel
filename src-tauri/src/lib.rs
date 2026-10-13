//! Desktop shell: starts the engine, exposes it to the UI as commands, forwards
//! engine events to the window, and integrates with the OS (tray, start at login,
//! notifications, right-click "Send with The Tunnel").

mod context_menu;
mod tray;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;
use tunnel_core::{
    DeviceView, Direction, Engine, EngineConfig, Event, Listing, Settings, SettingsPatch, Snapshot, TransferStatus,
};

type CmdResult<T> = Result<T, String>;

/// Passed by the login item so the app starts quietly in the tray.
const HIDDEN_ARG: &str = "--hidden";
/// Passed by the right-click menu, followed by the paths to send.
const SEND_ARG: &str = "--send";

fn msg(e: anyhow::Error) -> String {
    format!("{e:#}")
}

// ---------- Commands ----------

#[tauri::command]
fn get_state(engine: State<Engine>) -> Snapshot {
    engine.snapshot()
}

#[tauri::command]
fn update_settings(app: AppHandle, engine: State<Engine>, patch: SettingsPatch) -> CmdResult<Settings> {
    let settings = engine.update_settings(patch).map_err(msg)?;
    apply_os_settings(&app, &settings);
    tray::refresh(&app);
    Ok(settings)
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
fn cancel_transfer(engine: State<Engine>, transfer_id: String) {
    engine.cancel_transfer(&transfer_id);
}

#[tauri::command]
async fn retry_transfer(engine: State<'_, Engine>, transfer_id: String) -> CmdResult<()> {
    engine.retry_transfer(&transfer_id).await.map_err(msg)
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
fn clear_transfers(engine: State<Engine>) {
    engine.clear_finished_transfers();
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
fn native_effects(look: State<NativeLook>) -> bool {
    look.effects
}

/// Paths from the right-click menu that are waiting for the user to pick a device.
#[tauri::command]
fn take_send_request(queue: State<SendQueue>) -> Vec<String> {
    std::mem::take(&mut *queue.waiting.lock().unwrap())
}

// ---------- Window ----------

/// Whether the native window material (macOS vibrancy, Windows 11 Mica) is active,
/// so the UI knows whether to let it show through.
struct NativeLook {
    effects: bool,
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

pub(crate) fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn window_in_front(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

// ---------- OS integration ----------

/// Keep the login item and the right-click menu in line with the settings.
/// Skipped in development builds so `tauri dev` doesn't register itself.
fn apply_os_settings(app: &AppHandle, s: &Settings) {
    if cfg!(debug_assertions) {
        return;
    }
    let autostart = app.autolaunch();
    let result = if s.launch_at_login { autostart.enable() } else { autostart.disable() };
    if let Err(e) = result {
        eprintln!("couldn't update the login item: {e}");
    }
    if let Err(e) = context_menu::set_enabled(s.context_menu) {
        eprintln!("couldn't update the right-click menu: {e}");
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if app.state::<Engine>().settings().notifications {
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

/// Turn engine events into system notifications when the window isn't in front.
fn notify_for(app: &AppHandle, event: &Event, reviews_seen: &mut usize) {
    match event {
        Event::PairingCode { device, incoming: true, .. } => {
            show_main(app);
            notify(app, "Pairing request", &format!("{} wants to pair. Check that the codes match on both screens.", device.device_name));
        }
        Event::IncomingOffer { device_name, label, .. } => {
            show_main(app);
            notify(app, &format!("{device_name} wants to send you files"), label);
        }
        Event::TransferChanged { transfer: t } if !window_in_front(app) => match (t.status, t.direction) {
            (TransferStatus::Done, Direction::Receive) => notify(app, &format!("Received from {}", t.device_name), &t.label),
            (TransferStatus::Done, Direction::Send) => notify(app, &format!("Sent to {}", t.device_name), &t.label),
            (TransferStatus::Failed, _) => notify(app, &format!("Couldn't transfer {}", t.label), t.error.as_deref().unwrap_or("The transfer failed.")),
            _ => {}
        },
        Event::SyncChanged => {
            let pending = app.state::<Engine>().snapshot().sync.pending_deletions;
            if pending.len() > *reviews_seen && !window_in_front(app) {
                let from = pending.first().map(|p| p.device_name.clone()).unwrap_or_default();
                let n = pending.len();
                notify(
                    app,
                    "Deletions to review",
                    &format!("{n} file{} deleted on {from}. Nothing has been deleted here yet. Open The Tunnel to review.", if n == 1 { " was" } else { "s were" }),
                );
            }
            *reviews_seen = pending.len();
        }
        _ => {}
    }
}

// ---------- "Send with The Tunnel" ----------

#[derive(Default)]
struct SendQueue {
    incoming: Mutex<Vec<String>>,
    last_added: Mutex<Option<Instant>>,
    /// Waiting for the user to choose a device in the window.
    waiting: Mutex<Vec<String>>,
}

fn send_args(args: &[String]) -> Option<Vec<String>> {
    let at = args.iter().position(|a| a == SEND_ARG)?;
    Some(args[at + 1..].iter().filter(|a| !a.starts_with("--")).cloned().collect())
}

/// Windows starts one process per selected file, so collect paths for a moment
/// and send them together.
fn queue_send(app: &AppHandle, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let queue = app.state::<SendQueue>();
    queue.incoming.lock().unwrap().extend(paths);
    *queue.last_added.lock().unwrap() = Some(Instant::now());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        let queue = app.state::<SendQueue>();
        if queue.last_added.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(550)) {
            return; // more paths arrived; their timer will send everything
        }
        let paths = std::mem::take(&mut *queue.incoming.lock().unwrap());
        if !paths.is_empty() {
            dispatch_send(&app, paths).await;
        }
    });
}

/// Send straight to the only connected device, or let the user pick one.
async fn dispatch_send(app: &AppHandle, paths: Vec<String>) {
    let engine = app.state::<Engine>().inner().clone();
    // Just launched by the menu? Give discovery a moment to find the other device.
    let mut online = Vec::new();
    for _ in 0..16 {
        online = engine.snapshot().devices.into_iter().filter(|d| d.paired && d.online).collect::<Vec<_>>();
        if !online.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if let [device] = online.as_slice() {
        let count = paths.len();
        match engine.send_paths(&device.id, paths.into_iter().map(PathBuf::from).collect()).await {
            Ok(_) => notify(app, &format!("Sending to {}", device.device_name), &format!("{count} item{}", if count == 1 { "" } else { "s" })),
            Err(e) => {
                show_main(app);
                notify(app, "Couldn't send", &msg(e));
            }
        }
        return;
    }
    app.state::<SendQueue>().waiting.lock().unwrap().extend(paths.clone());
    show_main(app);
    let _ = app.emit("send-request", paths);
}

// ---------- Startup ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Must be first: a second launch (e.g. from the right-click menu) hands its
        // arguments to the running copy and exits.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| match send_args(&args) {
            Some(paths) => queue_send(app, paths),
            None => show_main(app),
        }))
        .plugin(autostart_plugin())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(SendQueue::default())
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
                let mut reviews_seen = 0;
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            notify_for(&handle, &event, &mut reviews_seen);
                            if !matches!(event, Event::PairingCode { .. } | Event::PairingFinished { .. } | Event::IncomingOffer { .. }) {
                                tray::refresh(&handle);
                            }
                            let _ = handle.emit("tunnel", event);
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            let settings = engine.settings();
            app.manage(engine);
            apply_os_settings(app.handle(), &settings);
            tray::create(app.handle())?;

            let window = app.get_webview_window("main");
            let effects = window.as_ref().is_some_and(apply_native_effects);
            app.manage(NativeLook { effects });

            let args: Vec<String> = std::env::args().collect();
            let quiet = args.iter().any(|a| a == HIDDEN_ARG) || send_args(&args).is_some();
            if !quiet {
                show_main(app.handle());
            }
            if let Some(paths) = send_args(&args) {
                queue_send(app.handle(), paths);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps The Tunnel running in the menu bar / tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                let app = window.app_handle();
                let engine = app.state::<Engine>();
                if !engine.settings().tray_hint_shown {
                    let place = if cfg!(target_os = "macos") { "menu bar" } else { "system tray" };
                    notify(app, "The Tunnel is still running", &format!("It keeps your devices in sync from the {place}. Quit from the {place} icon."));
                    let _ = engine.update_settings(SettingsPatch { tray_hint_shown: Some(true), ..Default::default() });
                }
            }
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
            cancel_transfer,
            retry_transfer,
            list_remote,
            pull_remote,
            clear_transfers,
            resolve_deletions,
            sync_now,
            native_effects,
            take_send_request,
        ])
        .build(tauri::generate_context!())
        .expect("error while building The Tunnel");

    app.run(|app, event| {
        // macOS: clicking the Dock icon brings the window back.
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            show_main(app);
        }
        let _ = (app, event);
    });
}

fn autostart_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let builder = tauri_plugin_autostart::Builder::new().args([HIDDEN_ARG]);
    #[cfg(target_os = "macos")]
    let builder = builder.macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent);
    builder.build()
}
