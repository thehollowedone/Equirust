use super::types::ResolvedCaptureSource;
use image::{
    imageops::{overlay, resize, FilterType},
    codecs::jpeg::JpegEncoder,
    DynamicImage, Rgba, RgbaImage,
};
use std::{io::Cursor, sync::mpsc::Receiver};
use xcap::{Frame, Monitor, VideoRecorder, Window};
#[cfg(windows)]
use windows::{
    Win32::Foundation::HWND,
    Win32::UI::WindowsAndMessaging::IsWindow,
};

pub fn resolve_source(source_id: &str) -> Result<ResolvedCaptureSource, String> {
    let (kind, native_id) = parse_source_id(source_id)
        .ok_or_else(|| format!("Unsupported native capture source id: {source_id}"))?;

    match kind {
        "screen" => {
            let monitors = Monitor::all().map_err(|err| err.to_string())?;
            let monitor = monitors
                .into_iter()
                .find(|entry| entry.id().ok() == Some(native_id))
                .ok_or_else(|| format!("Screen source not found: {source_id}"))?;
            let display_name = monitor
                .name()
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("Screen {native_id}"));
            Ok(ResolvedCaptureSource {
                source_id: source_id.to_owned(),
                source_kind: "screen".to_owned(),
                native_id,
                display_name,
                process_id: None,
            })
        }
        "window" => {
            let windows = Window::all().map_err(|err| err.to_string())?;
            let window = windows
                .into_iter()
                .filter(|entry| !entry.title().unwrap_or_default().trim().is_empty())
                .find(|entry| entry.id().ok() == Some(native_id))
                .ok_or_else(|| format!("Window source not found: {source_id}"))?;
            let display_name = window
                .title()
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("Window {native_id}"));
            let process_id = window.pid().ok();
            Ok(ResolvedCaptureSource {
                source_id: source_id.to_owned(),
                source_kind: "window".to_owned(),
                native_id,
                display_name,
                process_id,
            })
        }
        _ => Err(format!("Unsupported native capture source kind: {kind}")),
    }
}

pub struct PreparedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn capture_frame_rgba(
    source: &ResolvedCaptureSource,
    target_width: u32,
    target_height: u32,
) -> Result<PreparedVideoFrame, String> {
    let frame = capture_source_image(source)?;
    prepare_rgba_frame(
        frame.width(),
        frame.height(),
        frame.into_raw(),
        target_width,
        target_height,
    )
}

pub fn source_is_alive(source: &ResolvedCaptureSource) -> bool {
    match source.source_kind.as_str() {
        "window" => window_handle_is_alive(source.native_id),
        _ => true,
    }
}

pub fn prepare_rgba_frame(
    frame_width: u32,
    frame_height: u32,
    frame_raw: Vec<u8>,
    target_width: u32,
    target_height: u32,
) -> Result<PreparedVideoFrame, String> {
    let frame = RgbaImage::from_raw(frame_width, frame_height, frame_raw)
        .ok_or_else(|| "Native capture frame had an invalid RGBA buffer".to_owned())?;
    let target_width = normalize_even_dimension(target_width.max(1));
    let target_height = normalize_even_dimension(target_height.max(1));
    let composited = letterbox_image(&frame, target_width, target_height);

    Ok(PreparedVideoFrame {
        width: composited.width(),
        height: composited.height(),
        rgba: composited.into_raw(),
    })
}

pub fn encode_rgba_frame_jpeg(
    frame_width: u32,
    frame_height: u32,
    frame_raw: Vec<u8>,
    target_width: u32,
    target_height: u32,
    jpeg_quality: u8,
) -> Result<Vec<u8>, String> {
    let prepared = prepare_rgba_frame(
        frame_width,
        frame_height,
        frame_raw,
        target_width,
        target_height,
    )?;
    let composited = RgbaImage::from_raw(prepared.width, prepared.height, prepared.rgba)
        .ok_or_else(|| "Native capture frame had an invalid RGBA buffer".to_owned())?;
    let rgb = DynamicImage::ImageRgba8(composited).to_rgb8();
    let mut buffer = Cursor::new(Vec::new());
    let mut encoder = JpegEncoder::new_with_quality(&mut buffer, jpeg_quality.max(30));
    encoder
        .encode_image(&DynamicImage::ImageRgb8(rgb))
        .map_err(|err| err.to_string())?;
    Ok(buffer.into_inner())
}

