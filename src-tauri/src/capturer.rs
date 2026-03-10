#[cfg(windows)]
use crate::win32_window_snapshot::{
    capture_window_snapshot_bgra, get_window_physical_dimensions,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use image::{imageops::thumbnail, DynamicImage, ImageFormat, RgbaImage};
#[cfg(not(windows))]
use scap::{
    capturer::{
        Capturer as ScapCapturer, Options as ScapCapturerOptions, Resolution as ScapResolution,
    },
    frame::{Frame as ScapFrame, FrameType as ScapFrameType, VideoFrame as ScapVideoFrame},
    get_all_targets as scap_get_all_targets, Target as ScapTarget,
};
use serde::Serialize;
#[cfg(not(windows))]
use std::sync::{Condvar, Mutex, OnceLock};
use std::{
    collections::HashMap,
    io::Cursor,
    path::Path,
    time::{Duration, Instant},
};
#[cfg(windows)]
use windows::core::{BOOL, PWSTR};
#[cfg(windows)]
use windows::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, RECT},
    Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HDC, HMONITOR, MONITORINFOEXW, SRCCOPY,
    },
    System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::WindowsAndMessaging::{
        EnumChildWindows, GetClientRect, GetDesktopWindow, GetWindowLongPtrW, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE,
        WS_CHILD, WS_EX_TOOLWINDOW,
    },
};

const PLACEHOLDER_THUMBNAIL_DATA_URL: &str =
    "data:image/gif;base64,R0lGODlhAQABAPAAAAAAAAAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";
const SOURCE_ENUMERATION_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(windows)]
const WINDOW_SOURCE_ENUMERATION_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(windows)]
const WINDOW_SOURCE_PROCESS_METADATA_BUDGET: Duration = Duration::from_millis(200);
const EXCLUDED_WINDOW_PROCESS_NAMES: &[&str] =
    &["textinputhost", "nvidia app", "nvidia share", "nvoverlay"];
const EXCLUDED_WINDOW_TITLE_SUBSTRINGS: &[&str] =
    &["windows input experience", "nvidia geforce overlay"];

// Scap-based capture on non-Windows platforms requires a concurrency gate because
// WGC sessions are heavyweight. GDI on Windows does not need this.
#[cfg(not(windows))]
const MAX_CONCURRENT_PREVIEW_CAPTURES: usize = 2;

#[cfg(not(windows))]
struct PreviewCaptureGate {
    active: Mutex<usize>,
    wake: Condvar,
}

#[cfg(not(windows))]
impl PreviewCaptureGate {
    fn acquire(&self) -> PreviewCapturePermit<'_> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *active >= MAX_CONCURRENT_PREVIEW_CAPTURES {
            active = self
                .wake
                .wait(active)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *active += 1;
        PreviewCapturePermit { gate: self }
    }
}

#[cfg(not(windows))]
struct PreviewCapturePermit<'a> {
    gate: &'a PreviewCaptureGate,
}

#[cfg(not(windows))]
impl Drop for PreviewCapturePermit<'_> {
    fn drop(&mut self) {
        let mut active = self
            .gate
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *active = active.saturating_sub(1);
        self.gate.wake.notify_one();
    }
}

#[cfg(not(windows))]
fn preview_capture_gate() -> &'static PreviewCaptureGate {
    static GATE: OnceLock<PreviewCaptureGate> = OnceLock::new();
    GATE.get_or_init(|| PreviewCaptureGate {
        active: Mutex::new(0),
        wake: Condvar::new(),
    })
}

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
    pub capture_width: Option<u32>,
    pub capture_height: Option<u32>,
    pub max_frame_rate: Option<u32>,
}

#[cfg(windows)]
struct WindowSourceDescriptor {
    hwnd: HWND,
    title: String,
    process_id: Option<u32>,
    process_name: Option<String>,
    native_width: Option<u32>,
    native_height: Option<u32>,
}

#[cfg(windows)]
struct EnumWindowsContext {
    current_process_id: u32,
    windows: Vec<HWND>,
}

