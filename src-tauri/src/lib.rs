pub mod audio;
pub mod autoaccept;
pub mod banassist;
pub mod lcu;
pub mod league;
pub mod logger;
pub mod schedule;
pub mod settings;
pub mod updates;
pub mod watcher;

use audio::{AudioPlayer, SoundSource};
use league::LeagueClient;
use logger::log;
use schedule::format_clock;
use serde::Serialize;
use settings::{
    role_name, CustomSound, Settings, SettingsStore, BUILTIN_SOUNDS, DEFAULT_SOUND, ROLES, ROLE_AUTO, SOUND_TOGGLE_OFF,
    SOUND_TOGGLE_ON,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, Wry};
use updates::{Phase as UpdatePhase, UpdateStatus, Updates};
use watcher::{Event, State, Status, Watcher};

const AUDIO_EXTS: &[&str] = &["mp3", "wav", "ogg", "m4a", "flac"];
const MAX_SOUND_BYTES: u64 = 10 * 1024 * 1024;
const WINDOW: &str = "main";

struct TrayUi {
    tray: TrayIcon,
    status_item: MenuItem<Wry>,
    enabled_item: CheckMenuItem<Wry>,
    update_item: MenuItem<Wry>,
    accept_item: CheckMenuItem<Wry>,
    ban_item: CheckMenuItem<Wry>,
    role_menu: Submenu<Wry>,
    role_items: Vec<(String, CheckMenuItem<Wry>)>,
    last_key: String,
    icon_kind: Option<IconKind>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Idle,
    Client,
    InGame,
}

