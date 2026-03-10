#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoCaptureConfig {
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
}
