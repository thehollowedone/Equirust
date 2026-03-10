use crate::{paths::AppPaths, privacy, processes, settings::Settings};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    hash::{Hash, Hasher},
    io,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, Runtime, State as TauriState};
use tungstenite::{connect, error::Error as WsError, stream::MaybeTlsStream, Message, WebSocket};

#[cfg(target_os = "windows")]
use {
    interprocess::os::windows::named_pipe::{
        pipe_mode, DuplexPipeStream, PipeListener, PipeListenerOptions,
    },
    std::path::Path as StdPath,
};

const ACTIVITY_EVENT: &str = "equirust:arrpc-activity";
const STATUS_EVENT: &str = "equirust:arrpc-status";
const STATE_FILE_PREFIX: &str = "arrpc-state-";
const STATE_FILE_STALE_MS: i64 = 60_000;
const INIT_TIMEOUT: Duration = Duration::from_secs(10);
const SUPERVISOR_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SUPERVISOR_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(1200);
const SUPERVISOR_BACKGROUND_POLL_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_RECONNECT_INTERVAL_MS: u64 = 5_000;
const DEFAULT_WS_HOST: &str = "127.0.0.1";
const DEFAULT_BRIDGE_PORT: u16 = 60_000;
const PROCESS_SCAN_INTERVAL: Duration = Duration::from_millis(500);
const PROCESS_CLEAR_GRACE: Duration = Duration::from_secs(2);
const EXTERNAL_ACTIVITY_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const DETECTABLE_APPS_URL: &str = "https://discord.com/api/v9/applications/detectable";

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "windows")]
type RpcPipeListener = PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>;

