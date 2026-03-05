use crate::{file_manager, privacy, settings::Settings, store::PersistedStore};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuEvent},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

const TRAY_ID: &str = "main-tray";
const MENU_OPEN: &str = "tray-open";
const MENU_RESTART: &str = "tray-restart";
const MENU_QUIT: &str = "tray-quit";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayVariant {
    Tray,
    TrayUnread,
    TraySpeaking,
    TrayIdle,
    TrayMuted,
    TrayDeafened,
}

#[derive(Clone, Copy, Debug)]
struct VisualState {
    unread_active: bool,
    in_voice_call: bool,
    current_variant: TrayVariant,
}

impl Default for VisualState {
    fn default() -> Self {
        Self {
            unread_active: false,
            in_voice_call: false,
            current_variant: TrayVariant::Tray,
        }
    }
}

pub struct RuntimeState {
    is_quitting: AtomicBool,
    visual: Mutex<VisualState>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            is_quitting: AtomicBool::new(false),
            visual: Mutex::new(VisualState::default()),
        }
    }
}

pub fn is_tray_enabled(settings: &Settings) -> bool {
    !cfg!(target_os = "macos") && settings.tray != Some(false)
}

pub fn should_minimize_to_tray(app: &AppHandle) -> bool {
    if cfg!(target_os = "macos") || is_quitting(app) {
        return false;
    }

    let snapshot = app.state::<PersistedStore>().snapshot();
    is_tray_enabled(&snapshot.settings) && snapshot.settings.minimize_to_tray != Some(false)
}

pub fn mark_quitting(app: &AppHandle) {
    if let Some(state) = app.try_state::<RuntimeState>() {
        state.is_quitting.store(true, Ordering::SeqCst);
    }
}

pub fn is_quitting(app: &AppHandle) -> bool {
    app.try_state::<RuntimeState>()
        .map(|state| state.is_quitting.load(Ordering::SeqCst))
        .unwrap_or(false)
}

pub fn sync(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    if is_tray_enabled(settings) {
        if app.tray_by_id(TRAY_ID).is_none() {
            create_tray(app)?;
        } else {
            sync_tray_icon(app)?;
        }
    } else {
        let _ = app.remove_tray_by_id(TRAY_ID);
    }

    Ok(())
}

pub fn restore_main_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is not available".to_string())?;

    if window.is_minimized().map_err(|err| err.to_string())? {
        window.unminimize().map_err(|err| err.to_string())?;
    }

    window.show().map_err(|err| err.to_string())?;
    let _ = window.set_skip_taskbar(false);
    let _ = window.set_focus();
    Ok(())
}

pub fn hide_main_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is not available".to_string())?;

    let _ = window.set_skip_taskbar(true);
    window.hide().map_err(|err| err.to_string())
}

fn toggle_main_window(app: &AppHandle) -> Result<(), String> {
    let snapshot = app.state::<PersistedStore>().snapshot();
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is not available".to_string())?;

    let visible = window.is_visible().map_err(|err| err.to_string())?;
    let minimize_on_click = snapshot.settings.click_tray_to_show_hide == Some(true);

    if visible && minimize_on_click {
        hide_main_window(app)
    } else {
        restore_main_window(app)
    }
}

fn create_tray(app: &AppHandle) -> Result<(), String> {
    let menu = MenuBuilder::new(app)
        .text(MENU_OPEN, "Open Equirust")
        .separator()
        .text(MENU_RESTART, "Restart")
        .text(MENU_QUIT, "Quit")
        .build()
        .map_err(|err| err.to_string())?;

    let icon = load_tray_image(app, current_tray_variant(app)?)?;

    let app_handle = app.clone();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Equirust")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event);
        })
        .on_tray_icon_event(move |_tray, event| {
            handle_tray_event(&app_handle, event);
        })
        .build(app)
        .map(|_| {
            let _ = sync_tray_icon(app);
        })
        .map_err(|err: tauri::Error| err.to_string())
}

fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        MENU_OPEN => {
            let _ = restore_main_window(app);
        }
        MENU_RESTART => {
            mark_quitting(app);
            app.restart();
        }
        MENU_QUIT => {
            mark_quitting(app);
            app.exit(0);
        }
        _ => {}
    }
}

fn handle_tray_event(app: &AppHandle, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        let _ = toggle_main_window(app);
    }
}

#[tauri::command]
pub fn set_tray_voice_state(variant: String, app: AppHandle) -> Result<(), String> {
    let parsed_variant = parse_voice_variant(&variant)
        .ok_or_else(|| format!("unsupported tray voice variant: {variant}"))?;

    update_visual_state(&app, |visual| {
        visual.in_voice_call = true;
        visual.current_variant = parsed_variant;
    })?;
    sync_tray_icon(&app)
}

