use crate::{
    browser_runtime, csp, desktop, discord, mod_runtime, privacy,
    settings::{Settings, TransparencyOption, WindowBounds},
    store::PersistedStore,
    tray,
};
use std::{
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};
use tauri::{
    utils::config::BackgroundThrottlingPolicy,
    webview::PageLoadEvent,
    window::{Color, Effect, EffectsBuilder},
    AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const MIN_WIDTH: u32 = 940;
const MIN_HEIGHT: u32 = 500;
const DEFAULT_WEBVIEW_BACKGROUND: Color = Color(5, 6, 8, 255);

#[derive(Default)]
pub struct WebviewRuntimeState {
    main_browser_pid: Mutex<Option<u32>>,
    process_failure_recovery: Mutex<ProcessFailureRecoveryState>,
}

#[derive(Clone)]
struct PersistedWindowState {
    bounds: WindowBounds,
    maximized: bool,
    minimized: bool,
}

static WINDOW_STATE_SAVE_TX: OnceLock<mpsc::Sender<PersistedWindowState>> = OnceLock::new();
const WINDOW_STATE_SAVE_DEBOUNCE: Duration = Duration::from_millis(225);
const PROCESS_FAILURE_RELOAD_BACKOFF: Duration = Duration::from_secs(8);
const PROCESS_FAILURE_RESET_WINDOW: Duration = Duration::from_secs(90);
const PROCESS_FAILURE_MAX_RELOADS: u32 = 2;

#[derive(Default)]
struct ProcessFailureRecoveryState {
    last_failure_at: Option<Instant>,
    last_reload_at: Option<Instant>,
    reload_attempts: u32,
    recovery_latched: bool,
}

enum ProcessFailureRecoveryDecision {
    Reload { attempt: u32 },
    Suppress { reason: &'static str },
}

fn record_process_failure_recovery<R: Runtime>(
    app: &AppHandle<R>,
) -> ProcessFailureRecoveryDecision {
    let Some(state) = app.try_state::<WebviewRuntimeState>() else {
        return ProcessFailureRecoveryDecision::Suppress {
            reason: "webview runtime state unavailable",
        };
    };

    let mut guard = match state.process_failure_recovery.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::error!("WebView2 recovery state mutex was poisoned; suppressing auto-reload");
            poisoned.into_inner()
        }
    };
    let now = Instant::now();

    if let Some(last_failure_at) = guard.last_failure_at {
        if now.duration_since(last_failure_at) >= PROCESS_FAILURE_RESET_WINDOW {
            guard.reload_attempts = 0;
            guard.recovery_latched = false;
            guard.last_reload_at = None;
        }
    }
    guard.last_failure_at = Some(now);

    if guard.recovery_latched {
        return ProcessFailureRecoveryDecision::Suppress {
            reason: "automatic recovery already disabled after repeated failures",
        };
    }

    if let Some(last_reload_at) = guard.last_reload_at {
        if now.duration_since(last_reload_at) < PROCESS_FAILURE_RELOAD_BACKOFF {
            return ProcessFailureRecoveryDecision::Suppress {
                reason: "automatic recovery backoff window is still active",
            };
        }
    }

    if guard.reload_attempts >= PROCESS_FAILURE_MAX_RELOADS {
        guard.recovery_latched = true;
        return ProcessFailureRecoveryDecision::Suppress {
            reason:
                "automatic recovery limit reached; relaunch with --control-runtime for safe mode",
        };
    }

    guard.reload_attempts = guard.reload_attempts.saturating_add(1);
    guard.last_reload_at = Some(now);
    ProcessFailureRecoveryDecision::Reload {
        attempt: guard.reload_attempts,
    }
}

pub fn set_main_browser_pid<R: Runtime>(app: &AppHandle<R>, pid: Option<u32>) {
    let Some(state) = app.try_state::<WebviewRuntimeState>() else {
        return;
    };
    let mut guard = match state.main_browser_pid.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    *guard = pid;
}

pub fn clear_main_browser_pid_if_matches<R: Runtime>(app: &AppHandle<R>, pid: u32) {
    let Some(state) = app.try_state::<WebviewRuntimeState>() else {
        return;
    };
    let mut guard = match state.main_browser_pid.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if guard.as_ref().copied() == Some(pid) {
        *guard = None;
    }
}

