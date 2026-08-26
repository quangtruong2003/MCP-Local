//! System tray: keeps the app reachable while the main window is hidden.
//!
//! The menu mirrors live state: a disabled status line (● Online / ● Offline
//! …), Start/Stop tunnel enabled only when meaningful, and a Pause/Resume
//! label that stays in sync no matter which surface toggled it. Updates
//! arrive from the connection-status emitter and the pause command and are
//! always applied on the main thread.

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent},
    Manager,
};

use crate::{
    commands, connection::manager::ConnectionManager as TailscaleManager,
    connection::state::ConnectionState, AppState,
};

/// Handles of the live menu items plus the tray icon, so status changes can
/// rewrite labels without rebuilding the menu.
#[derive(Default)]
pub struct TrayMenuItems {
    pub status: Mutex<Option<MenuItem<tauri::Wry>>>,
    pub start: Mutex<Option<MenuItem<tauri::Wry>>>,
    pub stop: Mutex<Option<MenuItem<tauri::Wry>>>,
    pub toggle_pause: Mutex<Option<MenuItem<tauri::Wry>>>,
    pub tray_icon: Mutex<Option<TrayIcon<tauri::Wry>>>,
    /// Cached short connection label for recomposing the status line when
    /// only the pause flag changed.
    pub state_label: Mutex<String>,
    /// Cached hostname shown next to Online.
    pub fqdn: Mutex<String>,
}

/// Builds the tray icon + menu. Call once from setup.
pub fn build(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let status = MenuItem::with_id(app, "status", "● Offline", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let start_tunnel = MenuItem::with_id(app, "start_tunnel", "Start tunnel", true, None::<&str>)?;
    let stop_tunnel = MenuItem::with_id(app, "stop_tunnel", "Stop tunnel", false, None::<&str>)?;
    let toggle_pause =
        MenuItem::with_id(app, "toggle_pause", "Pause MCP access", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
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

    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Local Workspace MCP Manager — Offline")
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
        status: Mutex::new(Some(status)),
        start: Mutex::new(Some(start_tunnel)),
        stop: Mutex::new(Some(stop_tunnel)),
        toggle_pause: Mutex::new(Some(toggle_pause)),
        tray_icon: Mutex::new(Some(tray)),
        state_label: Mutex::new("Offline".into()),
        fqdn: Mutex::new(String::new()),
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

/// Short status-line label for a connection state.
fn short_label(state: &ConnectionState) -> &'static str {
    match state {
        ConnectionState::Stopped => "Offline",
        ConnectionState::StartingServer | ConnectionState::StartingTunnel => "Connecting…",
        ConnectionState::NeedsLogin { .. } => "Sign-in needed",
        ConnectionState::NeedsFunnelPermission { .. } | ConnectionState::NeedsHttpsSetup { .. } => {
            "Setup needed"
        }
        ConnectionState::Verifying { .. } => "Verifying…",
        ConnectionState::Online { .. } => "Online",
        ConnectionState::Degraded { .. } => "Degraded",
        ConnectionState::Stopping => "Stopping…",
        ConnectionState::Error { .. } => "Error",
    }
}

/// Hostname for states that carry one.
fn fqdn_of(state: &ConnectionState) -> Option<String> {
    match state {
        ConnectionState::Online { fqdn, .. } | ConnectionState::Verifying { fqdn } => {
            Some(fqdn.clone())
        }
        ConnectionState::Degraded { fqdn, .. } => fqdn.clone(),
        _ => None,
    }
}

/// Called on every connection-status emission (any thread): caches the new
/// state and schedules the menu rewrite on the main thread.
pub fn update_status<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    status: &crate::connection::state::ConnectionStatus,
) {
    let label = short_label(&status.state).to_string();
    let fqdn = fqdn_of(&status.state).unwrap_or_default();
    let app_for_menu = app.clone();
    let _ = app.run_on_main_thread(move || {
        let items = app_for_menu.state::<TrayMenuItems>();
        *items.state_label.lock().unwrap() = label;
        *items.fqdn.lock().unwrap() = fqdn;
        refresh(&app_for_menu);
    });
}

/// Called when the pause flag changes from any surface (UI command, tray).
pub fn update_paused<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let app_for_menu = app.clone();
    let _ = app.run_on_main_thread(move || {
        refresh(&app_for_menu);
    });
}

/// Rewrites the live menu items + tray tooltip from the cached state. Must
/// run on the main thread.
fn refresh<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let items = app.state::<TrayMenuItems>();
    let label = items.state_label.lock().unwrap().clone();
    let fqdn = items.fqdn.lock().unwrap().clone();
    let paused = app.state::<AppState>().core.pause.is_paused();

    let running = !matches!(label.as_str(), "Offline" | "Stopping…");
    let status_text = if paused {
        "⏸ MCP paused".to_string()
    } else if label == "Online" && !fqdn.is_empty() {
        format!("● Online · {fqdn}")
    } else {
        format!("● {label}")
    };

    if let Some(item) = items.status.lock().unwrap().as_ref() {
        let _ = item.set_text(&status_text);
    }
    if let Some(item) = items.start.lock().unwrap().as_ref() {
        let _ = item.set_enabled(!running);
    }
    if let Some(item) = items.stop.lock().unwrap().as_ref() {
        let _ = item.set_enabled(running);
    }
    if let Some(item) = items.toggle_pause.lock().unwrap().as_ref() {
        let _ = item.set_text(if paused {
            "Resume MCP access"
        } else {
            "Pause MCP access"
        });
    }
    let tray_guard = items.tray_icon.lock().unwrap();
    if let Some(tray) = tray_guard.as_ref() {
        let tooltip = if paused {
            "Local Workspace MCP Manager — MCP paused".to_string()
        } else if label == "Online" && !fqdn.is_empty() {
            format!("Local Workspace MCP Manager — Online ({fqdn})")
        } else {
            format!("Local Workspace MCP Manager — {label}")
        };
        let _ = tray.set_tooltip(Some(tooltip));
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
            let next = !app.state::<AppState>().core.pause.is_paused();
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                if let Err(e) = commands::set_paused(handle.clone(), state, next).await {
                    eprintln!("tray pause toggle failed: {e}");
                }
            });
            // set_paused emits access-paused and calls update_paused, which
            // rewrites the label — no direct menu edit here.
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::state::ConnectionState;

    #[test]
    fn short_labels_cover_representative_states() {
        assert_eq!(short_label(&ConnectionState::Stopped), "Offline");
        assert_eq!(short_label(&ConnectionState::StartingTunnel), "Connecting…");
        assert_eq!(
            short_label(&ConnectionState::NeedsLogin {
                login_url: "https://login.tailscale.com/a".into()
            }),
            "Sign-in needed"
        );
        assert_eq!(
            short_label(&ConnectionState::Online {
                fqdn: "x.ts.net".into(),
                endpoint: "https://x.ts.net/mcp".into()
            }),
            "Online"
        );
        assert_eq!(
            short_label(&ConnectionState::Error {
                code: "X".into(),
                message: "m".into()
            }),
            "Error"
        );
    }

    #[test]
    fn fqdn_only_for_states_that_carry_one() {
        assert_eq!(
            fqdn_of(&ConnectionState::Online {
                fqdn: "x.ts.net".into(),
                endpoint: "u".into()
            }),
            Some("x.ts.net".into())
        );
        assert_eq!(fqdn_of(&ConnectionState::Stopped), None);
    }
}
