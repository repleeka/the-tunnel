//! Menu bar (macOS) / system tray (Windows) icon: shows status and keeps
//! The Tunnel reachable while its window is closed.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tunnel_core::{Engine, PeerSyncState, SettingsPatch, Snapshot, TransferStatus};

struct TrayItems {
    status: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    shown: Mutex<(String, bool)>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open The Tunnel", true, None::<&str>)?;
    let sync_now = MenuItem::with_id(app, "sync-now", "Sync Now", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause Sync", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit The Tunnel", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&status, &PredefinedMenuItem::separator(app)?, &open, &sync_now, &pause, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    // macOS menu bar icons are monochrome "template" images that adapt to light/dark.
    #[cfg(target_os = "macos")]
    let (icon, template) = (Image::from_bytes(include_bytes!("../icons/tray-template.png"))?, true);
    #[cfg(not(target_os = "macos"))]
    let (icon, template) = (Image::from_bytes(include_bytes!("../icons/32x32.png"))?, false);

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(template)
        .tooltip("The Tunnel")
        .menu(&menu)
        // macOS: click opens the menu. Windows: left click opens the window, right click the menu.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::show_main(app),
            "sync-now" => {
                let engine = app.state::<Engine>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = engine.sync_now().await;
                });
            }
            "pause" => {
                let engine = app.state::<Engine>();
                let enabled = engine.settings().sync_enabled;
                let _ = engine.update_settings(SettingsPatch { sync_enabled: Some(!enabled), ..Default::default() });
                refresh(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if cfg!(not(target_os = "macos"))
                && let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    crate::show_main(tray.app_handle());
                }
        })
        .build(app)?;

    app.manage(TrayItems { status, pause, shown: Mutex::new((String::new(), true)) });
    refresh(app);
    Ok(())
}

/// Update the status line, tooltip and pause item from the engine's current state.
pub fn refresh(app: &AppHandle) {
    let (Some(items), Some(engine)) = (app.try_state::<TrayItems>(), app.try_state::<Engine>()) else { return };
    let snap = engine.snapshot();
    let line = status_line(&snap);
    let sync_on = snap.settings.sync_enabled;
    let mut shown = items.shown.lock().unwrap();
    if *shown == (line.clone(), sync_on) {
        return;
    }
    let _ = items.status.set_text(&line);
    let _ = items.pause.set_text(if sync_on { "Pause Sync" } else { "Resume Sync" });
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(format!("The Tunnel — {line}")));
    }
    *shown = (line, sync_on);
}

fn status_line(s: &Snapshot) -> String {
    let review = s.sync.pending_deletions.len();
    if review > 0 {
        return format!("{review} deletion{} to review", if review == 1 { "" } else { "s" });
    }
    let active = s.transfers.iter().filter(|t| t.status == TransferStatus::Active).count();
    if active > 0 {
        return format!("Transferring {active} item{}…", if active == 1 { "" } else { "s" });
    }
    if s.sync.enabled && s.sync.peers.iter().any(|p| p.state == PeerSyncState::Syncing && p.files_left > 0) {
        return "Syncing…".into();
    }
    let paired: Vec<_> = s.devices.iter().filter(|d| d.paired).collect();
    match (paired.iter().find(|d| d.online), paired.first()) {
        (Some(d), _) if s.sync.enabled => format!("Connected to {} · in sync", d.device_name),
        (Some(d), _) => format!("Connected to {} · sync paused", d.device_name),
        (None, Some(d)) => format!("Waiting for {}", d.device_name),
        (None, None) => "No paired devices".into(),
    }
}
