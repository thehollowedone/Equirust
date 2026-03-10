use super::{
    capture_sources,
    contracts::{
        DesktopStreamEncoderPreviewRequest, DesktopStreamEncoderPreviewResponse,
        DesktopStreamEvent, DesktopStreamSessionState, DesktopStreamStartRequest,
        DesktopStreamStartResponse, ResolvedCaptureSource,
    },
    sink_contract::DesktopStreamSinkDescriptor,
    system_audio, video_encoder,
};
use crate::{privacy, window};
use serde_json::json;
use socket2::SockRef;
use std::io::{ErrorKind, Read, Write};
use std::{
    collections::HashMap,
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, Runtime, State as TauriState};
use tungstenite::{
    accept_hdr_with_config,
    handshake::server::{ErrorResponse, Request as WsRequest, Response as WsResponse},
    protocol::WebSocketConfig,
    Error as WebSocketError, Message,
};
use url::form_urlencoded;

const EVENT_NAME: &str = "equirust:desktop-stream-event";
const IDLE_ACCEPT_SLEEP: Duration = Duration::from_millis(50);
const IDLE_STREAM_SLEEP: Duration = Duration::from_millis(10);
const VIDEO_PACKET_KIND: u8 = 0x01;
const AUDIO_PACKET_KIND: u8 = 0x02;
const VIDEO_PACKET_HEADER_BYTES: usize = 18;
const MAX_VIDEO_PACKET_BYTES: usize = 16 * 1024 * 1024;
const MAX_AUDIO_PACKET_BYTES: usize = 512 * 1024;
const DESKTOP_STREAM_WS_WRITE_BUFFER_BYTES: usize = 0;
const DESKTOP_STREAM_WS_MAX_WRITE_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const DESKTOP_STREAM_TCP_SEND_BUFFER_BYTES: usize = 256 * 1024;
const DESKTOP_STREAM_TCP_RECV_BUFFER_BYTES: usize = 64 * 1024;
const DESKTOP_STREAM_STARTUP_FRAME_RATE_CAP: u32 = 20;
const DESKTOP_STREAM_STARTUP_RECOVERY_DELAY: Duration = Duration::from_millis(900);
const DESKTOP_STREAM_BACKPRESSURE_RECOVERY_DELAY: Duration = Duration::from_millis(1800);
const DESKTOP_STREAM_AUDIO_START_DELAY: Duration = Duration::from_millis(800);
const DESKTOP_STREAM_AUDIO_BACKPRESSURE_DELAY: Duration = Duration::from_millis(1500);
const DESKTOP_STREAM_MAX_AUDIO_CHUNKS_PER_SLICE: usize = 2;
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureAudioMode {
    Disabled,
    WindowApp,
    SystemExcludingHost,
}

fn is_transient_transport_backpressure(err: &WebSocketError) -> bool {
    match err {
        WebSocketError::Io(io_err) => {
            matches!(
                io_err.kind(),
                ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
            )
        }
        WebSocketError::WriteBufferFull(_) => true,
        _ => false,
    }
}

fn is_transport_disconnect(err: &WebSocketError) -> bool {
    matches!(
        err,
        WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed | WebSocketError::Io(_)
    )
}

fn is_transient_transport_idle(err: &WebSocketError) -> bool {
    matches!(
        err,
        WebSocketError::Io(io_err)
            if matches!(
                io_err.kind(),
                ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
            )
    )
}

fn is_authorized_transport_request(request: &WsRequest, expected_auth_token: &str) -> bool {
    if request.uri().path() != "/desktop-stream" {
        return false;
    }

    request
        .uri()
        .query()
        .map(|query| {
            form_urlencoded::parse(query.as_bytes())
                .any(|(key, value)| key == "token" && value == expected_auth_token)
        })
        .unwrap_or(false)
}

fn websocket_handshake_error(status: u16, message: &str) -> ErrorResponse {
    WsResponse::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Some(message.to_owned()))
        .unwrap_or_else(|_| ErrorResponse::new(Some(message.to_owned())))
}

fn frame_interval_for_frame_rate(frame_rate: u32) -> Duration {
    Duration::from_millis((1000 / frame_rate.max(1) as u64).max(1))
}

fn desktop_stream_websocket_config() -> WebSocketConfig {
    #[allow(deprecated)]
    WebSocketConfig {
        max_send_queue: None,
        write_buffer_size: DESKTOP_STREAM_WS_WRITE_BUFFER_BYTES,
        max_write_buffer_size: DESKTOP_STREAM_WS_MAX_WRITE_BUFFER_BYTES,
        max_message_size: Some(64 << 20),
        max_frame_size: Some(16 << 20),
        accept_unmasked_frames: false,
    }
}

fn tune_transport_stream(stream: &TcpStream, session_id: &str, source_id: &str) {
    if let Err(err) = stream.set_nodelay(true) {
        log::warn!(
            "Desktop stream failed to enable TCP_NODELAY session_id={} source_id={} error={}",
            session_id,
            source_id,
            err
        );
    }
    let socket_ref = SockRef::from(stream);
    if let Err(err) = socket_ref.set_send_buffer_size(DESKTOP_STREAM_TCP_SEND_BUFFER_BYTES) {
        log::warn!(
            "Desktop stream failed to reduce transport send buffer session_id={} source_id={} error={}",
            session_id,
            source_id,
            err
        );
    }
    if let Err(err) = socket_ref.set_recv_buffer_size(DESKTOP_STREAM_TCP_RECV_BUFFER_BYTES) {
        log::warn!(
            "Desktop stream failed to reduce transport recv buffer session_id={} source_id={} error={}",
            session_id,
            source_id,
            err
        );
    }
}

fn flush_transport_pending<S: Read + Write>(
    socket: &mut tungstenite::WebSocket<S>,
) -> Result<bool, WebSocketError> {
    match socket.flush() {
        Ok(()) => Ok(true),
        Err(err) if is_transient_transport_backpressure(&err) => Ok(false),
        Err(err) => Err(err),
    }
}

fn next_transport_frame_rate_cap(
    requested_frame_rate: u32,
    current_cap: Option<u32>,
) -> Option<u32> {
    let requested = requested_frame_rate.max(1);
    let base = current_cap.unwrap_or(requested).min(requested).max(1);
    let next = if base > 45 {
        45
    } else if base > 30 {
        30
    } else if base > 24 {
        24
    } else if base > 20 {
        20
    } else if base > 15 {
        15
    } else if base > 10 {
        10
    } else {
        base
    };
    if next >= requested {
        None
    } else {
        Some(next)
    }
}

fn should_clear_transport_frame_rate_cap(
    first_video_packet_sent_at: Option<Instant>,
    last_video_backpressure_at: Option<Instant>,
    now: Instant,
) -> bool {
    let Some(first_video_sent_at) = first_video_packet_sent_at else {
        return false;
    };
    if now.duration_since(first_video_sent_at) < DESKTOP_STREAM_STARTUP_RECOVERY_DELAY {
        return false;
    }
    last_video_backpressure_at
        .map(|last_backpressure_at| {
            now.duration_since(last_backpressure_at) >= DESKTOP_STREAM_BACKPRESSURE_RECOVERY_DELAY
        })
        .unwrap_or(true)
}