#[tauri::command]
pub fn get_capturer_sources() -> Result<Vec<CapturerSource>, String> {
    let started_at = Instant::now();
    log::info!("Capturer source enumeration requested");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let worker_started_at = Instant::now();
        let _ = tx.send(list_sources());
        log::info!(
            "Capturer source enumeration worker finished in {} ms",
            worker_started_at.elapsed().as_millis()
        );
    });
    match rx.recv_timeout(SOURCE_ENUMERATION_TIMEOUT) {
        Ok(result) => {
            let elapsed = started_at.elapsed();
            log::info!(
                "Capturer source enumeration request completed in {} ms",
                elapsed.as_millis()
            );
            if elapsed >= Duration::from_secs(3) {
                log::warn!(
                    "Capturer source enumeration completed in {} ms",
                    elapsed.as_millis()
                );
            }
            result
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let message = format!(
                "Source enumeration timed out after {} ms",
                started_at.elapsed().as_millis()
            );
            log::error!("{message}");
            Err(message)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let message = "Source enumeration worker disconnected".to_owned();
            log::error!("{message}");
            Err(message)
        }
    }
}

#[tauri::command]
pub fn get_capturer_thumbnail(id: String) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(encode_source_thumbnail(&id, 176, 99));
    });
    rx.recv_timeout(Duration::from_secs(3))
        .map_err(|_| "Thumbnail capture timed out".to_owned())?
}

#[tauri::command]
pub fn get_capturer_large_thumbnail(id: String) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(encode_source_thumbnail(&id, 1920, 1080));
    });
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "Large thumbnail capture timed out".to_owned())?
}

#[cfg(windows)]
unsafe extern "system" fn enum_monitors_callback(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut windows::Win32::Foundation::RECT,
    param: LPARAM,
) -> BOOL {
    let monitors = &mut *(param.0 as *mut Vec<(u32, String, u32, u32)>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _).as_bool() {
        let name = String::from_utf16_lossy(&info.szDevice)
            .trim_end_matches('\0')
            .trim()
            .to_owned();
        let rect = info.monitorInfo.rcMonitor;
        let w = rect.right.saturating_sub(rect.left) as u32;
        let h = rect.bottom.saturating_sub(rect.top) as u32;
        monitors.push((monitor.0 as usize as u32, name, w, h));
    }
    BOOL(1)
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_callback(hwnd: HWND, param: LPARAM) -> BOOL {
    let context = &mut *(param.0 as *mut EnumWindowsContext);
    if is_window_enumeration_candidate(hwnd, context.current_process_id) {
        context.windows.push(hwnd);
    }

    BOOL(1)
}

#[cfg(windows)]
fn list_sources() -> Result<Vec<CapturerSource>, String> {
    let started_at = Instant::now();
    log::info!("Capturer list_sources start platform=windows");
    let mut sources = list_display_sources_windows()?;
    log::info!(
        "Capturer display source enumeration completed count={} duration_ms={}",
        sources
            .iter()
            .filter(|source| source.kind == "screen")
            .count(),
        started_at.elapsed().as_millis()
    );

    let (window_tx, window_rx) = std::sync::mpsc::channel();
    let window_started_at = Instant::now();
    std::thread::spawn(move || {
        let _ = window_tx.send(list_window_sources_windows());
    });

    match window_rx.recv_timeout(WINDOW_SOURCE_ENUMERATION_TIMEOUT) {
        Ok(Ok(mut windows)) => {
            log::info!(
                "Capturer window source enumeration completed count={} duration_ms={}",
                windows.len(),
                window_started_at.elapsed().as_millis()
            );
            sources.append(&mut windows);
        }
        Ok(Err(err)) => {
            log::warn!("Window source enumeration failed; returning displays only: {err}");
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!(
                "Window source enumeration timed out after {} ms; returning displays only",
                WINDOW_SOURCE_ENUMERATION_TIMEOUT.as_millis()
            );
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            log::warn!("Window source enumeration worker disconnected; returning displays only");
        }
    }

    let hwnd_vals: Vec<u32> = sources
        .iter()
        .filter(|s| s.kind == "window")
        .filter_map(|s| s.id.strip_prefix("window:").and_then(|n| n.parse().ok()))
        .collect();
    if !hwnd_vals.is_empty() {
        std::thread::spawn(move || {
            crate::desktop_stream::wgc_window_capture::prewarm_window_captures(hwnd_vals);
        });
    }

    log::info!(
        "Capturer list_sources completed total_sources={} duration_ms={}",
        sources.len(),
        started_at.elapsed().as_millis()
    );
    Ok(sources)
}

#[cfg(windows)]
fn list_display_sources_windows() -> Result<Vec<CapturerSource>, String> {
    let started_at = Instant::now();
    let mut sources = Vec::new();
    let mut monitors: Vec<(u32, String, u32, u32)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            Some(HDC::default()),
            None,
            Some(enum_monitors_callback),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }

    for (id, name, w, h) in monitors {
        let name = if name.is_empty() {
            format!("Screen {id}")
        } else {
            name
        };
        let native_width = (w > 0).then_some(w);
        let native_height = (h > 0).then_some(h);
        sources.push(CapturerSource {
            id: format!("screen:{id}"),
            name,
            kind: "screen".to_owned(),
            url: PLACEHOLDER_THUMBNAIL_DATA_URL.to_owned(),
            process_id: None,
            process_name: None,
            native_width,
            native_height,
            capture_width: native_width,
            capture_height: native_height,
            max_frame_rate: Some(60),
        });
    }

    log::info!(
        "list_display_sources_windows monitors={} duration_ms={}",
        sources.len(),
        started_at.elapsed().as_millis()
    );
    Ok(sources)
}