const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(125);
const FRAME_MAX_LEN: usize = 1024 * 1024;
const OPCODE_HANDSHAKE: u32 = 0;
const OPCODE_FRAME: u32 = 1;
const OPCODE_CLOSE: u32 = 2;
const OPCODE_PING: u32 = 3;
const OPCODE_PONG: u32 = 4;
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArrpcActivitySummary {
    pub socket_id: String,
    pub name: String,
    pub application_id: String,
    pub pid: Option<u32>,
    pub start_time: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArrpcStatus {
    pub enabled: bool,
    pub builtin_enabled: bool,
    pub running: bool,
    pub managed: bool,
    pub ready: bool,
    pub stale: bool,
    pub pid: Option<u32>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub binary_path: Option<String>,
    pub app_version: Option<String>,
    pub activities: Vec<ArrpcActivitySummary>,
    pub last_error: Option<String>,
    pub last_exit_code: Option<i32>,
    pub uptime_ms: Option<i64>,
    pub ready_ms: Option<i64>,
    pub restart_count: u32,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateFileContent {
    app_version: Option<String>,
    timestamp: Option<i64>,
    #[serde(default)]
    servers: StateServers,
    #[serde(default)]
    activities: Vec<StateActivity>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateServers {
    bridge: Option<HostPort>,
    websocket: Option<HostPort>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct HostPort {
    host: String,
    port: u16,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateActivity {
    socket_id: Option<String>,
    name: Option<String>,
    application_id: Option<String>,
    pid: Option<u32>,
    start_time: Option<i64>,
}

struct ManagedChild {
    generation: u64,
    child: Child,
    started_at: Instant,
}

struct InnerState {
    generation: u64,
    child: Option<ManagedChild>,
    status: ArrpcStatus,
    latest_activity: Option<Value>,
    external_rpc_activities: HashMap<String, ExternalRpcActivity>,
}

impl Default for InnerState {
    fn default() -> Self {
        Self {
            generation: 0,
            child: None,
            status: ArrpcStatus::default(),
            latest_activity: None,
            external_rpc_activities: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct ExternalRpcActivity {
    summary: ArrpcActivitySummary,
    payload: Value,
    updated_at: i64,
}

#[derive(Default)]
pub struct RuntimeState {
    inner: Mutex<InnerState>,
}

#[tauri::command]
pub fn get_arrpc_status(runtime: TauriState<'_, RuntimeState>) -> Result<ArrpcStatus, String> {
    Ok(runtime
        .inner
        .lock()
        .map_err(|_| "arRPC runtime state is poisoned".to_owned())?
        .status
        .clone())
}

#[tauri::command]
pub fn get_arrpc_current_activity(runtime: TauriState<'_, RuntimeState>) -> Result<Value, String> {
    Ok(runtime
        .inner
        .lock()
        .map_err(|_| "arRPC runtime state is poisoned".to_owned())?
        .latest_activity
        .clone()
        .unwrap_or_else(|| json!({ "activity": null })))
}

#[tauri::command]
pub fn restart_arrpc(
    app: AppHandle,
    runtime: TauriState<'_, RuntimeState>,
    store: TauriState<'_, crate::store::PersistedStore>,
) -> Result<ArrpcStatus, String> {
    let settings = store.snapshot().settings;
    sync(&app, &settings)?;
    Ok(runtime
        .inner
        .lock()
        .map_err(|_| "arRPC runtime state is poisoned".to_owned())?
        .status
        .clone())
}

pub fn sync<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    let should_enable = arrpc_enabled(settings);
    let builtin_enabled = builtin_enabled(settings);
    let runtime = app.state::<RuntimeState>();
    let generation = {
        let mut inner = runtime
            .inner
            .lock()
            .map_err(|_| "arRPC runtime state is poisoned".to_owned())?;
        inner.generation = inner.generation.saturating_add(1);
        inner.status.enabled = should_enable;
        inner.status.builtin_enabled = builtin_enabled;
        inner.status.running = false;
        inner.status.managed = false;
        inner.status.ready = false;
        inner.status.stale = false;
        inner.status.pid = None;
        inner.status.host = None;
        inner.status.port = None;
        inner.status.binary_path = None;
        inner.status.app_version = None;
        inner.status.activities.clear();
        inner.status.last_error = None;
        inner.status.last_exit_code = None;
        inner.status.uptime_ms = None;
        inner.status.ready_ms = None;
        inner.latest_activity = None;
        inner.external_rpc_activities.clear();
        if inner.child.is_some() {
            inner.status.restart_count = inner.status.restart_count.saturating_add(1);
        }
        inner.status.checked_at = now_millis();
        stop_child(&mut inner.child);
        inner.generation
    };
    publish_status(app);
    clear_activity(app);

    if !should_enable {
        return Ok(());
    }

    let app_handle = app.clone();
    let settings = settings.clone();
    thread::spawn(move || supervisor_loop(app_handle, generation, settings));
    Ok(())
}

fn supervisor_loop<R: Runtime>(app: AppHandle<R>, generation: u64, settings: Settings) {
    let builtin_enabled = builtin_enabled(&settings);
    let native_builtin_backend = native_builtin_backend(&settings);
    let native_process_scanning =
        builtin_enabled && settings.ar_rpc_process_scanning != Some(false);
    let custom_ws = custom_websocket_endpoint(&settings);
    let auto_reconnect = settings.ar_rpc_web_socket_auto_reconnect.unwrap_or(true);
    let reconnect_interval = Duration::from_millis(
        settings
            .ar_rpc_web_socket_reconnect_interval
            .unwrap_or(DEFAULT_RECONNECT_INTERVAL_MS),
    );
    let mut launched_child = false;
    let mut websocket: Option<WebSocket<MaybeTlsStream<TcpStream>>> = None;
    let mut last_ws_target: Option<String> = None;
    let mut next_ws_retry = Instant::now();
    let mut websocket_attempted = false;
    let mut ready_at: Option<Instant> = None;
    let init_started_at = Instant::now();
    let mut native_detector = native_process_scanning.then(NativeProcessDetector::new);
    let mut active_pids = HashSet::new();
    let mut next_external_activity_refresh = Instant::now();

    start_native_rpc_bridge(&app, generation);

    if native_builtin_backend {
        ready_at = Some(Instant::now());
        update_status(&app, |status| {
            status.running = true;
            status.managed = true;
            status.ready = true;
            status.stale = false;
            status.app_version = Some("native-rust".into());
            status.last_error = None;
            status.ready_ms = Some(0);
        });
    } else if builtin_enabled {
        match start_builtin_process(&app, &settings, generation, native_process_scanning) {
            Ok(binary_path) => {
                launched_child = true;
                update_status(&app, |status| {
                    status.managed = true;
                    status.binary_path = Some(binary_path.display().to_string());
                    status.last_error = None;
                });
            }
            Err(err) => {
                update_status(&app, |status| {
                    status.last_error = Some(err.clone());
                });
                return;
            }
        }
    }

    loop {
        if !generation_is_current(&app, generation) {
            break;
        }

        let child_snapshot = inspect_child(&app, generation);
        if let Some(exit_code) = child_snapshot.exit_code {
            update_status(&app, |status| {
                status.running = false;
                status.ready = false;
                status.last_exit_code = Some(exit_code);
                if status.last_error.is_none() {
                    status.last_error = Some(format!("arRPC exited with code {exit_code}"));
                }
            });
            clear_activity(&app);
            if builtin_enabled {
                break;
            }
        } else if let Some((pid, uptime_ms)) = child_snapshot.running {
            update_status(&app, |status| {
                status.running = true;
                status.pid = Some(pid);
                status.uptime_ms = Some(uptime_ms);
            });
        } else if !builtin_enabled && custom_ws.is_some() {
            update_status(&app, |status| {
                status.running = true;
                status.managed = false;
            });
        }

        let state_file = if native_builtin_backend {
            None
        } else {
            find_latest_state_file(child_snapshot.pid)
        };
        if let Some(found) = state_file.as_ref() {
            let ready_now = !found.stale
                && found
                    .content
                    .servers
                    .bridge
                    .as_ref()
                    .map(|server| !server.host.trim().is_empty())
                    .unwrap_or(false);
            if ready_now && ready_at.is_none() {
                ready_at = Some(Instant::now());
            }
            update_status(&app, |status| {
                status.host = found
                    .content
                    .servers
                    .bridge
                    .as_ref()
                    .map(|server| server.host.clone());
                status.port = found
                    .content
                    .servers
                    .bridge
                    .as_ref()
                    .map(|server| server.port);
                status.app_version = found.content.app_version.clone();
                status.activities = found
                    .content
                    .activities
                    .iter()
                    .map(|activity| ArrpcActivitySummary {
                        socket_id: activity.socket_id.clone().unwrap_or_default(),
                        name: activity.name.clone().unwrap_or_default(),
                        application_id: activity.application_id.clone().unwrap_or_default(),
                        pid: activity.pid,
                        start_time: activity.start_time,
                    })
                    .collect();
                status.ready = ready_now || custom_ws.is_some();
                status.stale = found.stale;
                if let Some(ready_at) = ready_at {
                    status.ready_ms = Some(ready_at.elapsed().as_millis() as i64);
                } else if ready_now {
                    status.ready_ms = Some(init_started_at.elapsed().as_millis() as i64);
                }
            });
            if websocket.is_none() {
                if let Some(payload) = payload_from_state_activities(&found.content.activities) {
                    emit_activity(&app, payload);
                } else {
                    clear_activity(&app);
                }
            }
        } else if !launched_child && custom_ws.is_none() && !native_builtin_backend {
            update_status(&app, |status| {
                status.running = false;
                status.ready = false;
                status.stale = false;
            });
            if websocket.is_none() {
                clear_activity(&app);
            }
        }

        let external_has_activity = state_file
            .as_ref()
            .map(|found| !found.stale && !found.content.activities.is_empty())
            .unwrap_or(false);
        let has_external_rpc_entries = app
            .state::<RuntimeState>()
            .inner
            .lock()
            .ok()
            .map(|inner| !inner.external_rpc_activities.is_empty())
            .unwrap_or(false);
        let native_override_active = if has_external_rpc_entries {
            if next_external_activity_refresh <= Instant::now() {
                active_pids = enumerate_processes()
                    .into_iter()
                    .map(|process| process.pid)
                    .collect();
                next_external_activity_refresh =
                    Instant::now() + EXTERNAL_ACTIVITY_REFRESH_INTERVAL;
            }
            refresh_external_rpc_activity_state(&app, &active_pids)
        } else {
            false
        };
        if let Some(detector) = native_detector.as_mut() {
            let detection = detector.poll();
            if !external_has_activity && !native_override_active {
                if let Some(activity) = detection.current {
                    if detection.changed {
                        log::info!(
                            "Native arRPC detected activity name={} application_id={} pid={}",
                            activity.name,
                            activity.application_id,
                            activity.pid
                        );
                    }
                    emit_activity(&app, activity.payload);
                    update_status(&app, |status| {
                        status.activities = vec![ArrpcActivitySummary {
                            socket_id: activity.socket_id,
                            name: activity.name,
                            application_id: activity.application_id,
                            pid: Some(activity.pid),
                            start_time: Some(activity.start_time),
                        }];
                    });
                } else {
                    if detection.changed {
                        log::info!("Native arRPC cleared activity");
                    }
                    clear_activity(&app);
                    update_status(&app, |status| {
                        status.activities.clear();
                    });
                }
            }
        }

        let ws_target = resolve_websocket_target(&settings, state_file.as_ref());
        let should_try_ws = ws_target.is_some()
            && !settings.ar_rpc_disabled.unwrap_or(false)
            && (launched_child || custom_ws.is_some() || state_file.is_some())
            && (!websocket_attempted || auto_reconnect);

        if should_try_ws && next_ws_retry <= Instant::now() {
            if let Some(target) = ws_target.clone() {
                let target_changed = last_ws_target.as_deref() != Some(target.as_str());
                if target_changed {
                    if websocket.is_some() {
                        clear_activity(&app);
                    }
                    websocket = None;
                    last_ws_target = Some(target.clone());
                    websocket_attempted = false;
                }

                if websocket.is_none() {
                    websocket_attempted = true;
                    match connect_websocket(&target) {
                        Ok(socket) => {
                            websocket = Some(socket);
                            next_ws_retry = Instant::now() + reconnect_interval;
                            update_status(&app, |status| {
                                status.running = true;
                                status.ready = true;
                                if status.host.is_none() || status.port.is_none() {
                                    if let Some((host, port)) = split_host_port(&target) {
                                        status.host = Some(host);
                                        status.port = Some(port);
                                    }
                                }
                                if status.ready_ms.is_none() {
                                    status.ready_ms =
                                        Some(init_started_at.elapsed().as_millis() as i64);
                                }
                            });
                        }
                        Err(err) => {
                            update_status(&app, |status| {
                                status.last_error = Some(err.clone());
                            });
                            if auto_reconnect {
                                next_ws_retry = Instant::now() + reconnect_interval;
                            }
                        }
                    }
                }
            }
        }

        if let Some(socket) = websocket.as_mut() {
            match socket.read() {
                Ok(message) => {
                    if let Some(payload) = parse_websocket_message(message) {
                        emit_activity(&app, payload);
                    }
                }
                Err(WsError::Io(err))
                    if err.kind() == io::ErrorKind::WouldBlock
                        || err.kind() == io::ErrorKind::TimedOut => {}
                Err(err) => {
                    websocket = None;
                    clear_activity(&app);
                    update_status(&app, |status| {
                        status.last_error = Some(err.to_string());
                    });
                    if auto_reconnect {
                        next_ws_retry = Instant::now() + reconnect_interval;
                    }
                }
            }
        }

        if builtin_enabled
            && launched_child
            && !native_builtin_backend
            && init_started_at.elapsed() > INIT_TIMEOUT
        {
            let ready = app
                .state::<RuntimeState>()
                .inner
                .lock()
                .ok()
                .map(|inner| inner.status.ready)
                .unwrap_or(false);
            if !ready {
                update_status(&app, |status| {
                    status.last_error = Some("arRPC failed to become ready within timeout".into());
                });
                stop_child_for_generation(&app, generation);
                break;
            }
        }

        let sleep_interval = if websocket.is_some()
            || child_snapshot.running.is_some()
            || external_has_activity
            || native_override_active
            || native_detector
                .as_ref()
                .and_then(|detector| detector.current.as_ref())
                .is_some()
        {
            SUPERVISOR_POLL_INTERVAL
        } else if launched_child || builtin_enabled || custom_ws.is_some() {
            SUPERVISOR_BACKGROUND_POLL_INTERVAL
        } else {
            SUPERVISOR_IDLE_POLL_INTERVAL
        };
        thread::sleep(sleep_interval);
    }

    if websocket.is_some() {
        clear_activity(&app);
    }
    stop_child_for_generation(&app, generation);
}

fn arrpc_enabled(settings: &Settings) -> bool {
    !settings.ar_rpc_disabled.unwrap_or(false) && builtin_enabled(settings)
}

fn builtin_enabled(settings: &Settings) -> bool {
    settings.ar_rpc.unwrap_or(false)
}

fn native_builtin_backend(settings: &Settings) -> bool {
    builtin_enabled(settings)
}

fn arrpc_debug_logging_enabled(settings: &Settings) -> bool {
    settings.debug_standard_diagnostics_enabled()
}

fn custom_websocket_endpoint(settings: &Settings) -> Option<(String, u16)> {
    let _ = settings;
    None
}

fn resolve_websocket_target(
    settings: &Settings,
    state_file: Option<&DiscoveredStateFile>,
) -> Option<String> {
    if let Some((host, port)) = custom_websocket_endpoint(settings) {
        return Some(format!("ws://{host}:{port}"));
    }

    if native_builtin_backend(settings) {
        return None;
    }

    if let Some(bridge) = state_file
        .and_then(|found| found.content.servers.bridge.clone())
        .or_else(|| state_file.and_then(|found| found.content.servers.websocket.clone()))
    {
        return Some(format!("ws://{}:{}", bridge.host, bridge.port));
    }

    if builtin_enabled(settings) {
        let host = settings
            .ar_rpc_bridge_host
            .clone()
            .unwrap_or_else(|| DEFAULT_WS_HOST.into());
        let port = settings.ar_rpc_bridge_port.unwrap_or(DEFAULT_BRIDGE_PORT);
        return Some(format!("ws://{host}:{port}"));
    }

    None
}

fn start_builtin_process<R: Runtime>(
    app: &AppHandle<R>,
    settings: &Settings,
    generation: u64,
    native_process_scanning: bool,
) -> Result<PathBuf, String> {
    let binary_path = resolve_binary_path(app)?;
    let paths = AppPaths::resolve(app).map_err(|err| err.to_string())?;
    let data_dir = paths.app_cache_dir.join("arrpc");
    fs::create_dir_all(&data_dir).map_err(|err| err.to_string())?;

    let mut command = Command::new(&binary_path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env("ARRPC_STATE_FILE", "1");
    command.env("ARRPC_PARENT_MONITOR", "1");
    command.env("ARRPC_DATA_DIR", &data_dir);

    if arrpc_debug_logging_enabled(settings) {
        command.env("ARRPC_DEBUG", "1");
    }
    if settings.ar_rpc_process_scanning == Some(false) || native_process_scanning {
        command.env("ARRPC_NO_PROCESS_SCANNING", "1");
    }
    if settings.ar_rpc_bridge == Some(false) {
        command.env("ARRPC_NO_BRIDGE", "1");
    }
    if native_process_scanning {
        let bridge_host = settings
            .ar_rpc_bridge_host
            .clone()
            .unwrap_or_else(|| DEFAULT_WS_HOST.into());
        let bridge_port = settings.ar_rpc_bridge_port.unwrap_or(DEFAULT_BRIDGE_PORT);
        command.env("ARRPC_BRIDGE_HOST", &bridge_host);
        command.env("ARRPC_BRIDGE_PORT", bridge_port.to_string());
    } else {
        if let Some(host) = settings.ar_rpc_bridge_host.as_deref() {
            command.env("ARRPC_BRIDGE_HOST", host);
        }
        if let Some(port) = settings.ar_rpc_bridge_port {
            command.env("ARRPC_BRIDGE_PORT", port.to_string());
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child =
        processes::spawn_managed_child(app, &mut command, "arrpc-backend").map_err(|err| {
            format!(
                "Failed to start arRPC backend at {}: {}",
                binary_path.display(),
                err
            )
        })?;
    spawn_child_output_drainers(app, generation, &mut child);

    {
        let runtime = app.state::<RuntimeState>();
        let mut inner = runtime
            .inner
            .lock()
            .map_err(|_| "arRPC runtime state is poisoned".to_owned())?;
        if inner.generation != generation {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            return Err("arRPC generation changed before process startup completed".into());
        }
        inner.status.pid = Some(child.id());
        inner.status.running = true;
        inner.status.managed = true;
        inner.status.binary_path = Some(binary_path.display().to_string());
        inner.child = Some(ManagedChild {
            generation,
            child,
            started_at: Instant::now(),
        });
    }

    publish_status(app);
    Ok(binary_path)
}

fn spawn_child_output_drainers<R: Runtime>(app: &AppHandle<R>, generation: u64, child: &mut Child) {
    if let Some(stdout) = child.stdout.take() {
        let app_handle = app.clone();
        thread::spawn(move || {
            let reader = io::BufReader::new(stdout);
            for line in io::BufRead::lines(reader).map_while(Result::ok) {
                if !generation_is_current(&app_handle, generation) {
                    break;
                }
                handle_child_output_line(&app_handle, &line);
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let app_handle = app.clone();
        thread::spawn(move || {
            let reader = io::BufReader::new(stderr);
            for line in io::BufRead::lines(reader).map_while(Result::ok) {
                if !generation_is_current(&app_handle, generation) {
                    break;
                }
                handle_child_output_line(&app_handle, &line);
            }
        });
    }
}

fn handle_child_output_line<R: Runtime>(app: &AppHandle<R>, line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    if let Ok(payload) = serde_json::from_str::<Value>(trimmed) {
        if payload.get("type").and_then(Value::as_str) == Some("STREAMERMODE") {
            if let Some(data) = payload.get("data") {
                match data {
                    Value::String(text) => {
                        if let Ok(streamer_payload) = serde_json::from_str::<Value>(text) {
                            emit_activity(app, streamer_payload);
                            return;
                        }
                    }
                    Value::Object(_) => {
                        emit_activity(app, data.clone());
                        return;
                    }
                    _ => {}
                }
            }
        }
    }

    update_status(app, |status| {
        status.last_error = Some(privacy::sanitize_text_for_log(trimmed));
    });
}

fn resolve_binary_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("EQUIRUST_ARRPC_BINARY").map(PathBuf::from) {
        if is_executable_file(&path) {
            return Ok(path);
        }
    }

    let asset_name = platform_asset_name()?;
    let paths = AppPaths::resolve(app).map_err(|err| err.to_string())?;
    let cached_dir = paths.app_cache_dir.join("arrpc").join("bin");
    let cached_path = cached_dir.join(asset_name);
    if is_executable_file(&cached_path) {
        return Ok(cached_path);
    }

    for candidate in system_binary_candidates(asset_name) {
        if is_executable_file(&candidate) {
            return Ok(candidate);
        }
    }

    fs::create_dir_all(&cached_dir).map_err(|err| err.to_string())?;
    download_release_binary(asset_name, &cached_path)?;
    Ok(cached_path)
}

fn system_binary_candidates(asset_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            candidates.push(local_app_data.join("arrpc-bun").join("arrpc-bun.exe"));
        }
        if let Some(program_files) = env::var_os("PROGRAMFILES").map(PathBuf::from) {
            candidates.push(program_files.join("arrpc-bun").join("arrpc-bun.exe"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/arrpc-bun"));
        candidates.push(PathBuf::from("/usr/local/bin/arrpc-bun"));
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from("/usr/local/bin/arrpc-bun"));
        candidates.push(PathBuf::from("/opt/homebrew/bin/arrpc-bun"));
    }
    candidates.push(PathBuf::from(asset_name));
    candidates
}

fn download_release_binary(asset_name: &str, destination: &Path) -> Result<(), String> {
    let _ = (asset_name, destination);
    Err(
        "External arRPC backend downloads are disabled. Equirust now uses native Rust arRPC only."
            .to_owned(),
    )
}

fn platform_asset_name() -> Result<&'static str, String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("windows", "x86_64") => Ok("arrpc-bun-windows-x64.exe"),
        ("linux", "x86_64") => Ok("arrpc-bun-linux-x64"),
        ("linux", "aarch64") => Ok("arrpc-bun-linux-arm64"),
        ("macos", "x86_64") => Ok("arrpc-bun-darwin-x64"),
        ("macos", "aarch64") => Ok("arrpc-bun-darwin-arm64"),
        (os, arch) => Err(format!("arRPC backend does not support {os}/{arch}")),
    }
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn connect_websocket(target: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let (mut socket, _) =
        connect(target).map_err(|err| format!("Failed to connect to {target}: {err}"))?;
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    }
    Ok(socket)
}

fn parse_websocket_message(message: Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str(&text).ok(),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok()),
        Message::Close(_) => Some(json!({ "activity": null })),
        _ => None,
    }
}

fn emit_activity<R: Runtime>(app: &AppHandle<R>, payload: Value) {
    let should_emit = {
        let runtime = app.state::<RuntimeState>();
        let result = match runtime.inner.lock() {
            Ok(mut inner) => {
                if inner.latest_activity.as_ref() == Some(&payload) {
                    false
                } else {
                    inner.latest_activity = Some(payload.clone());
                    true
                }
            }
            Err(_) => true,
        };
        result
    };

    if should_emit {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.emit(ACTIVITY_EVENT, payload);
        }
    }
}

fn clear_activity<R: Runtime>(app: &AppHandle<R>) {
    let payload = app
        .state::<RuntimeState>()
        .inner
        .lock()
        .ok()
        .and_then(|inner| inner.latest_activity.clone())
        .and_then(|latest| {
            latest
                .get("socketId")
                .and_then(Value::as_str)
                .map(|socket_id| json!({ "socketId": socket_id, "activity": null }))
        })
        .unwrap_or_else(|| json!({ "activity": null }));

    emit_activity(app, payload);
}

fn refresh_external_rpc_activity_state<R: Runtime>(
    app: &AppHandle<R>,
    active_pids: &HashSet<u32>,
) -> bool {
    let (changed, most_recent_payload, has_active_pid_entry) = {
        let runtime = app.state::<RuntimeState>();
        let Ok(mut inner) = runtime.inner.lock() else {
            return false;
        };

        let before_len = inner.external_rpc_activities.len();
        inner.external_rpc_activities.retain(|socket_id, entry| {
            let Some(pid) = entry.summary.pid else {
                return true;
            };
            if active_pids.contains(&pid) {
                return true;
            }
            log::info!(
                "Native Discord IPC stale activity removed socket_id={} pid={}",
                socket_id,
                pid
            );
            false
        });

        let changed = inner.external_rpc_activities.len() != before_len;

        let mut activities: Vec<_> = inner.external_rpc_activities.values().cloned().collect();
        activities.sort_by_key(|entry| entry.updated_at);
        let has_active_pid_entry = activities.iter().any(|entry| {
            entry
                .summary
                .pid
                .map(|pid| active_pids.contains(&pid))
                .unwrap_or(false)
        });

        let mut most_recent_payload = None;
        if changed {
            inner.status.activities = activities
                .iter()
                .rev()
                .map(|entry| entry.summary.clone())
                .collect();
            most_recent_payload = activities.last().map(|entry| entry.payload.clone());
        }
        (changed, most_recent_payload, has_active_pid_entry)
    };

    if changed {
        if let Some(payload) = most_recent_payload {
            emit_activity(app, payload);
        }
        publish_status(app);
    }

    has_active_pid_entry
}

pub(crate) fn set_external_rpc_activity<R: Runtime>(
    app: &AppHandle<R>,
    socket_id: String,
    pid: Option<u32>,
    name: Option<String>,
    activity: Value,
) {
    let mut payload = serde_json::Map::new();
    payload.insert("socketId".into(), Value::String(socket_id.clone()));
    if let Some(pid) = pid {
        payload.insert("pid".into(), Value::Number(pid.into()));
    }
    if let Some(name) = name.clone().filter(|value| !value.trim().is_empty()) {
        payload.insert("name".into(), Value::String(name));
    }
    payload.insert("activity".into(), activity.clone());
    let payload = Value::Object(payload);

    let summary = ArrpcActivitySummary {
        socket_id: socket_id.clone(),
        name: name
            .or_else(|| {
                activity
                    .get("name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| socket_id.clone()),
        application_id: activity
            .get("application_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| socket_id.clone()),
        pid,
        start_time: activity
            .get("timestamps")
            .and_then(|timestamps| timestamps.get("start"))
            .and_then(Value::as_i64),
    };

    let latest_payload = {
        let runtime = app.state::<RuntimeState>();
        let result = match runtime.inner.lock() {
            Ok(mut inner) => {
                inner.external_rpc_activities.insert(
                    socket_id,
                    ExternalRpcActivity {
                        summary,
                        payload: payload.clone(),
                        updated_at: now_millis(),
                    },
                );
                let mut activities: Vec<_> =
                    inner.external_rpc_activities.values().cloned().collect();
                activities.sort_by_key(|entry| entry.updated_at);
                inner.status.activities = activities
                    .iter()
                    .rev()
                    .map(|entry| entry.summary.clone())
                    .collect();
                activities.last().map(|entry| entry.payload.clone())
            }
            Err(_) => None,
        };
        result
    };

    if let Some(payload) = latest_payload {
        emit_activity(app, payload);
        publish_status(app);
    }
}

pub(crate) fn clear_external_rpc_activity<R: Runtime>(app: &AppHandle<R>, socket_id: &str) {
    let next_payload = {
        let runtime = app.state::<RuntimeState>();
        let result = match runtime.inner.lock() {
            Ok(mut inner) => {
                inner.external_rpc_activities.remove(socket_id);
                let mut activities: Vec<_> =
                    inner.external_rpc_activities.values().cloned().collect();
                activities.sort_by_key(|entry| entry.updated_at);
                inner.status.activities = activities
                    .iter()
                    .rev()
                    .map(|entry| entry.summary.clone())
                    .collect();
                activities.last().map(|entry| entry.payload.clone())
            }
            Err(_) => None,
        };
        result
    };

    if let Some(payload) = next_payload {
        emit_activity(app, payload);
    }
    publish_status(app);
}

fn publish_status<R: Runtime>(app: &AppHandle<R>) {
    let status = app
        .state::<RuntimeState>()
        .inner
        .lock()
        .ok()
        .map(|inner| inner.status.clone());
    if let (Some(window), Some(status)) = (app.get_webview_window("main"), status) {
        let _ = window.emit(STATUS_EVENT, status);
    }
}

fn update_status<R: Runtime>(app: &AppHandle<R>, update: impl FnOnce(&mut ArrpcStatus)) {
    let runtime = app.state::<RuntimeState>();
    let changed = {
        let mut inner = match runtime.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        let before = serde_json::to_string(&inner.status).ok();
        update(&mut inner.status);
        inner.status.checked_at = now_millis();
        let after = serde_json::to_string(&inner.status).ok();
        before != after
    };

    if changed {
        publish_status(app);
    }
}

pub(crate) fn generation_is_current<R: Runtime>(app: &AppHandle<R>, generation: u64) -> bool {
    app.state::<RuntimeState>()
        .inner
        .lock()
        .map(|inner| inner.generation == generation)
        .unwrap_or(false)
}

fn inspect_child<R: Runtime>(app: &AppHandle<R>, generation: u64) -> ChildSnapshot {
    let runtime = app.state::<RuntimeState>();
    let mut inner = match runtime.inner.lock() {
        Ok(inner) => inner,
        Err(_) => return ChildSnapshot::default(),
    };
    let Some(mut managed) = inner.child.take() else {
        return ChildSnapshot::default();
    };
    if managed.generation != generation {
        inner.child = Some(managed);
        return ChildSnapshot::default();
    }

    match managed.child.try_wait() {
        Ok(Some(status)) => {
            let code = status.code().unwrap_or_default();
            let pid = managed.child.id();
            ChildSnapshot {
                pid: Some(pid),
                exit_code: Some(code),
                ..Default::default()
            }
        }
        Ok(None) => {
            let pid = managed.child.id();
            let uptime = managed.started_at.elapsed().as_millis() as i64;
            inner.child = Some(managed);
            ChildSnapshot {
                pid: Some(pid),
                running: Some((pid, uptime)),
                ..Default::default()
            }
        }
        Err(err) => {
            inner.status.last_error = Some(err.to_string());
            inner.child = Some(managed);
            ChildSnapshot::default()
        }
    }
}

fn stop_child(child: &mut Option<ManagedChild>) {
    if let Some(mut managed) = child.take() {
        let _ = managed.child.kill();
        let _ = managed.child.wait();
    }
}

fn stop_child_for_generation<R: Runtime>(app: &AppHandle<R>, generation: u64) {
    let runtime = app.state::<RuntimeState>();
    let mut inner = match runtime.inner.lock() {
        Ok(inner) => inner,
        Err(_) => return,
    };
    if inner
        .child
        .as_ref()
        .map(|managed| managed.generation == generation)
        .unwrap_or(false)
    {
        stop_child(&mut inner.child);
    }
}

#[derive(Default)]
struct ChildSnapshot {
    pid: Option<u32>,
    running: Option<(u32, i64)>,
    exit_code: Option<i32>,
}

struct DiscoveredStateFile {
    content: StateFileContent,
    stale: bool,
}

fn payload_from_state_activities(activities: &[StateActivity]) -> Option<Value> {
    let activity = activities.first()?;
    let application_id = activity
        .application_id
        .clone()
        .or_else(|| activity.socket_id.clone())?;

    let mut payload = serde_json::Map::new();
    if let Some(socket_id) = activity.socket_id.clone().filter(|value| !value.is_empty()) {
        payload.insert("socketId".into(), Value::String(socket_id));
    }
    if let Some(pid) = activity.pid {
        payload.insert("pid".into(), Value::Number(pid.into()));
    }
    if let Some(name) = activity.name.clone().filter(|value| !value.is_empty()) {
        payload.insert("name".into(), Value::String(name));
    }

    let mut activity_payload = serde_json::Map::new();
    activity_payload.insert("application_id".into(), Value::String(application_id));
    activity_payload.insert("type".into(), Value::Number(0.into()));
    if let Some(name) = activity.name.clone().filter(|value| !value.is_empty()) {
        activity_payload.insert("name".into(), Value::String(name));
    }
    if let Some(start_time) = activity.start_time {
        let mut timestamps = serde_json::Map::new();
        timestamps.insert("start".into(), Value::Number(start_time.into()));
        activity_payload.insert("timestamps".into(), Value::Object(timestamps));
    }

    payload.insert("activity".into(), Value::Object(activity_payload));
    Some(Value::Object(payload))
}

fn find_latest_state_file(pid: Option<u32>) -> Option<DiscoveredStateFile> {
    let entries = fs::read_dir(env::temp_dir()).ok()?;
    let pid_name = pid.map(|value| format!("{STATE_FILE_PREFIX}{value}"));
    let mut chosen: Option<(StateFileContent, bool)> = None;
    let mut chosen_timestamp = i64::MIN;
    let mut exact_pid_match: Option<(StateFileContent, bool)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.starts_with(STATE_FILE_PREFIX) {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(content) = serde_json::from_str::<StateFileContent>(&contents) else {
            continue;
        };
        let timestamp = content.timestamp.unwrap_or_default();
        let stale = now_millis().saturating_sub(timestamp) >= STATE_FILE_STALE_MS;
        if pid_name.as_deref() == Some(file_name) {
            exact_pid_match = Some((content, stale));
            continue;
        }
        if timestamp > chosen_timestamp {
            chosen_timestamp = timestamp;
            chosen = Some((content, stale));
        }
    }

    exact_pid_match
        .or(chosen)
        .map(|(content, stale)| DiscoveredStateFile { content, stale })
}

fn split_host_port(target: &str) -> Option<(String, u16)> {
    let url = url::Url::parse(target).ok()?;
    Some((url.host_str()?.to_owned(), url.port_or_known_default()?))
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Clone)]
struct NativeDetectedActivity {
    socket_id: String,
    name: String,
    application_id: String,
    pid: u32,
    start_time: i64,
    payload: Value,
}

#[derive(Debug, Clone)]
struct NativeDetectionResult {
    current: Option<NativeDetectedActivity>,
    changed: bool,
}

struct NativeProcessDetector {
    current: Option<NativeDetectedActivity>,
    next_scan_at_ms: i64,
    clear_grace_started_at_ms: Option<i64>,
    observed_starts: HashMap<u32, i64>,
    steam_apps: Vec<SteamAppEntry>,
    detectable_index: DetectableIndex,
}

impl NativeProcessDetector {
    fn new() -> Self {
        Self {
            current: None,
            next_scan_at_ms: 0,
            clear_grace_started_at_ms: None,
            observed_starts: HashMap::new(),
            steam_apps: steam_apps_index(),
            detectable_index: DetectableIndex::load(),
        }
    }

    fn poll(&mut self) -> NativeDetectionResult {
        let now = now_millis();
        if now < self.next_scan_at_ms {
            return NativeDetectionResult {
                current: self.current.clone(),
                changed: false,
            };
        }

        self.next_scan_at_ms = now.saturating_add(PROCESS_SCAN_INTERVAL.as_millis() as i64);
        let processes = enumerate_processes();
        let visible_windows = visible_windows_by_pid(&processes);
        let detected = processes
            .iter()
            .find_map(|process| self.match_process(process, now, &visible_windows));
        let next = if let Some(activity) = detected {
            self.clear_grace_started_at_ms = None;
            Some(activity)
        } else if let Some(current) = self.current.clone() {
            let grace_started = self.clear_grace_started_at_ms.get_or_insert(now);
            if now.saturating_sub(*grace_started) < PROCESS_CLEAR_GRACE.as_millis() as i64 {
                Some(current)
            } else {
                self.clear_grace_started_at_ms = None;
                None
            }
        } else {
            self.clear_grace_started_at_ms = None;
            None
        };

        let changed = !same_native_activity(self.current.as_ref(), next.as_ref());
        self.current = next;
        let active_pid = self.current.as_ref().map(|activity| activity.pid);
        self.observed_starts.retain(|pid, _| {
            Some(*pid) == active_pid || processes.iter().any(|process| process.pid == *pid)
        });

        NativeDetectionResult {
            current: self.current.clone(),
            changed,
        }
    }

    fn match_process(
        &mut self,
        process: &RunningProcess,
        now: i64,
        visible_windows: &HashMap<u32, Vec<VisibleWindowInfo>>,
    ) -> Option<NativeDetectedActivity> {
        if is_ignored_process(&process.exe_name) {
            return None;
        }

        let steam_match = process
            .path
            .as_ref()
            .and_then(|path| self.match_steam_process(path));
        let executable_name = process.exe_name.to_ascii_lowercase();
        let detectable_match = self
            .detectable_index
            .resolve_process(executable_name.as_str(), process.path.as_deref());
        let windows = visible_windows.get(&process.pid);

        let (name, application_id) = if let Some(entry) = detectable_match {
            let has_eligible_window = windows
                .map(|windows| has_eligible_game_window(windows, entry.entry.display_name.as_str()))
                .unwrap_or(false);
            if !has_eligible_window && !entry.strict_executable_match {
                return None;
            }
            (
                entry.entry.display_name.clone(),
                entry.entry.application_id.clone(),
            )
        } else if let Some(entry) = steam_match {
            let windows = windows?;
            if !has_eligible_game_window(windows, entry.display_name.as_str()) {
                return None;
            }
            if self
                .detectable_index
                .has_known_mapping(entry.steam_id.as_deref(), Some(&entry.display_name))
            {
                return None;
            }
            (
                entry.display_name.clone(),
                synthetic_application_id("steam", &entry.install_dir),
            )
        } else {
            return None;
        };

        let start_time = *self.observed_starts.entry(process.pid).or_insert(now);
        let socket_id = application_id.clone();
        let payload = json!({
            "socketId": socket_id,
            "pid": process.pid,
            "name": name,
            "activity": {
                "application_id": application_id,
                "name": name,
                "type": 0,
                "timestamps": {
                    "start": start_time,
                }
            }
        });

        Some(NativeDetectedActivity {
            socket_id,
            name,
            application_id,
            pid: process.pid,
            start_time,
            payload,
        })
    }

    fn match_steam_process<'a>(&'a self, path: &Path) -> Option<&'a SteamAppEntry> {
        self.steam_apps
            .iter()
            .find(|entry| windows_path_starts_with(path, &entry.install_dir))
    }
}

#[derive(Debug, Clone)]
struct SteamAppEntry {
    display_name: String,
    install_dir: PathBuf,
    steam_id: Option<String>,
}

#[derive(Debug, Clone)]
struct RunningProcess {
    pid: u32,
    exe_name: String,
    path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct VisibleWindowInfo {
    title: String,
    app_name: String,
}

#[derive(Debug, Clone, Default)]
struct DetectableIndex {
    by_steam_id: HashMap<String, DetectableMatch>,
    by_name: HashMap<String, DetectableMatch>,
    by_executable: HashMap<String, DetectableMatch>,
    executable_suffixes: Vec<(String, DetectableMatch)>,
}

#[derive(Debug, Clone)]
struct DetectableMatch {
    display_name: String,
    application_id: String,
}

#[derive(Debug, Clone, Copy)]
struct DetectableResolution<'a> {
    entry: &'a DetectableMatch,
    strict_executable_match: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct DetectableApplication {
    id: String,
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    executables: Vec<DetectableExecutable>,
    #[serde(default, rename = "third_party_skus")]
    third_party_skus: Vec<DetectableThirdPartySku>,
}

#[derive(Debug, Clone, Deserialize)]
struct DetectableExecutable {
    name: String,
    os: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DetectableThirdPartySku {
    distributor: String,
    id: Option<String>,
}

impl DetectableIndex {
    fn load() -> Self {
        static INDEX: OnceLock<DetectableIndex> = OnceLock::new();
        INDEX.get_or_init(fetch_detectable_index).clone()
    }

    fn resolve_process(
        &self,
        executable_name: &str,
        process_path: Option<&Path>,
    ) -> Option<DetectableResolution<'_>> {
        let normalized_executable = normalize_detectable_key(executable_name);
        let normalized_process_path =
            process_path.map(|path| normalize_path_key(path.to_string_lossy()));

        if let Some(path) = normalized_process_path.as_deref() {
            for (suffix, entry) in &self.executable_suffixes {
                if path.ends_with(suffix) {
                    let strict_executable_match = normalized_executable
                        == normalize_detectable_key(executable_basename(suffix));
                    return Some(DetectableResolution {
                        entry,
                        strict_executable_match,
                    });
                }
            }
        }

        self.by_executable
            .get(&normalized_executable)
            .map(|entry| DetectableResolution {
                entry,
                strict_executable_match: true,
            })
    }

    fn has_known_mapping(&self, steam_id: Option<&str>, display_name: Option<&str>) -> bool {
        steam_id
            .and_then(|value| self.by_steam_id.get(&normalize_detectable_key(value)))
            .is_some()
            || display_name
                .and_then(|value| self.by_name.get(&normalize_detectable_key(value)))
                .is_some()
    }
}

fn same_native_activity(
    current: Option<&NativeDetectedActivity>,
    next: Option<&NativeDetectedActivity>,
) -> bool {
    match (current, next) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.pid == right.pid
                && left.socket_id == right.socket_id
                && left.name == right.name
                && left.application_id == right.application_id
                && left.start_time == right.start_time
        }
        _ => false,
    }
}

fn synthetic_application_id(prefix: &str, path: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    prefix.hash(&mut hasher);
    path.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("native-{}-{:016x}", prefix, hasher.finish())
}

fn steam_apps_index() -> Vec<SteamAppEntry> {
    let mut entries = Vec::new();
    for library in steam_library_dirs() {
        let manifests_dir = library.join("steamapps");
        let Ok(manifests) = fs::read_dir(&manifests_dir) else {
            continue;
        };

        for manifest in manifests.flatten() {
            let path = manifest.path();
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            let Some(name) = extract_vdf_string(&contents, "name") else {
                continue;
            };
            let Some(install_dir_name) = extract_vdf_string(&contents, "installdir") else {
                continue;
            };

            entries.push(SteamAppEntry {
                display_name: name,
                install_dir: manifests_dir.join("common").join(install_dir_name),
                steam_id: steam_app_id_from_manifest_name(file_name),
            });
        }
    }

    entries.sort_by(|left, right| {
        right
            .install_dir
            .as_os_str()
            .len()
            .cmp(&left.install_dir.as_os_str().len())
    });
    entries
}

fn steam_library_dirs() -> Vec<PathBuf> {
    let mut libraries = Vec::new();
    let mut candidates = Vec::new();

    if let Some(program_files_x86) = std::env::var_os("PROGRAMFILES(X86)") {
        candidates.push(
            PathBuf::from(program_files_x86)
                .join("Steam")
                .join("steamapps")
                .join("libraryfolders.vdf"),
        );
    }
    if let Some(program_files) = std::env::var_os("PROGRAMFILES") {
        candidates.push(
            PathBuf::from(program_files)
                .join("Steam")
                .join("steamapps")
                .join("libraryfolders.vdf"),
        );
    }

    for candidate in candidates {
        let Ok(contents) = fs::read_to_string(&candidate) else {
            continue;
        };
        for line in contents.lines() {
            let normalized = line.replace("\\\\", "\\");
            if let Some(path) = extract_vdf_line_value(&normalized, "path") {
                let library = PathBuf::from(path);
                if !libraries.iter().any(|existing| existing == &library) {
                    libraries.push(library);
                }
            }
        }
    }

    libraries
}

fn extract_vdf_string(contents: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    for line in contents.lines() {
        if !line.contains(&needle) {
            continue;
        }
        if let Some(value) = extract_vdf_line_value(line, key) {
            return Some(value);
        }
    }
    None
}

fn extract_vdf_line_value(line: &str, key: &str) -> Option<String> {
    let mut quote_positions = line.match_indices('"').map(|(index, _)| index);
    let key_start = quote_positions.next()?;
    let key_end = quote_positions.next()?;
    if line.get(key_start + 1..key_end)? != key {
        return None;
    }

    let value_start = quote_positions.next()?;
    let value_end = quote_positions.next()?;
    let value = line.get(value_start + 1..value_end)?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn fetch_detectable_index() -> DetectableIndex {
    let client = match Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(crate::browser_runtime::standard_http_user_agent())
        .build()
    {
        Ok(client) => client,
        Err(_) => return DetectableIndex::default(),
    };

    let applications = match client
        .get(DETECTABLE_APPS_URL)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json::<Vec<DetectableApplication>>())
    {
        Ok(applications) => applications,
        Err(_) => return DetectableIndex::default(),
    };

    let mut index = DetectableIndex::default();
    for application in applications {
        let application_id = application.id.trim().to_owned();
        if application_id.is_empty() {
            continue;
        }

        let entry = DetectableMatch {
            display_name: application.name.clone(),
            application_id,
        };

        for executable in application
            .executables
            .iter()
            .filter(|executable| executable.os.eq_ignore_ascii_case("win32"))
        {
            let normalized = normalize_path_key(executable.name.as_str());
            if normalized.is_empty() {
                continue;
            }

            index
                .by_executable
                .entry(executable_basename(&normalized).to_owned())
                .or_insert_with(|| entry.clone());

            if normalized.contains('\\') {
                index
                    .executable_suffixes
                    .push((normalized.clone(), entry.clone()));
            }
        }

        for sku in application
            .third_party_skus
            .iter()
            .filter(|sku| sku.distributor.eq_ignore_ascii_case("steam"))
        {
            if let Some(id) = sku.id.as_deref().filter(|value| !value.trim().is_empty()) {
                index
                    .by_steam_id
                    .entry(normalize_detectable_key(id))
                    .or_insert_with(|| entry.clone());
            }
        }

        index
            .by_name
            .entry(normalize_detectable_key(&application.name))
            .or_insert_with(|| entry.clone());

        for alias in application.aliases {
            if !alias.trim().is_empty() {
                index
                    .by_name
                    .entry(normalize_detectable_key(&alias))
                    .or_insert_with(|| entry.clone());
            }
        }
    }

    index
        .executable_suffixes
        .sort_by(|left, right| right.0.len().cmp(&left.0.len()));

    index
}

fn steam_app_id_from_manifest_name(file_name: &str) -> Option<String> {
    file_name
        .strip_prefix("appmanifest_")
        .and_then(|value| value.strip_suffix(".acf"))
        .map(ToOwned::to_owned)
}

fn executable_basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn normalize_detectable_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_path_key(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .trim()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn windows_path_starts_with(path: &Path, base: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let path = normalize_path_key(path.to_string_lossy());
        let base = normalize_path_key(base.to_string_lossy());
        path == base || path.starts_with(&(base + "\\"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.starts_with(base)
    }
}

#[cfg(target_os = "windows")]
fn visible_windows_by_pid(processes: &[RunningProcess]) -> HashMap<u32, Vec<VisibleWindowInfo>> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible,
    };

    struct EnumContext<'a> {
        by_pid: &'a mut HashMap<u32, Vec<VisibleWindowInfo>>,
        process_names: HashMap<u32, String>,
    }

    unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let context = &mut *(lparam.0 as *mut EnumContext<'_>);

        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }

        let text_length = GetWindowTextLengthW(hwnd);
        if text_length <= 0 {
            return BOOL(1);
        }

        let mut title_buffer = vec![0_u16; text_length as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut title_buffer);
        if copied <= 0 {
            return BOOL(1);
        }

        let title = String::from_utf16_lossy(&title_buffer[..copied as usize])
            .trim()
            .to_owned();
        if title.is_empty() {
            return BOOL(1);
        }

        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }

        let app_name = context.process_names.get(&pid).cloned().unwrap_or_default();

        context
            .by_pid
            .entry(pid)
            .or_default()
            .push(VisibleWindowInfo { title, app_name });

        BOOL(1)
    }

    let process_names = processes
        .iter()
        .map(|process| (process.pid, process.exe_name.clone()))
        .collect::<HashMap<_, _>>();
    let mut by_pid = HashMap::<u32, Vec<VisibleWindowInfo>>::new();
    let mut context = EnumContext {
        by_pid: &mut by_pid,
        process_names,
    };

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM((&mut context as *mut EnumContext<'_>) as isize),
        );
    }

    by_pid
}

#[cfg(not(target_os = "windows"))]
fn visible_windows_by_pid(_processes: &[RunningProcess]) -> HashMap<u32, Vec<VisibleWindowInfo>> {
    HashMap::new()
}

fn has_eligible_game_window(windows: &[VisibleWindowInfo], display_name: &str) -> bool {
    let normalized_display_name = normalize_detectable_key(display_name);
    windows.iter().any(|window| {
        let title = normalize_detectable_key(&window.title);
        let app_name = normalize_detectable_key(&window.app_name);

        if window_looks_like_launcher(&title) || window_looks_like_launcher(&app_name) {
            return false;
        }

        if normalized_display_name.is_empty() {
            return true;
        }

        window_matches_game_identity(&title, &app_name, &normalized_display_name)
    })
}

fn window_matches_game_identity(title: &str, app_name: &str, display_name: &str) -> bool {
    if title.contains(display_name)
        || app_name.contains(display_name)
        || display_name.contains(title)
        || display_name.contains(app_name)
    {
        return true;
    }

    let display_tokens = significant_identity_tokens(display_name);
    if display_tokens.is_empty() {
        return false;
    }

    display_tokens
        .iter()
        .all(|token| title.contains(token.as_str()) || app_name.contains(token.as_str()))
}

fn significant_identity_tokens(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter_map(|segment| {
            let cleaned = segment.trim_matches(|character: char| !character.is_alphanumeric());
            if cleaned.len() >= 4 {
                Some(cleaned.to_owned())
            } else {
                None
            }
        })
        .collect()
}

fn window_looks_like_launcher(value: &str) -> bool {
    let normalized = normalize_detectable_key(value);
    normalized.contains(" launcher")
        || normalized.contains("launchpad")
        || normalized.contains(" updater")
        || normalized.contains(" patcher")
        || normalized.contains(" installer")
        || normalized.contains(" setup")
        || normalized.contains(" crash reporter")
}

fn is_ignored_process(exe_name: &str) -> bool {
    let name = exe_name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "discord.exe"
            | "discordcanary.exe"
            | "discordptb.exe"
            | "equirust.exe"
            | "steam.exe"
            | "steamservice.exe"
            | "steamwebhelper.exe"
            | "explorer.exe"
            | "searchhost.exe"
            | "searchindexer.exe"
            | "shellexperiencehost.exe"
            | "startmenuexperiencehost.exe"
            | "applicationframehost.exe"
            | "runtimebroker.exe"
            | "dwm.exe"
            | "audiodg.exe"
            | "ctfmon.exe"
            | "systemsettings.exe"
            | "powershell.exe"
            | "cmd.exe"
            | "conhost.exe"
            | "bun.exe"
            | "node.exe"
            | "chrome.exe"
            | "msedge.exe"
            | "firefox.exe"
            | "brave.exe"
            | "opera.exe"
            | "code.exe"
            | "devenv.exe"
            | "discordhook64.dll"
    )
}

#[cfg(target_os = "windows")]
fn enumerate_processes() -> Vec<RunningProcess> {
    use std::mem::size_of;
    use windows::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };

    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(handle) if handle != INVALID_HANDLE_VALUE => handle,
        _ => return Vec::new(),
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut processes = Vec::new();

    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while has_entry {
        let pid = entry.th32ProcessID;
        if pid != 0 {
            let exe_name = wide_to_string(&entry.szExeFile);
            let path = process_image_path(pid);
            processes.push(RunningProcess {
                pid,
                exe_name,
                path,
            });
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }

    let _ = unsafe { CloseHandle(snapshot) };
    processes
}

#[cfg(not(target_os = "windows"))]
fn enumerate_processes() -> Vec<RunningProcess> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn process_image_path(pid: u32) -> Option<PathBuf> {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

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
    let path = String::from_utf16_lossy(&buffer);
    (!path.trim().is_empty()).then_some(PathBuf::from(path))
}

#[cfg(target_os = "windows")]
fn wide_to_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

#[cfg(target_os = "windows")]
fn start_native_rpc_bridge<R: Runtime>(app: &AppHandle<R>, generation: u64) {
    let app = app.clone();
    thread::spawn(move || run_native_rpc_bridge(app, generation));
}

#[cfg(not(target_os = "windows"))]
fn start_native_rpc_bridge<R: Runtime>(_app: &AppHandle<R>, _generation: u64) {}

#[cfg(target_os = "windows")]
fn run_native_rpc_bridge<R: Runtime>(app: AppHandle<R>, generation: u64) {
    let (listener, pipe_path) = match bind_listener() {
        Ok(value) => value,
        Err(err) => {
            log::warn!("Failed to start native Discord IPC bridge: {err}");
            return;
        }
    };

    log::info!("Native Discord IPC bridge listening on {}", pipe_path);

    while generation_is_current(&app, generation) {
        match listener.accept() {
            Ok(connection) => {
                log::info!("Native Discord IPC client connected on {}", pipe_path);
                let app_handle = app.clone();
                thread::spawn(move || handle_rpc_connection(app_handle, connection));
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_RETRY_DELAY);
            }
            Err(err) => {
                log::warn!("Native Discord IPC accept failed: {err}");
                thread::sleep(ACCEPT_RETRY_DELAY);
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn bind_listener() -> io::Result<(RpcPipeListener, String)> {
    let mut last_error: Option<io::Error> = None;

    for index in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{index}");
        match PipeListenerOptions::new()
            .path(StdPath::new(&path))
            .nonblocking(true)
            .create_duplex::<pipe_mode::Bytes>()
        {
            Ok(listener) => return Ok((listener, path)),
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| io::Error::other("no Discord IPC pipe slot available")))
}

#[cfg(target_os = "windows")]
fn handle_rpc_connection<R: Runtime>(
    app: AppHandle<R>,
    mut connection: DuplexPipeStream<pipe_mode::Bytes>,
) {
    let client_ordinal = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let mut socket_id = format!("native-rpc-client-{client_ordinal}");
    let mut application_id = String::new();

    let handshake = match read_rpc_frame(&mut connection) {
        Ok(frame) => frame,
        Err(err) => {
            log::warn!("Native Discord IPC handshake read failed: {err}");
            return;
        }
    };

    if handshake.opcode != OPCODE_HANDSHAKE {
        let _ = write_rpc_frame(
            &mut connection,
            OPCODE_CLOSE,
            &json!({
                "code": 4001,
                "message": "Expected handshake frame",
            }),
        );
        return;
    }

    if let Some(client_id) = handshake
        .payload
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        application_id = client_id.to_owned();
        socket_id = format!("native-rpc:{client_id}:{client_ordinal}");
    }
    log::info!(
        "Native Discord IPC handshake accepted socket_id={}",
        socket_id
    );

    let _ = write_rpc_frame(
        &mut connection,
        OPCODE_FRAME,
        &json!({
            "cmd": "DISPATCH",
            "data": {
                "v": 1,
                "config": {
                    "cdn_host": "cdn.discordapp.com",
                    "api_endpoint": "//discord.com/api",
                    "environment": "production",
                },
                "user": {
                    "id": "0",
                    "username": "Equirust",
                    "discriminator": "0000",
                    "bot": false,
                }
            },
            "evt": "READY",
            "nonce": Value::Null,
        }),
    );

    loop {
        let frame = match read_rpc_frame(&mut connection) {
            Ok(frame) => frame,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => break,
            Err(err) => {
                log::warn!("Native Discord IPC frame read failed: {err}");
                break;
            }
        };

        match frame.opcode {
            OPCODE_CLOSE => break,
            OPCODE_PING => {
                let _ = write_rpc_frame(&mut connection, OPCODE_PONG, &frame.payload);
            }
            OPCODE_FRAME => {
                let response = handle_rpc_request(&app, &socket_id, &application_id, frame.payload);
                let _ = write_rpc_frame(&mut connection, OPCODE_FRAME, &response);
            }
            _ => {}
        }
    }

    clear_external_rpc_activity(&app, &socket_id);
}

#[cfg(target_os = "windows")]
fn handle_rpc_request<R: Runtime>(
    app: &AppHandle<R>,
    socket_id: &str,
    application_id: &str,
    payload: Value,
) -> Value {
    let cmd = payload
        .get("cmd")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let nonce = payload.get("nonce").cloned().unwrap_or(Value::Null);

    match cmd.as_str() {
        "SET_ACTIVITY" => {
            let pid = payload
                .get("args")
                .and_then(|args| args.get("pid"))
                .and_then(Value::as_u64)
                .map(|value| value as u32);
            let mut activity = payload
                .get("args")
                .and_then(|args| args.get("activity"))
                .cloned()
                .unwrap_or(Value::Null);

            if activity.is_null() {
                log::info!("Native Discord IPC cleared activity for {}", socket_id);
                clear_external_rpc_activity(app, socket_id);
            } else {
                if let Some(activity_object) = activity.as_object_mut() {
                    if !activity_object.contains_key("application_id")
                        && !application_id.trim().is_empty()
                    {
                        activity_object.insert(
                            "application_id".into(),
                            Value::String(application_id.to_owned()),
                        );
                    }
                    activity_object
                        .entry("type")
                        .or_insert_with(|| Value::Number(0.into()));
                }

                let name = activity
                    .get("name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                log::info!(
                    "Native Discord IPC set activity socket_id={} application_id={}",
                    socket_id,
                    application_id
                );
                set_external_rpc_activity(app, socket_id.to_owned(), pid, name, activity.clone());
            }

            json!({
                "cmd": "SET_ACTIVITY",
                "data": {
                    "activity": activity,
                },
                "evt": Value::Null,
                "nonce": nonce,
            })
        }
        "SUBSCRIBE" => json!({
            "cmd": "SUBSCRIBE",
            "data": payload.get("args").cloned().unwrap_or(Value::Null),
            "evt": Value::Null,
            "nonce": nonce,
        }),
        "UNSUBSCRIBE" => json!({
            "cmd": "UNSUBSCRIBE",
            "data": payload.get("args").cloned().unwrap_or(Value::Null),
            "evt": Value::Null,
            "nonce": nonce,
        }),
        _ => json!({
            "cmd": cmd,
            "data": {
                "code": 4000,
                "message": format!("Unsupported native RPC command: {cmd}"),
            },
            "evt": "ERROR",
            "nonce": nonce,
        }),
    }
}

#[cfg(target_os = "windows")]
struct RpcFrame {
    opcode: u32,
    payload: Value,
}

#[cfg(target_os = "windows")]
fn read_rpc_frame(stream: &mut DuplexPipeStream<pipe_mode::Bytes>) -> io::Result<RpcFrame> {
    let mut header = [0_u8; 8];
    stream.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
    let length = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    if length > FRAME_MAX_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Discord IPC frame exceeded maximum size",
        ));
    }

    let mut body = vec![0_u8; length];
    if length > 0 {
        stream.read_exact(&mut body)?;
    }
    let payload = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?
    };

    Ok(RpcFrame { opcode, payload })
}