fn should_defer_audio_transport(
    first_video_packet_sent_at: Option<Instant>,
    last_video_backpressure_at: Option<Instant>,
    now: Instant,
) -> bool {
    let Some(first_video_sent_at) = first_video_packet_sent_at else {
        return true;
    };
    if now.duration_since(first_video_sent_at) < DESKTOP_STREAM_AUDIO_START_DELAY {
        return true;
    }
    last_video_backpressure_at
        .map(|last_backpressure_at| {
            now.duration_since(last_backpressure_at) < DESKTOP_STREAM_AUDIO_BACKPRESSURE_DELAY
        })
        .unwrap_or(false)
}

fn drain_transport_control<S: Read + Write>(
    socket: &mut tungstenite::WebSocket<S>,
    video_encoder: &mut video_encoder::VideoStreamEncoder,
    session_id: &str,
    source_id: &str,
    requested_frame_rate: u32,
    effective_frame_rate: &mut u32,
) -> Result<bool, String> {
    loop {
        match socket.read() {
            Ok(Message::Text(payload)) => {
                let Ok(control) = serde_json::from_str::<serde_json::Value>(payload.as_ref())
                else {
                    continue;
                };
                let Some(control_type) = control.get("type").and_then(|value| value.as_str())
                else {
                    continue;
                };
                if control_type == "request_keyframe" {
                    if let Err(err) = video_encoder.request_keyframe() {
                        log::warn!(
                            "Desktop stream keyframe request failed session_id={} source_id={} error={}",
                            session_id,
                            source_id,
                            err
                        );
                    }
                } else if control_type == "set_pacing_hint" {
                    let next_frame_rate = control
                        .get("maxFrameRate")
                        .and_then(|value| value.as_u64())
                        .map(|value| value.clamp(10, requested_frame_rate.max(10) as u64) as u32)
                        .unwrap_or(requested_frame_rate.max(1));
                    if *effective_frame_rate != next_frame_rate {
                        let reason = control
                            .get("reason")
                            .and_then(|value| value.as_str())
                            .unwrap_or("unknown");
                        let queue = control
                            .get("queue")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0);
                        let dropped = control
                            .get("dropped")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0);
                        let severity = control
                            .get("severity")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0);
                        log::info!(
                            "Desktop stream pacing update session_id={} source_id={} requested_fps={} effective_fps={} reason={} queue={} dropped={} severity={}",
                            session_id,
                            source_id,
                            requested_frame_rate,
                            next_frame_rate,
                            reason,
                            queue,
                            dropped,
                            severity
                        );
                        *effective_frame_rate = next_frame_rate;
                    }
                }
            }
            Ok(Message::Close(_)) => return Ok(true),
            Ok(_) => {}
            Err(err) if is_transient_transport_idle(&err) => return Ok(false),
            Err(err) if is_transport_disconnect(&err) => return Ok(true),
            Err(err) => return Err(err.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameFailureKind {
    Pending,
    Capture,
    Encode,
    Pipeline,
}

fn split_frame_failure(raw: String) -> (FrameFailureKind, String) {
    if let Some(message) = raw.strip_prefix("pending::") {
        return (FrameFailureKind::Pending, message.to_owned());
    }
    if let Some(message) = raw.strip_prefix("capture::") {
        return (FrameFailureKind::Capture, message.to_owned());
    }
    if let Some(message) = raw.strip_prefix("encode::") {
        return (FrameFailureKind::Encode, message.to_owned());
    }
    if let Some(message) = raw.strip_prefix("pipeline::") {
        return (FrameFailureKind::Pipeline, message.to_owned());
    }
    (FrameFailureKind::Pipeline, raw)
}

struct ManagedSession {
    state: DesktopStreamSessionState,
    stop_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

#[derive(Clone)]
pub struct RuntimeState {
    inner: Arc<Mutex<HashMap<String, ManagedSession>>>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[tauri::command]
pub fn start_desktop_stream_session<R: Runtime>(
    request: DesktopStreamStartRequest,
    app: AppHandle<R>,
    runtime: TauriState<'_, RuntimeState>,
) -> Result<DesktopStreamStartResponse, String> {
    let mut resolved = capture_sources::resolve_source(&request.source_id)?;
    let capture_backend = capture_sources::resolve_session_capture_backend(
        &resolved,
        capture_sources::resolve_capture_backend(request.capture_backend.as_deref()),
    );
    if request.source_process_id.is_some() {
        resolved.process_id = request.source_process_id;
    }
    let mut audio_mode = resolve_audio_mode(&request, &resolved);
    if audio_mode == CaptureAudioMode::WindowApp && resolved.process_id.is_none() {
        log::warn!(
            "Desktop stream app-audio routing requested without a process-backed window source; falling back to system_excluding_host source_id={}",
            request.source_id
        );
        audio_mode = CaptureAudioMode::SystemExcludingHost;
    }
    let system_audio_exclude_pid =
        window::get_main_browser_pid(&app).unwrap_or_else(std::process::id);
    let (width, height, dimensions_clamped) =
        clamp_desktop_stream_dimensions(request.width, request.height);
    let frame_rate = request.frame_rate.max(1);
    if dimensions_clamped {
        log::warn!(
            "Desktop stream request dimensions clamped source_id={} requested={}x{} applied={}x{}",
            request.source_id,
            request.width,
            request.height,
            width,
            height
        );
    }
    let (preview_codec, preview_descriptor) =
        video_encoder::describe_preferred_encoder(width, height, frame_rate);
    #[cfg(target_os = "windows")]
    if preview_codec == video_encoder::VideoCodec::Jpeg {
        return Err(
            "Desktop streaming requires H.264 encoding, but no supported encoder is available on this device."
                .to_owned(),
        );
    }
    let video_codec = preview_codec.label().to_owned();
    let session_id = format!(
        "desktop-stream-{}",
        NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let auth_token = generate_token();

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|err| err.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|err| err.to_string())?;
    let local_addr = listener.local_addr().map_err(|err| err.to_string())?;

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let sink_descriptor = DesktopStreamSinkDescriptor::browser_generated_track_bridge();
    let session_state = DesktopStreamSessionState {
        session_id: session_id.clone(),
        running: true,
        source_id: resolved.source_id.clone(),
        source_kind: resolved.source_kind.clone(),
        sink_descriptor: sink_descriptor.clone(),
        capture_backend: Some(capture_backend.label().to_owned()),
        source_adapter_name: resolved.adapter_name.clone(),
        source_output_name: resolved.output_name.clone(),
        video_codec: Some(video_codec.clone()),
        encoder_mode: Some(preview_descriptor.mode_label.clone()),
        encoder_detail: preview_descriptor.detail_label.clone(),
        color_mode: Some(preview_descriptor.color_mode_label.clone()),
        width,
        height,
        frame_rate,
        audio_enabled: request.include_system_audio,
        started_at: now_millis(),
        last_frame_at: None,
        video_frames_sent: 0,
        video_bytes_sent: 0,
        audio_packets_sent: 0,
        last_error: None,
    };
    let app_for_thread = app.clone();
    let session_id_for_thread = session_state.session_id.clone();
    let source_id_for_thread = session_state.source_id.clone();
    let runtime_map_for_thread = runtime.inner.clone();
    let target_width = session_state.width;
    let target_height = session_state.height;
    let frame_rate = session_state.frame_rate;
    let include_audio = session_state.audio_enabled;
    let auth_token_for_thread = auth_token.clone();
    let join_handle = thread::spawn(move || {
        run_transport_stream(
            listener,
            stop_rx,
            &app_for_thread,
            runtime_map_for_thread,
            session_id_for_thread,
            source_id_for_thread,
            auth_token_for_thread,
            resolved,
            target_width,
            target_height,
            frame_rate,
            include_audio,
            capture_backend,
            audio_mode,
            system_audio_exclude_pid,
        );
    });

    runtime
        .inner
        .lock()
        .map_err(|_| "Desktop stream runtime mutex was poisoned".to_owned())?
        .insert(
            session_state.session_id.clone(),
            ManagedSession {
                state: session_state.clone(),
                stop_tx,
                join_handle: Some(join_handle),
            },
        );

    emit_event(
        &app,
        DesktopStreamEvent {
            kind: "started".to_owned(),
            session_id: session_state.session_id.clone(),
            source_id: Some(session_state.source_id.clone()),
            message: None,
        },
    );

    log::info!(
        "Desktop stream started session_id={} source_id={} backend={} size={}x{} fps={} audio={} mode={:?} system_audio_exclude_pid={}",
        session_state.session_id,
        session_state.source_id,
        capture_backend.label(),
        session_state.width,
        session_state.height,
        session_state.frame_rate,
        session_state.audio_enabled,
        audio_mode,
        system_audio_exclude_pid
    );
    if session_state.source_adapter_name.is_some() || session_state.source_output_name.is_some() {
        log::info!(
            "Desktop stream source topology session_id={} source_id={} adapter={} output={}",
            session_state.session_id,
            session_state.source_id,
            session_state
                .source_adapter_name
                .as_deref()
                .unwrap_or("<unknown>"),
            session_state
                .source_output_name
                .as_deref()
                .unwrap_or("<unknown>")
        );
    }

    Ok(DesktopStreamStartResponse {
        session_id: session_state.session_id,
        websocket_url: format!(
            "ws://127.0.0.1:{}/desktop-stream?token={}",
            local_addr.port(),
            auth_token
        ),
        auth_token,
        sink_descriptor,
        capture_backend: capture_backend.label().to_owned(),
        source_adapter_name: session_state.source_adapter_name,
        source_output_name: session_state.source_output_name,
        video_codec,
        encoder_mode: preview_descriptor.mode_label,
        encoder_detail: preview_descriptor.detail_label,
        color_mode: preview_descriptor.color_mode_label,
        width: session_state.width,
        height: session_state.height,
        frame_rate: session_state.frame_rate,
        audio_enabled: session_state.audio_enabled,
        audio_sample_rate: if session_state.audio_enabled {
            48_000
        } else {
            0
        },
        audio_channels: if session_state.audio_enabled { 2 } else { 0 },
    })
}

#[tauri::command]
pub fn get_desktop_stream_encoder_preview(
    request: DesktopStreamEncoderPreviewRequest,
) -> Result<DesktopStreamEncoderPreviewResponse, String> {
    let width = normalize_even_dimension(request.width.max(1));
    let height = normalize_even_dimension(request.height.max(1));
    let frame_rate = request.frame_rate.max(1);
    let (video_codec, descriptor) =
        video_encoder::describe_preferred_encoder(width, height, frame_rate);

    Ok(DesktopStreamEncoderPreviewResponse {
        video_codec: video_codec.label().to_owned(),
        encoder_mode: descriptor.mode_label,
        encoder_detail: descriptor.detail_label,
        color_mode: descriptor.color_mode_label,
    })
}

#[tauri::command]
pub fn stop_desktop_stream_session<R: Runtime>(
    session_id: String,
    app: AppHandle<R>,
    runtime: TauriState<'_, RuntimeState>,
) -> Result<(), String> {
    let mut session = runtime
        .inner
        .lock()
        .map_err(|_| "Desktop stream runtime mutex was poisoned".to_owned())?
        .remove(&session_id)
        .ok_or_else(|| format!("Desktop stream session was not found: {session_id}"))?;

    let _ = session.stop_tx.send(());
    if let Some(handle) = session.join_handle.take() {
        let session_id_for_join = session_id.clone();
        thread::spawn(move || {
            if let Err(err) = handle.join() {
                log::error!(
                    "Desktop stream join failed session_id={} panic={:?}",
                    session_id_for_join,
                    err
                );
            } else {
                log::info!(
                    "Desktop stream worker joined session_id={}",
                    session_id_for_join
                );
            }
        });
    }

    emit_event(
        &app,
        DesktopStreamEvent {
            kind: "ended".to_owned(),
            session_id: session_id.clone(),
            source_id: Some(session.state.source_id),
            message: None,
        },
    );

    log::info!("Desktop stream stopped session_id={session_id}");
    Ok(())
}

#[tauri::command]
pub fn get_desktop_stream_session_state(
    session_id: String,
    runtime: TauriState<'_, RuntimeState>,
) -> Result<DesktopStreamSessionState, String> {
    runtime
        .inner
        .lock()
        .map_err(|_| "Desktop stream runtime mutex was poisoned".to_owned())?
        .get(&session_id)
        .map(|session| session.state.clone())
        .ok_or_else(|| format!("Desktop stream session was not found: {session_id}"))
}

#[allow(clippy::too_many_arguments)]
fn run_transport_stream<R: Runtime>(
    listener: TcpListener,
    stop_rx: mpsc::Receiver<()>,
    app: &AppHandle<R>,
    runtime_map: Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_id: String,
    source_id: String,
    expected_auth_token: String,
    resolved: ResolvedCaptureSource,
    target_width: u32,
    target_height: u32,
    frame_rate: u32,
    include_system_audio: bool,
    capture_backend: capture_sources::CaptureBackend,
    audio_mode: CaptureAudioMode,
    system_audio_exclude_pid: u32,
) {
    #[cfg(target_os = "windows")]
    let shared_capture_device = capture_sources::create_shared_capture_device(
        &resolved,
        capture_sources::CaptureFrameMode::SourceHdrTextureFrame,
    )
    .unwrap_or_else(|err| {
        log::debug!(
            "Shared D3D11 capture device unavailable session_id={} source_id={} error={}",
            session_id,
            source_id,
            err
        );
        None
    });
    #[cfg(target_os = "windows")]
    let mut video_encoder = video_encoder::VideoStreamEncoder::new_with_shared_device(
        target_width,
        target_height,
        frame_rate,
        shared_capture_device.clone(),
    );
    #[cfg(not(target_os = "windows"))]
    let mut video_encoder =
        video_encoder::VideoStreamEncoder::new(target_width, target_height, frame_rate);
    let capture_frame_mode = video_encoder.capture_frame_mode();
    let video_codec = video_encoder.codec().label().to_owned();
    #[cfg(target_os = "windows")]
    if video_encoder.codec() == video_encoder::VideoCodec::Jpeg {
        finish_with_error(
            app,
            &runtime_map,
            &session_id,
            &source_id,
            "fatal",
            "Desktop streaming requires H.264 encoding, but no supported encoder is available.",
        );
        return;
    }
    let encoder_descriptor = video_encoder.descriptor().clone();
    let sink_descriptor = DesktopStreamSinkDescriptor::browser_generated_track_bridge();
    update_session_encoder_descriptor(
        &runtime_map,
        &session_id,
        &video_codec,
        &encoder_descriptor.mode_label,
        encoder_descriptor.detail_label.clone(),
        &encoder_descriptor.color_mode_label,
    );
    let hello_payload = json!({
        "type": "hello",
        "sessionId": session_id,
        "video": {
            "codec": video_codec,
            "encoderMode": encoder_descriptor.mode_label,
            "encoderDetail": encoder_descriptor.detail_label,
            "colorMode": encoder_descriptor.color_mode_label,
            "width": target_width,
            "height": target_height,
            "frameRate": frame_rate,
        },
        "audio": {
            "enabled": include_system_audio,
            "sampleRate": if include_system_audio { 48_000 } else { 0 },
            "channels": if include_system_audio { 2 } else { 0 },
        },
        "sink": sink_descriptor,
        "source": {
            "id": resolved.source_id.clone(),
            "kind": resolved.source_kind.clone(),
            "nativeId": resolved.native_id,
            "name": resolved.display_name.clone(),
            "adapterName": resolved.adapter_name.clone(),
            "outputName": resolved.output_name.clone(),
        }
    })
    .to_string();

    let mut target_frame_rate = frame_rate.max(1);
    let mut transport_frame_rate_cap = if frame_rate > DESKTOP_STREAM_STARTUP_FRAME_RATE_CAP {
        Some(DESKTOP_STREAM_STARTUP_FRAME_RATE_CAP)
    } else {
        None
    };
    let mut effective_frame_rate =
        target_frame_rate.min(transport_frame_rate_cap.unwrap_or(target_frame_rate));
    let mut frame_interval = frame_interval_for_frame_rate(effective_frame_rate);
    let mut screen_capture: Option<capture_sources::ScreenCaptureSession> = None;
    let mut window_capture = None;
    let mut audio_capture = None;
    let mut audio_rx = None;
    let mut capture_error_streak: u32 = 0;
    let mut capture_restart_attempts: u32 = 0;
    let mut audio_backpressure_drop_count: u32 = 0;
    let mut audio_deferred_drop_count: u32 = 0;
    let mut video_backpressure_drop_count: u32 = 0;
    let mut transport_backpressure_wait_count: u32 = 0;
    let mut first_video_packet_sent_at: Option<Instant> = None;
    let mut last_video_backpressure_at: Option<Instant> = None;

    if resolved.source_kind == "screen" {
        #[cfg(target_os = "windows")]
        let screen_capture_result = capture_sources::ScreenCaptureSession::new(
            &resolved,
            frame_rate,
            capture_backend,
            shared_capture_device.clone(),
            capture_frame_mode,
        );
        #[cfg(not(target_os = "windows"))]
        let screen_capture_result =
            capture_sources::ScreenCaptureSession::new(&resolved, frame_rate, capture_backend);

        match screen_capture_result {
            Ok(session) => {
                screen_capture = Some(session);
            }
            Err(err) => {
                finish_with_error(app, &runtime_map, &session_id, &source_id, "fatal", &err);
                return;
            }
        }
    } else if resolved.source_kind == "window" {
        #[cfg(target_os = "windows")]
        let window_capture_result = capture_sources::WindowCaptureSession::new(
            &resolved,
            frame_rate,
            capture_backend,
            shared_capture_device.clone(),
            capture_frame_mode,
        );
        #[cfg(not(target_os = "windows"))]
        let window_capture_result =
            capture_sources::WindowCaptureSession::new(&resolved, frame_rate, capture_backend);

        match window_capture_result {
            Ok(session) => {
                window_capture = Some(session);
            }
            Err(err) => {
                finish_with_error(
                    app,
                    &runtime_map,
                    &session_id,
                    &source_id,
                    "source_closed",
                    &err,
                );
                return;
            }
        }
    }

    if include_system_audio {
        let audio_target = match audio_mode {
            CaptureAudioMode::Disabled => None,
            CaptureAudioMode::WindowApp => resolved
                .process_id
                .map(system_audio::AudioCaptureTarget::ProcessTree),
            CaptureAudioMode::SystemExcludingHost => Some(
                system_audio::AudioCaptureTarget::SystemExcludingProcessTree(
                    system_audio_exclude_pid,
                ),
            ),
        };
        let Some(audio_target) = audio_target else {
            finish_with_error(
                app,
                &runtime_map,
                &session_id,
                &source_id,
                "fatal",
                "Desktop stream audio routing requires a process-backed source.",
            );
            return;
        };
        match system_audio::start_loopback_capture(
            system_audio::AudioCaptureConfig::default(),
            audio_target,
        ) {
            Ok((capture, rx)) => {
                audio_capture = Some(capture);
                audio_rx = Some(rx);
            }
            Err(err) => {
                finish_with_error(app, &runtime_map, &session_id, &source_id, "fatal", &err);
                return;
            }
        }
    }

    loop {
        if stop_rx.try_recv().is_ok() {
            mark_session_stopped(&runtime_map, &session_id, None);
            break;
        }

        match listener.accept() {
            Ok((stream, _)) => {
                tune_transport_stream(&stream, &session_id, &source_id);
                let session_id_for_handshake = session_id.clone();
                let source_id_for_handshake = source_id.clone();
                let expected_auth_token_for_handshake = expected_auth_token.clone();
                match accept_hdr_with_config(
                    stream,
                    move |request: &WsRequest, response: WsResponse| {
                        if !is_authorized_transport_request(
                            request,
                            &expected_auth_token_for_handshake,
                        ) {
                            log::warn!(
                            "Desktop stream rejected unauthorized websocket session_id={} source_id={} path={}",
                            session_id_for_handshake,
                            source_id_for_handshake,
                            request.uri().path()
                        );
                            return Err(websocket_handshake_error(
                                401,
                                "Unauthorized desktop stream session",
                            ));
                        }
                        Ok(response)
                    },
                    Some(desktop_stream_websocket_config()),
                ) {
                    Ok(mut socket) => {
                        let _ = socket.send(Message::Text(hello_payload.clone().into()));
                        tune_transport_stream(socket.get_mut(), &session_id, &source_id);
                        if let Err(err) = socket.get_mut().set_nonblocking(true) {
                            log::warn!(
                            "Desktop stream failed to set websocket transport nonblocking mode session_id={} source_id={} error={}",
                            session_id,
                            source_id,
                            err
                        );
                        }
                        if let Err(err) = video_encoder.request_keyframe() {
                            log::warn!(
                                "Desktop stream failed to request startup keyframe session_id={} source_id={} error={}",
                                session_id,
                                source_id,
                                err
                            );
                        }
                        let mut next_frame_deadline = Instant::now();

                        loop {
                            if stop_rx.try_recv().is_ok() {
                                mark_session_stopped(&runtime_map, &session_id, None);
                                let _ = socket.send(Message::Text(
                                    json!({ "type": "ended" }).to_string().into(),
                                ));
                                let _ = socket.close(None);
                                break;
                            }

                            match drain_transport_control(
                                &mut socket,
                                &mut video_encoder,
                                &session_id,
                                &source_id,
                                frame_rate,
                                &mut target_frame_rate,
                            ) {
                                Ok(true) => {
                                    mark_session_stopped(&runtime_map, &session_id, None);
                                    if let Some(capture) = screen_capture.as_mut() {
                                        capture.stop();
                                    }
                                    if let Some(capture) = audio_capture.as_mut() {
                                        capture.stop();
                                    }
                                    return;
                                }
                                Ok(false) => {}
                                Err(err) => {
                                    finish_with_error(
                                        app,
                                        &runtime_map,
                                        &session_id,
                                        &source_id,
                                        "fatal",
                                        &format!(
                                            "Desktop stream transport control handling failed: {err}"
                                        ),
                                    );
                                    if let Some(capture) = screen_capture.as_mut() {
                                        capture.stop();
                                    }
                                    if let Some(capture) = audio_capture.as_mut() {
                                        capture.stop();
                                    }
                                    let _ = socket.send(Message::Text(
                                        json!({
                                            "type": "fatal",
                                            "message": "Desktop stream transport control handling failed."
                                        })
                                        .to_string()
                                        .into(),
                                    ));
                                    let _ = socket.close(None);
                                    return;
                                }
                            }
                            let refreshed_frame_interval = frame_interval_for_frame_rate(
                                target_frame_rate
                                    .min(transport_frame_rate_cap.unwrap_or(target_frame_rate)),
                            );
                            if refreshed_frame_interval != frame_interval {
                                effective_frame_rate = target_frame_rate
                                    .min(transport_frame_rate_cap.unwrap_or(target_frame_rate));
                                frame_interval = refreshed_frame_interval;
                                next_frame_deadline = Instant::now() + frame_interval;
                            }

                            let now = Instant::now();
                            if transport_frame_rate_cap.is_some()
                                && should_clear_transport_frame_rate_cap(
                                    first_video_packet_sent_at,
                                    last_video_backpressure_at,
                                    now,
                                )
                            {
                                transport_frame_rate_cap = None;
                                let restored_frame_rate = target_frame_rate
                                    .min(transport_frame_rate_cap.unwrap_or(target_frame_rate));
                                if restored_frame_rate != effective_frame_rate {
                                    effective_frame_rate = restored_frame_rate;
                                    frame_interval =
                                        frame_interval_for_frame_rate(effective_frame_rate);
                                    next_frame_deadline = now + frame_interval;
                                }
                                log::info!(
                                    "Desktop stream transport pacing recovered session_id={} source_id={} frame_rate={}",
                                    session_id,
                                    source_id,
                                    effective_frame_rate
                                );
                            }
                            if now < next_frame_deadline {
                                let transport_ready = match flush_transport_pending(&mut socket) {
                                    Ok(ready) => ready,
                                    Err(err) => {
                                        finish_with_error(
                                            app,
                                            &runtime_map,
                                            &session_id,
                                            &source_id,
                                            "fatal",
                                            &err.to_string(),
                                        );
                                        if let Some(capture) = screen_capture.as_mut() {
                                            capture.stop();
                                        }
                                        if let Some(capture) = audio_capture.as_mut() {
                                            capture.stop();
                                        }
                                        return;
                                    }
                                };
                                let defer_audio = should_defer_audio_transport(
                                    first_video_packet_sent_at,
                                    last_video_backpressure_at,
                                    now,
                                );
                                if !transport_ready {
                                    transport_backpressure_wait_count =
                                        transport_backpressure_wait_count.saturating_add(1);
                                    last_video_backpressure_at = Some(now);
                                    let next_cap = next_transport_frame_rate_cap(
                                        frame_rate,
                                        transport_frame_rate_cap,
                                    );
                                    if next_cap != transport_frame_rate_cap {
                                        transport_frame_rate_cap = next_cap;
                                        effective_frame_rate = target_frame_rate.min(
                                            transport_frame_rate_cap.unwrap_or(target_frame_rate),
                                        );
                                        frame_interval =
                                            frame_interval_for_frame_rate(effective_frame_rate);
                                        next_frame_deadline = now + frame_interval;
                                        log::warn!(
                                            "Desktop stream transport pacing tightened session_id={} source_id={} frame_rate={} reason=flush_pending",
                                            session_id,
                                            source_id,
                                            effective_frame_rate
                                        );
                                    }
                                    if transport_backpressure_wait_count == 1
                                        || transport_backpressure_wait_count % 120 == 0
                                    {
                                        log::warn!(
                                            "Desktop stream transport pending session_id={} source_id={} waits={}",
                                            session_id,
                                            source_id,
                                            transport_backpressure_wait_count
                                        );
                                    }
                                }
                                if let Some(rx) = audio_rx.as_ref() {
                                    let mut audio_chunks_sent_this_slice = 0usize;
                                    loop {
                                        match rx.try_recv() {
                                            Ok(chunk) => {
                                                if !transport_ready || defer_audio {
                                                    audio_deferred_drop_count =
                                                        audio_deferred_drop_count.saturating_add(1);
                                                    if audio_deferred_drop_count == 1
                                                        || audio_deferred_drop_count % 240 == 0
                                                    {
                                                        log::info!(
                                                            "Desktop stream audio deferred session_id={} source_id={} dropped_chunks={} reason={}",
                                                            session_id,
                                                            source_id,
                                                            audio_deferred_drop_count,
                                                            if !transport_ready {
                                                                "transport_pending"
                                                            } else {
                                                                "video_priority"
                                                            }
                                                        );
                                                    }
                                                    continue;
                                                }
                                                if audio_chunks_sent_this_slice
                                                    >= DESKTOP_STREAM_MAX_AUDIO_CHUNKS_PER_SLICE
                                                {
                                                    break;
                                                }
                                                if chunk.len() > MAX_AUDIO_PACKET_BYTES {
                                                    finish_with_error(
                                                    app,
                                                    &runtime_map,
                                                    &session_id,
                                                    &source_id,
                                                    "fatal",
                                                    "Desktop stream system-audio transport produced an oversized packet.",
                                                );
                                                    let _ = socket.send(Message::Text(
                                                    json!({
                                                        "type": "fatal",
                                                        "message": "Desktop stream system-audio transport produced an oversized packet."
                                                    })
                                                    .to_string()
                                                    .into(),
                                                ));
                                                    let _ = socket.close(None);
                                                    if let Some(capture) = screen_capture.as_mut() {
                                                        capture.stop();
                                                    }
                                                    if let Some(capture) = audio_capture.as_mut() {
                                                        capture.stop();
                                                    }
                                                    return;
                                                }
                                                let mut packet = Vec::with_capacity(
                                                    chunk.len().saturating_add(1),
                                                );
                                                packet.push(AUDIO_PACKET_KIND);
                                                packet.extend_from_slice(&chunk);
                                                if let Err(err) =
                                                    socket.send(Message::Binary(packet.into()))
                                                {
                                                    if is_transient_transport_backpressure(&err) {
                                                        last_video_backpressure_at = Some(now);
                                                        audio_backpressure_drop_count =
                                                            audio_backpressure_drop_count
                                                                .saturating_add(1);
                                                        if audio_backpressure_drop_count == 1
                                                            || audio_backpressure_drop_count % 120
                                                                == 0
                                                        {
                                                            log::warn!(
                                                            "Desktop stream audio backpressure session_id={} source_id={} dropped_chunks={}",
                                                            session_id,
                                                            source_id,
                                                            audio_backpressure_drop_count
                                                        );
                                                        }
                                                        continue;
                                                    }
                                                    if is_transport_disconnect(&err) {
                                                        mark_session_stopped(
                                                            &runtime_map,
                                                            &session_id,
                                                            None,
                                                        );
                                                        if let Some(capture) =
                                                            screen_capture.as_mut()
                                                        {
                                                            capture.stop();
                                                        }
                                                        if let Some(capture) =
                                                            audio_capture.as_mut()
                                                        {
                                                            capture.stop();
                                                        }
                                                        return;
                                                    }
                                                    if let Some(capture) = screen_capture.as_mut() {
                                                        capture.stop();
                                                    }
                                                    if let Some(capture) = audio_capture.as_mut() {
                                                        capture.stop();
                                                    }
                                                    finish_with_error(
                                                        app,
                                                        &runtime_map,
                                                        &session_id,
                                                        &source_id,
                                                        "audio_device_lost",
                                                        &err.to_string(),
                                                    );
                                                    return;
                                                }
                                                note_audio_packet_sent(&runtime_map, &session_id);
                                                audio_chunks_sent_this_slice =
                                                    audio_chunks_sent_this_slice.saturating_add(1);
                                            }
                                            Err(TryRecvError::Empty) => break,
                                            Err(TryRecvError::Disconnected) => {
                                                if let Some(capture) = screen_capture.as_mut() {
                                                    capture.stop();
                                                }
                                                if let Some(capture) = audio_capture.as_mut() {
                                                    capture.stop();
                                                }
                                                finish_with_error(
                                                    app,
                                                    &runtime_map,
                                                    &session_id,
                                                    &source_id,
                                                    "audio_device_lost",
                                                    "Desktop stream system-audio capture disconnected.",
                                                );
                                                let _ = socket.send(
                                                Message::Text(
                                                    json!({
                                                        "type": "audio_device_lost",
                                                        "message": "Desktop stream system-audio capture disconnected."
                                                    })
                                                    .to_string()
                                                    .into(),
                                                ),
                                            );
                                                let _ = socket.close(None);
                                                return;
                                            }
                                        }
                                    }
                                }
                                let sleep_for = next_frame_deadline
                                    .saturating_duration_since(now)
                                    .min(IDLE_STREAM_SLEEP);
                                if !sleep_for.is_zero() {
                                    thread::sleep(sleep_for);
                                }
                                continue;
                            }
                            match flush_transport_pending(&mut socket) {
                                Ok(true) => {}
                                Ok(false) => {
                                    transport_backpressure_wait_count =
                                        transport_backpressure_wait_count.saturating_add(1);
                                    last_video_backpressure_at = Some(now);
                                    let next_cap = next_transport_frame_rate_cap(
                                        frame_rate,
                                        transport_frame_rate_cap,
                                    );
                                    if next_cap != transport_frame_rate_cap {
                                        transport_frame_rate_cap = next_cap;
                                        effective_frame_rate = target_frame_rate.min(
                                            transport_frame_rate_cap.unwrap_or(target_frame_rate),
                                        );
                                        frame_interval =
                                            frame_interval_for_frame_rate(effective_frame_rate);
                                        next_frame_deadline = now + frame_interval;
                                        log::warn!(
                                            "Desktop stream transport pacing tightened session_id={} source_id={} frame_rate={} reason=pre_capture_flush",
                                            session_id,
                                            source_id,
                                            effective_frame_rate
                                        );
                                    }
                                    if transport_backpressure_wait_count == 1
                                        || transport_backpressure_wait_count % 120 == 0
                                    {
                                        log::warn!(
                                            "Desktop stream skipped frame production session_id={} source_id={} waits={}",
                                            session_id,
                                            source_id,
                                            transport_backpressure_wait_count
                                        );
                                    }
                                    let sleep_for = frame_interval.min(IDLE_STREAM_SLEEP);
                                    if !sleep_for.is_zero() {
                                        thread::sleep(sleep_for);
                                    }
                                    continue;
                                }
                                Err(err) => {
                                    finish_with_error(
                                        app,
                                        &runtime_map,
                                        &session_id,
                                        &source_id,
                                        "fatal",
                                        &err.to_string(),
                                    );
                                    if let Some(capture) = screen_capture.as_mut() {
                                        capture.stop();
                                    }
                                    if let Some(capture) = audio_capture.as_mut() {
                                        capture.stop();
                                    }
                                    return;
                                }
                            }
                            next_frame_deadline = next_frame_deadline
                                .checked_add(frame_interval)
                                .unwrap_or_else(|| now + frame_interval);
                            if next_frame_deadline <= now {
                                next_frame_deadline = now + frame_interval;
                            }

                            let frame_bytes = if resolved.source_kind == "window" {
                                match window_capture.as_mut() {
                                    Some(capture) => {
                                        match capture.capture_frame(
                                            target_width,
                                            target_height,
                                            capture_frame_mode,
                                        ) {
                                            Ok(prepared) => video_encoder
                                                .encode_frame(&prepared)
                                                .map_err(|err| format!("encode::{err}")),
                                            Err(err) => Err(format!("capture::{err}")),
                                        }
                                    }
                                    None => Err(
                                        "pipeline::Desktop stream window capture session was unavailable."
                                            .to_owned(),
                                    ),
                                }
                            } else if let Some(capture) = screen_capture.as_mut() {
                                match capture.capture_frame(
                                    target_width,
                                    target_height,
                                    capture_frame_mode,
                                ) {
                                    Ok(prepared) => video_encoder
                                        .encode_frame(&prepared)
                                        .map_err(|err| format!("encode::{err}")),
                                    Err(err) => Err(format!("capture::{err}")),
                                }
                            } else {
                                Err(format!(
                                "pipeline::{}",
                                "Desktop stream recorder pipeline was unavailable for the selected source."
                            ))
                            };

                            match frame_bytes {
                                Ok(frame_bytes) => {
                                    capture_error_streak = 0;
                                    capture_restart_attempts = 0;
                                    if frame_bytes.bytes.len() > MAX_VIDEO_PACKET_BYTES {
                                        finish_with_error(
                                        app,
                                        &runtime_map,
                                        &session_id,
                                        &source_id,
                                        "fatal",
                                        "Desktop stream video transport produced an oversized frame packet.",
                                    );
                                        let _ = socket.send(Message::Text(
                                        json!({
                                            "type": "fatal",
                                            "message": "Desktop stream video transport produced an oversized frame packet."
                                        })
                                        .to_string()
                                        .into(),
                                    ));
                                        let _ = socket.close(None);
                                        if let Some(capture) = screen_capture.as_mut() {
                                            capture.stop();
                                        }
                                        if let Some(capture) = audio_capture.as_mut() {
                                            capture.stop();
                                        }
                                        return;
                                    }
                                    let mut packet = Vec::with_capacity(
                                        frame_bytes
                                            .bytes
                                            .len()
                                            .saturating_add(VIDEO_PACKET_HEADER_BYTES),
                                    );
                                    packet.push(VIDEO_PACKET_KIND);
                                    packet.push(u8::from(frame_bytes.keyframe));
                                    packet.extend_from_slice(
                                        &frame_bytes.timestamp_micros.to_le_bytes(),
                                    );
                                    packet.extend_from_slice(&now_micros_u64().to_le_bytes());
                                    packet.extend_from_slice(&frame_bytes.bytes);
                                    if let Err(err) = socket.send(Message::Binary(packet.into())) {
                                        if is_transient_transport_backpressure(&err) {
                                            last_video_backpressure_at = Some(Instant::now());
                                            let next_cap = next_transport_frame_rate_cap(
                                                frame_rate,
                                                transport_frame_rate_cap,
                                            );
                                            if next_cap != transport_frame_rate_cap {
                                                transport_frame_rate_cap = next_cap;
                                                effective_frame_rate = target_frame_rate.min(
                                                    transport_frame_rate_cap
                                                        .unwrap_or(target_frame_rate),
                                                );
                                                frame_interval = frame_interval_for_frame_rate(
                                                    effective_frame_rate,
                                                );
                                                next_frame_deadline =
                                                    Instant::now() + frame_interval;
                                                log::warn!(
                                                    "Desktop stream transport pacing tightened session_id={} source_id={} frame_rate={} reason=video_send_backpressure",
                                                    session_id,
                                                    source_id,
                                                    effective_frame_rate
                                                );
                                            }
                                            if video_backpressure_drop_count == 0
                                                || (video_backpressure_drop_count + 1) % 30 == 0
                                            {
                                                let _ = video_encoder.request_keyframe();
                                            }
                                            video_backpressure_drop_count =
                                                video_backpressure_drop_count.saturating_add(1);
                                            if video_backpressure_drop_count == 1
                                                || video_backpressure_drop_count % 60 == 0
                                            {
                                                log::warn!(
                                                "Desktop stream video backpressure session_id={} source_id={} dropped_frames={}",
                                                session_id,
                                                source_id,
                                                video_backpressure_drop_count
                                            );
                                            }
                                            continue;
                                        }
                                        if !is_transport_disconnect(&err) {
                                            finish_with_error(
                                                app,
                                                &runtime_map,
                                                &session_id,
                                                &source_id,
                                                "fatal",
                                                &err.to_string(),
                                            );
                                        } else {
                                            mark_session_stopped(&runtime_map, &session_id, None);
                                        }
                                        if let Some(capture) = screen_capture.as_mut() {
                                            capture.stop();
                                        }
                                        if let Some(capture) = audio_capture.as_mut() {
                                            capture.stop();
                                        }
                                        return;
                                    }
                                    if first_video_packet_sent_at.is_none() {
                                        first_video_packet_sent_at = Some(Instant::now());
                                        log::info!(
                                            "Desktop stream first video packet sent session_id={} source_id={} frame_rate={} codec={}",
                                            session_id,
                                            source_id,
                                            effective_frame_rate,
                                            video_codec
                                        );
                                    }
                                    note_video_frame_sent(
                                        &runtime_map,
                                        &session_id,
                                        frame_bytes.bytes.len() as u64,
                                    );
                                }
                                Err(err) => {
                                    let (kind, message) = split_frame_failure(err);
                                    if kind == FrameFailureKind::Pending {
                                        capture_error_streak = 0;
                                        continue;
                                    }
                                    if kind == FrameFailureKind::Encode {
                                        finish_with_error(
                                            app,
                                            &runtime_map,
                                            &session_id,
                                            &source_id,
                                            "fatal",
                                            &message,
                                        );
                                        let _ = socket.send(Message::Text(
                                            json!({ "type": "fatal", "message": message })
                                                .to_string()
                                                .into(),
                                        ));
                                        let _ = socket.close(None);
                                        if let Some(capture) = screen_capture.as_mut() {
                                            capture.stop();
                                        }
                                        if let Some(capture) = audio_capture.as_mut() {
                                            capture.stop();
                                        }
                                        return;
                                    }

                                    if kind == FrameFailureKind::Capture
                                        && capture_sources::source_is_alive(&resolved)
                                    {
                                        capture_error_streak =
                                            capture_error_streak.saturating_add(1);
                                        if capture_error_streak <= 10 {
                                            if capture_error_streak == 1
                                                || capture_error_streak % 5 == 0
                                            {
                                                let sanitized =
                                                    privacy::sanitize_text_for_log(&message);
                                                log::warn!(
                                                "Desktop stream transient frame failure session_id={} source_id={} streak={} message={}",
                                                session_id,
                                                source_id,
                                                capture_error_streak,
                                                sanitized
                                            );
                                            }
                                            continue;
                                        }

                                        if capture_restart_attempts >= 5 {
                                            finish_with_error(
                                            app,
                                            &runtime_map,
                                            &session_id,
                                            &source_id,
                                            "source_closed",
                                            "Desktop stream exceeded restart attempts after repeated frame failures.",
                                        );
                                            let _ = socket.send(Message::Text(
                                            json!({
                                                "type": "source_closed",
                                                "message": "Desktop stream ended after repeated frame failures."
                                            })
                                            .to_string()
                                            .into(),
                                        ));
                                            let _ = socket.close(None);
                                            if let Some(capture) = screen_capture.as_mut() {
                                                capture.stop();
                                            }
                                            if let Some(capture) = audio_capture.as_mut() {
                                                capture.stop();
                                            }
                                            return;
                                        }

                                        capture_restart_attempts =
                                            capture_restart_attempts.saturating_add(1);
                                        let restart_result = if resolved.source_kind == "window" {
                                            #[cfg(target_os = "windows")]
                                            let window_restart_result =
                                                capture_sources::WindowCaptureSession::new(
                                                    &resolved,
                                                    frame_rate,
                                                    capture_backend,
                                                    shared_capture_device.clone(),
                                                    capture_frame_mode,
                                                );
                                            #[cfg(not(target_os = "windows"))]
                                            let window_restart_result =
                                                capture_sources::WindowCaptureSession::new(
                                                    &resolved,
                                                    frame_rate,
                                                    capture_backend,
                                                );

                                            window_restart_result.map(|session| {
                                                window_capture = Some(session);
                                            })
                                        } else {
                                            #[cfg(target_os = "windows")]
                                            let screen_restart_result =
                                                capture_sources::ScreenCaptureSession::new(
                                                    &resolved,
                                                    frame_rate,
                                                    capture_backend,
                                                    shared_capture_device.clone(),
                                                    capture_frame_mode,
                                                );
                                            #[cfg(not(target_os = "windows"))]
                                            let screen_restart_result =
                                                capture_sources::ScreenCaptureSession::new(
                                                    &resolved,
                                                    frame_rate,
                                                    capture_backend,
                                                );

                                            screen_restart_result.map(|session| {
                                                screen_capture = Some(session);
                                            })
                                        };

                                        match restart_result {
                                            Ok(()) => {
                                                capture_error_streak = 0;
                                                log::warn!(
                                                "Desktop stream restarted after transient failures session_id={} source_id={} restart_attempt={}",
                                                session_id,
                                                source_id,
                                                capture_restart_attempts
                                            );
                                                continue;
                                            }
                                            Err(restart_err) => {
                                                let sanitized =
                                                    privacy::sanitize_text_for_log(&restart_err);
                                                log::warn!(
                                                "Desktop stream restart attempt failed session_id={} source_id={} restart_attempt={} message={}",
                                                session_id,
                                                source_id,
                                                capture_restart_attempts,
                                                sanitized
                                            );
                                                continue;
                                            }
                                        }
                                    }
                                    finish_with_error(
                                        app,
                                        &runtime_map,
                                        &session_id,
                                        &source_id,
                                        "source_closed",
                                        &message,
                                    );
                                    let _ = socket.send(Message::Text(
                                        json!({ "type": "source_closed", "message": message })
                                            .to_string()
                                            .into(),
                                    ));
                                    let _ = socket.close(None);
                                    if let Some(capture) = screen_capture.as_mut() {
                                        capture.stop();
                                    }
                                    if let Some(capture) = audio_capture.as_mut() {
                                        capture.stop();
                                    }
                                    return;
                                }
                            }
                        }

                        break;
                    }
                    Err(err) => {
                        finish_with_error(
                            app,
                            &runtime_map,
                            &session_id,
                            &source_id,
                            "fatal",
                            &err.to_string(),
                        );
                        return;
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                match stop_rx.recv_timeout(IDLE_ACCEPT_SLEEP) {
                    Ok(()) => {
                        mark_session_stopped(&runtime_map, &session_id, None);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        mark_session_stopped(&runtime_map, &session_id, None);
                        break;
                    }
                }
            }
            Err(err) => {
                finish_with_error(
                    app,
                    &runtime_map,
                    &session_id,
                    &source_id,
                    "fatal",
                    &err.to_string(),
                );
                return;
            }
        }
    }

    if let Some(capture) = screen_capture.as_mut() {
        capture.stop();
    }
    if let Some(capture) = window_capture.as_mut() {
        capture.stop();
    }
    if let Some(capture) = audio_capture.as_mut() {
        capture.stop();
    }
}

fn finish_with_error<R: Runtime>(
    app: &AppHandle<R>,
    runtime_map: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_id: &str,
    source_id: &str,
    kind: &str,
    message: &str,
) {
    let sanitized_message = privacy::sanitize_text_for_log(message);
    mark_session_stopped(runtime_map, session_id, Some(sanitized_message.clone()));
    emit_event(
        app,
        DesktopStreamEvent {
            kind: kind.to_owned(),
            session_id: session_id.to_owned(),
            source_id: Some(source_id.to_owned()),
            message: Some(sanitized_message.clone()),
        },
    );
    log::error!(
        "Desktop stream {} session_id={} source_id={} message={}",
        kind,
        session_id,
        source_id,
        sanitized_message
    );
}

fn mark_session_stopped(
    runtime_map: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_id: &str,
    last_error: Option<String>,
) {
    if let Ok(mut sessions) = runtime_map.lock() {
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.running = false;
            session.state.last_error = last_error;
        }
    }
}

fn update_session_encoder_descriptor(
    runtime_map: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_id: &str,
    codec: &str,
    mode: &str,
    detail: Option<String>,
    color_mode: &str,
) {
    if let Ok(mut sessions) = runtime_map.lock() {
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.video_codec = Some(codec.to_owned());
            session.state.encoder_mode = Some(mode.to_owned());
            session.state.encoder_detail = detail;
            session.state.color_mode = Some(color_mode.to_owned());
        }
    }
}

fn note_video_frame_sent(
    runtime_map: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_id: &str,
    bytes: u64,
) {
    if let Ok(mut sessions) = runtime_map.lock() {
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.video_frames_sent = session.state.video_frames_sent.saturating_add(1);
            session.state.video_bytes_sent = session.state.video_bytes_sent.saturating_add(bytes);
            session.state.last_frame_at = Some(now_millis());
        }
    }
}

fn note_audio_packet_sent(
    runtime_map: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_id: &str,
) {
    if let Ok(mut sessions) = runtime_map.lock() {
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.audio_packets_sent = session.state.audio_packets_sent.saturating_add(1);
        }
    }
}

fn emit_event<R: Runtime>(app: &AppHandle<R>, payload: DesktopStreamEvent) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(EVENT_NAME, payload);
    }
}

