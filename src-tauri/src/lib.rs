//! mirzabenevis desktop app.
//!
//! Lives in the tray. The popup is shown by:
//!   - the global hotkey from config (X11 only; Wayland forbids global grabs), or
//!   - running `mirzabenevis --popup` again: the single-instance plugin forwards
//!     the argument to the running process. Bind that in your compositor.
//!
//! The popup grabs the current selection, lets you pick an action, streams the
//! result, and pastes it back (or copies it) when you press Enter.

use mirzabenevis_core::{text, Action, Config};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewWindow,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct AppState {
    cfg: Mutex<Config>,
}

#[derive(Serialize, Clone)]
struct ActionView {
    id: String,
    name: String,
    key: String,
}

#[derive(Serialize, Clone)]
struct Info {
    model: String,
    provider: String,
    auto_paste: bool,
    default_action: String,
}

fn popup(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("popup")
}

/// Show the popup near the mouse cursor and tell the UI to refresh.
fn show_popup(app: &AppHandle) {
    let Some(w) = popup(app) else { return };
    if let Ok(pos) = app.cursor_position() {
        // Nudge so the window doesn't sit under the pointer.
        let _ = w.set_position(tauri::PhysicalPosition::new(pos.x + 12.0, pos.y + 12.0));
    }
    let _ = w.show();
    let _ = w.set_focus();
    let _ = w.emit("popup-shown", ());
}

fn hide_popup(app: &AppHandle) {
    if let Some(w) = popup(app) {
        let _ = w.hide();
    }
}

#[tauri::command]
fn get_actions(state: State<AppState>) -> Vec<ActionView> {
    state
        .cfg
        .lock()
        .unwrap()
        .actions
        .iter()
        .map(|a: &Action| ActionView {
            id: a.id.clone(),
            name: a.name.clone(),
            key: a.key.clone(),
        })
        .collect()
}

#[tauri::command]
fn get_info(state: State<AppState>) -> Info {
    let c = state.cfg.lock().unwrap();
    Info {
        model: c.provider.model.clone(),
        provider: format!("{:?}", c.provider.kind).to_lowercase(),
        auto_paste: c.general.auto_paste,
        default_action: c.general.default_action.clone(),
    }
}

#[tauri::command]
fn grab(state: State<AppState>) -> Result<String, String> {
    let c = state.cfg.lock().unwrap();
    text::grab(&c).map_err(|e| e.to_string())
}

/// Stream an action. Emits `chunk` events with text deltas and resolves with the full result.
#[tauri::command]
async fn run_action(
    app: AppHandle,
    state: State<'_, AppState>,
    action_id: String,
    input: String,
) -> Result<String, String> {
    let cfg = state.cfg.lock().unwrap().clone();
    mirzabenevis_core::run_action(&cfg, &action_id, &input, |chunk| {
        let _ = app.emit("chunk", chunk);
    })
    .await
    .map_err(|e| e.to_string())
}

/// Hide the popup first so keystrokes go to the previously focused app, then paste.
#[tauri::command]
async fn accept(app: AppHandle, state: State<'_, AppState>, result: String) -> Result<String, String> {
    let cfg = state.cfg.lock().unwrap().clone();
    hide_popup(&app);
    // Give the compositor a moment to move focus back.
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let r = if cfg.general.auto_paste {
        text::paste(&cfg, &result).map_err(|e| e.to_string())?
    } else {
        text::set_clipboard(&result).map_err(|e| e.to_string())?;
        text::PasteResult::Clipboard
    };
    Ok(match r {
        text::PasteResult::Typed => "pasted".into(),
        text::PasteResult::Clipboard => "copied".into(),
    })
}

#[tauri::command]
fn copy(result: String) -> Result<(), String> {
    text::set_clipboard(&result).map_err(|e| e.to_string())
}

#[tauri::command]
fn close(app: AppHandle) {
    hide_popup(&app);
}

#[tauri::command]
fn reload_config(state: State<AppState>) -> Result<(), String> {
    let fresh = Config::load().map_err(|e| e.to_string())?;
    *state.cfg.lock().unwrap() = fresh;
    Ok(())
}

#[tauri::command]
fn config_path() -> Result<String, String> {
    Config::path().map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("MIRZABENEVIS_LOG"))
        .init();

    let cfg = Config::load().unwrap_or_else(|e| {
        tracing::error!("config error, using defaults: {e:#}");
        Config::default()
    });
    let hotkey = cfg.general.hotkey.clone();

    tauri::Builder::default()
        .manage(AppState { cfg: Mutex::new(cfg) })
        // `mirzabenevis --popup` while running => show the popup. This is the
        // Wayland-friendly "hotkey": bind that command in your compositor.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--popup") {
                show_popup(app);
            }
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        show_popup(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_actions,
            get_info,
            grab,
            run_action,
            accept,
            copy,
            close,
            reload_config,
            config_path
        ])
        .setup(move |app| {
            // Global hotkey: works on X11, silently unavailable on Wayland.
            if !hotkey.is_empty() {
                match app.global_shortcut().register(hotkey.as_str()) {
                    Ok(()) => tracing::info!("registered hotkey {hotkey}"),
                    Err(e) => tracing::warn!(
                        "could not register hotkey {hotkey}: {e}. On Wayland bind `mirzabenevis --popup` in your compositor."
                    ),
                }
            }

            // Tray menu.
            let open = MenuItem::with_id(app, "open", "Open popup", true, None::<&str>)?;
            let reload = MenuItem::with_id(app, "reload", "Reload config", true, None::<&str>)?;
            let cfg_item = MenuItem::with_id(app, "config", "Config file path", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &reload, &cfg_item, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().expect("icon in tauri.conf.json"))
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "open" => show_popup(app),
                    "reload" => {
                        let s: State<AppState> = app.state();
                        if let Err(e) = reload_config(s) {
                            tracing::error!("reload failed: {e}");
                        }
                    }
                    "config" => {
                        if let Ok(p) = Config::path() {
                            let _ = text::set_clipboard(&p.display().to_string());
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Hide on focus loss, like a launcher.
            if let Some(w) = popup(app.handle()) {
                let handle = app.handle().clone();
                w.on_window_event(move |ev| {
                    if let tauri::WindowEvent::Focused(false) = ev {
                        hide_popup(&handle);
                    }
                });
            }

            // Started with --popup: show it right away.
            if std::env::args().any(|a| a == "--popup") {
                show_popup(app.handle());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error building mirzabenevis")
        .run(|_app, event| {
            // Keep running in the tray when the popup is closed.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
