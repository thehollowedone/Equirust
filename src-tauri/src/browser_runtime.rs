use crate::settings::{PersistedState, Settings};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRuntimeKind {
    WebView2,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserWindowOptions {
    pub user_agent: Option<String>,
    pub additional_args: Option<String>,
}

pub fn active_browser_runtime() -> BrowserRuntimeKind {
    BrowserRuntimeKind::WebView2
}

pub fn active_browser_runtime_name() -> &'static str {
    match active_browser_runtime() {
        BrowserRuntimeKind::WebView2 => "webview2",
    }
}

pub fn profiling_diagnostics_enabled() -> bool {
    std::env::args().any(|arg| arg == "--profiling-diagnostics")
        || matches!(
            std::env::var("EQUIRUST_PROFILE_DIAGNOSTICS").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

pub fn control_runtime_enabled() -> bool {
    std::env::args().any(|arg| arg == "--control-runtime")
        || matches!(
            std::env::var("EQUIRUST_CONTROL_RUNTIME").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

pub fn host_runtime_enabled(control_runtime: bool) -> bool {
    !control_runtime
        && !std::env::args().any(|arg| arg == "--no-host-runtime" || arg == "--no-host-ui")
        && !matches!(
            std::env::var("EQUIRUST_NO_HOST_RUNTIME").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

pub fn mod_runtime_enabled(control_runtime: bool) -> bool {
    !control_runtime
        && !std::env::args().any(|arg| arg == "--no-mod-runtime" || arg == "--no-vencord")
        && !matches!(
            std::env::var("EQUIRUST_NO_MOD_RUNTIME").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

pub fn edge_client_hints_enabled() -> bool {
    let explicitly_enabled = std::env::args()
        .any(|arg| arg == "--edge-ua" || arg == "--edge-client-hints")
        || matches!(
            std::env::var("EQUIRUST_EDGE_CLIENT_HINTS").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
    let explicitly_disabled = std::env::args()
        .any(|arg| arg == "--no-edge-client-hints" || arg == "--no-edge-ua")
        || matches!(
            std::env::var("EQUIRUST_EDGE_CLIENT_HINTS").as_deref(),
            Ok("0") | Ok("false") | Ok("FALSE") | Ok("no") | Ok("NO")
        );

    #[cfg(target_os = "windows")]
    {
        explicitly_enabled || !explicitly_disabled
    }

    #[cfg(not(target_os = "windows"))]
    {
        explicitly_enabled && !explicitly_disabled
    }
}

#[cfg(target_os = "windows")]
pub fn runtime_diagnostics_enabled(settings: &Settings) -> bool {
    settings.debug_standard_diagnostics_enabled()
}

#[cfg(target_os = "windows")]
pub fn read_process_working_set_kb(pid: u32) -> Option<u64> {
    use std::mem::size_of;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let process_handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()? };
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let success = unsafe {
        K32GetProcessMemoryInfo(
            process_handle,
            &mut counters,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .as_bool()
    };
    let _ = unsafe { CloseHandle(process_handle) };
    if !success {
        return None;
    }
    Some((counters.WorkingSetSize / 1024) as u64)
}

#[cfg(target_os = "windows")]
pub fn resolve_download_target_path(
    app: &AppHandle,
    uri: &str,
    suggested_path: &str,
) -> Option<String> {
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    if !suggested_path.trim().is_empty() {
        return Some(suggested_path.to_owned());
    }

    let fallback_name = url::Url::parse(uri).ok().and_then(|parsed| {
        parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .map(ToOwned::to_owned)
            .and_then(|value| (!value.trim().is_empty()).then_some(value))
    });
    let file_name = Path::new(uri)
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .and_then(|value| (!value.trim().is_empty()).then_some(value))
        .or(fallback_name)
        .map(|value| sanitize_download_file_name(&value))
        .unwrap_or_else(|| "download.bin".to_owned());

    let mut base_dir = app
        .path()
        .download_dir()
        .ok()
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|path| path.join("Downloads"))
        })?
        .join("Equirust");
    if std::fs::create_dir_all(&base_dir).is_err() {
        return None;
    }

    let candidate = base_dir.join(&file_name);
    if !candidate.exists() {
        return Some(candidate.to_string_lossy().to_string());
    }

    let stem = Path::new(&file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("download");
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let unique_name = if extension.is_empty() {
        format!("{stem}-{timestamp}")
    } else {
        format!("{stem}-{timestamp}.{extension}")
    };
    base_dir.push(unique_name);
    Some(base_dir.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
fn sanitize_download_file_name(file_name: &str) -> String {
    const INVALID_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let sanitized: String = file_name
        .chars()
        .map(|character| {
            if INVALID_CHARS.contains(&character) || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect();
    if sanitized.trim().is_empty() {
        "download.bin".to_owned()
    } else {
        sanitized
    }
}

#[cfg(target_os = "windows")]
pub fn is_trusted_discord_origin(uri: &str) -> bool {
    let Ok(parsed) = url::Url::parse(uri) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    host == "discord.com"
        || host.ends_with(".discord.com")
        || host == "discordapp.com"
        || host.ends_with(".discordapp.com")
}

#[cfg(target_os = "windows")]
pub fn should_reload_after_process_failure(
    kind: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND,
) -> bool {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    };

    kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
        || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE
        || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED
}

#[cfg(target_os = "windows")]
pub fn webview_permission_kind_name(
    kind: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND,
) -> &'static str {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
    };

    if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
        "microphone"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA {
        "camera"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS {
        "notifications"
    } else if kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ {
        "clipboard-read"
    } else {
        "other"
    }
}

#[cfg(target_os = "windows")]
pub fn webview_script_dialog_kind_name(
    kind: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_SCRIPT_DIALOG_KIND,
) -> &'static str {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT, COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD,
        COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM, COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT,
    };

    if kind == COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT {
        "alert"
    } else if kind == COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM {
        "confirm"
    } else if kind == COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD {
        "beforeunload"
    } else if kind == COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT {
        "prompt"
    } else {
        "unknown"
    }
}

#[cfg(target_os = "windows")]
pub fn webview_process_failed_kind_name(
    kind: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND,
) -> &'static str {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED,
    };

    if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
        "render-exited"
    } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE {
        "render-unresponsive"
    } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED {
        "gpu-exited"
    } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
        "browser-exited"
    } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED {
        "utility-exited"
    } else {
        "other"
    }
}

#[cfg(target_os = "windows")]
pub fn webview_process_failed_reason_name(
    reason: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_REASON,
) -> &'static str {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED,
        COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED,
        COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY,
        COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED,
        COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED,
        COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED,
        COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE,
    };

    if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED {
        "crashed"
    } else if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY {
        "out-of-memory"
    } else if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE {
        "unresponsive"
    } else if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED {
        "terminated"
    } else if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED {
        "launch-failed"
    } else if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED {
        "profile-deleted"
    } else if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED {
        "unexpected"
    } else {
        "unknown"
    }
}

pub fn main_window_options(settings: &Settings, state: &PersistedState) -> BrowserWindowOptions {
    BrowserWindowOptions {
        user_agent: browser_user_agent(settings),
        additional_args: browser_additional_args(settings, state),
    }
}

pub fn external_window_options(
    settings: &Settings,
    state: &PersistedState,
) -> BrowserWindowOptions {
    main_window_options(settings, state)
}

pub fn browser_user_agent(_settings: &Settings) -> Option<String> {
    match active_browser_runtime() {
        BrowserRuntimeKind::WebView2 => None,
    }
}

pub fn standard_http_user_agent() -> String {
    match active_browser_runtime() {
        BrowserRuntimeKind::WebView2 => webview2_http_user_agent().unwrap_or_else(|_| {
            format!(
                "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36 Edg/145.0.0.0",
                edge_platform_token()
            )
        }),
    }
}

pub fn browser_additional_args(settings: &Settings, state: &PersistedState) -> Option<String> {
    match active_browser_runtime() {
        BrowserRuntimeKind::WebView2 => webview2_additional_args(settings, state),
    }
}

pub fn debug_page_targets(target: &str) -> Result<(&'static str, &'static str), String> {
    match active_browser_runtime() {
        BrowserRuntimeKind::WebView2 => match target {
            "gpu" => Ok(("edge://gpu", "chrome://gpu")),
            "webrtc-internals" => Ok(("edge://webrtc-internals", "chrome://webrtc-internals")),
            _ => Err(format!("unsupported debug target: {target}")),
        },
    }
}

fn webview2_http_user_agent() -> Result<String, String> {
    let version = tauri::webview_version().map_err(|err| err.to_string())?;
    let platform = edge_platform_token();

    Ok(format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version} Safari/537.36 Edg/{version}"
    ))
}

