use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use image::{imageops::thumbnail, DynamicImage, ImageFormat, RgbaImage};
use serde::Serialize;
use std::io::Cursor;
use xcap::{Monitor, Window};

const PLACEHOLDER_THUMBNAIL_DATA_URL: &str =
    "data:image/gif;base64,R0lGODlhAQABAPAAAAAAAAAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturerSource {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub url: String,
    pub process_id: Option<u32>,
    pub process_name: Option<String>,
    pub native_width: Option<u32>,
    pub native_height: Option<u32>,
    pub max_frame_rate: Option<u32>,
}

#[tauri::command]
pub fn get_capturer_sources() -> Result<Vec<CapturerSource>, String> {
    list_sources()
}

#[tauri::command]
pub fn get_capturer_thumbnail(id: String) -> Result<String, String> {
    encode_source_thumbnail(&id, 176, 99)
}

#[tauri::command]
pub fn get_capturer_large_thumbnail(id: String) -> Result<String, String> {
    encode_source_thumbnail(&id, 1920, 1080)
}

fn list_sources() -> Result<Vec<CapturerSource>, String> {
    let mut sources = Vec::new();

    let monitors = Monitor::all().map_err(|err| err.to_string())?;
    for (index, monitor) in monitors.into_iter().enumerate() {
        let Some(id) = monitor.id().ok() else {
            continue;
        };
        let name = monitor
            .name()
            .ok()
            .and_then(|value| {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value)
                }
            })
            .unwrap_or_else(|| format!("Screen {}", index + 1));
        sources.push(CapturerSource {
            id: format!("screen:{id}"),
            name,
            kind: "screen".to_owned(),
            url: PLACEHOLDER_THUMBNAIL_DATA_URL.to_owned(),
            process_id: None,
            process_name: None,
            native_width: monitor.width().ok(),
            native_height: monitor.height().ok(),
            max_frame_rate: monitor.frequency().ok().map(|value| value.max(1.0).round() as u32),
        });
    }

    let windows = Window::all().map_err(|err| err.to_string())?;
    for (index, window) in windows
        .into_iter()
        .filter(|entry| !entry.title().unwrap_or_default().trim().is_empty())
        .enumerate()
    {
        let Some(id) = window.id().ok() else {
            continue;
        };
        let name = window
            .title()
            .ok()
            .and_then(|value| {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value)
                }
            })
            .unwrap_or_else(|| format!("Window {}", index + 1));
        let process_id = window.pid().ok();
        let process_name = window
            .app_name()
            .ok()
            .filter(|value| !value.trim().is_empty());
        let native_width = window.width().ok();
        let native_height = window.height().ok();
        let max_frame_rate = window
            .current_monitor()
            .ok()
            .and_then(|monitor| monitor.frequency().ok())
            .map(|value| value.max(1.0).round() as u32);
        sources.push(CapturerSource {
            id: format!("window:{id}"),
            name,
            kind: "window".to_owned(),
            url: PLACEHOLDER_THUMBNAIL_DATA_URL.to_owned(),
            process_id,
            process_name,
            native_width,
            native_height,
            max_frame_rate,
        });
    }

    Ok(sources)
}

fn encode_source_thumbnail(id: &str, width: u32, height: u32) -> Result<String, String> {
    if let Some((kind, source_id)) = parse_source_id(id) {
        match kind {
            "screen" => {
                let monitors = Monitor::all().map_err(|err| err.to_string())?;
                let monitor = monitors
                    .into_iter()
                    .find(|entry| entry.id().ok() == Some(source_id))
                    .ok_or_else(|| format!("screen source not found: {id}"))?;
                return encode_thumbnail(
                    monitor.capture_image().map_err(|err| err.to_string())?,
                    width,
                    height,
                );
            }
            "window" => {
                let windows = Window::all().map_err(|err| err.to_string())?;
                let window = windows
                    .into_iter()
                    .filter(|entry| !entry.title().unwrap_or_default().trim().is_empty())
                    .find(|entry| entry.id().ok() == Some(source_id))
                    .ok_or_else(|| format!("window source not found: {id}"))?;
                return encode_thumbnail(
                    window.capture_image().map_err(|err| err.to_string())?,
                    width,
                    height,
                );
            }
            _ => {}
        }
    }

    Err(format!("unsupported capturer source id: {id}"))
}

fn parse_source_id(value: &str) -> Option<(&str, u32)> {
    let (kind, index) = value.split_once(':')?;
    Some((kind, index.parse().ok()?))
}

fn encode_thumbnail(image: RgbaImage, width: u32, height: u32) -> Result<String, String> {
    let resized = thumbnail(&image, width.max(1), height.max(1));
    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    DynamicImage::ImageRgba8(resized)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|err| err.to_string())?;

    Ok(format!("data:image/png;base64,{}", BASE64.encode(bytes)))
}