pub fn get_main_browser_pid<R: Runtime>(app: &AppHandle<R>) -> Option<u32> {
    let state = app.try_state::<WebviewRuntimeState>()?;
    let guard = state.main_browser_pid.lock().ok()?;
    *guard
}

pub fn create_main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let snapshot = app.state::<PersistedStore>().snapshot();
    let control_runtime = browser_runtime::control_runtime_enabled();
    let spoof_edge_client_hints = browser_runtime::edge_client_hints_enabled();
    let install_host_runtime = browser_runtime::host_runtime_enabled(control_runtime);
    let install_mod_runtime = browser_runtime::mod_runtime_enabled(control_runtime);
    let use_custom_title_bar = should_use_custom_title_bar(&snapshot, control_runtime);
    let mod_runtime_profile = mod_runtime::resolve_mod_runtime_profile();
    let start_hidden = should_start_hidden(&snapshot);
    let disable_min_size = snapshot.settings.disable_min_size == Some(true);
    let static_title = snapshot.settings.static_title == Some(true);
    let startup_route = discord::route_from_process_args();
    let initial_webview_url = WebviewUrl::External(discord::discord_target_url(
        &snapshot.settings,
        startup_route.clone(),
    )?);
    let fallback_bounds = WindowBounds {
        x: 0,
        y: 0,
        width: DEFAULT_WIDTH as i32,
        height: DEFAULT_HEIGHT as i32,
    };
    let bounds = snapshot
        .state
        .window_bounds
        .clone()
        .unwrap_or(fallback_bounds);
    let width = clamp_dimension(bounds.width, disable_min_size, MIN_WIDTH);
    let height = clamp_dimension(bounds.height, disable_min_size, MIN_HEIGHT);
    let browser_options = browser_runtime::main_window_options(&snapshot.settings, &snapshot.state);
    let bridge_seed = mod_runtime::bridge_seed(
        app,
        &snapshot.paths,
        &snapshot.settings,
        &mod_runtime_profile,
    )
    .map_err(|err| err.to_string())?;
    let mod_runtime_renderer = mod_runtime::renderer_script(app).ok();
    let bootstrap_script = desktop::bootstrap_script(
        &bridge_seed,
        mod_runtime_renderer.as_deref(),
        control_runtime,
        install_host_runtime,
        install_mod_runtime,
        spoof_edge_client_hints,
    )
    .map_err(|err| err.to_string())?;

    let mut builder = WebviewWindowBuilder::new(app, "main", initial_webview_url)
        .title("Equirust")
        .inner_size(width as f64, height as f64)
        .resizable(true)
        .visible(!start_hidden)
        .background_color(DEFAULT_WEBVIEW_BACKGROUND)
        .background_throttling(BackgroundThrottlingPolicy::Disabled)
        .initialization_script(bootstrap_script)
        .on_web_resource_request({
            let app = app.clone();
            move |request, response| {
                csp::apply_response_overrides(&app, &request, response);
            }
        })
        .on_navigation({
            let app = app.clone();
            move |url| discord::handle_main_window_navigation(&app, url)
        })
        .on_new_window({
            let app = app.clone();
            move |url, _features| discord::handle_main_window_new_window(&app, &url)
        })
        .on_document_title_changed(move |window, title| {
            if !static_title {
                let _ = window.set_title(&title);
            }
        })
        .on_page_load(|_window, payload| match payload.event() {
            PageLoadEvent::Started => {
                log::info!(
                    "Page load started: {}",
                    privacy::sanitize_url_for_log(payload.url().as_str())
                );
            }
            PageLoadEvent::Finished => {
                log::info!(
                    "Page load finished: {}",
                    privacy::sanitize_url_for_log(payload.url().as_str())
                );
            }
        });

    #[cfg(target_os = "windows")]
    {
        if use_custom_title_bar {
            builder = builder.decorations(false);
        }

        if let Some(effects) = resolve_window_effects(&snapshot.settings) {
            builder = builder
                .transparent(true)
                .background_color(Color(0, 0, 0, 0))
                .effects(effects);
            log::info!(
                "Applying Windows transparency effect {:?}",
                snapshot.settings.transparency_option
            );
        }
    }

    if control_runtime {
        log::info!("Starting main window in control-runtime mode");
    }
    log::info!("Starting main window directly on Discord");
    if use_custom_title_bar {
        log::info!("Using Rust-managed Discord titlebar");
    }
    log::info!(
        "Runtime profile: control_runtime={} host_runtime={} mod_runtime={} minimal_mod_runtime={} disabled_mod_plugins={} edge_client_hints={}",
        control_runtime,
        install_host_runtime,
        install_mod_runtime,
        mod_runtime_profile.minimal,
        mod_runtime_profile.disabled_plugins.len(),
        spoof_edge_client_hints
    );

    if let Some(user_agent) = browser_options.user_agent.as_deref() {
        builder = builder.user_agent(user_agent);
    }

    if let Some(additional_browser_args) = browser_options.additional_args.as_deref() {
        log::info!(
            "Applying WebView browser args: {}",
            privacy::sanitize_text_for_log(additional_browser_args)
        );
        builder = builder.additional_browser_args(additional_browser_args);
    }

    if let Some((x, y)) = resolve_window_position(app, &snapshot, &bounds, width, height) {
        builder = builder.position(x, y);
    }

    if !disable_min_size {
        builder = builder.min_inner_size(MIN_WIDTH as f64, MIN_HEIGHT as f64);
    }

    if snapshot.state.maximized == Some(true) {
        builder = builder.maximized(true);
    }

    let window = builder.build().map_err(|err| err.to_string())?;
    #[cfg(target_os = "windows")]
    install_windows_webview2_host_hooks(&window, app, &snapshot.settings);
    ensure_window_state_worker(app.clone());
    if start_hidden {
        let _ = window.set_skip_taskbar(true);
    }
    attach_main_window_tracking(window.clone());

    Ok(window)
}

