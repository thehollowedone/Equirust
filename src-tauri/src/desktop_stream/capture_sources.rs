use super::contracts::ResolvedCaptureSource;
#[cfg(windows)]
use super::d3d11_device::{
    create_default_video_device, create_video_device_for_monitor, SharedD3D11Device,
};
#[cfg(windows)]
use super::dxgi_duplication::{DxgiFrameResult, DxgiScreenCapture};
#[cfg(windows)]
use super::wgc_window_capture::{WgcFrameResult, WgcWindowCapture};
#[cfg(windows)]
use crate::win32_window_snapshot::capture_window_snapshot_bgra;
use image::{
    codecs::jpeg::JpegEncoder,
    imageops::{overlay, resize, thumbnail, FilterType},
    DynamicImage, Rgba, RgbaImage,
};
#[cfg(not(windows))]
use scap::{
    capturer::{
        Capturer as ScapCapturer, CapturerBuildError as ScapCapturerBuildError,
        Options as ScapCapturerOptions, Resolution as ScapResolution,
    },
    frame::{Frame as ScapFrame, FrameType as ScapFrameType, VideoFrame as ScapVideoFrame},
    get_all_targets as scap_get_all_targets, has_permission as scap_has_permission,
    is_supported as scap_is_supported, Target as ScapTarget,
};
use std::io::Cursor;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(not(windows))]
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows::Win32::{
    Foundation::HWND,
    Graphics::{
        Direct3D11::ID3D11Texture2D,
        Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput},
        Gdi::{
            GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
        },
    },
    UI::WindowsAndMessaging::{
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    },
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    Auto,
    #[cfg(windows)]
    Wgc,
    #[cfg(not(windows))]
    Scap,
    #[cfg(windows)]
    Dxgi,
}

impl CaptureBackend {
    pub fn label(self) -> &'static str {
        match self {
            CaptureBackend::Auto => "auto",
            #[cfg(windows)]
            CaptureBackend::Wgc => "wgc",
            #[cfg(not(windows))]
            CaptureBackend::Scap => "scap",
            #[cfg(windows)]
            CaptureBackend::Dxgi => "dxgi",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFrameMode {
    TargetRgbaFrame,
    SourceBgraFrame,
    SourceBgraTextureFrame,
    SourceHdrTextureFrame,
}

impl CaptureFrameMode {
    #[cfg(windows)]
    fn uses_shared_texture(self) -> bool {
        matches!(
            self,
            CaptureFrameMode::SourceBgraTextureFrame | CaptureFrameMode::SourceHdrTextureFrame
        )
    }
}

pub fn resolve_capture_backend(value: Option<&str>) -> CaptureBackend {
    let normalized = value
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_ascii_lowercase);
    match normalized.as_deref() {
        #[cfg(windows)]
        Some("scap") | Some("windows_wgc") | Some("wgc") | Some("windows") => CaptureBackend::Wgc,
        #[cfg(not(windows))]
        Some("scap") => CaptureBackend::Scap,
        #[cfg(windows)]
        Some("dxgi") => CaptureBackend::Dxgi,
        _ => CaptureBackend::Auto,
    }
}

#[cfg(windows)]
pub fn resolve_session_capture_backend(
    source: &ResolvedCaptureSource,
    backend: CaptureBackend,
) -> CaptureBackend {
    match backend {
        CaptureBackend::Auto if source.source_kind == "screen" => CaptureBackend::Dxgi,
        CaptureBackend::Auto => CaptureBackend::Wgc,
        backend => backend,
    }
}

#[cfg(not(windows))]
pub fn resolve_session_capture_backend(
    _source: &ResolvedCaptureSource,
    backend: CaptureBackend,
) -> CaptureBackend {
    match backend {
        CaptureBackend::Auto => CaptureBackend::Scap,
        backend => backend,
    }
}

#[cfg(windows)]
pub fn create_shared_capture_device(
    source: &ResolvedCaptureSource,
    frame_mode: CaptureFrameMode,
) -> Result<Option<Arc<SharedD3D11Device>>, String> {
    if !frame_mode.uses_shared_texture() {
        return Ok(None);
    }

    let device = match source.source_kind.as_str() {
        "screen" => {
            let monitor = HMONITOR(source.native_id as usize as *mut core::ffi::c_void);
            create_video_device_for_monitor(monitor)?
        }
        "window" => {
            let monitor =
                unsafe { MonitorFromWindow(HWND(source.native_id as _), MONITOR_DEFAULTTONEAREST) };
            if monitor.0.is_null() {
                create_default_video_device()?
            } else {
                create_video_device_for_monitor(monitor)
                    .or_else(|_| create_default_video_device())?
            }
        }
        _ => return Ok(None),
    };

    Ok(Some(device))
}

#[cfg(not(windows))]
pub fn create_shared_capture_device(
    _source: &ResolvedCaptureSource,
    _frame_mode: CaptureFrameMode,
) -> Result<Option<()>, String> {
    Ok(None)
}

#[cfg(windows)]
fn resolve_screen_name(source_id: &str, native_id: u32) -> Result<String, String> {
    let monitor = HMONITOR(native_id as usize as *mut core::ffi::c_void);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _) }.as_bool() {
        let name = String::from_utf16_lossy(&info.szDevice)
            .trim_end_matches('\0')
            .trim()
            .to_owned();
        Ok(if name.is_empty() {
            format!("Screen {native_id}")
        } else {
            name
        })
    } else {
        Err(format!("Screen source not found: {source_id}"))
    }
}