fn webview2_additional_args(settings: &Settings, state: &PersistedState) -> Option<String> {
    let mut args = Vec::new();
    let mut disable_features = vec![
        "WinRetrieveSuggestionsOnlyOnDemand".to_owned(),
        "HardwareMediaKeyHandling".to_owned(),
        "MediaSessionService".to_owned(),
        "AcceleratedVideoEncoder".to_owned(),
        "AcceleratedVideoDecoder".to_owned(),
    ];
    let _ = settings.hardware_video_acceleration;
    if settings.hardware_acceleration != Some(true) {
        args.extend([
            "--disable-gpu".to_owned(),
            "--disable-gpu-compositing".to_owned(),
            "--disable-gpu-rasterization".to_owned(),
            "--disable-zero-copy".to_owned(),
        ]);
    }

    if settings.disable_smooth_scroll == Some(true) {
        args.push("--disable-smooth-scrolling".to_owned());
    }

    if settings.middle_click_autoscroll == Some(true) {
        args.push("--enable-blink-features=MiddleClickAutoscroll".to_owned());
    }

    args.push("--autoplay-policy=no-user-gesture-required".to_owned());
    args.push("--no-first-run".to_owned());

    #[cfg(target_os = "windows")]
    {
        disable_features.push("CalculateNativeWinOcclusion".to_owned());
        args.extend([
            "--edge-webview-foreground-boost-opt-in".to_owned(),
            "--msWebView2NativeEventDispatch".to_owned(),
            "--msWebView2CodeCache".to_owned(),
            "--site-per-process".to_owned(),
            "--UseNativeThreadPool".to_owned(),
            "--UseBackgroundNativeThreadPool".to_owned(),
        ]);
    }

    if !disable_features.is_empty() {
        args.push(format!("--disable-features={}", disable_features.join(",")));
    }

    if let Some(extra) = state
        .launch_arguments
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(extra.to_owned());
    }

    if args.is_empty() {
        None
    } else {
        Some(args.join(" "))
    }
}

fn edge_platform_token() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Windows NT 10.0; Win64; x64"
    }

    #[cfg(target_os = "macos")]
    {
        "Macintosh; Intel Mac OS X 10_15_7"
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "X11; Linux x86_64"
    }
}
