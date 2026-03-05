use super::{
    audio,
    encoder,
    source,
    types::{
        NativeCaptureEncoderPreviewRequest, NativeCaptureEncoderPreviewResponse,
        NativeCaptureEvent, NativeCaptureSessionState, NativeCaptureStartRequest,
        NativeCaptureStartResponse, ResolvedCaptureSource,
    },
};
use crate::privacy;
use serde_json::json;
use std::{
    collections::HashMap,
    net::TcpListener,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use std::io::ErrorKind;
use tauri::{AppHandle, Emitter, Manager, Runtime, State as TauriState};
use tungstenite::{accept, Error as WebSocketError, Message};
use xcap::{Frame, VideoRecorder};

const EVENT_NAME: &str = "equirust:native-capture-event";
const IDLE_ACCEPT_SLEEP: Duration = Duration::from_millis(50);
const IDLE_STREAM_SLEEP: Duration = Duration::from_millis(10);
const WINDOW_CAPTURE_TRANSIENT_GRACE: Duration = Duration::from_secs(20);
const VIDEO_PACKET_KIND: u8 = 0x01;
const AUDIO_PACKET_KIND: u8 = 0x02;
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn is_transient_transport_backpressure(err: &WebSocketError) -> bool {
    match err {
        WebSocketError::Io(io_err) => io_err.kind() == ErrorKind::WouldBlock,
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

struct ManagedSession {
    state: NativeCaptureSessionState,
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
pub fn start_native_capture_session<R: Runtime>(
    request: NativeCaptureStartRequest,
    app: AppHandle<R>,
    runtime: TauriState<'_, RuntimeState>,
) -> Result<NativeCaptureStartResponse, String> {
    let mut resolved = source::resolve_source(&request.source_id)?;
    if request.source_process_id.is_some() {
        resolved.process_id = request.source_process_id;
    }
    let width = normalize_even_dimension(request.width.max(1));
    let height = normalize_even_dimension(request.height.max(1));
    let frame_rate = request.frame_rate.max(1);
    let (preview_codec, preview_descriptor) =
        encoder::describe_preferred_encoder(width, height, frame_rate);
    let video_codec = preview_codec.label().to_owned();
    let session_id = format!(
        "native-capture-{}",
        NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let auth_token = generate_token();

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|err| err.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|err| err.to_string())?;
    let local_addr = listener.local_addr().map_err(|err| err.to_string())?;

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let session_state = NativeCaptureSessionState {
        session_id: session_id.clone(),
        running: true,
        source_id: resolved.source_id.clone(),
        source_kind: resolved.source_kind.clone(),
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
    let include_system_audio = session_state.audio_enabled;
    let join_handle = thread::spawn(move || {
        run_transport_stream(
            listener,
            stop_rx,
            &app_for_thread,
            runtime_map_for_thread,
            session_id_for_thread,
            source_id_for_thread,
            resolved,
            target_width,
            target_height,
            frame_rate,
            include_system_audio,
        );
    });

    runtime
        .inner
        .lock()
        .map_err(|_| "Native capture runtime mutex was poisoned".to_owned())?
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
        NativeCaptureEvent {
            kind: "started".to_owned(),
            session_id: session_state.session_id.clone(),
            source_id: Some(session_state.source_id.clone()),
            message: None,
        },
    );

    log::info!(
        "Native capture started session_id={} source_id={} size={}x{} fps={} audio={}",
        session_state.session_id,
        session_state.source_id,
        session_state.width,
        session_state.height,
        session_state.frame_rate,
        session_state.audio_enabled
    );

    Ok(NativeCaptureStartResponse {
        session_id: session_state.session_id,
        websocket_url: format!(
            "ws://127.0.0.1:{}/native-capture?token={}",
            local_addr.port(),
            auth_token
        ),
        auth_token,
        video_codec,
        encoder_mode: preview_descriptor.mode_label,
        encoder_detail: preview_descriptor.detail_label,
        color_mode: preview_descriptor.color_mode_label,
        width: session_state.width,
        height: session_state.height,
        frame_rate: session_state.frame_rate,
        audio_enabled: session_state.audio_enabled,
        audio_sample_rate: if session_state.audio_enabled { 48_000 } else { 0 },
        audio_channels: if session_state.audio_enabled { 2 } else { 0 },
    })
}

#[tauri::command]
pub fn get_native_capture_encoder_preview(
    request: NativeCaptureEncoderPreviewRequest,
) -> Result<NativeCaptureEncoderPreviewResponse, String> {
    let width = normalize_even_dimension(request.width.max(1));
    let height = normalize_even_dimension(request.height.max(1));
    let frame_rate = request.frame_rate.max(1);
    let (video_codec, descriptor) = encoder::describe_preferred_encoder(width, height, frame_rate);

    Ok(NativeCaptureEncoderPreviewResponse {
        video_codec: video_codec.label().to_owned(),
        encoder_mode: descriptor.mode_label,
        encoder_detail: descriptor.detail_label,
        color_mode: descriptor.color_mode_label,
    })
}

#[tauri::command]
pub fn stop_native_capture_session<R: Runtime>(
    session_id: String,
    app: AppHandle<R>,
    runtime: TauriState<'_, RuntimeState>,
) -> Result<(), String> {
    let mut session = runtime
        .inner
        .lock()
        .map_err(|_| "Native capture runtime mutex was poisoned".to_owned())?
        .remove(&session_id)
        .ok_or_else(|| format!("Native capture session was not found: {session_id}"))?;

    let _ = session.stop_tx.send(());
    if let Some(handle) = session.join_handle.take() {
        let session_id_for_join = session_id.clone();
        thread::spawn(move || {
            if let Err(err) = handle.join() {
                log::error!(
                    "Native capture join failed session_id={} panic={:?}",
                    session_id_for_join,
                    err
                );
            } else {
                log::info!(
                    "Native capture worker joined session_id={}",
                    session_id_for_join
                );
            }
        });
    }

    emit_event(
        &app,
        NativeCaptureEvent {
            kind: "ended".to_owned(),
            session_id: session_id.clone(),
            source_id: Some(session.state.source_id),
            message: None,
        },
    );

    log::info!("Native capture stopped session_id={session_id}");
    Ok(())
}

#[tauri::command]
pub fn get_native_capture_session_state(
    session_id: String,
    runtime: TauriState<'_, RuntimeState>,
) -> Result<NativeCaptureSessionState, String> {
    runtime
        .inner
        .lock()
        .map_err(|_| "Native capture runtime mutex was poisoned".to_owned())?
        .get(&session_id)
        .map(|session| session.state.clone())
        .ok_or_else(|| format!("Native capture session was not found: {session_id}"))
}

#[allow(clippy::too_many_arguments)]
fn run_transport_stream<R: Runtime>(
    listener: TcpListener,
    stop_rx: mpsc::Receiver<()>,
    app: &AppHandle<R>,
    runtime_map: Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_id: String,
    source_id: String,
    resolved: ResolvedCaptureSource,
    target_width: u32,
    target_height: u32,
    frame_rate: u32,
    include_system_audio: bool,
) {
    let mut video_encoder = encoder::VideoStreamEncoder::new(target_width, target_height, frame_rate);
    let video_codec = video_encoder.codec().label().to_owned();
    let encoder_descriptor = video_encoder.descriptor().clone();
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
        "source": {
            "id": resolved.source_id.clone(),
            "kind": resolved.source_kind.clone(),
            "nativeId": resolved.native_id,
            "name": resolved.display_name.clone(),
        }
    })
    .to_string();

    let frame_interval = Duration::from_millis((1000 / frame_rate.max(1) as u64).max(1));
    let mut screen_recorder: Option<VideoRecorder> = None;
    let mut screen_frame_rx = None;
    let mut latest_screen_frame: Option<Frame> = None;
    let mut audio_capture = None;
    let mut audio_rx = None;
    let mut window_capture_error_started_at: Option<Instant> = None;
    let mut window_capture_error_count: u32 = 0;
    let mut audio_backpressure_drop_count: u32 = 0;
    let mut video_backpressure_drop_count: u32 = 0;

    if resolved.source_kind == "screen" {
        if let Ok((recorder, rx)) = source::start_screen_video_recorder(&resolved) {
            if recorder.start().is_ok() {
                screen_frame_rx = Some(rx);
                screen_recorder = Some(recorder);
            }
        }
    }

    if include_system_audio {
        let audio_target = match (resolved.source_kind.as_str(), resolved.process_id) {
            ("window", Some(process_id)) => audio::AudioCaptureTarget::ProcessTree(process_id),
            _ => audio::AudioCaptureTarget::SystemExcludingProcessTree(std::process::id()),
        };
        match audio::start_loopback_capture(audio::AudioCaptureConfig::default(), audio_target) {
            Ok((capture, rx)) => {
                audio_capture = Some(capture);
                audio_rx = Some(rx);
            }
            Err(err) => {
                finish_with_error(
                    app,
                    &runtime_map,
                    &session_id,
                    &source_id,
                    "fatal",
                    &err,
                );
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
            Ok((stream, _)) => match accept(stream) {
                Ok(mut socket) => {
                    let _ = socket.send(Message::Text(hello_payload.clone().into()));
                    let mut next_frame_deadline = Instant::now();

                    loop {
                        if stop_rx.try_recv().is_ok() {
                            mark_session_stopped(&runtime_map, &session_id, None);
                            let _ = socket
                                .send(Message::Text(json!({ "type": "ended" }).to_string().into()));
                            let _ = socket.close(None);
                            break;
                        }

                        if Instant::now() < next_frame_deadline {
                            if let Some(rx) = audio_rx.as_ref() {
                                loop {
                                    match rx.try_recv() {
                                        Ok(chunk) => {
                                            let mut packet =
                                                Vec::with_capacity(chunk.len().saturating_add(1));
                                            packet.push(AUDIO_PACKET_KIND);
                                            packet.extend_from_slice(&chunk);
                                            if let Err(err) =
                                                socket.send(Message::Binary(packet.into()))
                                            {
                                                if is_transient_transport_backpressure(&err) {
                                                    audio_backpressure_drop_count =
                                                        audio_backpressure_drop_count
                                                            .saturating_add(1);
                                                    if audio_backpressure_drop_count == 1
                                                        || audio_backpressure_drop_count % 120 == 0
                                                    {
                                                        log::warn!(
                                                            "Native capture audio backpressure session_id={} source_id={} dropped_chunks={}",
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
                                                    if let Some(recorder) = screen_recorder.as_ref() {
                                                        let _ = recorder.stop();
                                                    }
                                                    if let Some(capture) = audio_capture.as_mut() {
                                                        capture.stop();
                                                    }
                                                    return;
                                                }
                                                if let Some(recorder) = screen_recorder.as_ref() {
                                                    let _ = recorder.stop();
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
                                        }
                                        Err(TryRecvError::Empty) => break,
                                        Err(TryRecvError::Disconnected) => {
                                            if let Some(recorder) = screen_recorder.as_ref() {
                                                let _ = recorder.stop();
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
                                                "Native system-audio capture disconnected.",
                                            );
                                            let _ = socket.send(
                                                Message::Text(
                                                    json!({
                                                        "type": "audio_device_lost",
                                                        "message": "Native system-audio capture disconnected."
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
                            thread::sleep(IDLE_STREAM_SLEEP);
                            continue;
                        }
                        next_frame_deadline = Instant::now() + frame_interval;

                        let frame_bytes = if let Some(rx) = screen_frame_rx.as_ref() {
                            loop {
                                match rx.try_recv() {
                                    Ok(frame) => latest_screen_frame = Some(frame),
                                    Err(TryRecvError::Empty) => break,
                                    Err(TryRecvError::Disconnected) => {
                                        finish_with_error(
                                            app,
                                            &runtime_map,
                                            &session_id,
                                            &source_id,
                                            "source_closed",
                                            "Native screen recorder disconnected.",
                                        );
                                        let _ = socket.send(
                                            Message::Text(
                                                json!({
                                                    "type": "source_closed",
                                                    "message": "Native screen recorder disconnected."
                                                })
                                                .to_string()
                                                .into(),
                                            ),
                                        );
                                        let _ = socket.close(None);
                                        if let Some(recorder) = screen_recorder.as_ref() {
                                            let _ = recorder.stop();
                                        }
                                        return;
                                    }
                                }
                            }

                            if let Some(frame) = latest_screen_frame.as_ref() {
                                match source::prepare_rgba_frame(
                                    frame.width,
                                    frame.height,
                                    frame.raw.clone(),
                                    target_width,
                                    target_height,
                                ) {
                                    Ok(prepared) => video_encoder.encode_rgba(
                                        prepared.width,
                                        prepared.height,
                                        &prepared.rgba,
                                    ),
                                    Err(err) => Err(err),
                                }
                            } else {
                                continue;
                            }
                        } else {
                            match source::capture_frame_rgba(&resolved, target_width, target_height)
                            {
                                Ok(prepared) => video_encoder.encode_rgba(
                                    prepared.width,
                                    prepared.height,
                                    &prepared.rgba,
                                ),
                                Err(err) => Err(err),
                            }
                        };

                        match frame_bytes {
                            Ok(frame_bytes) => {
                                window_capture_error_started_at = None;
                                window_capture_error_count = 0;
                                let mut packet =
                                    Vec::with_capacity(frame_bytes.bytes.len().saturating_add(10));
                                packet.push(VIDEO_PACKET_KIND);
                                packet.push(u8::from(frame_bytes.keyframe));
                                packet.extend_from_slice(&frame_bytes.timestamp_micros.to_le_bytes());
                                packet.extend_from_slice(&frame_bytes.bytes);
                                if let Err(err) = socket.send(Message::Binary(packet.into())) {
                                    if is_transient_transport_backpressure(&err) {
                                        video_backpressure_drop_count =
                                            video_backpressure_drop_count.saturating_add(1);
                                        if video_backpressure_drop_count == 1
                                            || video_backpressure_drop_count % 60 == 0
                                        {
                                            log::warn!(
                                                "Native capture video backpressure session_id={} source_id={} dropped_frames={}",
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
                                    if let Some(recorder) = screen_recorder.as_ref() {
                                        let _ = recorder.stop();
                                    }
                                    if let Some(capture) = audio_capture.as_mut() {
                                        capture.stop();
                                    }
                                    return;
                                }
                                note_video_frame_sent(
                                    &runtime_map,
                                    &session_id,
                                    frame_bytes.bytes.len() as u64,
                                );
                            }
                            Err(err) => {
                                if resolved.source_kind == "window" && source::source_is_alive(&resolved) {
                                    window_capture_error_count =
                                        window_capture_error_count.saturating_add(1);
                                    let started_at = window_capture_error_started_at
                                        .get_or_insert_with(Instant::now);
                                    if started_at.elapsed() < WINDOW_CAPTURE_TRANSIENT_GRACE {
                                        if window_capture_error_count == 1
                                            || window_capture_error_count % 120 == 0
                                        {
                                            let sanitized = privacy::sanitize_text_for_log(&err);
                                            log::warn!(
                                                "Native capture transient window frame failure session_id={} source_id={} count={} message={}",
                                                session_id,
                                                source_id,
                                                window_capture_error_count,
                                                sanitized
                                            );
                                        }
                                        continue;
                                    }
                                }
                                finish_with_error(
                                    app,
                                    &runtime_map,
                                    &session_id,
                                    &source_id,
                                    "source_closed",
                                    &err,
                                );
                                let _ = socket.send(
                                    Message::Text(
                                        json!({ "type": "source_closed", "message": err })
                                            .to_string()
                                            .into(),
                                    ),
                                );
                                let _ = socket.close(None);
                                if let Some(recorder) = screen_recorder.as_ref() {
                                    let _ = recorder.stop();
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
            },
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(IDLE_ACCEPT_SLEEP);
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

    if let Some(recorder) = screen_recorder.as_ref() {
        let _ = recorder.stop();
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
        NativeCaptureEvent {
            kind: kind.to_owned(),
            session_id: session_id.to_owned(),
            source_id: Some(source_id.to_owned()),
            message: Some(sanitized_message.clone()),
        },
    );
    log::error!(
        "Native capture {} session_id={} source_id={} message={}",
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

fn emit_event<R: Runtime>(app: &AppHandle<R>, payload: NativeCaptureEvent) {
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

fn normalize_even_dimension(value: u32) -> u32 {
    let adjusted = value.max(2);
    if adjusted % 2 == 0 {
        adjusted
    } else {
        adjusted.saturating_sub(1).max(2)
    }
}