#[cfg(windows)]
fn list_window_sources_windows() -> Result<Vec<CapturerSource>, String> {
    let started_at = Instant::now();
    let mut context = EnumWindowsContext {
        current_process_id: unsafe { GetCurrentProcessId() },
        windows: Vec::new(),
    };
    unsafe {
        EnumChildWindows(
            Some(GetDesktopWindow()),
            Some(enum_windows_callback),
            LPARAM((&mut context as *mut EnumWindowsContext) as isize),
        )
        .ok()
        .map_err(|err| err.to_string())?;
    }

    let mut windows = context
        .windows
        .into_iter()
        .filter_map(build_window_source_descriptor)
        .collect::<Vec<_>>();

    let metadata_deadline = started_at + WINDOW_SOURCE_PROCESS_METADATA_BUDGET;
    let mut process_names_by_pid = HashMap::<u32, Option<String>>::new();
    for window in &mut windows {
        if Instant::now() >= metadata_deadline {
            break;
        }

        let Some(process_id) = window.process_id else {
            continue;
        };

        let process_name = if let Some(cached) = process_names_by_pid.get(&process_id) {
            cached.clone()
        } else {
            let resolved = process_name_from_pid(process_id);
            process_names_by_pid.insert(process_id, resolved.clone());
            resolved
        };
        window.process_name = process_name;
    }

    windows.retain(|window| {
        !should_exclude_window_source(window.title.as_str(), window.process_name.as_deref())
    });

    windows.sort_by(|left, right| {
        left.title
            .to_ascii_lowercase()
            .cmp(&right.title.to_ascii_lowercase())
    });

    let unique_pid_count = process_names_by_pid.len();
    let sources = windows
        .into_iter()
        .enumerate()
        .map(|(index, window)| {
            let name = if window.title.is_empty() {
                format!("Window {}", index + 1)
            } else {
                window.title
            };
            CapturerSource {
                id: format!("window:{}", window.hwnd.0 as usize as u32),
                name,
                kind: "window".to_owned(),
                url: PLACEHOLDER_THUMBNAIL_DATA_URL.to_owned(),
                process_id: window.process_id,
                process_name: window.process_name,
                native_width: window.native_width,
                native_height: window.native_height,
                capture_width: window.native_width,
                capture_height: window.native_height,
                max_frame_rate: Some(60),
            }
        })
        .collect::<Vec<_>>();

    log::info!(
        "list_window_sources_windows windows={} labeled_pids={} duration_ms={}",
        sources.len(),
        unique_pid_count,
        started_at.elapsed().as_millis()
    );

    Ok(sources)
}

