//! mirzabenevis desktop app.
//!
//! Lives in the tray. The popup is shown by:
//!   - the global hotkey from config (X11 only; Wayland forbids global grabs), or
//!   - running `mirzabenevis --popup` again: the single-instance plugin forwards
//!     the argument to the running process. Bind that in your compositor.
//!
//! The popup grabs the current selection, lets you pick an action, streams the
//! result, and pastes it back (or copies it) when you press Enter.

use futures_util::future::{AbortHandle, Abortable};
use mirzabenevis_core::config::{
    self, default_actions, default_modifiers, General, Modifier, Output, PasteMode, Translate,
    RESERVED_KEYS,
};
use mirzabenevis_core::{backends, history, text, Action, Backend, Config, Detection, Turn};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewWindow,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct AppState {
    cfg: Mutex<Config>,
    /// Last detection result; refreshed whenever the popup opens.
    detection: Mutex<Option<Detection>>,
    /// Bumped when the config changes, so a detection started before that
    /// (with the old models) is not cached over the new state.
    generation: AtomicU64,
    /// The request in flight, so closing the popup can stop it.
    running: Mutex<Option<AbortHandle>>,
    /// Pinned popups stay open on focus loss (Escape still closes them). On by default.
    pinned: AtomicBool,
    /// Set while `accept` runs, so the focus-loss handler does not hide a popup
    /// that `accept` just showed again.
    accepting: AtomicBool,
}

/// A streamed piece of the answer, tagged with the run it belongs to so the
/// popup can ignore output from a run it already abandoned.
#[derive(Serialize, Clone)]
struct Chunk<'a> {
    run: u64,
    text: &'a str,
}

#[derive(Serialize, Clone)]
struct RunResult {
    text: String,
    /// Label of the backend that answered (may differ from the selected one in auto mode).
    backend: String,
    /// Its id, so a follow-up (explain-change) can use the same backend.
    backend_id: String,
}

impl RunResult {
    fn new(text: String, b: &Backend) -> Self {
        RunResult {
            text,
            backend: format!("{} · {}", b.label, b.model_label()),
            backend_id: b.id.clone(),
        }
    }
}

#[derive(Serialize, Clone)]
struct ActionView {
    id: String,
    name: String,
    key: String,
    output: Output,
    /// The prompt has `{target}` (the chip shows the target language).
    target: bool,
    /// Seeds a chat that continues from this action's result.
    prompt: String,
    /// One of the default actions (not removable from the popup).
    builtin: bool,
}

#[derive(Serialize, Clone)]
struct Info {
    /// What Enter would do with a result: "pasted", "typed" or "copied".
    paste: String,
    default_action: String,
    /// `translate.target`
    target: String,
    modifiers: Vec<Modifier>,
    max_input_chars: usize,
    /// `general.explain_in`
    explain_in: String,
}

/// What a result being accepted came from, for the history (`undo`).
#[derive(Deserialize)]
struct Applied {
    /// The text the user selected (for a refine chain: the first one).
    original: String,
    action: String,
    backend: String,
}

#[derive(Serialize, Clone)]
struct BackendView {
    id: String,
    label: String,
    model: String,
    category: backends::Category,
    source: String,
}

impl From<&Backend> for BackendView {
    fn from(b: &Backend) -> Self {
        BackendView {
            id: b.id.clone(),
            label: b.label.clone(),
            model: b.model_label().to_string(),
            category: b.category,
            source: b.source.clone(),
        }
    }
}

#[derive(Serialize, Clone)]
struct BackendsView {
    /// `provider.use`: "auto" or an id.
    wanted: String,
    /// The backend that will actually run, if any.
    chosen: Option<String>,
    found: Vec<BackendView>,
    /// Why nothing can run, when `chosen` is empty.
    error: Option<String>,
}

fn popup(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("popup")
}