fn attach_main_window_tracking(window: WebviewWindow) {
    let tracked_window = window.clone();

    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            if tray::should_minimize_to_tray(&tracked_window.app_handle()) {
                api.prevent_close();
                if let Err(err) = persist_main_window_state(&tracked_window) {
                    log::warn!("Failed to persist main window state before tray hide: {err}");
                }
                if let Err(err) = tray::hide_main_window(&tracked_window.app_handle()) {
                    log::warn!("Failed to minimize main window to tray: {err}");
                }
            } else if let Err(err) = persist_main_window_state(&tracked_window) {
                log::warn!("Failed to persist main window state: {err}");
            }
        }
        WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
            if let Err(err) = schedule_main_window_state_persist(&tracked_window) {
                log::warn!("Failed to persist main window state: {err}");
            }
        }
        _ => {}
    });
}

fn persist_main_window_state(window: &WebviewWindow) -> Result<(), String> {
    let state = capture_main_window_state(window)?;
    persist_main_window_state_snapshot(&window.app_handle(), state)
}

fn schedule_main_window_state_persist(window: &WebviewWindow) -> Result<(), String> {
    let state = capture_main_window_state(window)?;
    if let Some(sender) = WINDOW_STATE_SAVE_TX.get() {
        sender.send(state).map_err(|err| err.to_string())
    } else {
        persist_main_window_state_snapshot(&window.app_handle(), state)
    }
}

fn capture_main_window_state(window: &WebviewWindow) -> Result<PersistedWindowState, String> {
    let position = window.outer_position().map_err(|err| err.to_string())?;
    let size = window.inner_size().map_err(|err| err.to_string())?;
    let maximized = window.is_maximized().map_err(|err| err.to_string())?;
    let minimized = window.is_minimized().map_err(|err| err.to_string())?;

    Ok(PersistedWindowState {
        bounds: WindowBounds {
            x: position.x,
            y: position.y,
            width: size.width as i32,
            height: size.height as i32,
        },
        maximized,
        minimized,
    })
}

