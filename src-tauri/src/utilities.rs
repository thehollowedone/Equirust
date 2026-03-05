use arboard::{Clipboard, ImageData};
use serde::Serialize;
use std::{borrow::Cow, collections::BTreeMap, io};

#[tauri::command]
pub fn copy_image_to_clipboard(bytes: Vec<u8>) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("no image bytes were provided".into());
    }

    let decoded = image::load_from_memory(&bytes).map_err(|err| err.to_string())?;
    let rgba = decoded.to_rgba8();
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;
    let pixels = rgba.into_raw();

    let mut clipboard = Clipboard::new().map_err(|err| err.to_string())?;
    clipboard
        .set_image(ImageData {
            width,
            height,
            bytes: Cow::Owned(pixels),
        })
        .map_err(|err| err.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SystemThemeValues(pub BTreeMap<String, String>);

#[tauri::command]
pub fn get_system_theme_values() -> Result<SystemThemeValues, String> {
    Ok(SystemThemeValues(read_system_theme_values()))
}

#[tauri::command]
pub fn open_debug_page(target: String) -> Result<(), String> {
    let (primary_url, fallback_url) = match target.as_str() {
        "gpu" => ("edge://gpu", "chrome://gpu"),
        "webrtc-internals" => ("edge://webrtc-internals", "chrome://webrtc-internals"),
        _ => return Err(format!("unsupported debug target: {target}")),
    };

    open_debug_url(primary_url)
        .or_else(|_| open_debug_url(fallback_url))
        .map_err(|err| err.to_string())
}

fn open_debug_url(url: &str) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map(|_| ());
    }

    #[cfg(not(target_os = "windows"))]
    {
        return webbrowser::open(url)
            .map(|_| ())
            .map_err(|err| io::Error::other(err.to_string()));
    }
}

fn read_system_theme_values() -> BTreeMap<String, String> {
    let mut values = BTreeMap::from([("os-accent-color".into(), "#5865f2".into())]);

    #[cfg(target_os = "windows")]
    {
        if let Some(accent) = read_windows_accent_color() {
            values.insert("os-accent-color".into(), accent);
        }
    }

    values
}

#[cfg(target_os = "windows")]
fn read_windows_accent_color() -> Option<String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let explorer = hkcu
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent")
        .ok();
    if let Some(explorer) = explorer {
        if let Ok(value) = explorer.get_value::<u32, _>("AccentColorMenu") {
            return Some(format_windows_accent_color(value));
        }
    }

    let dwm = hkcu.open_subkey("Software\\Microsoft\\Windows\\DWM").ok();
    if let Some(dwm) = dwm {
        if let Ok(value) = dwm.get_value::<u32, _>("AccentColor") {
            return Some(format_windows_accent_color(value));
        }
        if let Ok(value) = dwm.get_value::<u32, _>("ColorizationColor") {
            return Some(format_windows_accent_color(value));
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn format_windows_accent_color(value: u32) -> String {
    let red = (value & 0xff) as u8;
    let green = ((value >> 8) & 0xff) as u8;
    let blue = ((value >> 16) & 0xff) as u8;
    format!("#{red:02x}{green:02x}{blue:02x}")
}