#[cfg(not(windows))]
fn list_sources() -> Result<Vec<CapturerSource>, String> {
    let mut sources = Vec::new();

    for target in scap_get_all_targets() {
        match target {
            ScapTarget::Display(display) => {
                let dimensions =
                    target_dimensions_for_source(&ScapTarget::Display(display.clone()));
                let native_width = positive_dimension(dimensions.0);
                let native_height = positive_dimension(dimensions.1);
                let name = if display.title.trim().is_empty() {
                    format!("Screen {}", display.id)
                } else {
                    display.title
                };
                sources.push(CapturerSource {
                    id: format!("screen:{}", display.id),
                    name,
                    kind: "screen".to_owned(),
                    url: PLACEHOLDER_THUMBNAIL_DATA_URL.to_owned(),
                    process_id: None,
                    process_name: None,
                    native_width,
                    native_height,
                    capture_width: native_width,
                    capture_height: native_height,
                    max_frame_rate: Some(60),
                });
            }
            ScapTarget::Window(window) => {
                let dimensions = target_dimensions_for_source(&ScapTarget::Window(window.clone()));
                let native_width = positive_dimension(dimensions.0);
                let native_height = positive_dimension(dimensions.1);
                let name = if window.title.trim().is_empty() {
                    format!("Window {}", window.id)
                } else {
                    window.title
                };
                let process_id = window_process_id(window.id);
                let process_name = process_id.and_then(process_name_from_pid);
                if should_exclude_window_source(&name, process_name.as_deref()) {
                    continue;
                }
                sources.push(CapturerSource {
                    id: format!("window:{}", window.id),
                    name,
                    kind: "window".to_owned(),
                    url: PLACEHOLDER_THUMBNAIL_DATA_URL.to_owned(),
                    process_id,
                    process_name,
                    native_width,
                    native_height,
                    capture_width: native_width,
                    capture_height: native_height,
                    max_frame_rate: Some(60),
                });
            }
        }
    }

    Ok(sources)
}

fn should_exclude_window_source(title: &str, process_name: Option<&str>) -> bool {
    let normalized_title = title.trim().to_ascii_lowercase();
    if EXCLUDED_WINDOW_TITLE_SUBSTRINGS
        .iter()
        .any(|blocked| normalized_title.contains(blocked))
    {
        return true;
    }

    let Some(process_name) = process_name else {
        return false;
    };

    let normalized_process_name = process_name.trim().to_ascii_lowercase();
    EXCLUDED_WINDOW_PROCESS_NAMES
        .iter()
        .any(|blocked| normalized_process_name == *blocked)
}

#[cfg(windows)]
fn build_window_source_descriptor(hwnd: HWND) -> Option<WindowSourceDescriptor> {
    let title = window_title(hwnd)?;
    if title.is_empty() {
        return None;
    }

    if should_exclude_window_source(&title, None) {
        return None;
    }

    let process_id = window_process_id(hwnd.0 as usize as u32);
    let (width, height) = window_dimensions(hwnd);
    Some(WindowSourceDescriptor {
        hwnd,
        title,
        process_id,
        process_name: None,
        native_width: positive_dimension(width),
        native_height: positive_dimension(height),
    })
}

#[cfg(windows)]
fn window_title(hwnd: HWND) -> Option<String> {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return None;
    }

    let mut title_buffer = vec![0u16; (len + 1) as usize];
    let copied = unsafe { GetWindowTextW(hwnd, &mut title_buffer) };
    if copied <= 0 {
        return None;
    }

    title_buffer.truncate(copied as usize);
    let title = String::from_utf16_lossy(&title_buffer).trim().to_owned();
    (!title.is_empty()).then_some(title)
}

#[cfg(windows)]
fn is_window_enumeration_candidate(hwnd: HWND, current_process_id: u32) -> bool {
    // Derived from windows-capture 1.5.0, which is the Windows target
    // enumeration path used by scap.
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    let mut process_id = 0_u32;
    unsafe {
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == 0 || process_id == current_process_id {
        return false;
    }

    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
        return false;
    }

    let styles = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    let ex_styles = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    if (ex_styles & isize::try_from(WS_EX_TOOLWINDOW.0).unwrap_or_default()) != 0 {
        return false;
    }
    if (styles & isize::try_from(WS_CHILD.0).unwrap_or_default()) != 0 {
        return false;
    }

    true
}

