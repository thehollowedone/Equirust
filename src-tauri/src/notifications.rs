use crate::tray;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{image::Image, AppHandle, Manager, State, UserAttentionType, Window};

#[derive(Default)]
pub struct RuntimeState {
    flashing: AtomicBool,
    last_badge_index: Mutex<Option<u8>>,
}

#[tauri::command]
pub fn set_badge_count(
    count: i64,
    app: AppHandle,
    runtime_state: State<'_, RuntimeState>,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is not available".to_string())?;

    let has_unread = count != 0;
    tray::sync_unread_badge(&app, has_unread)?;

    #[cfg(target_os = "windows")]
    {
        let next_badge_index = badge_index_for_count(count);
        let mut last_badge_index = runtime_state
            .last_badge_index
            .lock()
            .map_err(|_| "notification state lock was poisoned".to_string())?;

        if *last_badge_index != next_badge_index {
            let overlay_icon = next_badge_index
                .map(load_badge_image)
                .transpose()
                .map_err(|err| err.to_string())?;
            window
                .set_overlay_icon(overlay_icon)
                .map_err(|err| err.to_string())?;
            *last_badge_index = next_badge_index;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let platform_count = if count < 0 { Some(1) } else { Some(count) };
        window
            .set_badge_count(platform_count.filter(|value| *value > 0))
            .map_err(|err| err.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub fn flash_frame(
    flag: bool,
    window: Window,
    runtime_state: State<'_, RuntimeState>,
) -> Result<(), String> {
    let already_flashing = runtime_state.flashing.load(Ordering::SeqCst);
    if already_flashing == flag {
        return Ok(());
    }

    let attention = if flag {
        Some(UserAttentionType::Critical)
    } else {
        None
    };

    window
        .request_user_attention(attention)
        .map_err(|err| err.to_string())?;
    runtime_state.flashing.store(flag, Ordering::SeqCst);
    Ok(())
}

#[cfg(target_os = "windows")]
fn badge_index_for_count(count: i64) -> Option<u8> {
    match count {
        i64::MIN..=-1 => Some(11),
        0 => None,
        1..=9 => Some(count as u8),
        _ => Some(10),
    }
}

#[cfg(target_os = "windows")]
fn load_badge_image(index: u8) -> tauri::Result<Image<'static>> {
    let bytes: &[u8] = match index {
        1 => include_bytes!("../../static/badges/1.ico"),
        2 => include_bytes!("../../static/badges/2.ico"),
        3 => include_bytes!("../../static/badges/3.ico"),
        4 => include_bytes!("../../static/badges/4.ico"),
        5 => include_bytes!("../../static/badges/5.ico"),
        6 => include_bytes!("../../static/badges/6.ico"),
        7 => include_bytes!("../../static/badges/7.ico"),
        8 => include_bytes!("../../static/badges/8.ico"),
        9 => include_bytes!("../../static/badges/9.ico"),
        10 => include_bytes!("../../static/badges/10.ico"),
        11 => include_bytes!("../../static/badges/11.ico"),
        _ => include_bytes!("../../static/badges/10.ico"),
    };

    Image::from_bytes(bytes)
}