struct AppState {
    store: Mutex<SettingsStore>,
    watcher: Mutex<Watcher>,
    audio: AudioPlayer,
    config_dir: PathBuf,
    tray: Mutex<Option<TrayUi>>,
    /// Champion auto-hovered for your ban this champ select.
    ban_hovered: Mutex<Option<String>>,
    /// Latest match-found popup state (for the settings page).
    ready_check: Mutex<Option<autoaccept::ReadyCheckStatus>>,
    /// Why the auto-accept shortcut couldn't be registered, if it couldn't.
    hotkey_error: Mutex<Option<String>>,
    /// True while the settings page is recording a new shortcut.
    hotkey_capturing: Mutex<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SoundInfo {
    id: String,
    name: String,
    builtin: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleInfo {
    id: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicState {
    settings: Settings,
    roles: Vec<RoleInfo>,
    sounds: Vec<SoundInfo>,
    status: Status,
    update: UpdateStatus,
    hotkey_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

// ---------------------------------------------------------------- helpers

fn current_settings(app: &AppHandle) -> Settings {
    app.state::<AppState>().store.lock().unwrap().get().clone()
}

fn current_status(app: &AppHandle) -> Status {
    let s = current_settings(app);
    let state = app.state::<AppState>();
    let mut st = state.watcher.lock().unwrap().status(&s);
    st.ready_check = state.ready_check.lock().unwrap().clone();
    st.ban_hovered = state.ban_hovered.lock().unwrap().clone();
    st
}

fn public_state(app: &AppHandle, error: Option<String>) -> PublicState {
    let settings = current_settings(app);
    let mut sounds: Vec<SoundInfo> = BUILTIN_SOUNDS
        .iter()
        .map(|b| SoundInfo { id: b.id.into(), name: b.name.into(), builtin: true })
        .collect();
    sounds.extend(settings.custom_sounds.iter().map(|c| SoundInfo { id: c.id.clone(), name: c.name.clone(), builtin: false }));
    let update = app.state::<Updates>().status();
    let roles = ROLES.iter().map(|(id, name)| RoleInfo { id, name }).collect();
    let hotkey_error = app.state::<AppState>().hotkey_error.lock().unwrap().clone();
    PublicState { status: current_status(app), settings, roles, sounds, update, hotkey_error, error }
}

fn sound_source(settings: &Settings, id: Option<&str>) -> SoundSource {
    let id = id.unwrap_or(&settings.sound_id);
    if let Some(c) = settings.custom_sounds.iter().find(|c| c.id == id) {
        return SoundSource::File(c.file.clone());
    }
    let b = BUILTIN_SOUNDS.iter().find(|b| b.id == id).unwrap_or(&BUILTIN_SOUNDS[0]);
    SoundSource::Builtin(b.bytes)
}

fn play(app: &AppHandle, id: Option<&str>) {
    let s = current_settings(app);
    app.state::<AppState>().audio.play(sound_source(&s, id), s.volume);
    if app.get_webview_window(WINDOW).is_some() {
        let _ = app.emit_to(WINDOW, "pinged", ());
    }
}

fn play_bytes(app: &AppHandle, bytes: &'static [u8]) {
    let s = current_settings(app);
    app.state::<AppState>().audio.play(SoundSource::Builtin(bytes), s.volume);
}

/// (Re-)register the auto-accept toggle shortcut from settings.
fn register_hotkey(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let state = app.state::<AppState>();
    let hk = current_settings(app).auto_accept_hotkey;
    let result: Result<(), String> = if hk.is_empty() || *state.hotkey_capturing.lock().unwrap() {
        Ok(())
    } else {
        hk.parse::<Shortcut>()
            .map_err(|e| format!("“{hk}” isn't a valid shortcut ({e})"))
            .and_then(|sc| {
                gs.register(sc)
                    .map_err(|e| format!("Couldn't use {hk}: another app may already be using it ({e})"))
            })
    };
    if let Err(e) = &result {
        log(&format!("hotkey: {e}"));
    }
    *state.hotkey_error.lock().unwrap() = result.err();
}

fn toggle_auto_accept(app: &AppHandle) {
    let on = !current_settings(app).auto_accept;
    let _ = apply_settings(app, &serde_json::json!({ "autoAccept": on }));
    play_bytes(app, if on { SOUND_TOGGLE_ON } else { SOUND_TOGGLE_OFF });
    log(&format!("auto-accept {} (shortcut)", if on { "on" } else { "off" }));
}

fn sync_autostart(app: &AppHandle, want: bool) {
    if cfg!(debug_assertions) {
        return; // don't register a dev build to run at login
    }
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if al.is_enabled().unwrap_or(!want) != want {
        let r = if want { al.enable() } else { al.disable() };
        if let Err(e) = r {
            log(&format!("autostart: {e}"));
        }
    }
}

fn apply_settings(app: &AppHandle, patch: &serde_json::Value) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (before, after) = {
        let mut store = state.store.lock().unwrap();
        let before = store.get().clone();
        let after = store.update(patch).map_err(|e| e.to_string())?.clone();
        (before, after)
    };
    if before.launch_at_startup != after.launch_at_startup {
        sync_autostart(app, after.launch_at_startup);
    }
    if before.auto_accept_hotkey != after.auto_accept_hotkey {
        register_hotkey(app);
    }
    state.watcher.lock().unwrap().recompute(&after); // uses the active role's schedule
    refresh_tray(app, &current_status(app));
    if app.get_webview_window(WINDOW).is_some() {
        let _ = app.emit_to(WINDOW, "settings", &after);
    }
    Ok(())
}

fn open_folder(path: &std::path::Path) {
    let cmd = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    if let Err(e) = std::process::Command::new(cmd).arg(path).spawn() {
        log(&format!("could not open folder: {e}"));
    }
}

// ---------------------------------------------------------------- window

fn open_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("index.html".into()))
        .title("Minimap Nudge")
        .inner_size(560.0, 720.0)
        .min_inner_size(460.0, 560.0)
        .background_color(tauri::webview::Color(15, 20, 25, 255))
        .build();
    if let Err(e) = built {
        log(&format!("could not open settings window: {e}"));
    }
}

// ---------------------------------------------------------------- tray

fn tray_icon(kind: IconKind) -> Image<'static> {
    let bytes: &[u8] = match kind {
        IconKind::InGame => include_bytes!("../icons/tray-active.png"),
        IconKind::Client => include_bytes!("../icons/tray-client.png"),
        IconKind::Idle => include_bytes!("../icons/tray-idle.png"),
    };
    Image::from_bytes(bytes).expect("tray icon")
}

fn coarse_line(st: &Status) -> String {
    if st.state == State::Active {
        format!("In game · next ping {}", format_clock(st.next_alert))
    } else {
        st.line.clone()
    }
}

