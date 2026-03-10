use super::sink_contract::DesktopStreamSinkDescriptor;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStreamStartRequest {
    pub source_id: String,
    pub source_kind: String,
    pub source_process_id: Option<u32>,
    pub capture_backend: Option<String>,
    pub audio_mode: Option<String>,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
    pub content_hint: String,
    pub include_system_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStreamEncoderPreviewRequest {
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStreamEncoderPreviewResponse {
    pub video_codec: String,
    pub encoder_mode: String,
    pub encoder_detail: Option<String>,
    pub color_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStreamStartResponse {
    pub session_id: String,
    pub websocket_url: String,
    pub auth_token: String,
    pub sink_descriptor: DesktopStreamSinkDescriptor,
    pub capture_backend: String,
    pub source_adapter_name: Option<String>,
    pub source_output_name: Option<String>,
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
pub struct DesktopStreamSessionState {
    pub session_id: String,
    pub running: bool,
    pub source_id: String,
    pub source_kind: String,
    pub sink_descriptor: DesktopStreamSinkDescriptor,
    pub capture_backend: Option<String>,
    pub source_adapter_name: Option<String>,
    pub source_output_name: Option<String>,
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
pub struct DesktopStreamEvent {
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
    pub adapter_name: Option<String>,
    pub output_name: Option<String>,
}