fn persist_main_window_state_snapshot(
    app: &AppHandle,
    state: PersistedWindowState,
) -> Result<(), String> {
    let store = app.state::<PersistedStore>();
    let PersistedWindowState {
        bounds,
        maximized,
        minimized,
    } = state;

    store
        .update_state(|state| {
            state.maximized = Some(maximized);
            state.minimized = Some(minimized);
            state.window_bounds = Some(bounds.clone());
        })
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn ensure_window_state_worker(app: AppHandle) {
    let _ = WINDOW_STATE_SAVE_TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<PersistedWindowState>();
        thread::spawn(move || {
            let app_handle = app;
            loop {
                let mut pending = match rx.recv() {
                    Ok(state) => state,
                    Err(_) => break,
                };

                while let Ok(next_state) = rx.recv_timeout(WINDOW_STATE_SAVE_DEBOUNCE) {
                    pending = next_state;
                }

                if let Err(err) = persist_main_window_state_snapshot(&app_handle, pending) {
                    log::warn!("Failed to persist debounced main window state: {err}");
                }
            }
        });
        tx
    });
}

fn clamp_dimension(saved: i32, disable_min_size: bool, min: u32) -> u32 {
    let saved = u32::try_from(saved).unwrap_or(min);
    if disable_min_size {
        saved.max(1)
    } else {
        saved.max(min)
    }
}

fn should_start_hidden(snapshot: &crate::store::StoreSnapshot) -> bool {
    std::env::args().any(|arg| arg == "--start-minimized")
        && tray::is_tray_enabled(&snapshot.settings)
        && snapshot.settings.minimize_to_tray != Some(false)
}

fn should_use_custom_title_bar(
    snapshot: &crate::store::StoreSnapshot,
    control_runtime: bool,
) -> bool {
    #[cfg(target_os = "windows")]
    {
        !control_runtime && snapshot.settings.custom_title_bar != Some(false)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = snapshot;
        let _ = control_runtime;
        false
    }
}

#[cfg(target_os = "windows")]
fn resolve_window_effects(
    settings: &Settings,
) -> Option<tauri::utils::config::WindowEffectsConfig> {
    let effect = match settings
        .transparency_option
        .clone()
        .unwrap_or(TransparencyOption::None)
    {
        TransparencyOption::None => return None,
        TransparencyOption::Mica => Effect::Mica,
        TransparencyOption::Tabbed => Effect::Tabbed,
        TransparencyOption::Acrylic => Effect::Acrylic,
    };

    Some(EffectsBuilder::new().effect(effect).build())
}

#[cfg(not(target_os = "windows"))]
fn resolve_window_effects(
    _settings: &Settings,
) -> Option<tauri::utils::config::WindowEffectsConfig> {
    None
}

fn resolve_window_position(
    app: &AppHandle,
    snapshot: &crate::store::StoreSnapshot,
    bounds: &WindowBounds,
    width: u32,
    height: u32,
) -> Option<(f64, f64)> {
    if snapshot.state.window_bounds.is_none() {
        return None;
    }

    if is_bounds_visible(app, bounds.x, bounds.y, width, height) {
        Some((bounds.x as f64, bounds.y as f64))
    } else {
        log::warn!(
            "Saved window bounds are outside the current monitor layout, falling back to centered defaults"
        );
        None
    }
}

fn is_bounds_visible(app: &AppHandle, x: i32, y: i32, width: u32, height: u32) -> bool {
    let Ok(monitors) = app.available_monitors() else {
        return true;
    };

    if monitors.is_empty() {
        return true;
    }

    let right = x.saturating_add(width as i32);
    let bottom = y.saturating_add(height as i32);

    monitors.into_iter().any(|monitor| {
        let area = monitor.work_area();
        let left_edge = area.position.x;
        let top_edge = area.position.y;
        let right_edge = area.position.x.saturating_add(area.size.width as i32);
        let bottom_edge = area.position.y.saturating_add(area.size.height as i32);

        !(right < left_edge || x > right_edge || bottom < top_edge || y > bottom_edge)
    })
}

