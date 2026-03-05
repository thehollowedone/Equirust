use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

#[allow(dead_code)]
pub const RENDERER_COMMAND_EVENT: &str = "equirust:ipc-command";
#[allow(dead_code)]
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererCommand {
    pub nonce: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererCommandResponse {
    #[allow(dead_code)]
    pub nonce: String,
    pub ok: bool,
    pub data: Option<Value>,
}

#[derive(Default)]
pub struct RuntimeState {
    #[allow(dead_code)]
    next_nonce: AtomicU64,
    pending: Mutex<HashMap<String, mpsc::Sender<RendererCommandResponse>>>,
}

#[tauri::command]
pub fn respond_renderer_command(
    nonce: String,
    ok: bool,
    data: Option<Value>,
    state: State<'_, RuntimeState>,
) -> Result<(), String> {
    let sender = state
        .pending
        .lock()
        .map_err(|_| "renderer command state is poisoned".to_owned())?
        .remove(&nonce);

    if let Some(sender) = sender {
        sender
            .send(RendererCommandResponse { nonce, ok, data })
            .map_err(|_| "renderer command receiver dropped".to_owned())?;
    } else {
        log::warn!("Received renderer command response for unknown nonce");
    }

    Ok(())
}

#[allow(dead_code)]
pub fn send_renderer_command<R: Runtime>(
    app: &AppHandle<R>,
    message: impl Into<String>,
    data: Option<Value>,
) -> Result<Value, String> {
    send_renderer_command_with_timeout(app, message, data, DEFAULT_TIMEOUT)
}

#[allow(dead_code)]
pub fn send_renderer_command_with_timeout<R: Runtime>(
    app: &AppHandle<R>,
    message: impl Into<String>,
    data: Option<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is not available".to_owned())?;
    let state = app.state::<RuntimeState>();
    let nonce = format!(
        "renderer-cmd-{}",
        state.next_nonce.fetch_add(1, Ordering::Relaxed)
    );
    let (sender, receiver) = mpsc::channel();

    state
        .pending
        .lock()
        .map_err(|_| "renderer command state is poisoned".to_owned())?
        .insert(nonce.clone(), sender);

    let request = RendererCommand {
        nonce: nonce.clone(),
        message: message.into(),
        data,
    };

    if let Err(err) = window.emit(RENDERER_COMMAND_EVENT, &request) {
        let _ = state
            .pending
            .lock()
            .map(|mut pending| pending.remove(&nonce));
        return Err(err.to_string());
    }

    let response = match receiver.recv_timeout(timeout) {
        Ok(response) => response,
        Err(err) => {
            let _ = state
                .pending
                .lock()
                .map(|mut pending| pending.remove(&nonce));
            return Err(format!(
                "renderer command '{}' timed out: {}",
                request.message, err
            ));
        }
    };

    if response.ok {
        Ok(response.data.unwrap_or(Value::Null))
    } else {
        Err(match response.data {
            Some(Value::String(message)) => message,
            Some(other) => other.to_string(),
            None => format!("renderer command '{}' failed", request.message),
        })
    }
}