/// Show the popup near the mouse cursor and tell the UI to refresh.
fn show_popup(app: &AppHandle) {
    let Some(w) = popup(app) else { return };
    if let Ok(pos) = app.cursor_position() {
        // Nudge so the window doesn't sit under the pointer, but keep it on screen.
        let (mut x, mut y) = (pos.x + 12.0, pos.y + 12.0);
        if let (Ok(Some(m)), Ok(size)) = (app.monitor_from_point(pos.x, pos.y), w.outer_size()) {
            let (mx, my) = (m.position().x as f64, m.position().y as f64);
            let (mw, mh) = (m.size().width as f64, m.size().height as f64);
            let (ww, wh) = (size.width as f64, size.height as f64);
            x = x.min(mx + mw - ww - 8.0).max(mx);
            y = y.min(my + mh - wh - 8.0).max(my);
        }
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
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

fn action_views(cfg: &Config) -> Vec<ActionView> {
    let builtin: Vec<String> = default_actions().into_iter().map(|a| a.id).collect();
    cfg.actions
        .iter()
        .map(|a: &Action| ActionView {
            id: a.id.clone(),
            name: a.name.clone(),
            key: a.key.clone(),
            output: a.output,
            target: a.prompt.contains("{target}"),
            prompt: a.prompt.clone(),
            builtin: builtin.contains(&a.id),
        })
        .collect()
}

#[tauri::command]
fn get_actions(state: State<AppState>) -> Vec<ActionView> {
    action_views(&state.cfg.lock().unwrap())
}

/// Save a one-off instruction as an action (a new chip) in config.toml.
#[tauri::command]
fn add_action(state: State<AppState>, prompt: String) -> Result<Vec<ActionView>, String> {
    let fresh = Config::add_action(&prompt).map_err(|e| format!("{e:#}"))?;
    let mut c = state.cfg.lock().unwrap();
    c.actions = fresh.actions;
    Ok(action_views(&c))
}

/// Remove an action added with `add_action`.
#[tauri::command]
fn remove_action(state: State<AppState>, id: String) -> Result<Vec<ActionView>, String> {
    let mut c = state.cfg.lock().unwrap();
    let fresh = c.remove_action(&id).map_err(|e| format!("{e:#}"))?;
    c.actions = fresh.actions;
    Ok(action_views(&c))
}

/// An action or modifier for the settings page.
#[derive(Serialize)]
struct Entry<T> {
    #[serde(flatten)]
    item: T,
    /// One of the defaults: reset, not remove.
    builtin: bool,
}

/// What the settings page edits. Nothing from `provider.custom`: no API keys
/// or commands reach the page.
#[derive(Serialize)]
struct SettingsView {
    general: General,
    paste: PasteMode,
    translate: Translate,
    max_tokens: u32,
    temperature: f64,
    timeout_secs: u64,
    actions: Vec<Entry<Action>>,
    modifiers: Vec<Entry<Modifier>>,
    reserved: &'static [&'static str],
    config_path: String,
}

fn entries<T: Clone>(items: &[T], defaults: &[T], id: fn(&T) -> &str) -> Vec<Entry<T>> {
    items
        .iter()
        .map(|item| Entry {
            builtin: defaults.iter().any(|d| id(d) == id(item)),
            item: item.clone(),
        })
        .collect()
}

fn settings_view(c: &Config) -> SettingsView {
    SettingsView {
        general: c.general.clone(),
        paste: c.linux.paste,
        translate: c.translate.clone(),
        max_tokens: c.provider.max_tokens,
        temperature: c.provider.temperature,
        timeout_secs: c.provider.timeout_secs,
        actions: entries(&c.actions, &default_actions(), |a| &a.id),
        modifiers: entries(&c.modifiers, &default_modifiers(), |m| &m.id),
        reserved: RESERVED_KEYS,
        config_path: Config::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    }
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> SettingsView {
    settings_view(&state.cfg.lock().unwrap())
}

/// Take a config just saved by `Config::edit_file` as the running one.
fn saved(state: &AppState, fresh: anyhow::Result<Config>) -> Result<SettingsView, String> {
    let fresh = fresh.map_err(|e| format!("{e:#}"))?;
    let view = settings_view(&fresh);
    *state.cfg.lock().unwrap() = fresh;
    config_changed(state, false);
    Ok(view)
}

/// What `set_setting` may change (never keys, URLs or commands).
const SETTINGS: &[&str] = &[
    "general.default_action",
    "general.auto_paste",
    "general.notify",
    "general.history",
    "general.explain_in",
    "general.hotkey",
    "general.max_input_chars",
    "linux.paste",
    "translate.target",
    "translate.targets",
    "provider.max_tokens",
    "provider.temperature",
    "provider.timeout_secs",
];

/// Save one setting (text, boolean, number or list of texts) in config.toml.
#[tauri::command]
fn set_setting(
    state: State<AppState>,
    path: Vec<String>,
    value: serde_json::Value,
) -> Result<SettingsView, String> {
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    if !SETTINGS
        .iter()
        .any(|s| s.split('.').eq(path.iter().copied()))
    {
        return Err(format!("`{}` cannot be set here", path.join(".")));
    }
    let value = config::toml_value(&value).map_err(|e| format!("{e:#}"))?;
    config::check_setting(&path, &value).map_err(|e| format!("{e:#}"))?;
    saved(
        &state,
        Config::edit_file(|doc| config::set_doc_value(doc, &path, value)),
    )
}

#[tauri::command]
fn set_action(
    state: State<AppState>,
    id: String,
    patch: BTreeMap<String, String>,
) -> Result<SettingsView, String> {
    saved(
        &state,
        Config::edit_file(|doc| config::patch_entry(doc, "actions", &id, &patch)),
    )
}

/// Move an action (its chip) `delta` places.
#[tauri::command]
fn move_action(state: State<AppState>, id: String, delta: isize) -> Result<SettingsView, String> {
    saved(
        &state,
        Config::edit_file(|doc| config::move_entry(doc, "actions", &id, delta)),
    )
}

#[tauri::command]
fn reset_action(state: State<AppState>, id: String) -> Result<SettingsView, String> {
    saved(
        &state,
        Config::edit_file(|doc| config::reset_entry(doc, "actions", &id)),
    )
}

#[tauri::command]
fn set_modifier(
    state: State<AppState>,
    id: String,
    patch: BTreeMap<String, String>,
) -> Result<SettingsView, String> {
    saved(
        &state,
        Config::edit_file(|doc| config::patch_entry(doc, "modifiers", &id, &patch)),
    )
}

#[tauri::command]
fn add_modifier(state: State<AppState>) -> Result<SettingsView, String> {
    saved(
        &state,
        Config::edit_file(|doc| config::add_modifier_to(doc).map(drop)),
    )
}

#[tauri::command]
fn remove_modifier(state: State<AppState>, id: String) -> Result<SettingsView, String> {
    saved(
        &state,
        Config::edit_file(|doc| config::remove_entry(doc, "modifiers", &id)),
    )
}

#[tauri::command]
fn reset_modifier(state: State<AppState>, id: String) -> Result<SettingsView, String> {
    saved(
        &state,
        Config::edit_file(|doc| config::reset_entry(doc, "modifiers", &id)),
    )
}

#[tauri::command]
fn get_info(state: State<AppState>) -> Info {
    let c = state.cfg.lock().unwrap();
    Info {
        paste: paste_how(&c).into(),
        default_action: c.general.default_action.clone(),
        target: c.translate.target.clone(),
        modifiers: c.modifiers.clone(),
        max_input_chars: c.general.max_input_chars,
        explain_in: c.general.explain_in.clone(),
    }
}

/// Switch `translate.target` to the next of `translate.targets` and save it.
#[tauri::command]
fn cycle_target(state: State<AppState>) -> Result<String, String> {
    let mut c = state.cfg.lock().unwrap();
    let t = &c.translate;
    let i = t
        .targets
        .iter()
        .position(|x| *x == t.target)
        .map_or(0, |i| i + 1);
    let next = t
        .targets
        .get(i % t.targets.len().max(1))
        .cloned()
        .ok_or("translate.targets is empty")?;
    Config::set_in_file(&["translate", "target"], &next).map_err(|e| format!("{e:#}"))?;
    c.translate.target = next.clone();
    Ok(next)
}

/// The last distinct one-off instructions, newest first, from the history.
#[tauri::command]
async fn recent_prompts() -> Vec<String> {
    history::read()
        .map(|h| history::recent_prompts(&h, 20))
        .unwrap_or_default()
}

fn backends_view(cfg: &Config, det: &Detection) -> BackendsView {
    let picked = det.pick(&cfg.provider.use_backend);
    BackendsView {
        wanted: cfg.provider.use_backend.clone(),
        chosen: picked.as_ref().ok().map(|b| b.id.clone()),
        found: det.found.iter().map(BackendView::from).collect(),
        error: picked.err().map(|e| format!("{e:#}")),
    }
}

/// Look for backends again (API keys, agent CLIs, local servers) unless a
/// cached result is fine.
async fn detection(state: &State<'_, AppState>, refresh: bool) -> (Config, Detection) {
    let generation = state.generation.load(Ordering::SeqCst);
    let cfg = state.cfg.lock().unwrap().clone();
    if !refresh {
        if let Some(d) = state.detection.lock().unwrap().clone() {
            return (cfg, d);
        }
    }
    // Local servers are probed again; the login shell's environment comes from
    // its short-lived cache (refreshed by "Reload config").
    let d = backends::detect_current(&cfg, false).await;
    let mut cached = state.detection.lock().unwrap();
    if state.generation.load(Ordering::SeqCst) == generation {
        *cached = Some(d.clone());
    }
    (cfg, d)
}

/// The config changed (call after updating `state.cfg`): a detection in
/// flight is not cached; `forget` also drops the cached one. Under the
/// detection lock, so a detection cannot store between the two.
fn config_changed(state: &AppState, forget: bool) {
    let mut cached = state.detection.lock().unwrap();
    state.generation.fetch_add(1, Ordering::SeqCst);
    if forget {
        *cached = None;
    }
}

#[tauri::command]
async fn get_backends(state: State<'_, AppState>, refresh: bool) -> Result<BackendsView, String> {
    let (cfg, det) = detection(&state, refresh).await;
    Ok(backends_view(&cfg, &det))
}

/// Choose a backend ("auto" or an id) and remember it in the config file.
#[tauri::command]
async fn set_backend(state: State<'_, AppState>, id: String) -> Result<BackendsView, String> {
    if id != "auto" && !backends::known_ids().contains(&id.as_str()) {
        return Err(format!("unknown backend `{id}`"));
    }
    Config::set_in_file(&["provider", "use"], &id).map_err(|e| format!("{e:#}"))?;
    state.cfg.lock().unwrap().provider.use_backend = id;
    config_changed(&state, false);
    let (cfg, det) = detection(&state, false).await;
    Ok(backends_view(&cfg, &det))
}

/// Set a backend's model (`provider.models.<id>`; empty = its default) and
/// remember it in the config file.
#[tauri::command]
async fn set_model(
    state: State<'_, AppState>,
    id: String,
    model: String,
) -> Result<BackendsView, String> {
    if !backends::known_ids().contains(&id.as_str()) {
        return Err(format!("unknown backend `{id}`"));
    }
    let model = model.trim().to_string();
    Config::set_in_file(&["provider", "models", &id], &model).map_err(|e| format!("{e:#}"))?;
    state.cfg.lock().unwrap().provider.models.insert(id, model);
    config_changed(&state, true); // the model is part of what was detected
    let (cfg, det) = detection(&state, false).await;
    Ok(backends_view(&cfg, &det))
}

#[tauri::command]
fn grab(state: State<AppState>) -> Result<String, String> {
    let c = state.cfg.lock().unwrap();
    text::grab(&c).map_err(|e| e.to_string())
}

/// Stream an action. Emits `chunk` events with text deltas and resolves with the full result.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // command arguments are named fields from the UI
async fn run_action(
    app: AppHandle,
    state: State<'_, AppState>,
    run: u64,
    action_id: String,
    input: String,
    prompt: Option<String>,
    modifiers: Option<Vec<String>>,
    context: Option<String>,
    // A backend id: run on exactly that one (no fallback, not in the history).
    backend: Option<String>,
) -> Result<RunResult, String> {
    let work = async {
        let (mut cfg, det) = detection(&state, false).await;
        // One-off prompt from the popup's custom field: inject a temporary action.
        if let Some(p) = prompt.filter(|p| !p.trim().is_empty()) {
            cfg.actions.push(Action {
                id: action_id.clone(),
                name: "custom".into(),
                prompt: p,
                ..Default::default()
            });
        }
        cfg.run_extra = cfg.extras(&modifiers.unwrap_or_default(), &context.unwrap_or_default());
        let on_chunk = |text: &str| {
            let _ = app.emit("chunk", Chunk { run, text });
        };
        let (text, b) = match backend {
            Some(id) => {
                let b = det.pick(&id)?;
                let text =
                    mirzabenevis_core::run_action(&cfg, b, &action_id, &input, on_chunk).await?;
                (text, b)
            }
            None => {
                mirzabenevis_core::run_action_auto(
                    &cfg,
                    &det,
                    &cfg.provider.use_backend,
                    &action_id,
                    &input,
                    on_chunk,
                )
                .await?
            }
        };
        anyhow::Ok(RunResult::new(text, b))
    };
    registered(&state, work).await
}

/// Answer the last turn of a chat on the selected backend (no fallback, not
/// in the history). Streams `chunk` events like `run_action`.
#[tauri::command]
async fn chat(
    app: AppHandle,
    state: State<'_, AppState>,
    run: u64,
    turns: Vec<Turn>,
) -> Result<RunResult, String> {
    let work = async {
        let (cfg, det) = detection(&state, false).await;
        let b = det.pick(&cfg.provider.use_backend)?;
        let text = mirzabenevis_core::chat(&cfg, b, &turns, |text: &str| {
            let _ = app.emit("chunk", Chunk { run, text });
        })
        .await?;
        anyhow::Ok(RunResult::new(text, b))
    };
    registered(&state, work).await
}

/// Run `work` as the request in flight: it aborts the previous one, and
/// `cancel` (Esc) or a newer request aborts it. Registered before `work`
/// starts, so Esc also cancels detection and an older request can never abort
/// a newer one. Dropping the aborted future also kills an agent CLI.
async fn registered<T>(
    state: &AppState,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> Result<T, String> {
    let (handle, registration) = AbortHandle::new_pair();
    if let Some(previous) = state.running.lock().unwrap().replace(handle) {
        previous.abort();
    }
    match Abortable::new(work, registration).await {
        Err(_) => Err("cancelled".into()),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Ok(Ok(r)) => Ok(r),
    }
}

/// Stop the request in flight, if any.
#[tauri::command]
fn cancel(state: State<AppState>) {
    if let Some(h) = state.running.lock().unwrap().take() {
        h.abort();
    }
}

fn paste_how(cfg: &Config) -> &'static str {
    if !cfg.general.auto_paste {
        return "copied";
    }
    match text::paste_method(cfg) {
        text::PasteResult::Pasted => "pasted",
        text::PasteResult::Typed => "typed",
        text::PasteResult::Clipboard => "copied",
    }
}

/// `set_clipboard` waits for `wl-copy`; keep that off the async runtime.
async fn copy_blocking(s: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || text::set_clipboard(&s))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

/// Insert the result into the previously focused app. If it can only reach the
/// clipboard, copy it and keep the popup open so the user sees that. Otherwise
/// hide first (keystrokes must go to the other app), then paste; on failure the
/// popup comes back (without `popup-shown`, which would reset it) and the
/// result goes to the clipboard.
#[tauri::command]
async fn accept(
    app: AppHandle,
    state: State<'_, AppState>,
    result: String,
    applied: Option<Applied>,
) -> Result<String, String> {
    let cfg = state.cfg.lock().unwrap().clone();
    state.accepting.store(true, Ordering::Relaxed);
    let out = insert(&app, cfg.clone(), result.clone()).await;
    state.accepting.store(false, Ordering::Relaxed);
    if let (Ok("pasted" | "typed"), Some(a)) = (out.as_deref(), applied) {
        let entry = history::Entry {
            replaced: true,
            ..history::Entry::new(&a.action, None, &a.backend, &a.original, &result)
        };
        history::append(&cfg, &entry);
    }
    out
}

/// Paste the original of the last run that replaced a selection, like `accept`.
#[tauri::command]
async fn undo(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let h = history::read().map_err(|e| format!("{e:#}"))?;
    let original = history::last_undoable(&h)
        .ok_or("nothing to undo")?
        .original
        .clone();
    accept(app, state, original, None).await
}

async fn insert(app: &AppHandle, cfg: Config, result: String) -> Result<String, String> {
    let reshow = || {
        if let Some(w) = popup(app) {
            let _ = w.show();
            let _ = w.set_focus();
        }
    };
    if paste_how(&cfg) == "copied" {
        copy_blocking(result).await?;
        return Ok("copied".into());
    }
    hide_popup(app);
    // Give the compositor a moment to move focus back.
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let r = result.clone();
    let pasted = tauri::async_runtime::spawn_blocking(move || text::paste(&cfg, &r))
        .await
        .map_err(|e| e.to_string())?;
    match pasted {
        Ok(text::PasteResult::Pasted) => Ok("pasted".into()),
        Ok(text::PasteResult::Typed) => Ok("typed".into()),
        Ok(text::PasteResult::Clipboard) => {
            reshow(); // no tool worked after all; say so in the popup
            Ok("copied".into())
        }
        Err(e) => {
            reshow();
            Err(match copy_blocking(result).await {
                Ok(()) => format!("{e:#} — copied to clipboard instead"),
                Err(_) => format!("{e:#}"),
            })
        }
    }
}

#[tauri::command]
fn set_pinned(state: State<AppState>, pinned: bool) {
    state.pinned.store(pinned, Ordering::Relaxed);
}

#[tauri::command]
async fn copy(result: String) -> Result<(), String> {
    copy_blocking(result).await
}

#[tauri::command]
fn close(app: AppHandle, state: State<AppState>) {
    cancel(state);
    hide_popup(&app);
}

#[tauri::command]
fn reload_config(state: State<AppState>) -> Result<(), String> {
    let fresh = Config::load().map_err(|e| e.to_string())?;
    *state.cfg.lock().unwrap() = fresh.clone();
    config_changed(&state, true);
    // Re-read the login shell too (new API keys in ~/.zshrc, new CLIs on PATH).
    tauri::async_runtime::spawn(async move {
        let _ = backends::detect_current(&fresh, true).await;
    });
    Ok(())
}

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    config_path: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        config_path: Config::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    }
}

