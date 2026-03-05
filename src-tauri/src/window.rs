use crate::{
    csp, desktop, discord, privacy,
    settings::{Settings, TransparencyOption, WindowBounds},
    store::PersistedStore,
    tray, vencord,
};
use tauri::{
    utils::config::BackgroundThrottlingPolicy,
    webview::PageLoadEvent,
    window::{Color, Effect, EffectsBuilder},
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const MIN_WIDTH: u32 = 940;
const MIN_HEIGHT: u32 = 500;

pub fn create_main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let snapshot = app.state::<PersistedStore>().snapshot();
    let control_runtime = should_use_control_runtime();
    let spoof_edge_client_hints = should_spoof_edge_client_hints();
    let install_host_runtime = should_install_host_runtime(control_runtime);
    let install_mod_runtime = should_install_mod_runtime(control_runtime);
    let use_custom_title_bar = should_use_custom_title_bar(&snapshot, control_runtime);
    let mod_runtime_profile = vencord::resolve_mod_runtime_profile();
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
    let user_agent = discord::browser_user_agent(&snapshot.settings);
    let additional_browser_args =
        discord::browser_additional_args(&snapshot.settings, &snapshot.state);
    let bridge_seed = vencord::bridge_seed(
        app,
        &snapshot.paths,
        &snapshot.settings,
        &mod_runtime_profile,
    )
    .map_err(|err| err.to_string())?;
    let vencord_renderer = vencord::renderer_script(app).ok();
    let bootstrap_script = desktop::bootstrap_script(
        &bridge_seed,
        vencord_renderer.as_deref(),
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

    if let Some(user_agent) = user_agent.as_deref() {
        builder = builder.user_agent(user_agent);
    }

    if let Some(additional_browser_args) = additional_browser_args.as_deref() {
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
            if let Err(err) = persist_main_window_state(&tracked_window) {
                log::warn!("Failed to persist main window state: {err}");
            }
        }
        _ => {}
    });
}

fn persist_main_window_state(window: &WebviewWindow) -> Result<(), String> {
    let store = window.state::<PersistedStore>();

    let position = window.outer_position().map_err(|err| err.to_string())?;
    let size = window.inner_size().map_err(|err| err.to_string())?;
    let maximized = window.is_maximized().map_err(|err| err.to_string())?;
    let minimized = window.is_minimized().map_err(|err| err.to_string())?;

    store
        .update_state(|state| {
            state.maximized = Some(maximized);
            state.minimized = Some(minimized);
            state.window_bounds = Some(WindowBounds {
                x: position.x,
                y: position.y,
                width: size.width as i32,
                height: size.height as i32,
            });
        })
        .map(|_| ())
        .map_err(|err| err.to_string())
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

fn should_use_control_runtime() -> bool {
    std::env::args().any(|arg| arg == "--control-runtime")
        || matches!(
            std::env::var("EQUIRUST_CONTROL_RUNTIME").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

fn should_install_host_runtime(control_runtime: bool) -> bool {
    !control_runtime
        && !std::env::args().any(|arg| arg == "--no-host-runtime" || arg == "--no-host-ui")
        && !matches!(
            std::env::var("EQUIRUST_NO_HOST_RUNTIME").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

fn should_install_mod_runtime(control_runtime: bool) -> bool {
    !control_runtime
        && !std::env::args().any(|arg| arg == "--no-mod-runtime" || arg == "--no-vencord")
        && !matches!(
            std::env::var("EQUIRUST_NO_MOD_RUNTIME").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
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

fn should_spoof_edge_client_hints() -> bool {
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
fn install_windows_webview2_host_hooks(window: &WebviewWindow, app: &AppHandle, settings: &Settings) {
    let label = window.label().to_string();
    let app_handle = app.clone();
    let diagnostics_enabled_at_start = runtime_diagnostics_enabled(settings);
    let callback_label = label.clone();

    let with_webview_result = window.with_webview(move |webview| unsafe {
        use std::{
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };
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
                cfg!(debug_assertions) || diagnostics_enabled_at_start,
            );
            if let Ok(settings4) = core_settings.cast::<ICoreWebView2Settings4>() {
                let _ = settings4.SetIsPasswordAutosaveEnabled(false);
                let _ = settings4.SetIsGeneralAutofillEnabled(false);
            }
            log::info!(
                "Applied hardened WebView2 settings for {callback_label} (devtools={})",
                cfg!(debug_assertions) || diagnostics_enabled_at_start
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
            let trusted_origin = is_trusted_discord_origin(&uri);

            let decision = if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA
                || kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
                || kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ
            {
                Some(if trusted_origin {
                    COREWEBVIEW2_PERMISSION_STATE_ALLOW
                } else {
                    COREWEBVIEW2_PERMISSION_STATE_DENY
                })
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

                let kind_name = webview_permission_kind_name(kind);
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
                    let default_text = take_pwstr(default_text_ptr);
                    let _ = args.SetResultText(&HSTRING::from(default_text));
                }
                let _ = args.Accept();
                log::warn!(
                    "Suppressed WebView2 script dialog for {} kind={} (default host UI disabled)",
                    script_dialog_label,
                    webview_script_dialog_kind_name(kind)
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

                if let Some(download_path) = resolve_download_target_path(&download_app, &uri, &suggested) {
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
        let last_reload_at = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(30)));
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
                webview_process_failed_kind_name(kind),
                webview_process_failed_reason_name(reason),
                exit_code,
                privacy::sanitize_text_for_log(&process_description)
            );

            if should_reload_after_process_failure(kind) {
                let mut should_reload = false;
                if let Ok(mut guard) = last_reload_at.lock() {
                    if guard.elapsed() >= Duration::from_secs(4) {
                        *guard = Instant::now();
                        should_reload = true;
                    }
                }

                if should_reload {
                    if let Some(main_window) = process_failed_app.get_webview_window(&process_failed_label) {
                        if let Err(err) = main_window.eval("window.location.reload();") {
                            log::error!(
                                "Failed to request WebView2 reload for {} after process failure: {}",
                                process_failed_label,
                                privacy::sanitize_text_for_log(&err.to_string())
                            );
                        } else {
                            log::warn!(
                                "Requested WebView2 reload for {} after {}",
                                process_failed_label,
                                webview_process_failed_kind_name(kind)
                            );
                        }
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
            if !runtime_diagnostics_enabled(&snapshot.settings) {
                continue;
            }

            let host_pid = std::process::id();
            let host_ws_kb = read_process_working_set_kb(host_pid).unwrap_or(0);
            let browser_ws_kb = read_process_working_set_kb(browser_pid).unwrap_or(0);
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

#[cfg(target_os = "windows")]
fn runtime_diagnostics_enabled(settings: &Settings) -> bool {
    cfg!(debug_assertions) && settings.runtime_diagnostics == Some(true)
}

#[cfg(target_os = "windows")]
fn read_process_working_set_kb(pid: u32) -> Option<u64> {
    use std::mem::size_of;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    let process_handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid).ok()? };
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
fn resolve_download_target_path(app: &AppHandle, uri: &str, suggested_path: &str) -> Option<String> {
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
fn is_trusted_discord_origin(uri: &str) -> bool {
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
fn should_reload_after_process_failure(
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
fn webview_permission_kind_name(
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
fn webview_script_dialog_kind_name(
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
fn webview_process_failed_kind_name(
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
fn webview_process_failed_reason_name(
    reason: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_REASON,
) -> &'static str {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED, COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED,
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