#[cfg(target_os = "windows")]
fn write_rpc_frame(
    stream: &mut DuplexPipeStream<pipe_mode::Bytes>,
    opcode: u32,
    payload: &Value,
) -> io::Result<()> {
    let body = serde_json::to_vec(payload)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let mut header = [0_u8; 8];
    header[..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..].copy_from_slice(&(body.len() as u32).to_le_bytes());
    stream.write_all(&header)?;
    if !body.is_empty() {
        stream.write_all(&body)?;
    }
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        executable_basename, extract_vdf_line_value, extract_vdf_string, has_eligible_game_window,
        normalize_path_key, steam_app_id_from_manifest_name, windows_path_starts_with,
        VisibleWindowInfo,
    };
    use std::path::Path;

    #[test]
    fn extracts_vdf_line_values() {
        assert_eq!(
            extract_vdf_line_value("\"name\"\t\t\"Warframe\"", "name").as_deref(),
            Some("Warframe")
        );
        assert_eq!(
            extract_vdf_line_value("\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"", "path")
                .as_deref(),
            Some("C:\\\\Program Files (x86)\\\\Steam")
        );
    }

    #[test]
    fn extracts_vdf_strings_from_manifest_content() {
        let contents =
            "\"AppState\"\n{\n\t\"name\"\t\t\"Warframe\"\n\t\"installdir\"\t\t\"Warframe\"\n}\n";
        assert_eq!(
            extract_vdf_string(contents, "name").as_deref(),
            Some("Warframe")
        );
        assert_eq!(
            extract_vdf_string(contents, "installdir").as_deref(),
            Some("Warframe")
        );
    }

    #[test]
    fn windows_prefix_match_is_case_insensitive() {
        assert!(windows_path_starts_with(
            Path::new("C:/Program Files (x86)/Steam/steamapps/common/Warframe/Warframe.x64.exe"),
            Path::new("c:/program files (x86)/steam/steamapps/common/warframe")
        ));
    }

    #[test]
    fn extracts_steam_app_id_from_manifest_name() {
        assert_eq!(
            steam_app_id_from_manifest_name("appmanifest_230410.acf").as_deref(),
            Some("230410")
        );
    }

    #[test]
    fn normalizes_executable_patterns() {
        assert_eq!(normalize_path_key("_retail_/WoW.exe"), "_retail_\\wow.exe");
        assert_eq!(executable_basename("_retail_\\wow.exe"), "wow.exe");
    }

    #[test]
    fn rejects_launcher_only_windows_for_activity_detection() {
        let windows = vec![VisibleWindowInfo {
            title: "Warframe Launcher".into(),
            app_name: "Warframe Launcher".into(),
        }];

        assert!(!has_eligible_game_window(&windows, "Warframe"));
    }

    #[test]
    fn accepts_real_game_windows_for_activity_detection() {
        let windows = vec![VisibleWindowInfo {
            title: "Warframe".into(),
            app_name: "Warframe".into(),
        }];

        assert!(has_eligible_game_window(&windows, "Warframe"));
    }

    #[test]
    fn rejects_unrelated_windows_for_activity_detection() {
        let windows = vec![VisibleWindowInfo {
            title: "Steam".into(),
            app_name: "Steam".into(),
        }];

        assert!(!has_eligible_game_window(&windows, "Warframe"));
    }

    #[test]
    fn accepts_windows_with_game_name_and_subtitle() {
        let windows = vec![VisibleWindowInfo {
            title: "Warframe - DirectX 12".into(),
            app_name: "Warframe".into(),
        }];

        assert!(has_eligible_game_window(&windows, "Warframe"));
    }
}