fn refresh_tray(app: &AppHandle, st: &Status) {
    let settings = current_settings(app);
    let enabled = settings.enabled;
    let state = app.state::<AppState>();
    let mut guard = state.tray.lock().unwrap();
    let Some(ui) = guard.as_mut() else { return };

    let role_note = match (st.role.as_deref().and_then(role_name), st.role_schedule) {
        (Some(r), true) => format!(" · {r} schedule"),
        _ => String::new(),
    };
    let tip: String = format!("Minimap Nudge — {}{role_note}", st.line).chars().take(127).collect();
    let _ = ui.tray.set_tooltip(Some(&tip));

    let kind = match st.state {
        State::Active => IconKind::InGame,
        State::Client | State::Spectating => IconKind::Client,
        _ => IconKind::Idle,
    };
    if ui.icon_kind != Some(kind) {
        let _ = ui.tray.set_icon(Some(tray_icon(kind)));
        ui.icon_kind = Some(kind);
    }

    let coarse = coarse_line(st);
    let role_label = match (st.role.as_deref().and_then(role_name), st.role_source) {
        (Some(r), Some("manual")) => format!("Role: {r}"),
        (Some(r), _) => format!("Role: Auto ({r})"),
        _ => "Role: Auto".to_string(),
    };
    let accept_label = if settings.auto_accept_hotkey.is_empty() {
        "Auto-accept matches".to_string()
    } else {
        format!("Auto-accept matches ({})", settings.auto_accept_hotkey)
    };
    let ban_label = "Auto-hover last ban".to_string();
    let key = format!(
        "{coarse}|{enabled}|{}|{role_label}|{}|{accept_label}|{}|{ban_label}",
        settings.role_mode, settings.auto_accept, settings.auto_hover_ban
    );
    if key != ui.last_key {
        let _ = ui.ban_item.set_checked(settings.auto_hover_ban);
        let _ = ui.ban_item.set_text(&ban_label);
        let _ = ui.accept_item.set_checked(settings.auto_accept);
        let _ = ui.accept_item.set_text(&accept_label);
        let _ = ui.status_item.set_text(&coarse);
        let _ = ui.enabled_item.set_checked(enabled);
        let _ = ui.role_menu.set_text(&role_label);
        for (mode, item) in &ui.role_items {
            let _ = item.set_checked(*mode == settings.role_mode);
        }
        ui.last_key = key;
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<TrayUi> {
    let enabled = current_settings(app).enabled;
    let status_item = MenuItem::with_id(app, "status", "Waiting for League", false, None::<&str>)?;
    let enabled_item = CheckMenuItem::with_id(app, "enabled", "Alerts enabled", true, enabled, None::<&str>)?;
    let update_item = MenuItem::with_id(app, "update", "Check for updates", true, None::<&str>)?;
    let ban_item = CheckMenuItem::with_id(
        app,
        "autoban",
        "Auto-hover last ban",
        true,
        current_settings(app).auto_hover_ban,
        None::<&str>,
    )?;
    let accept_item = CheckMenuItem::with_id(
        app,
        "autoaccept",
        "Auto-accept matches",
        true,
        current_settings(app).auto_accept,
        None::<&str>,
    )?;
    let role_mode = current_settings(app).role_mode;
    let mut role_items = vec![(
        ROLE_AUTO.to_string(),
        CheckMenuItem::with_id(app, "role:auto", "Auto (from champ select)", true, role_mode == ROLE_AUTO, None::<&str>)?,
    )];
    for (id, name) in ROLES {
        role_items.push((
            id.to_string(),
            CheckMenuItem::with_id(app, format!("role:{id}"), *name, true, role_mode == *id, None::<&str>)?,
        ));
    }
    let role_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> =
        role_items.iter().map(|(_, i)| i as &dyn tauri::menu::IsMenuItem<Wry>).collect();
    let role_menu = Submenu::with_id_and_items(app, "role", "Role: Auto", true, &role_refs)?;
    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?,
            &enabled_item,
            &role_menu,
            &accept_item,
            &ban_item,
            &MenuItem::with_id(app, "test", "Test sound", true, None::<&str>)?,
            &MenuItem::with_id(app, "logs", "Open log folder", true, None::<&str>)?,
            &update_item,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(tray_icon(IconKind::Idle))
        .tooltip("Minimap Nudge — Waiting for League")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            id if id.starts_with("role:") => {
                let _ = apply_settings(app, &serde_json::json!({ "roleMode": &id[5..] }));
            }
            "settings" => open_window(app),
            "autoban" => {
                let state = app.state::<AppState>();
                let checked = state
                    .tray
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|ui| ui.ban_item.is_checked().ok())
                    .unwrap_or(false);
                let _ = apply_settings(app, &serde_json::json!({ "autoHoverBan": checked }));
            }
            "autoaccept" => {
                let state = app.state::<AppState>();
                let checked = state
                    .tray
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|ui| ui.accept_item.is_checked().ok())
                    .unwrap_or(false);
                let _ = apply_settings(app, &serde_json::json!({ "autoAccept": checked }));
            }
            "enabled" => {
                let state = app.state::<AppState>();
                let checked = state
                    .tray
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|ui| ui.enabled_item.is_checked().ok())
                    .unwrap_or(true);
                let _ = apply_settings(app, &serde_json::json!({ "enabled": checked }));
            }
            "test" => play(app, None),
            "logs" => open_folder(&app.state::<AppState>().config_dir),
            "update" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if app.state::<Updates>().status().phase == UpdatePhase::Available {
                        let _ = updates::install(&app).await;
                    } else {
                        open_window(&app);
                        updates::check(&app).await;
                    }
                });
            }
            "quit" => {
                log("quit from tray");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                open_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(TrayUi {
        tray,
        status_item,
        enabled_item,
        update_item,
        accept_item,
        ban_item,
        role_menu,
        role_items,
        last_key: String::new(),
        icon_kind: Some(IconKind::Idle),
    })
}