fn encode_source_thumbnail(id: &str, width: u32, height: u32) -> Result<String, String> {
    let image = capture_source_image(id)?;
    encode_thumbnail(image, width, height)
}

#[cfg(not(windows))]
fn find_target(kind: &str, source_id: u32) -> Option<ScapTarget> {
    scap_get_all_targets()
        .into_iter()
        .find(|target| match target {
            ScapTarget::Display(display) => kind == "screen" && display.id == source_id,
            ScapTarget::Window(window) => kind == "window" && window.id == source_id,
        })
}

fn capture_source_image(id: &str) -> Result<RgbaImage, String> {
    let (kind, source_id) =
        parse_source_id(id).ok_or_else(|| format!("unsupported capturer source id: {id}"))?;

    #[cfg(windows)]
    {
        return match kind {
            "window" => capture_window_thumbnail_gdi(HWND(source_id as _)),
            "screen" => {
                capture_screen_thumbnail_gdi(HMONITOR(source_id as usize as *mut core::ffi::c_void))
            }
            _ => Err(format!("Unsupported source kind for thumbnail: {kind}")),
        };
    }

    #[cfg(not(windows))]
    capture_target_image_scap(kind.to_owned(), source_id)
}

#[cfg(not(windows))]
fn capture_target_image_scap(kind: String, source_id: u32) -> Result<RgbaImage, String> {
    let _permit = preview_capture_gate().acquire();
    let target = find_target(kind.as_str(), source_id)
        .ok_or_else(|| format!("{} source not found: {}:{}", kind, kind, source_id))?;
    let mut capturer = ScapCapturer::build(ScapCapturerOptions {
        fps: 60,
        show_cursor: true,
        show_highlight: false,
        target: Some(target),
        output_type: ScapFrameType::BGRAFrame,
        output_resolution: ScapResolution::Captured,
        captures_audio: false,
        exclude_current_process_audio: true,
        ..Default::default()
    })
    .map_err(|err| format!("Failed to build thumbnail capture session: {err}"))?;
    capturer
        .start_capture()
        .map_err(|err| format!("Failed to start thumbnail capture session: {err}"))?;
    let frame = capturer
        .get_next_frame()
        .map_err(|err| format!("Failed to receive thumbnail frame: {err}"))?;
    let _ = capturer.stop_capture();

    match frame {
        ScapFrame::Video(video) => video_frame_to_image(video),
        ScapFrame::Audio(_) => Err("Unexpected audio frame during thumbnail capture".to_owned()),
    }
}

#[cfg(not(windows))]
fn positive_dimension(value: u64) -> Option<u32> {
    let clamped = value.min(u32::MAX as u64) as u32;
    (clamped > 0).then_some(clamped)
}

#[cfg(not(windows))]
fn target_dimensions_for_source(target: &ScapTarget) -> (u64, u64) {
    let _ = target;
    (0, 0)
}

#[cfg(windows)]
fn window_dimensions(hwnd: HWND) -> (u64, u64) {
    get_window_physical_dimensions(hwnd)
}

#[cfg(windows)]
fn positive_dimension(value: u64) -> Option<u32> {
    let clamped = value.min(u32::MAX as u64) as u32;
    (clamped > 0).then_some(clamped)
}

// GDI-based thumbnail capture for Windows.
// PrintWindow with PW_RENDERFULLCONTENT captures the DWM-composited output,
// including DirectX / Direct3D rendered content. BitBlt from the desktop DC
// covers screen regions directly. Both are synchronous and ~10-50× faster than
// opening a WGC (scap) session per source, which is reserved for live streaming.

#[cfg(windows)]
fn capture_window_thumbnail_gdi(hwnd: HWND) -> Result<RgbaImage, String> {
    let (width, height, bgra) = capture_window_snapshot_bgra(hwnd)?;
    let rgba = bgrx_to_rgba(bgra);
    RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "Invalid GDI window bitmap buffer".to_owned())
}