fn generate_token() -> String {
    let counter = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{now:032x}{counter:016x}")
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn now_micros_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn clamp_desktop_stream_dimensions(
    requested_width: u32,
    requested_height: u32,
) -> (u32, u32, bool) {
    const MAX_WIDTH: u32 = 3840;
    const MAX_HEIGHT: u32 = 2160;
    let requested_width = normalize_even_dimension(requested_width.max(1));
    let requested_height = normalize_even_dimension(requested_height.max(1));

    let width_scale = requested_width as f64 / MAX_WIDTH as f64;
    let height_scale = requested_height as f64 / MAX_HEIGHT as f64;
    let scale = width_scale.max(height_scale).max(1.0);
    let scaled_width = normalize_even_dimension(((requested_width as f64) / scale).round() as u32);
    let scaled_height =
        normalize_even_dimension(((requested_height as f64) / scale).round() as u32);

    let width = scaled_width.min(MAX_WIDTH);
    let height = scaled_height.min(MAX_HEIGHT);
    let clamped = width != requested_width || height != requested_height;
    (width, height, clamped)
}

fn normalize_even_dimension(value: u32) -> u32 {
    let adjusted = value.max(2);
    if adjusted % 2 == 0 {
        adjusted
    } else {
        adjusted.saturating_sub(1).max(2)
    }
}

fn resolve_audio_mode(
    request: &DesktopStreamStartRequest,
    resolved: &ResolvedCaptureSource,
) -> CaptureAudioMode {
    if request.include_system_audio != true {
        return CaptureAudioMode::Disabled;
    }

    let requested = request
        .audio_mode
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "auto".to_owned());

    match requested.as_str() {
        "off" | "disabled" | "none" => CaptureAudioMode::Disabled,
        "window_app" | "app" | "app_audio" => CaptureAudioMode::WindowApp,
        "system_excluding_host" | "system" | "system_audio" => {
            CaptureAudioMode::SystemExcludingHost
        }
        _ => {
            if resolved.source_kind == "window" && resolved.process_id.is_some() {
                CaptureAudioMode::WindowApp
            } else {
                CaptureAudioMode::SystemExcludingHost
            }
        }
    }
}