/// Keeps the tray's update item in step with the updater.
pub(crate) fn on_update_status(app: &AppHandle, st: &UpdateStatus) {
    let text = match st.phase {
        UpdatePhase::Available => format!("Update to {}…", st.version.as_deref().unwrap_or("new version")),
        UpdatePhase::Checking => "Checking for updates…".into(),
        UpdatePhase::Downloading | UpdatePhase::Installing => "Updating…".into(),
        _ => "Check for updates".into(),
    };
    if let Some(ui) = app.state::<AppState>().tray.lock().unwrap().as_ref() {
        let _ = ui.update_item.set_text(&text);
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_state(app: AppHandle) -> PublicState {
    public_state(&app, None)
}

#[tauri::command]
fn update_settings(app: AppHandle, patch: serde_json::Value) -> Result<PublicState, String> {
    apply_settings(&app, &patch)?;
    Ok(public_state(&app, None))
}

#[tauri::command]
async fn add_sound(app: AppHandle) -> Result<PublicState, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog_app = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut d = dialog_app.dialog().file().set_title("Choose an alert sound").add_filter("Audio", AUDIO_EXTS);
        if let Some(w) = dialog_app.get_webview_window(WINDOW) {
            d = d.set_parent(&w);
        }
        d.blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(src) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(public_state(&app, None)); // cancelled
    };
    let error = |msg: &str| Ok(public_state(&app, Some(msg.to_string())));

    let ext = src.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    if !AUDIO_EXTS.contains(&ext.as_str()) {
        return error("Unsupported file type.");
    }
    if std::fs::metadata(&src).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_SOUND_BYTES {
        return error("That file is over 10 MB — pick a short clip.");
    }
    if let Err(e) = audio::decode_file(&src) {
        log(&format!("rejected sound {}: {e}", src.display()));
        return error("Couldn't read that file as audio.");
    }

    let uuid = uuid::Uuid::new_v4().to_string();
    let dest = app.state::<AppState>().store.lock().unwrap().sounds_dir.join(format!("{uuid}.{ext}"));
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    let name: String = src.file_stem().and_then(|s| s.to_str()).unwrap_or("Custom sound").chars().take(40).collect();
    let id = format!("custom:{uuid}");

    let mut customs = current_settings(&app).custom_sounds;
    customs.push(CustomSound { id: id.clone(), name, file: dest });
    apply_settings(&app, &serde_json::json!({ "customSounds": customs, "soundId": id }))?;
    log(&format!("added custom sound {}", src.display()));
    Ok(public_state(&app, None))
}

#[tauri::command]
fn remove_sound(app: AppHandle, id: String) -> Result<PublicState, String> {
    let s = current_settings(&app);
    if let Some(c) = s.custom_sounds.iter().find(|c| c.id == id) {
        let _ = std::fs::remove_file(&c.file);
    }
    let customs: Vec<_> = s.custom_sounds.iter().filter(|c| c.id != id).cloned().collect();
    let sound_id = if s.sound_id == id { DEFAULT_SOUND.to_string() } else { s.sound_id.clone() };
    apply_settings(&app, &serde_json::json!({ "customSounds": customs, "soundId": sound_id }))?;
    Ok(public_state(&app, None))
}

