//! System tray: keeps the app reachable while the main window is hidden.
//!
//! Left-click opens this menu ("panel"); double-click reopens the window.
//! Tunnel start/stop reuse the same [`ConnectionManager`] the UI drives, so
//! status events stay consistent across both surfaces.

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Manager,
};

use crate::{
    commands,
    connection::manager::ConnectionManager as TailscaleManager,
    AppState,
};

/// Handles of menu items whose labels change at runtime.
#[derive(Default)]
pub struct TrayMenuItems {
    /// Managed behind a Mutex because `State` gives shared access only.
    pub toggle_pause: Mutex<Option<MenuItem<tauri::Wry>>>,
}

/// Builds the tray icon + menu. Call once from setup.
pub fn build(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let start_tunnel =
        MenuItem::with_id(app, "start_tunnel", "Start tunnel", true, None::<&str>)?;
    let stop_tunnel = MenuItem::with_id(app, "stop_tunnel", "Stop tunnel", true, None::<&str>)?;
    let paused = app.state::<AppState>().core.pause.is_paused();
    let toggle_pause = MenuItem::with_id(
        app,
        "toggle_pause",
        if paused {
            "Resume MCP access"
        } else {
            "Pause MCP access"
        },
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &start_tunnel,
            &stop_tunnel,
            &toggle_pause,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("app has no default window icon")?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Local Workspace MCP Manager")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_tray_icon_event(|tray, event| {
            // Double-click restores the window; single left-click opens this
            // menu via show_menu_on_left_click.
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .on_menu_event(handle_menu)
        .build(app)?;

    app.manage(TrayMenuItems {
        toggle_pause: Mutex::new(Some(toggle_pause)),
    });
    Ok(())
}

/// Shows and focuses the main window; used by tray open and by the
/// single-instance handler when a second launch is attempted.
pub fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn handle_menu(app: &tauri::AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "open" => show_main_window(app),
        "start_tunnel" | "stop_tunnel" => {
            let start = event.id().as_ref() == "start_tunnel";
            let manager = app.state::<TailscaleManager>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let result = if start {
                    manager.start().await
                } else {
                    manager.stop().await
                };
                if let Err(e) = result {
                    eprintln!(
                        "tray tunnel {} failed: {}",
                        if start { "start" } else { "stop" },
                        e.message
                    );
                }
            });
        }
        "toggle_pause" => {
            let state = app.state::<AppState>();
            let next = !state.core.pause.is_paused();
            if let Err(e) = commands::set_paused(app.clone(), state, next) {
                eprintln!("tray pause toggle failed: {e}");
            } else if let Some(item) = app
                .state::<TrayMenuItems>()
                .toggle_pause
                .lock()
                .unwrap()
                .as_ref()
            {
                let _ = item.set_text(if next {
                    "Resume MCP access"
                } else {
                    "Pause MCP access"
                });
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}
