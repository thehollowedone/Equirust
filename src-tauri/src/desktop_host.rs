use crate::{privacy, store::PersistedStore};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State as TauriState, Window};
use tauri_runtime::ResizeDirection;

const CLIENT_RUNTIME_LOG_QUEUE_CAPACITY: usize = 2_048;
static CLIENT_RUNTIME_LOG_TX: OnceLock<SyncSender<String>> = OnceLock::new();
static CLIENT_RUNTIME_LOG_DROPPED: AtomicU64 = AtomicU64::new(0);

fn ensure_client_runtime_log_writer(app: &tauri::AppHandle) -> Option<&'static SyncSender<String>> {
    let log_dir = app.path().app_log_dir().ok()?;

    Some(CLIENT_RUNTIME_LOG_TX.get_or_init(move || {
        let worker_log_dir = log_dir.clone();
        let (tx, rx) = mpsc::sync_channel::<String>(CLIENT_RUNTIME_LOG_QUEUE_CAPACITY);

        std::thread::spawn(move || {
            let debug_log_path = worker_log_dir.join("Equirust-debug.log");

            while let Ok(first_message) = rx.recv() {
                if fs::create_dir_all(&worker_log_dir).is_err() {
                    continue;
                }

                let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&debug_log_path)
                else {
                    continue;
                };

                let mut messages = vec![first_message];
                while messages.len() < 128 {
                    match rx.try_recv() {
                        Ok(message) => messages.push(message),
                        Err(_) => break,
                    }
                }

                let dropped = CLIENT_RUNTIME_LOG_DROPPED.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let _ = writeln!(file, "[{timestamp}] dropped_client_runtime_logs={dropped}");
                }

                for message in messages {
                    let sanitized = privacy::sanitize_text_for_log(&message);
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let _ = writeln!(file, "[{timestamp}] {sanitized}");
                }
            }
        });

        tx
    }))
}

fn append_client_runtime_logs(messages: &[String], app: &tauri::AppHandle) {
    if messages.is_empty() {
        return;
    }

    let Some(sender) = ensure_client_runtime_log_writer(app) else {
        return;
    };

    for message in messages {
        match sender.try_send(message.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                CLIENT_RUNTIME_LOG_DROPPED.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

#[tauri::command]
pub fn log_client_runtime(
    message: String,
    app: tauri::AppHandle,
    _store: TauriState<'_, PersistedStore>,
) -> Result<(), String> {
    if !cfg!(debug_assertions) && !crate::browser_runtime::profiling_diagnostics_enabled() {
        return Ok(());
    }

    append_client_runtime_logs(&[message], &app);
    Ok(())
}

#[tauri::command]
pub fn log_client_runtime_batch(
    messages: Vec<String>,
    app: tauri::AppHandle,
    _store: TauriState<'_, PersistedStore>,
) -> Result<(), String> {
    if !cfg!(debug_assertions) && !crate::browser_runtime::profiling_diagnostics_enabled() {
        return Ok(());
    }

    append_client_runtime_logs(&messages, &app);
    Ok(())
}

#[tauri::command]
pub fn app_relaunch(app: tauri::AppHandle) -> Result<(), String> {
    app.restart();
}

#[tauri::command]
pub fn window_focus(window: Window) -> Result<(), String> {
    window.set_focus().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_close(window: Window) -> Result<(), String> {
    window.close().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_minimize(window: Window) -> Result<(), String> {
    window.minimize().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_toggle_maximize(window: Window) -> Result<(), String> {
    if window.is_maximized().map_err(|err| err.to_string())? {
        window.unmaximize().map_err(|err| err.to_string())
    } else {
        window.maximize().map_err(|err| err.to_string())
    }
}

#[tauri::command]
pub fn window_is_maximized(window: Window) -> Result<bool, String> {
    window.is_maximized().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_start_dragging(window: Window) -> Result<(), String> {
    window.start_dragging().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_set_title(title: String, window: Window) -> Result<(), String> {
    window.set_title(&title).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_start_resize_dragging(direction: String, window: Window) -> Result<(), String> {
    let direction = parse_resize_direction(&direction)
        .ok_or_else(|| format!("unsupported resize direction: {direction}"))?;

    window
        .start_resize_dragging(direction)
        .map_err(|err| err.to_string())
}

fn parse_resize_direction(direction: &str) -> Option<ResizeDirection> {
    match direction {
        "East" => Some(ResizeDirection::East),
        "North" => Some(ResizeDirection::North),
        "NorthEast" => Some(ResizeDirection::NorthEast),
        "NorthWest" => Some(ResizeDirection::NorthWest),
        "South" => Some(ResizeDirection::South),
        "SouthEast" => Some(ResizeDirection::SouthEast),
        "SouthWest" => Some(ResizeDirection::SouthWest),
        "West" => Some(ResizeDirection::West),
        _ => None,
    }
}