#[cfg(not(windows))]
fn resolve_screen_name(source_id: &str, native_id: u32) -> Result<String, String> {
    let name = scap_get_all_targets()
        .into_iter()
        .find_map(|target| match target {
            ScapTarget::Display(display) if display.id == native_id => {
                Some(display.title.trim().to_owned())
            }
            _ => None,
        })
        .ok_or_else(|| format!("Screen source not found: {source_id}"))?;
    Ok(if name.is_empty() {
        format!("Screen {native_id}")
    } else {
        name
    })
}

#[cfg(windows)]
fn resolve_window_name(source_id: &str, native_id: u32) -> Result<String, String> {
    let hwnd = HWND(native_id as _);
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return Err(format!("Window source not found: {source_id}"));
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if written <= 0 {
        return Err(format!("Window source not found: {source_id}"));
    }
    buf.truncate(written as usize);
    let title = String::from_utf16_lossy(&buf).trim().to_owned();
    Ok(if title.is_empty() {
        format!("Window {native_id}")
    } else {
        title
    })
}

#[cfg(not(windows))]
fn resolve_window_name(source_id: &str, native_id: u32) -> Result<String, String> {
    let name = scap_get_all_targets()
        .into_iter()
        .find_map(|target| match target {
            ScapTarget::Window(window) if window.id == native_id => {
                Some(window.title.trim().to_owned())
            }
            _ => None,
        })
        .ok_or_else(|| format!("Window source not found: {source_id}"))?;
    Ok(if name.is_empty() {
        format!("Window {native_id}")
    } else {
        name
    })
}

