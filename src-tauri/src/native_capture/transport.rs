#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCapturePacketKind {
    Video = 0x01,
    Audio = 0x02,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueLimits {
    pub max_video_frames: usize,
    pub max_audio_buffer_ms: u32,
}

impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            max_video_frames: 3,
            max_audio_buffer_ms: 250,
        }
    }
}
