use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureStartRequest {
    pub source_id: String,
    pub source_kind: String,
    pub source_process_id: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
    pub content_hint: String,
    pub include_system_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureEncoderPreviewRequest {
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureEncoderPreviewResponse {
    pub video_codec: String,
    pub encoder_mode: String,
    pub encoder_detail: Option<String>,
    pub color_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureStartResponse {
    pub session_id: String,
    pub websocket_url: String,
    pub auth_token: String,
    pub video_codec: String,
    pub encoder_mode: String,
    pub encoder_detail: Option<String>,
    pub color_mode: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
    pub audio_enabled: bool,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureSessionState {
    pub session_id: String,
    pub running: bool,
    pub source_id: String,
    pub source_kind: String,
    pub video_codec: Option<String>,
    pub encoder_mode: Option<String>,
    pub encoder_detail: Option<String>,
    pub color_mode: Option<String>,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
    pub audio_enabled: bool,
    pub started_at: i64,
    pub last_frame_at: Option<i64>,
    pub video_frames_sent: u64,
    pub video_bytes_sent: u64,
    pub audio_packets_sent: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureEvent {
    pub kind: String,
    pub session_id: String,
    pub source_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedCaptureSource {
    pub source_id: String,
    pub source_kind: String,
    pub native_id: u32,
    pub display_name: String,
    pub process_id: Option<u32>,
}