pub fn resolve_source(source_id: &str) -> Result<ResolvedCaptureSource, String> {
    let (kind, native_id) = parse_source_id(source_id)
        .ok_or_else(|| format!("Unsupported desktop stream source id: {source_id}"))?;

    match kind {
        "screen" => {
            let display_name = resolve_screen_name(source_id, native_id)?;
            let adapter_info = resolve_capture_adapter_info("screen", native_id);
            Ok(ResolvedCaptureSource {
                source_id: source_id.to_owned(),
                source_kind: "screen".to_owned(),
                native_id,
                display_name,
                process_id: None,
                adapter_name: adapter_info.adapter_name,
                output_name: adapter_info.output_name,
            })
        }
        "window" => {
            let display_name = resolve_window_name(source_id, native_id)?;
            let process_id = window_process_id(native_id);
            let adapter_info = resolve_capture_adapter_info("window", native_id);
            Ok(ResolvedCaptureSource {
                source_id: source_id.to_owned(),
                source_kind: "window".to_owned(),
                native_id,
                display_name,
                process_id,
                adapter_name: adapter_info.adapter_name,
                output_name: adapter_info.output_name,
            })
        }
        _ => Err(format!("Unsupported desktop stream source kind: {kind}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedFramePixelFormat {
    Rgba,
    Bgra,
    Rgba16FloatTexture,
}

#[derive(Clone)]
pub struct PreparedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PreparedFramePixelFormat,
    pub pixels: Vec<u8>,
    #[cfg(windows)]
    pub gpu_texture: Option<ID3D11Texture2D>,
}

#[cfg(not(windows))]
struct ScapCaptureSession {
    source_id: String,
    source_kind: String,
    native_id: u32,
    capturer: ScapCapturer,
    last_window_liveness_probe_at: Instant,
    last_prepared_frame: Option<PreparedVideoFrame>,
}

#[cfg(not(windows))]
impl ScapCaptureSession {
    fn new(source: &ResolvedCaptureSource, frame_rate: u32) -> Result<Self, String> {
        if !scap_is_supported() {
            return Err("Scap capture is not supported on this platform.".to_owned());
        }
        if !scap_has_permission() {
            return Err("Scap capture permission is not granted.".to_owned());
        }

        let target = find_scap_target(source)?;
        let mut capturer = ScapCapturer::build(ScapCapturerOptions {
            fps: frame_rate.max(1),
            show_cursor: true,
            show_highlight: false,
            target: Some(target),
            output_type: ScapFrameType::BGRAFrame,
            output_resolution: ScapResolution::Captured,
            captures_audio: false,
            exclude_current_process_audio: true,
            ..Default::default()
        })
        .map_err(|error| format_scap_build_error(&source.source_kind, &source.source_id, error))?;
        capturer
            .start_capture()
            .map_err(|error| format!("Scap start capture failed: {error}"))?;

        Ok(Self {
            source_id: source.source_id.clone(),
            source_kind: source.source_kind.clone(),
            native_id: source.native_id,
            capturer,
            last_window_liveness_probe_at: Instant::now()
                .checked_sub(Duration::from_millis(1000))
                .unwrap_or_else(Instant::now),
            last_prepared_frame: None,
        })
    }

    fn capture_frame(
        &mut self,
        target_width: u32,
        target_height: u32,
        _frame_mode: CaptureFrameMode,
    ) -> Result<PreparedVideoFrame, String> {
        if self.source_kind == "window"
            && self.last_window_liveness_probe_at.elapsed() >= Duration::from_millis(250)
        {
            self.last_window_liveness_probe_at = Instant::now();
            if !window_source_is_alive(self.native_id) {
                return Err(format!("Window source was closed: {}", self.source_id));
            }
        }

        let frame = match self
            .capturer
            .get_next_frame_timeout(Duration::from_millis(120))
        {
            Ok(frame) => frame,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if self.source_kind == "window" && !window_source_is_alive(self.native_id) {
                    return Err(format!("Window source was closed: {}", self.source_id));
                }
                if let Some(frame) = self.last_prepared_frame.clone() {
                    return Ok(frame);
                }
                return Err(format!(
                    "pending::Waiting for initial frame: {}",
                    self.source_id
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(format!(
                    "capture::Scap frame channel disconnected: {}",
                    self.source_id
                ));
            }
        };
        let prepared = prepared_frame_from_scap_frame(frame, target_width, target_height)?;
        self.last_prepared_frame = Some(prepared.clone());
        Ok(prepared)
    }

    fn stop(&mut self) {
        let _ = self.capturer.stop_capture();
    }
}

#[cfg(not(windows))]
impl Drop for ScapCaptureSession {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct WindowCaptureSession {
    inner: WindowCaptureInner,
}

enum WindowCaptureInner {
    #[cfg(windows)]
    Wgc(WgcWindowCaptureSession),
    #[cfg(not(windows))]
    Scap(ScapCaptureSession),
}

impl WindowCaptureSession {
    pub fn new(
        source: &ResolvedCaptureSource,
        frame_rate: u32,
        backend: CaptureBackend,
        #[cfg(windows)] shared_device: Option<Arc<SharedD3D11Device>>,
        #[cfg(windows)] frame_mode: CaptureFrameMode,
    ) -> Result<Self, String> {
        if source.source_kind != "window" {
            return Err(format!(
                "Desktop stream window capture session requires a window source: {}",
                source.source_id
            ));
        }
        if !window_source_is_alive(source.native_id) {
            return Err(format!("Window source was closed: {}", source.source_id));
        }
        #[cfg(windows)]
        let supported_backend = matches!(backend, CaptureBackend::Auto | CaptureBackend::Wgc);
        #[cfg(not(windows))]
        let supported_backend = matches!(backend, CaptureBackend::Auto | CaptureBackend::Scap);
        if !supported_backend {
            return Err(format!(
                "Unsupported capture backend '{}' for window source: {}",
                backend.label(),
                source.source_id
            ));
        }

        #[cfg(windows)]
        let inner = {
            let _ = frame_rate;
            let session = WgcWindowCaptureSession::new(source, shared_device, frame_mode)
                .map_err(|e| format!("WGC window capture failed: {e}"))?;
            log::info!(
                "Desktop stream using WGC window backend source_id={}",
                source.source_id
            );
            WindowCaptureInner::Wgc(session)
        };

        #[cfg(not(windows))]
        let inner = {
            let session = ScapCaptureSession::new(source, frame_rate)?;
            log::info!(
                "Desktop stream using scap window backend source_id={}",
                source.source_id
            );
            WindowCaptureInner::Scap(session)
        };

        Ok(Self { inner })
    }

    pub fn capture_frame(
        &mut self,
        target_width: u32,
        target_height: u32,
        frame_mode: CaptureFrameMode,
    ) -> Result<PreparedVideoFrame, String> {
        match &mut self.inner {
            #[cfg(windows)]
            WindowCaptureInner::Wgc(s) => s.capture_frame(target_width, target_height, frame_mode),
            #[cfg(not(windows))]
            WindowCaptureInner::Scap(s) => s.capture_frame(target_width, target_height, frame_mode),
        }
    }

    pub fn stop(&mut self) {
        match &mut self.inner {
            #[cfg(windows)]
            WindowCaptureInner::Wgc(s) => s.stop(),
            #[cfg(not(windows))]
            WindowCaptureInner::Scap(s) => s.stop(),
        }
    }
}

#[cfg(windows)]
struct WgcWindowCaptureSession {
    source_id: String,
    native_id: u32,
    capture: WgcWindowCapture,
    last_prepared_frame: Option<PreparedVideoFrame>,
    startup_snapshot_attempted: bool,
    last_liveness_probe: std::time::Instant,
}

#[cfg(windows)]
impl WgcWindowCaptureSession {
    fn new(
        source: &ResolvedCaptureSource,
        shared_device: Option<Arc<SharedD3D11Device>>,
        frame_mode: CaptureFrameMode,
    ) -> Result<Self, String> {
        let use_direct_texture = frame_mode.uses_shared_texture() && shared_device.is_some();
        let capture = if use_direct_texture {
            WgcWindowCapture::start_with_device(
                source.native_id,
                shared_device,
                matches!(frame_mode, CaptureFrameMode::SourceHdrTextureFrame),
            )?
        } else {
            super::wgc_window_capture::take_prewarmed_capture(source.native_id)
                .ok_or(())
                .or_else(|_| WgcWindowCapture::start(source.native_id))?
        };
        Ok(Self {
            source_id: source.source_id.clone(),
            native_id: source.native_id,
            capture,
            last_prepared_frame: None,
            startup_snapshot_attempted: false,
            last_liveness_probe: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(1000))
                .unwrap_or_else(std::time::Instant::now),
        })
    }

    fn capture_frame(
        &mut self,
        target_width: u32,
        target_height: u32,
        frame_mode: CaptureFrameMode,
    ) -> Result<PreparedVideoFrame, String> {
        if self.last_liveness_probe.elapsed() >= std::time::Duration::from_millis(250) {
            self.last_liveness_probe = std::time::Instant::now();
            if !window_handle_is_alive(self.native_id) {
                return Err(format!(
                    "capture::Window source was closed: {}",
                    self.source_id
                ));
            }
        }

        let initial_frame_wait_ms = if self.last_prepared_frame.is_none()
            && !self.startup_snapshot_attempted
        {
            24
        } else {
            120
        };

        match self.capture.acquire_frame(initial_frame_wait_ms) {
            WgcFrameResult::Frame {
                width,
                height,
                bgra,
            } => {
                let prepared = match frame_mode {
                    CaptureFrameMode::SourceBgraFrame
                    | CaptureFrameMode::SourceBgraTextureFrame
                    | CaptureFrameMode::SourceHdrTextureFrame => {
                        prepare_bgra_frame_for_upload(width, height, bgra)?
                    }
                    CaptureFrameMode::TargetRgbaFrame => {
                        let rgba = bgra_to_rgba(bgra);
                        prepare_rgba_frame(width, height, rgba, target_width, target_height)?
                    }
                };
                self.last_prepared_frame = Some(prepared.clone());
                Ok(prepared)
            }
            WgcFrameResult::Texture {
                width,
                height,
                texture,
                pixel_format,
            } => {
                let prepared = prepare_gpu_texture_frame(width, height, texture, pixel_format)?;
                self.last_prepared_frame = Some(prepared.clone());
                Ok(prepared)
            }
            WgcFrameResult::Timeout => {
                if !window_handle_is_alive(self.native_id) {
                    return Err(format!(
                        "capture::Window source was closed: {}",
                        self.source_id
                    ));
                }
                if let Some(frame) = self.last_prepared_frame.clone() {
                    Ok(frame)
                } else if !self.startup_snapshot_attempted {
                    self.startup_snapshot_attempted = true;
                    let prepared = prepare_initial_window_snapshot_frame(
                        self.native_id,
                        target_width,
                        target_height,
                        frame_mode,
                    )?;
                    self.last_prepared_frame = Some(prepared.clone());
                    Ok(prepared)
                } else {
                    Err(format!(
                        "pending::Waiting for initial WGC frame: {}",
                        self.source_id
                    ))
                }
            }
            WgcFrameResult::Error(msg) => Err(format!("capture::{msg}")),
        }
    }

    fn stop(&mut self) {
        self.capture.stop();
    }
}

#[cfg(windows)]
struct DxgiCaptureSession {
    source_id: String,
    capture: DxgiScreenCapture,
    last_prepared_frame: Option<PreparedVideoFrame>,
}

#[cfg(windows)]
impl DxgiCaptureSession {
    fn new(
        source: &ResolvedCaptureSource,
        shared_device: Option<Arc<SharedD3D11Device>>,
        frame_mode: CaptureFrameMode,
    ) -> Result<Self, String> {
        let monitor = HMONITOR(source.native_id as usize as *mut core::ffi::c_void);
        let capture = DxgiScreenCapture::new(monitor, shared_device, frame_mode)?;
        Ok(Self {
            source_id: source.source_id.clone(),
            capture,
            last_prepared_frame: None,
        })
    }

    fn capture_frame(
        &mut self,
        target_width: u32,
        target_height: u32,
        frame_mode: CaptureFrameMode,
    ) -> Result<PreparedVideoFrame, String> {
        match self.capture.acquire_frame(120)? {
            DxgiFrameResult::Timeout => {
                if let Some(frame) = self.last_prepared_frame.clone() {
                    return Ok(frame);
                }
                return Err(format!(
                    "pending::Waiting for initial DXGI frame: {}",
                    self.source_id
                ));
            }
            DxgiFrameResult::Frame {
                width,
                height,
                bgra,
            } => {
                let prepared = match frame_mode {
                    CaptureFrameMode::SourceBgraFrame
                    | CaptureFrameMode::SourceBgraTextureFrame
                    | CaptureFrameMode::SourceHdrTextureFrame => {
                        prepare_bgra_frame_for_upload(width, height, bgra)?
                    }
                    CaptureFrameMode::TargetRgbaFrame => {
                        let rgba = bgra_to_rgba(bgra);
                        prepare_rgba_frame(width, height, rgba, target_width, target_height)?
                    }
                };
                self.last_prepared_frame = Some(prepared.clone());
                Ok(prepared)
            }
            DxgiFrameResult::Texture {
                width,
                height,
                texture,
                pixel_format,
            } => {
                let prepared = prepare_gpu_texture_frame(width, height, texture, pixel_format)?;
                self.last_prepared_frame = Some(prepared.clone());
                Ok(prepared)
            }
        }
    }
}

pub struct ScreenCaptureSession {
    inner: ScreenCaptureInner,
}

enum ScreenCaptureInner {
    #[cfg(windows)]
    Dxgi(DxgiCaptureSession),
    #[cfg(not(windows))]
    Scap(ScapCaptureSession),
}

impl ScreenCaptureSession {
    pub fn new(
        source: &ResolvedCaptureSource,
        frame_rate: u32,
        backend: CaptureBackend,
        #[cfg(windows)] shared_device: Option<Arc<SharedD3D11Device>>,
        #[cfg(windows)] frame_mode: CaptureFrameMode,
    ) -> Result<Self, String> {
        if source.source_kind != "screen" {
            return Err(format!(
                "Desktop stream screen capture session requires a screen source: {}",
                source.source_id
            ));
        }

        #[cfg(windows)]
        let inner = {
            let _ = frame_rate;
            if !matches!(backend, CaptureBackend::Auto | CaptureBackend::Dxgi) {
                return Err(format!(
                    "Unsupported capture backend '{}' for screen source: {}",
                    backend.label(),
                    source.source_id
                ));
            }
            let session = DxgiCaptureSession::new(source, shared_device, frame_mode)?;
            log::info!(
                "Desktop stream using DXGI screen backend source_id={}",
                source.source_id
            );
            ScreenCaptureInner::Dxgi(session)
        };

        #[cfg(not(windows))]
        let inner = {
            if !matches!(backend, CaptureBackend::Auto | CaptureBackend::Scap) {
                return Err(format!(
                    "Unsupported capture backend '{}' for screen source: {}",
                    backend.label(),
                    source.source_id
                ));
            }
            let session = ScapCaptureSession::new(source, frame_rate)?;
            log::info!(
                "Desktop stream using scap screen backend source_id={}",
                source.source_id
            );
            ScreenCaptureInner::Scap(session)
        };

        Ok(Self { inner })
    }

    pub fn capture_frame(
        &mut self,
        target_width: u32,
        target_height: u32,
        frame_mode: CaptureFrameMode,
    ) -> Result<PreparedVideoFrame, String> {
        match &mut self.inner {
            #[cfg(windows)]
            ScreenCaptureInner::Dxgi(s) => s.capture_frame(target_width, target_height, frame_mode),
            #[cfg(not(windows))]
            ScreenCaptureInner::Scap(s) => s.capture_frame(target_width, target_height, frame_mode),
        }
    }

    pub fn stop(&mut self) {
        match &mut self.inner {
            #[cfg(windows)]
            ScreenCaptureInner::Dxgi(_) => {}
            #[cfg(not(windows))]
            ScreenCaptureInner::Scap(s) => s.stop(),
        }
    }
}

pub fn source_is_alive(source: &ResolvedCaptureSource) -> bool {
    match source.source_kind.as_str() {
        "window" => window_source_is_alive(source.native_id),
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
    let target_width = normalize_even_dimension(target_width.max(1));
    let target_height = normalize_even_dimension(target_height.max(1));

    if frame_width == target_width && frame_height == target_height {
        let expected_len = (frame_width as usize)
            .saturating_mul(frame_height as usize)
            .saturating_mul(4);
        if frame_raw.len() != expected_len {
            return Err("Desktop stream frame had an invalid RGBA buffer".to_owned());
        }
        return Ok(PreparedVideoFrame {
            width: frame_width,
            height: frame_height,
            pixel_format: PreparedFramePixelFormat::Rgba,
            pixels: frame_raw,
            #[cfg(windows)]
            gpu_texture: None,
        });
    }

    let frame = RgbaImage::from_raw(frame_width, frame_height, frame_raw)
        .ok_or_else(|| "Desktop stream frame had an invalid RGBA buffer".to_owned())?;
    let composited = letterbox_image(&frame, target_width, target_height);

    Ok(PreparedVideoFrame {
        width: composited.width(),
        height: composited.height(),
        pixel_format: PreparedFramePixelFormat::Rgba,
        pixels: composited.into_raw(),
        #[cfg(windows)]
        gpu_texture: None,
    })
}

#[cfg(windows)]
fn prepare_bgra_frame_for_upload(
    frame_width: u32,
    frame_height: u32,
    frame_raw: Vec<u8>,
) -> Result<PreparedVideoFrame, String> {
    let expected_len = (frame_width as usize)
        .saturating_mul(frame_height as usize)
        .saturating_mul(4);
    if frame_raw.len() != expected_len {
        return Err("Desktop stream frame had an invalid BGRA buffer".to_owned());
    }

    Ok(PreparedVideoFrame {
        width: frame_width,
        height: frame_height,
        pixel_format: PreparedFramePixelFormat::Bgra,
        pixels: frame_raw,
        #[cfg(windows)]
        gpu_texture: None,
    })
}

#[cfg(windows)]
fn prepare_initial_window_snapshot_frame(
    native_id: u32,
    target_width: u32,
    target_height: u32,
    frame_mode: CaptureFrameMode,
) -> Result<PreparedVideoFrame, String> {
    let (width, height, bgra) = capture_window_snapshot_bgra(HWND(native_id as _))
        .map_err(|err| format!("pending::{err}"))?;
    match frame_mode {
        CaptureFrameMode::SourceBgraFrame
        | CaptureFrameMode::SourceBgraTextureFrame
        | CaptureFrameMode::SourceHdrTextureFrame => {
            prepare_bgra_frame_for_upload(width, height, bgra)
        }
        CaptureFrameMode::TargetRgbaFrame => {
            let rgba = bgra_to_rgba(bgra);
            prepare_rgba_frame(width, height, rgba, target_width, target_height)
        }
    }
}

#[cfg(windows)]
fn prepare_gpu_texture_frame(
    frame_width: u32,
    frame_height: u32,
    texture: ID3D11Texture2D,
    pixel_format: PreparedFramePixelFormat,
) -> Result<PreparedVideoFrame, String> {
    if frame_width == 0 || frame_height == 0 {
        return Err("Desktop stream frame had an invalid GPU texture size".to_owned());
    }

    Ok(PreparedVideoFrame {
        width: frame_width,
        height: frame_height,
        pixel_format,
        pixels: Vec::new(),
        gpu_texture: Some(texture),
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
    let composited = RgbaImage::from_raw(prepared.width, prepared.height, prepared.pixels)
        .ok_or_else(|| "Desktop stream frame had an invalid RGBA buffer".to_owned())?;
    let rgb = DynamicImage::ImageRgba8(composited).to_rgb8();
    let mut buffer = Cursor::new(Vec::new());
    let mut encoder = JpegEncoder::new_with_quality(&mut buffer, jpeg_quality.max(30));
    encoder
        .encode_image(&DynamicImage::ImageRgb8(rgb))
        .map_err(|err| err.to_string())?;
    Ok(buffer.into_inner())
}

#[cfg(not(windows))]
fn format_scap_build_error(kind: &str, source_id: &str, error: ScapCapturerBuildError) -> String {
    match error {
        ScapCapturerBuildError::NotSupported => {
            format!("Scap {kind} capture is not supported for source: {source_id}")
        }
        ScapCapturerBuildError::PermissionNotGranted => {
            format!("Scap {kind} capture permission is not granted for source: {source_id}")
        }
        ScapCapturerBuildError::BackendInitializationFailed(message) => {
            format!(
                "Scap {kind} capture backend initialization failed for source {source_id}: {message}"
            )
        }
    }
}

#[cfg(not(windows))]
fn find_scap_target(source: &ResolvedCaptureSource) -> Result<ScapTarget, String> {
    for target in scap_get_all_targets() {
        match (&target, source.source_kind.as_str()) {
            (ScapTarget::Window(window), "window") if window.id == source.native_id => {
                return Ok(target);
            }
            (ScapTarget::Display(display), "screen") if display.id == source.native_id => {
                return Ok(target);
            }
            _ => {}
        }
    }
    Err(format!(
        "Scap target not found for source: {} ({})",
        source.source_id, source.source_kind
    ))
}

#[cfg(not(windows))]
fn prepared_frame_from_scap_frame(
    frame: ScapFrame,
    target_width: u32,
    target_height: u32,
) -> Result<PreparedVideoFrame, String> {
    match frame {
        ScapFrame::Video(video) => {
            prepared_frame_from_scap_video(video, target_width, target_height)
        }
        ScapFrame::Audio(_) => {
            Err("Scap returned an audio frame while video capture was expected.".to_owned())
        }
    }
}

#[cfg(not(windows))]
fn prepared_frame_from_scap_video(
    frame: ScapVideoFrame,
    target_width: u32,
    target_height: u32,
) -> Result<PreparedVideoFrame, String> {
    match frame {
        ScapVideoFrame::BGRA(frame) => {
            let rgba = bgra_to_rgba(frame.data);
            prepare_rgba_frame(
                frame.width.max(1) as u32,
                frame.height.max(1) as u32,
                rgba,
                target_width,
                target_height,
            )
        }
        ScapVideoFrame::BGRx(frame) => {
            let rgba = bgrx_to_rgba(frame.data);
            prepare_rgba_frame(
                frame.width.max(1) as u32,
                frame.height.max(1) as u32,
                rgba,
                target_width,
                target_height,
            )
        }
        ScapVideoFrame::BGR0(frame) => {
            let rgba = bgrx_to_rgba(frame.data);
            prepare_rgba_frame(
                frame.width.max(1) as u32,
                frame.height.max(1) as u32,
                rgba,
                target_width,
                target_height,
            )
        }
        ScapVideoFrame::RGB(frame) => {
            let rgba = rgb_to_rgba(frame.data);
            prepare_rgba_frame(
                frame.width.max(1) as u32,
                frame.height.max(1) as u32,
                rgba,
                target_width,
                target_height,
            )
        }
        ScapVideoFrame::RGBx(frame) => {
            let rgba = rgbx_to_rgba(frame.data);
            prepare_rgba_frame(
                frame.width.max(1) as u32,
                frame.height.max(1) as u32,
                rgba,
                target_width,
                target_height,
            )
        }
        ScapVideoFrame::XBGR(frame) => {
            let rgba = xbgr_to_rgba(frame.data);
            prepare_rgba_frame(
                frame.width.max(1) as u32,
                frame.height.max(1) as u32,
                rgba,
                target_width,
                target_height,
            )
        }
        ScapVideoFrame::YUVFrame(_) => Err("Scap YUV frames are not yet supported.".to_owned()),
    }
}

fn bgra_to_rgba(data: Vec<u8>) -> Vec<u8> {
    let mut rgba = data;
    for chunk in rgba.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
    rgba
}

#[cfg(not(windows))]
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

#[cfg(windows)]
pub fn window_handle_is_alive(native_id: u32) -> bool {
    unsafe { IsWindow(Some(HWND(native_id as _))).as_bool() }
}

#[cfg(not(windows))]
pub fn window_handle_is_alive(_native_id: u32) -> bool {
    true
}

#[cfg(windows)]
fn window_process_id(native_id: u32) -> Option<u32> {
    let mut pid = 0_u32;
    unsafe {
        let _ = GetWindowThreadProcessId(HWND(native_id as _), Some(&mut pid));
    }
    (pid != 0).then_some(pid)
}

#[cfg(not(windows))]
fn window_process_id(_native_id: u32) -> Option<u32> {
    None
}

#[cfg(not(windows))]
fn scap_window_target_exists(native_id: u32) -> bool {
    scap_get_all_targets()
        .into_iter()
        .any(|target| matches!(target, ScapTarget::Window(window) if window.id == native_id))
}

// On Windows, IsWindow() is sufficient — scap_window_target_exists() calls
// scap_get_all_targets() which enumerates every window on every 250 ms tick,
// far more expensive than needed for a simple liveness check.
#[cfg(windows)]
fn window_source_is_alive(native_id: u32) -> bool {
    window_handle_is_alive(native_id)
}

#[cfg(not(windows))]
fn window_source_is_alive(native_id: u32) -> bool {
    window_handle_is_alive(native_id) && scap_window_target_exists(native_id)
}

#[derive(Debug, Clone, Default)]
struct CaptureAdapterInfo {
    adapter_name: Option<String>,
    output_name: Option<String>,
}

#[cfg(windows)]
fn resolve_capture_adapter_info(source_kind: &str, native_id: u32) -> CaptureAdapterInfo {
    let monitor = match source_kind {
        "screen" => HMONITOR(native_id as usize as *mut core::ffi::c_void),
        "window" => unsafe { MonitorFromWindow(HWND(native_id as _), MONITOR_DEFAULTTONEAREST) },
        _ => HMONITOR::default(),
    };

    if monitor.0.is_null() {
        return CaptureAdapterInfo::default();
    }

    find_dxgi_output_for_monitor(monitor).unwrap_or_default()
}

#[cfg(not(windows))]
fn resolve_capture_adapter_info(_source_kind: &str, _native_id: u32) -> CaptureAdapterInfo {
    CaptureAdapterInfo::default()
}

#[cfg(windows)]
fn find_dxgi_output_for_monitor(monitor: HMONITOR) -> Option<CaptureAdapterInfo> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1().ok()? };
    let mut adapter_index = 0;
    loop {
        let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapters1(adapter_index).ok()? };
        let adapter_desc = unsafe { adapter.GetDesc1().ok()? };
        let adapter_name = trim_utf16_nul(&adapter_desc.Description);

        let mut output_index = 0;
        loop {
            let output: IDXGIOutput = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(_) => break,
            };
            let output_desc = unsafe { output.GetDesc().ok()? };
            if output_desc.Monitor == monitor {
                return Some(CaptureAdapterInfo {
                    adapter_name,
                    output_name: trim_utf16_nul(&output_desc.DeviceName),
                });
            }
            output_index += 1;
        }

        adapter_index += 1;
    }
}

#[cfg(windows)]
fn trim_utf16_nul(value: &[u16]) -> Option<String> {
    let end = value
        .iter()
        .position(|entry| *entry == 0)
        .unwrap_or(value.len());
    let trimmed = String::from_utf16_lossy(&value[..end]).trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn letterbox_image(frame: &RgbaImage, target_width: u32, target_height: u32) -> RgbaImage {
    if frame.width() == target_width && frame.height() == target_height {
        return frame.clone();
    }

    let (resized_width, resized_height) =
        compute_letterbox_fit(frame.width(), frame.height(), target_width, target_height);
    let resized = resize_frame_for_stream(frame, resized_width, resized_height);

    if resized_width == target_width && resized_height == target_height {
        return resized;
    }

    let mut canvas = RgbaImage::from_pixel(target_width, target_height, Rgba([0, 0, 0, 255]));
    let x = i64::from((target_width - resized_width) / 2);
    let y = i64::from((target_height - resized_height) / 2);
    overlay(&mut canvas, &resized, x, y);
    canvas
}

fn compute_letterbox_fit(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> (u32, u32) {
    let width_scale = target_width as f32 / source_width.max(1) as f32;
    let height_scale = target_height as f32 / source_height.max(1) as f32;
    let scale = width_scale.min(height_scale).max(0.000_1);
    let resized_width = ((source_width as f32 * scale).round() as u32).clamp(1, target_width);
    let resized_height = ((source_height as f32 * scale).round() as u32).clamp(1, target_height);
    (resized_width, resized_height)
}

fn resize_frame_for_stream(frame: &RgbaImage, target_width: u32, target_height: u32) -> RgbaImage {
    if frame.width() == target_width && frame.height() == target_height {
        return frame.clone();
    }

    let width_downscale = frame.width().max(1) as f32 / target_width.max(1) as f32;
    let height_downscale = frame.height().max(1) as f32 / target_height.max(1) as f32;
    let max_downscale = width_downscale.max(height_downscale);

    // Large 1440p/4K -> 1080p/720p reductions are the hottest path during capture.
    // `thumbnail` is materially cheaper there, while smaller resizes keep the
    // higher-quality filters for detail retention.
    if max_downscale >= 1.85 {
        return thumbnail(frame, target_width, target_height);
    }

    let filter = if max_downscale > 1.0 {
        FilterType::Triangle
    } else {
        FilterType::CatmullRom
    };
    resize(frame, target_width, target_height, filter)
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
    use super::{compute_letterbox_fit, letterbox_image, parse_source_id};
    use image::{Rgba, RgbaImage};

    #[test]
    fn parses_window_source_id() {
        assert_eq!(parse_source_id("window:42"), Some(("window", 42)));
    }

    #[test]
    fn rejects_invalid_source_id() {
        assert_eq!(parse_source_id("window:not-a-number"), None);
        assert_eq!(parse_source_id("window"), None);
    }

    #[test]
    fn computes_fit_for_matching_aspect_ratio() {
        assert_eq!(compute_letterbox_fit(3840, 2160, 1920, 1080), (1920, 1080));
    }

    #[test]
    fn computes_fit_for_letterboxed_aspect_ratio() {
        assert_eq!(compute_letterbox_fit(1024, 768, 1280, 720), (960, 720));
    }

    #[test]
    fn avoids_black_bars_when_resize_fills_target() {
        let frame = RgbaImage::from_pixel(1920, 1080, Rgba([255, 255, 255, 255]));
        let result = letterbox_image(&frame, 1280, 720);

        assert_eq!(result.width(), 1280);
        assert_eq!(result.height(), 720);
        assert_eq!(result.get_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn letterboxes_when_aspect_ratios_differ() {
        let frame = RgbaImage::from_pixel(1024, 768, Rgba([255, 255, 255, 255]));
        let result = letterbox_image(&frame, 1280, 720);

        assert_eq!(result.width(), 1280);
        assert_eq!(result.height(), 720);
        assert_eq!(result.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(result.get_pixel(640, 360).0, [255, 255, 255, 255]);
    }
}