#[cfg(windows)]
fn capture_screen_thumbnail_gdi(monitor: HMONITOR) -> Result<RgbaImage, String> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = core::mem::size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _) }.as_bool() {
        return Err("GetMonitorInfoW failed".to_owned());
    }

    let rect = info.monitorInfo.rcMonitor;
    let x = rect.left;
    let y = rect.top;
    let w = rect.right.saturating_sub(rect.left);
    let h = rect.bottom.saturating_sub(rect.top);
    if w <= 0 || h <= 0 {
        return Err("Monitor has zero dimensions".to_owned());
    }

    unsafe {
        // None hwnd = DC for the full virtual desktop
        let desktop_dc = GetDC(None);
        if desktop_dc.is_invalid() {
            return Err("GetDC failed for desktop".to_owned());
        }
        let mem_dc = CreateCompatibleDC(Some(desktop_dc));
        let bitmap = CreateCompatibleBitmap(desktop_dc, w, h);
        let old = SelectObject(mem_dc, bitmap.into());

        // Copy the monitor's rectangle from the virtual desktop DC
        let _ = BitBlt(mem_dc, 0, 0, w, h, Some(desktop_dc), x, y, SRCCOPY);

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;

        let mut pixels = vec![0u8; (w * h * 4) as usize];
        GetDIBits(
            mem_dc,
            bitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, desktop_dc);

        let rgba = bgrx_to_rgba(pixels);
        RgbaImage::from_raw(w as u32, h as u32, rgba)
            .ok_or_else(|| "Invalid GDI screen bitmap buffer".to_owned())
    }
}

#[cfg(not(windows))]
fn video_frame_to_image(video: ScapVideoFrame) -> Result<RgbaImage, String> {
    match video {
        ScapVideoFrame::BGRA(frame) => {
            rgba_image_from_raw(frame.width, frame.height, bgra_to_rgba(frame.data))
        }
        ScapVideoFrame::BGRx(frame) => {
            rgba_image_from_raw(frame.width, frame.height, bgrx_to_rgba(frame.data))
        }
        ScapVideoFrame::BGR0(frame) => {
            rgba_image_from_raw(frame.width, frame.height, bgrx_to_rgba(frame.data))
        }
        ScapVideoFrame::RGB(frame) => {
            rgba_image_from_raw(frame.width, frame.height, rgb_to_rgba(frame.data))
        }
        ScapVideoFrame::RGBx(frame) => {
            rgba_image_from_raw(frame.width, frame.height, rgbx_to_rgba(frame.data))
        }
        ScapVideoFrame::XBGR(frame) => {
            rgba_image_from_raw(frame.width, frame.height, xbgr_to_rgba(frame.data))
        }
        ScapVideoFrame::YUVFrame(_) => Err("YUV thumbnails are not supported".to_owned()),
    }
}

#[cfg(not(windows))]
fn rgba_image_from_raw(width: i32, height: i32, rgba: Vec<u8>) -> Result<RgbaImage, String> {
    RgbaImage::from_raw(width.max(1) as u32, height.max(1) as u32, rgba)
        .ok_or_else(|| "Captured frame had invalid RGBA buffer".to_owned())
}

fn parse_source_id(value: &str) -> Option<(&str, u32)> {
    let (kind, index) = value.split_once(':')?;
    Some((kind, index.parse().ok()?))
}

fn encode_thumbnail(image: RgbaImage, width: u32, height: u32) -> Result<String, String> {
    let resized = thumbnail(&image, width.max(1), height.max(1));
    // Convert RGBA → RGB (JPEG does not support alpha)
    let rgb = DynamicImage::ImageRgba8(resized).into_rgb8();
    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    DynamicImage::ImageRgb8(rgb)
        .write_to(&mut cursor, ImageFormat::Jpeg)
        .map_err(|err| err.to_string())?;

    Ok(format!("data:image/jpeg;base64,{}", BASE64.encode(bytes)))
}

#[cfg(windows)]
fn window_process_id(window_id: u32) -> Option<u32> {
    let mut pid = 0_u32;
    unsafe {
        let _ = GetWindowThreadProcessId(HWND(window_id as _), Some(&mut pid));
    }
    (pid != 0).then_some(pid)
}