#[tauri::command]
pub fn set_tray_voice_call_state(in_call: bool, app: AppHandle) -> Result<(), String> {
    update_visual_state(&app, |visual| {
        visual.in_voice_call = in_call;
        if !in_call {
            visual.current_variant = if visual.unread_active {
                TrayVariant::TrayUnread
            } else {
                TrayVariant::Tray
            };
        } else if !matches!(
            visual.current_variant,
            TrayVariant::TraySpeaking
                | TrayVariant::TrayIdle
                | TrayVariant::TrayMuted
                | TrayVariant::TrayDeafened
        ) {
            visual.current_variant = TrayVariant::TrayIdle;
        }
    })?;
    sync_tray_icon(&app)
}

pub fn sync_unread_badge(app: &AppHandle, has_unread: bool) -> Result<(), String> {
    update_visual_state(app, |visual| {
        visual.unread_active = has_unread;
        if !visual.in_voice_call {
            visual.current_variant = if has_unread {
                TrayVariant::TrayUnread
            } else {
                TrayVariant::Tray
            };
        }
    })?;
    sync_tray_icon(app)
}

fn update_visual_state(
    app: &AppHandle,
    update: impl FnOnce(&mut VisualState),
) -> Result<(), String> {
    let state = app
        .try_state::<RuntimeState>()
        .ok_or_else(|| "tray runtime state is not available".to_string())?;
    let mut visual = state
        .visual
        .lock()
        .map_err(|_| "tray visual state lock was poisoned".to_string())?;
    update(&mut visual);
    Ok(())
}

fn current_tray_variant(app: &AppHandle) -> Result<TrayVariant, String> {
    let state = app
        .try_state::<RuntimeState>()
        .ok_or_else(|| "tray runtime state is not available".to_string())?;
    let visual = state
        .visual
        .lock()
        .map_err(|_| "tray visual state lock was poisoned".to_string())?;
    Ok(visual.current_variant)
}

fn sync_tray_icon(app: &AppHandle) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };

    let variant = current_tray_variant(app)?;
    tray.set_icon(Some(load_tray_image(app, variant)?))
        .map_err(|err| err.to_string())?;
    tray.set_tooltip(Some(tray_tooltip(variant)))
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn parse_voice_variant(variant: &str) -> Option<TrayVariant> {
    match variant {
        "traySpeaking" => Some(TrayVariant::TraySpeaking),
        "trayIdle" => Some(TrayVariant::TrayIdle),
        "trayMuted" => Some(TrayVariant::TrayMuted),
        "trayDeafened" => Some(TrayVariant::TrayDeafened),
        _ => None,
    }
}

fn tray_tooltip(variant: TrayVariant) -> &'static str {
    match variant {
        TrayVariant::Tray => "Equirust",
        TrayVariant::TrayUnread => "Equirust • Unread Activity",
        TrayVariant::TraySpeaking => "Equirust • Speaking",
        TrayVariant::TrayIdle => "Equirust • In Voice Call",
        TrayVariant::TrayMuted => "Equirust • Muted",
        TrayVariant::TrayDeafened => "Equirust • Deafened",
    }
}

fn load_tray_image(app: &AppHandle, variant: TrayVariant) -> Result<Image<'static>, String> {
    let snapshot = app.state::<PersistedStore>().snapshot();
    if let Some(custom_path) =
        file_manager::resolve_user_asset_path(&snapshot.paths, tray_asset_name(variant))
    {
        match Image::from_path(&custom_path) {
            Ok(image) => return Ok(image),
            Err(err) => {
                log::warn!(
                    "Failed to load custom tray asset {}: {}",
                    privacy::file_name_for_log(&custom_path),
                    err
                );
            }
        }
    }

    let bytes: &[u8] = match variant {
        TrayVariant::Tray => include_bytes!("../../static/tray/tray.png"),
        TrayVariant::TrayUnread => include_bytes!("../../static/tray/trayUnread.png"),
        TrayVariant::TraySpeaking => include_bytes!("../../static/tray/speaking.png"),
        TrayVariant::TrayIdle => include_bytes!("../../static/tray/idle.png"),
        TrayVariant::TrayMuted => include_bytes!("../../static/tray/muted.png"),
        TrayVariant::TrayDeafened => include_bytes!("../../static/tray/deafened.png"),
    };

    Image::from_bytes(bytes).map_err(|err| err.to_string())
}

fn tray_asset_name(variant: TrayVariant) -> &'static str {
    match variant {
        TrayVariant::Tray => "tray",
        TrayVariant::TrayUnread => "trayUnread",
        TrayVariant::TraySpeaking => "traySpeaking",
        TrayVariant::TrayIdle => "trayIdle",
        TrayVariant::TrayMuted => "trayMuted",
        TrayVariant::TrayDeafened => "trayDeafened",
    }
}