/// Clipboard text for the popup's Paste button.
#[tauri::command]
async fn read_clipboard() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(text::clipboard_text)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Create the config file if needed and open it in the desktop's editor.
#[tauri::command]
fn open_config() -> Result<String, String> {
    let p = Config::path().map_err(|e| e.to_string())?;
    if !p.exists() {
        Config::init().map_err(|e| e.to_string())?;
    }
    xdg_open(p.to_str().unwrap_or_default())?;
    Ok(p.display().to_string())
}

/// Open a file or URL with the desktop's default handler (xdg-open, then gio).
fn xdg_open(target: &str) -> Result<(), String> {
    let null = std::process::Stdio::null;
    for cmd in [vec!["xdg-open", target], vec!["gio", "open", target]] {
        if std::process::Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(null())
            .stdout(null())
            .stderr(null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("neither xdg-open nor gio is available".into())
}

/// Open an https:// link (About page) in the browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https links".into());
    }
    xdg_open(&url)
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn config_path() -> Result<String, String> {
    Config::path()
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())
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
        .manage(AppState {
            cfg: Mutex::new(cfg),
            detection: Mutex::new(None),
            generation: AtomicU64::new(0),
            running: Mutex::new(None),
            pinned: AtomicBool::new(true),
            accepting: AtomicBool::new(false),
        })
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
            add_action,
            remove_action,
            get_info,
            get_settings,
            set_setting,
            set_action,
            move_action,
            reset_action,
            set_modifier,
            add_modifier,
            remove_modifier,
            reset_modifier,
            get_backends,
            set_backend,
            set_model,
            chat,
            cancel,
            grab,
            run_action,
            accept,
            undo,
            cycle_target,
            recent_prompts,
            copy,
            set_pinned,
            close,
            reload_config,
            config_path,
            app_info,
            read_clipboard,
            open_config,
            open_url,
            quit
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

            // Hide on focus loss, like a launcher, unless pinned.
            if let Some(w) = popup(app.handle()) {
                let handle = app.handle().clone();
                w.on_window_event(move |ev| {
                    if let tauri::WindowEvent::Focused(false) = ev {
                        let handle = handle.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            let st = handle.state::<AppState>();
                            let keep = st.pinned.load(Ordering::Relaxed) || st.accepting.load(Ordering::Relaxed);
                            let refocused = popup(&handle).and_then(|w| w.is_focused().ok()).unwrap_or(false);
                            if !keep && !refocused {
                                hide_popup(&handle);
                            }
                        });
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
