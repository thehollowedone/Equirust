use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopStreamSinkKind {
    BrowserMediaStream,
    NativeSender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopStreamTransportKind {
    WebsocketBinary,
    SharedMemoryFrame,
    NativeSenderControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopStreamVideoIngressKind {
    GeneratedTrack,
    CanvasCapture,
    NativeSenderInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopStreamAudioIngressKind {
    MediaStreamDestination,
    NativeSenderInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStreamSinkDescriptor {
    pub sink_kind: DesktopStreamSinkKind,
    pub transport_kind: DesktopStreamTransportKind,
    pub preferred_video_ingress: DesktopStreamVideoIngressKind,
    pub fallback_video_ingress: Option<DesktopStreamVideoIngressKind>,
    pub preferred_audio_ingress: DesktopStreamAudioIngressKind,
    pub browser_owned_peer_connection: bool,
    pub browser_owned_encoder: bool,
}

impl DesktopStreamSinkDescriptor {
    pub fn browser_generated_track_bridge() -> Self {
        Self {
            sink_kind: DesktopStreamSinkKind::BrowserMediaStream,
            transport_kind: DesktopStreamTransportKind::WebsocketBinary,
            preferred_video_ingress: DesktopStreamVideoIngressKind::GeneratedTrack,
            fallback_video_ingress: Some(DesktopStreamVideoIngressKind::CanvasCapture),
            preferred_audio_ingress: DesktopStreamAudioIngressKind::MediaStreamDestination,
            browser_owned_peer_connection: true,
            browser_owned_encoder: true,
        }
    }
}