#[tauri::command]
fn preview(app: AppHandle, id: Option<String>) {
    play(&app, id.as_deref());
}

/// The settings page is recording a new shortcut: release the current one meanwhile.
#[tauri::command]
fn set_hotkey_capture(app: AppHandle, on: bool) -> PublicState {
    *app.state::<AppState>().hotkey_capturing.lock().unwrap() = on;
    register_hotkey(&app);
    public_state(&app, None)
}

#[tauri::command]
async fn check_update(app: AppHandle) -> UpdateStatus {
    updates::check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    updates::install(&app).await
}

// ---------------------------------------------------------------- watcher loop

fn spawn_watcher(app: AppHandle) {
    let base = std::env::var("MINIMAP_NUDGE_API").unwrap_or_else(|_| league::DEFAULT_BASE.to_string());
    tauri::async_runtime::spawn(async move {
        let client = LeagueClient::new(base);
        let lcu = lcu::Lcu::from_env();
        let mut tick = tokio::time::interval(watcher::POLL_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_state = None;
        let mut acceptor = autoaccept::AutoAccept::new();
        let mut bans = banassist::BanAssist::new();
        loop {
            tick.tick().await;
            let settings = current_settings(&app);
            let state = app.state::<AppState>();
            let (events, mut status) = watcher::poll_once(&client, &lcu, &state.watcher, &settings).await;

            // Match found: auto-accept if it's on.
            let in_ready_check = status.client.as_ref().is_some_and(|c| c.phase == lcu::Phase::ReadyCheck);
            let rc = if in_ready_check { lcu.ready_check().await } else { None };
            let (action, mut rc_status) =
                acceptor.step(rc.as_ref(), settings.auto_accept, settings.auto_accept_delay, std::time::Instant::now());
            if action == autoaccept::Action::Accept {
                let ok = lcu.accept_ready_check().await;
                acceptor.accepted(ok);
                let queue = status.client.as_ref().and_then(|c| c.queue.clone()).unwrap_or_default();
                log(&format!("auto-accept: {} {queue}", if ok { "accepted match" } else { "accept failed for" }));
                if let Some(r) = rc_status.as_mut() {
                    r.auto_accepted = ok;
                }
            }
            *state.ready_check.lock().unwrap() = rc_status.clone();
            status.ready_check = rc_status;
            // Champ select: remember your ban, and hover it next time if that's on.
            let in_champ_select = status.client.as_ref().is_some_and(|c| c.phase == lcu::Phase::ChampSelect);
            if !in_champ_select {
                if bans.hovered.is_some() || state.ban_hovered.lock().unwrap().is_some() {
                    *state.ban_hovered.lock().unwrap() = None;
                }
                bans.reset();
            } else if let Some(session) = lcu.champ_select_session().await {
                // Bans are remembered per game mode + role.
                let queue_id = status.client.as_ref().and_then(|c| c.queue_id);
                let role = session.my_position();
                let key = settings::ban_key(queue_id, role.as_deref());
                let remembered = settings.last_bans.get(&key);
                if let Some(champ) = bans.my_completed_ban(&session) {
                    if remembered.map(|b| b.champion_id) != Some(champ) {
                        let name = lcu.champion_name(champ).await.unwrap_or_else(|| format!("Champion {champ}"));
                        let queue = status.client.as_ref().and_then(|c| c.queue.clone()).unwrap_or_default();
                        let role = role.clone().unwrap_or_default();
                        log(&format!("remembered ban: {name} ({key})"));
                        let mut all = settings.last_bans.clone();
                        all.insert(key.clone(), settings::LastBan { champion_id: champ, name, queue, role });
                        let _ = apply_settings(&app, &serde_json::json!({ "lastBans": all }));
                    }
                }
                let last = remembered.map(|b| b.champion_id);
                if let banassist::Decision::Hover { action_id, champion_id } =
                    bans.step(&session, settings.auto_hover_ban, last)
                {
                    let ok = lcu.hover_champion(action_id, champion_id).await;
                    bans.hovered(champion_id, ok);
                    let name = remembered.map(|b| b.name.clone()).unwrap_or_default();
                    log(&format!("auto-hover ban: {} {name}", if ok { "hovered" } else { "couldn't hover" }));
                    if ok {
                        *state.ban_hovered.lock().unwrap() = Some(name);
                    }
                }
                if bans.hovered.is_none() {
                    // a hover the client dropped
                    *state.ban_hovered.lock().unwrap() = None;
                }
            }
            status.ban_hovered = state.ban_hovered.lock().unwrap().clone();

            if let Some(r) = &status.ready_check {
                if r.auto_accepted || r.response == "accepted" {
                    status.line = "Match found — accepted".into();
                } else if let Some(left) = r.accept_in {
                    status.line = format!("Match found — auto-accepting in {}s", left.ceil() as u64);
                }
            }

            for ev in events {
                match ev {
                    Event::GameDetected { game_time, mode } => {
                        let schedule = match (status.role.as_deref().and_then(role_name), status.role_schedule) {
                            (Some(r), true) => format!("{r} schedule"),
                            (Some(r), false) => format!("default schedule; role {r}"),
                            _ => "default schedule".to_string(),
                        };
                        log(&format!("game detected: {mode} at {} ({schedule})", format_clock(Some(game_time))))
                    }
                    Event::Ping { game_time, due } => {
                        log(&format!("ping at {} (due {})", format_clock(Some(game_time)), format_clock(Some(due))));
                        play(&app, None);
                    }
                    Event::SkippedLate { due, game_time } => log(&format!(
                        "skipped ping due {} (woke at {})",
                        format_clock(Some(due)),
                        format_clock(Some(game_time))
                    )),
                    Event::GameEnded => log("game ended"),
                    Event::Spectating(true) => log("replay/spectator game: staying quiet"),
                    Event::Spectating(false) => log("your game after all: pings on"),
                    Event::GameClosed => log("game closed"),
                    Event::ClientPhase { phase, queue } => match phase {
                        Some(p) => log(&format!(
                            "client: {p:?}{}",
                            queue.map(|q| format!(" ({q})")).unwrap_or_default()
                        )),
                        None => log("client: not running"),
                    },
                }
            }
            if last_state != Some(status.state) {
                log(&format!("status: {}", status.line));
                last_state = Some(status.state);
            }
            refresh_tray(&app, &status);
            if app.get_webview_window(WINDOW).is_some() {
                let _ = app.emit_to(WINDOW, "status", &status);
            }
        }
    });
}

// ---------------------------------------------------------------- boot

pub fn run() {
    let start_hidden = std::env::args().any(|a| a == "--hidden");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_auto_accept(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_state,
            update_settings,
            add_sound,
            remove_sound,
            preview,
            check_update,
            install_update,
            set_hotkey_capture
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let config_dir = app.path().app_config_dir()?;
            let log_path = logger::init(&config_dir);
            let store = SettingsStore::open(&config_dir)?;
            log(&format!("Minimap Nudge {} starting (log: {})", env!("CARGO_PKG_VERSION"), log_path.display()));

            app.manage(Updates::new(&app.package_info().version.to_string()));
            app.manage(AppState {
                store: Mutex::new(store),
                watcher: Mutex::new(Watcher::new()),
                audio: AudioPlayer::spawn(),
                config_dir,
                tray: Mutex::new(None),
                ready_check: Mutex::new(None),
                ban_hovered: Mutex::new(None),
                hotkey_error: Mutex::new(None),
                hotkey_capturing: Mutex::new(false),
            });
            register_hotkey(&handle);

            let tray_ok = match build_tray(&handle) {
                Ok(ui) => {
                    *app.state::<AppState>().tray.lock().unwrap() = Some(ui);
                    true
                }
                Err(e) => {
                    log(&format!("could not create tray icon: {e}"));
                    false
                }
            };
            sync_autostart(&handle, current_settings(&handle).launch_at_startup);

            // Without a tray icon there'd be no way to reach the app, so show the window.
            if !start_hidden || !tray_ok {
                open_window(&handle);
            }
            updates::spawn_periodic(handle.clone());
            spawn_watcher(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Minimap Nudge");

    app.run(|_app, event| {
        // Closing the settings window destroys it (that's the memory saving);
        // keep running in the tray until "Quit" calls app.exit().
        if let RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
}