#[cfg(not(windows))]
fn window_process_id(_window_id: u32) -> Option<u32> {
    None
}

#[cfg(windows)]
fn process_name_from_pid(pid: u32) -> Option<String> {
    let started_at = Instant::now();
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = vec![0u16; 32_768];
    let mut size = buffer.len() as u32;
    let success = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(handle) };
    if !success || size == 0 {
        return None;
    }

    buffer.truncate(size as usize);
    let image_path = String::from_utf16_lossy(&buffer);
    let process_name = Path::new(image_path.trim())
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let elapsed = started_at.elapsed();
    if elapsed >= Duration::from_millis(250) {
        log::warn!(
            "Slow process name lookup pid={} duration_ms={} resolved={}",
            pid,
            elapsed.as_millis(),
            process_name.as_deref().unwrap_or("<none>")
        );
    }

    process_name
}

#[cfg(not(windows))]
fn process_name_from_pid(_pid: u32) -> Option<String> {
    None
}

// Only used in the non-Windows scap video frame path
#[cfg(not(windows))]
fn bgra_to_rgba(data: Vec<u8>) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        rgba.push(chunk[2]);
        rgba.push(chunk[1]);
        rgba.push(chunk[0]);
        rgba.push(chunk[3]);
    }
    rgba
}

// Used on all platforms: GDI returns BGR0 on Windows; scap BGRx/BGR0 on non-Windows
fn bgrx_to_rgba(data: Vec<u8>) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        rgba.push(chunk[2]);
        rgba.push(chunk[1]);
        rgba.push(chunk[0]);
        rgba.push(255);
    }
    rgba
}

#[cfg(not(windows))]
fn rgb_to_rgba(data: Vec<u8>) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((data.len() / 3) * 4);
    for chunk in data.chunks_exact(3) {
        rgba.push(chunk[0]);
        rgba.push(chunk[1]);
        rgba.push(chunk[2]);
        rgba.push(255);
    }
    rgba
}

#[cfg(not(windows))]
fn rgbx_to_rgba(data: Vec<u8>) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        rgba.push(chunk[0]);
        rgba.push(chunk[1]);
        rgba.push(chunk[2]);
        rgba.push(255);
    }
    rgba
}

#[cfg(not(windows))]
fn xbgr_to_rgba(data: Vec<u8>) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        rgba.push(chunk[3]);
        rgba.push(chunk[2]);
        rgba.push(chunk[1]);
        rgba.push(255);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::{get_capturer_sources, should_exclude_window_source};
    #[cfg(windows)]
    use std::time::Instant;

    #[test]
    fn excludes_known_overlay_titles() {
        assert!(should_exclude_window_source("NVIDIA GeForce Overlay", None));
        assert!(should_exclude_window_source(
            "Windows Input Experience",
            None
        ));
    }

    #[test]
    fn excludes_known_overlay_process_names() {
        assert!(should_exclude_window_source("Overlay", Some("nvoverlay")));
        assert!(should_exclude_window_source(
            "Window",
            Some("textinputhost")
        ));
    }

    #[test]
    fn keeps_normal_window_titles() {
        assert!(!should_exclude_window_source(
            "Visual Studio Code",
            Some("Code")
        ));
        assert!(!should_exclude_window_source("Discord", Some("Discord")));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "manual diagnostic"]
    fn manual_capture_source_probe() {
        let started_at = Instant::now();
        let result = get_capturer_sources();
        match result {
            Ok(sources) => {
                let windows = sources
                    .iter()
                    .filter(|source| source.kind == "window")
                    .count();
                let screens = sources
                    .iter()
                    .filter(|source| source.kind == "screen")
                    .count();
                println!(
                    "manual_capture_source_probe ok total={} windows={} screens={} duration_ms={}",
                    sources.len(),
                    windows,
                    screens,
                    started_at.elapsed().as_millis()
                );
            }
            Err(err) => {
                panic!(
                    "manual_capture_source_probe failed after {} ms: {}",
                    started_at.elapsed().as_millis(),
                    err
                );
            }
        }
    }
}