#[cfg(target_os = "windows")]
fn install_windows_webview2_host_hooks(
    window: &WebviewWindow,
    app: &AppHandle,
    settings: &Settings,
) {
    let label = window.label().to_string();
    let app_handle = app.clone();
    let diagnostics_enabled_at_start = browser_runtime::runtime_diagnostics_enabled(settings);
    let callback_label = label.clone();

    let with_webview_result = window.with_webview(move |webview| unsafe {
        use webview2_com::{
            take_pwstr, BrowserProcessExitedEventHandler, DownloadStartingEventHandler,
            PermissionRequestedEventHandler, ProcessFailedEventHandler,
            ScriptDialogOpeningEventHandler,
        };
        use windows_core_v61::{HSTRING, Interface, PWSTR};
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Environment5, ICoreWebView2PermissionRequestedEventArgs2,
            ICoreWebView2PermissionRequestedEventArgs3, ICoreWebView2ProcessFailedEventArgs2,
            ICoreWebView2Settings4, ICoreWebView2_4, COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND,
            COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
            COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
            COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
            COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_PROCESS_FAILED_KIND,
            COREWEBVIEW2_PROCESS_FAILED_REASON, COREWEBVIEW2_SCRIPT_DIALOG_KIND,
            COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD,
            COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT,
        };

        let Ok(core_webview) = webview.controller().CoreWebView2() else {
            log::warn!(
                "Failed to access CoreWebView2 when installing host hooks for {callback_label}"
            );
            return;
        };

        if let Ok(core_settings) = core_webview.Settings() {
            let _ = core_settings.SetIsStatusBarEnabled(false);
            let _ = core_settings.SetAreDefaultScriptDialogsEnabled(false);
            let _ = core_settings.SetAreDevToolsEnabled(
                cfg!(debug_assertions)
                    || diagnostics_enabled_at_start
                    || browser_runtime::profiling_diagnostics_enabled(),
            );
            if let Ok(settings4) = core_settings.cast::<ICoreWebView2Settings4>() {
                let _ = settings4.SetIsPasswordAutosaveEnabled(false);
                let _ = settings4.SetIsGeneralAutofillEnabled(false);
            }
            log::info!(
                "Applied hardened WebView2 settings for {callback_label} (devtools={})",
                cfg!(debug_assertions)
                    || diagnostics_enabled_at_start
                    || browser_runtime::profiling_diagnostics_enabled()
            );
        } else {
            log::warn!("Failed to access WebView2 settings for {callback_label}");
        }

        let mut permission_token = 0i64;
        let permission_label = callback_label.clone();
        let permission_handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };

            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            args.PermissionKind(&mut kind)?;

            let mut uri_ptr = PWSTR::null();
            let _ = args.Uri(&mut uri_ptr);
            let uri = take_pwstr(uri_ptr);
            let trusted_origin = browser_runtime::is_trusted_discord_origin(&uri);

            let decision = if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA
                || kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
            {
                Some(if trusted_origin {
                    COREWEBVIEW2_PERMISSION_STATE_ALLOW
                } else {
                    COREWEBVIEW2_PERMISSION_STATE_DENY
                })
            } else if kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ {
                Some(COREWEBVIEW2_PERMISSION_STATE_DENY)
            } else {
                None
            };

            if let Some(state) = decision {
                args.SetState(state)?;
                if let Ok(args2) = args.cast::<ICoreWebView2PermissionRequestedEventArgs2>() {
                    let _ = args2.SetHandled(true);
                }
                if let Ok(args3) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                    let _ = args3.SetSavesInProfile(false);
                }

                let kind_name = browser_runtime::webview_permission_kind_name(kind);
                if state == COREWEBVIEW2_PERMISSION_STATE_ALLOW {
                    log::info!(
                        "WebView2 permission allowed for {permission_label}: kind={kind_name} trusted_origin={} uri={}",
                        trusted_origin,
                        privacy::sanitize_url_for_log(&uri)
                    );
                } else {
                    log::warn!(
                        "WebView2 permission denied for {permission_label}: kind={kind_name} trusted_origin={} uri={}",
                        trusted_origin,
                        privacy::sanitize_url_for_log(&uri)
                    );
                }
            }

            Ok(())
        }));

        if let Err(err) = core_webview.add_PermissionRequested(&permission_handler, &mut permission_token) {
            log::warn!(
                "Failed to attach WebView2 permission handler for {callback_label}: {err}"
            );
        }

        let mut script_dialog_token = 0i64;
        let script_dialog_label = callback_label.clone();
        let script_dialog_handler =
            ScriptDialogOpeningEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };

                let mut kind = COREWEBVIEW2_SCRIPT_DIALOG_KIND::default();
                let _ = args.Kind(&mut kind);
                if kind == COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT {
                    let mut default_text_ptr = PWSTR::null();
                    let _ = args.DefaultText(&mut default_text_ptr);
                    let _ = take_pwstr(default_text_ptr);
                    let _ = args.SetResultText(&HSTRING::from(""));
                }
                if kind == COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD {
                    let _ = args.Accept();
                }
                log::warn!(
                    "Suppressed WebView2 script dialog for {} kind={} accepted={}",
                    script_dialog_label,
                    browser_runtime::webview_script_dialog_kind_name(kind),
                    kind
                        == COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD
                );
                Ok(())
            }));

        if let Err(err) = core_webview.add_ScriptDialogOpening(&script_dialog_handler, &mut script_dialog_token) {
            log::warn!(
                "Failed to attach WebView2 script-dialog handler for {callback_label}: {err}"
            );
        }

        if let Ok(core_webview4) = core_webview.cast::<ICoreWebView2_4>() {
            let mut download_token = 0i64;
            let download_label = callback_label.clone();
            let download_app = app_handle.clone();
            let download_handler = DownloadStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };

                let mut uri_ptr = PWSTR::null();
                let mut path_ptr = PWSTR::null();
                if let Ok(download_operation) = args.DownloadOperation() {
                    let _ = download_operation.Uri(&mut uri_ptr);
                }
                let _ = args.ResultFilePath(&mut path_ptr);
                let uri = take_pwstr(uri_ptr);
                let suggested = take_pwstr(path_ptr);

                if let Some(download_path) =
                    browser_runtime::resolve_download_target_path(&download_app, &uri, &suggested)
                {
                    let _ = args.SetResultFilePath(&HSTRING::from(download_path.clone()));
                    let _ = args.SetHandled(true);
                    let _ = args.SetCancel(false);
                    log::info!(
                        "WebView2 download routed natively for {} uri={} path={}",
                        download_label,
                        privacy::sanitize_url_for_log(&uri),
                        privacy::sanitize_text_for_log(download_path.as_str())
                    );
                } else {
                    log::warn!(
                        "WebView2 download path resolution failed for {} uri={} (allowing default flow)",
                        download_label,
                        privacy::sanitize_url_for_log(&uri)
                    );
                }
                Ok(())
            }));

            if let Err(err) = core_webview4.add_DownloadStarting(&download_handler, &mut download_token) {
                log::warn!(
                    "Failed to attach WebView2 download handler for {callback_label}: {err}"
                );
            }
        } else {
            log::warn!("WebView2 download API unavailable for {callback_label}");
        }

        let mut process_failed_token = 0i64;
        let process_failed_label = callback_label.clone();
        let process_failed_app = app_handle.clone();
        let process_failed_handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };

            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            let _ = args.ProcessFailedKind(&mut kind);

            let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
            let mut exit_code = 0i32;
            let mut process_description = String::new();
            if let Ok(args2) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                let _ = args2.Reason(&mut reason);
                let _ = args2.ExitCode(&mut exit_code);
                let mut desc_ptr = PWSTR::null();
                let _ = args2.ProcessDescription(&mut desc_ptr);
                process_description = take_pwstr(desc_ptr);
            }

            log::error!(
                "WebView2 process failure in {} kind={} reason={} exit_code={} process={}",
                process_failed_label,
                browser_runtime::webview_process_failed_kind_name(kind),
                browser_runtime::webview_process_failed_reason_name(reason),
                exit_code,
                privacy::sanitize_text_for_log(&process_description)
            );

            if browser_runtime::should_reload_after_process_failure(kind) {
                match record_process_failure_recovery(&process_failed_app) {
                    ProcessFailureRecoveryDecision::Reload { attempt } => {
                        if let Some(main_window) =
                            process_failed_app.get_webview_window(&process_failed_label)
                        {
                            if let Err(err) = main_window.eval("window.location.reload();") {
                                log::error!(
                                    "Failed to request WebView2 reload for {} after process failure attempt={} error={}",
                                    process_failed_label,
                                    attempt,
                                    privacy::sanitize_text_for_log(&err.to_string())
                                );
                            } else {
                                log::warn!(
                                    "Requested WebView2 reload for {} after {} attempt={}",
                                    process_failed_label,
                                    browser_runtime::webview_process_failed_kind_name(kind),
                                    attempt
                                );
                            }
                        }
                    }
                    ProcessFailureRecoveryDecision::Suppress { reason } => {
                        log::error!(
                            "Suppressed WebView2 auto-reload for {} after {} reason={}",
                            process_failed_label,
                            browser_runtime::webview_process_failed_kind_name(kind),
                            reason
                        );
                    }
                }
            }

            Ok(())
        }));

        if let Err(err) = core_webview.add_ProcessFailed(&process_failed_handler, &mut process_failed_token) {
            log::warn!(
                "Failed to attach WebView2 process-failed handler for {callback_label}: {err}"
            );
        }

        if let Ok(environment5) = webview.environment().cast::<ICoreWebView2Environment5>() {
            let mut browser_exit_token = 0i64;
            let browser_exit_label = callback_label.clone();
            let browser_exit_app = app_handle.clone();
            let browser_exit_handler = BrowserProcessExitedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };

                let mut process_id = 0u32;
                let mut exit_kind = COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND::default();
                let _ = args.BrowserProcessId(&mut process_id);
                let _ = args.BrowserProcessExitKind(&mut exit_kind);
                log::warn!(
                    "WebView2 browser process exited for {} pid={} exit_kind={:?}",
                    browser_exit_label,
                    process_id,
                    exit_kind
                );
                if process_id > 0 {
                    clear_main_browser_pid_if_matches(&browser_exit_app, process_id);
                }
                Ok(())
            }));

            if let Err(err) = environment5.add_BrowserProcessExited(&browser_exit_handler, &mut browser_exit_token) {
                log::warn!(
                    "Failed to attach WebView2 browser-exited handler for {callback_label}: {err}"
                );
            }
        }

        let mut browser_pid = 0u32;
        let _ = core_webview.BrowserProcessId(&mut browser_pid);
        if browser_pid > 0 {
            log::info!(
                "WebView2 browser process attached for {} pid={}",
                callback_label,
                browser_pid
            );
            set_main_browser_pid(&app_handle, Some(browser_pid));
            if cfg!(debug_assertions) {
                spawn_webview_memory_telemetry(
                    app_handle.clone(),
                    callback_label.clone(),
                    browser_pid,
                );
            }
        } else {
            log::warn!("WebView2 browser process id unavailable for {callback_label}");
        }
    });

    if let Err(err) = with_webview_result {
        log::warn!("Failed to initialize WebView2 host hooks for {label}: {err}");
    }
}

#[cfg(target_os = "windows")]
fn spawn_webview_memory_telemetry(app: AppHandle, window_label: String, browser_pid: u32) {
    std::thread::spawn(move || {
        use std::time::Duration;

        loop {
            std::thread::sleep(Duration::from_secs(15));
            if app.get_webview_window(&window_label).is_none() {
                break;
            }
            let snapshot = app.state::<PersistedStore>().snapshot();
            if !browser_runtime::runtime_diagnostics_enabled(&snapshot.settings) {
                continue;
            }

            let host_pid = std::process::id();
            let host_ws_kb = browser_runtime::read_process_working_set_kb(host_pid).unwrap_or(0);
            let browser_ws_kb =
                browser_runtime::read_process_working_set_kb(browser_pid).unwrap_or(0);
            log::info!(
                "webview2_telemetry window={} host_pid={} host_ws_kb={} browser_pid={} browser_ws_kb={}",
                window_label,
                host_pid,
                host_ws_kb,
                browser_pid,
                browser_ws_kb
            );
        }
    });
}