pub fn start_screen_video_recorder(
    source: &ResolvedCaptureSource,
) -> Result<(VideoRecorder, Receiver<Frame>), String> {
    if source.source_kind != "screen" {
        return Err(format!(
            "Native video recorder is only supported for screen sources: {}",
            source.source_id
        ));
    }

    let monitors = Monitor::all().map_err(|err| err.to_string())?;
    let monitor = monitors
        .into_iter()
        .find(|entry| entry.id().ok() == Some(source.native_id))
        .ok_or_else(|| format!("Screen source not found: {}", source.source_id))?;
    monitor.video_recorder().map_err(|err| err.to_string())
}

fn capture_source_image(source: &ResolvedCaptureSource) -> Result<RgbaImage, String> {
    match source.source_kind.as_str() {
        "screen" => {
            let monitors = Monitor::all().map_err(|err| err.to_string())?;
            let monitor = monitors
                .into_iter()
                .find(|entry| entry.id().ok() == Some(source.native_id))
                .ok_or_else(|| format!("Screen source not found: {}", source.source_id))?;
            monitor.capture_image().map_err(|err| err.to_string())
        }
        "window" => {
            if !window_handle_is_alive(source.native_id) {
                return Err(format!("Window source was closed: {}", source.source_id));
            }

            let windows = Window::all().map_err(|err| err.to_string())?;
            let Some(window) = windows
                .into_iter()
                .find(|entry| entry.id().ok() == Some(source.native_id))
            else {
                if window_handle_is_alive(source.native_id) {
                    return Err(format!(
                        "Window source temporarily unavailable: {}",
                        source.source_id
                    ));
                }
                return Err(format!("Window source was closed: {}", source.source_id));
            };
            window.capture_image().map_err(|err| err.to_string())
        }
        other => Err(format!("Unsupported native capture source kind: {other}")),
    }
}

#[cfg(windows)]
fn window_handle_is_alive(native_id: u32) -> bool {
    unsafe { IsWindow(Some(HWND(native_id as _))).as_bool() }
}

#[cfg(not(windows))]
fn window_handle_is_alive(_native_id: u32) -> bool {
    true
}

fn letterbox_image(frame: &RgbaImage, target_width: u32, target_height: u32) -> RgbaImage {
    if frame.width() == target_width && frame.height() == target_height {
        return frame.clone();
    }

    let width_scale = target_width as f32 / frame.width().max(1) as f32;
    let height_scale = target_height as f32 / frame.height().max(1) as f32;
    let scale = width_scale.min(height_scale).max(0.000_1);
    let resized_width = ((frame.width() as f32 * scale).round() as u32).clamp(1, target_width);
    let resized_height = ((frame.height() as f32 * scale).round() as u32).clamp(1, target_height);
    let resized = resize(frame, resized_width, resized_height, FilterType::Triangle);
    let mut canvas = RgbaImage::from_pixel(target_width, target_height, Rgba([0, 0, 0, 255]));
    let x = i64::from((target_width - resized_width) / 2);
    let y = i64::from((target_height - resized_height) / 2);
    overlay(&mut canvas, &resized, x, y);
    canvas
}

fn normalize_even_dimension(value: u32) -> u32 {
    let adjusted = value.max(2);
    if adjusted % 2 == 0 {
        adjusted
    } else {
        adjusted.saturating_sub(1).max(2)
    }
}

fn parse_source_id(value: &str) -> Option<(&str, u32)> {
    let (kind, native_id) = value.split_once(':')?;
    Some((kind, native_id.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::parse_source_id;

    #[test]
    fn parses_window_source_id() {
        assert_eq!(parse_source_id("window:42"), Some(("window", 42)));
    }

    #[test]
    fn rejects_invalid_source_id() {
        assert_eq!(parse_source_id("window:not-a-number"), None);
        assert_eq!(parse_source_id("window"), None);
    }
}
