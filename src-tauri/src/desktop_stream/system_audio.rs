use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCaptureConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCaptureTarget {
    ProcessTree(u32),
    SystemExcludingProcessTree(u32),
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
        }
    }
}

const MAX_AUDIO_QUEUE_DURATION_MS: usize = 250;

pub struct LoopbackAudioCapture {
    stop_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl LoopbackAudioCapture {
    pub fn stop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LoopbackAudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "windows")]
pub fn start_loopback_capture(
    config: AudioCaptureConfig,
    target: AudioCaptureTarget,
) -> Result<(LoopbackAudioCapture, Receiver<Vec<u8>>), String> {
    use wasapi::initialize_mta;

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (audio_tx, audio_rx) = mpsc::sync_channel::<Vec<u8>>(8);

    let join_handle = thread::Builder::new()
        .name("NativeAudioLoopback".to_owned())
        .spawn(move || {
            let _ = initialize_mta();
            if let Err(err) = run_loopback_capture(config, target, stop_rx, audio_tx) {
                log::warn!("Native loopback audio stopped: {}", err);
            }
        })
        .map_err(|err| err.to_string())?;

    Ok((
        LoopbackAudioCapture {
            stop_tx,
            join_handle: Some(join_handle),
        },
        audio_rx,
    ))
}

#[cfg(target_os = "windows")]
fn run_loopback_capture(
    config: AudioCaptureConfig,
    target: AudioCaptureTarget,
    stop_rx: mpsc::Receiver<()>,
    audio_tx: SyncSender<Vec<u8>>,
) -> Result<(), String> {
    use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat};

    let desired_format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        config.sample_rate as usize,
        config.channels as usize,
        None,
    );
    let block_align = desired_format.get_blockalign() as usize;
    let chunk_frames = (config.sample_rate / 50).max(240) as usize;
    let (target_process_id, include_tree) = match target {
        AudioCaptureTarget::ProcessTree(process_id) => (process_id, true),
        AudioCaptureTarget::SystemExcludingProcessTree(process_id) => (process_id, false),
    };
    let mut audio_client =
        AudioClient::new_application_loopback_client(target_process_id, include_tree)
            .map_err(|err| err.to_string())?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };
    audio_client
        .initialize_client(&desired_format, &Direction::Capture, &mode)
        .map_err(|err| err.to_string())?;

    let event_handle = audio_client
        .set_get_eventhandle()
        .map_err(|err| err.to_string())?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|err| err.to_string())?;
    let buffer_frame_count = audio_client
        .get_buffer_size()
        .map_err(|err| err.to_string())?;
    let mut sample_queue =
        VecDeque::<u8>::with_capacity(100 * block_align * (1024 + 2 * buffer_frame_count as usize));
    let max_queue_bytes = ((config.sample_rate as usize)
        .saturating_mul(config.channels as usize)
        .saturating_mul(4)
        .saturating_mul(MAX_AUDIO_QUEUE_DURATION_MS))
        / 1000;
    let max_queue_bytes = max_queue_bytes.max(chunk_frames * block_align * 4);
    let mut queue_drop_events: u32 = 0;

    audio_client.start_stream().map_err(|err| err.to_string())?;

    loop {
        if stop_rx.try_recv().is_ok() {
            let _ = audio_client.stop_stream();
            break;
        }

        capture_client
            .read_from_device_to_deque(&mut sample_queue)
            .map_err(|err| err.to_string())?;
        if sample_queue.len() > max_queue_bytes {
            let overflow = sample_queue.len().saturating_sub(max_queue_bytes);
            for _ in 0..overflow {
                let _ = sample_queue.pop_front();
            }
            queue_drop_events = queue_drop_events.saturating_add(1);
            if queue_drop_events == 1 || queue_drop_events % 30 == 0 {
                log::warn!(
                    "Native loopback audio queue trimmed dropped_bytes={} events={}",
                    overflow,
                    queue_drop_events
                );
            }
        }

        let chunk_size_bytes = chunk_frames * block_align;
        while sample_queue.len() >= chunk_size_bytes {
            let mut chunk = vec![0u8; chunk_size_bytes];
            for byte in &mut chunk {
                *byte = sample_queue.pop_front().unwrap_or(0);
            }
            match audio_tx.try_send(chunk) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    let _ = audio_client.stop_stream();
                    return Ok(());
                }
            }
        }

        let _ = event_handle.wait_for_event(250);
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn start_loopback_capture(
    _config: AudioCaptureConfig,
    _target: AudioCaptureTarget,
) -> Result<(LoopbackAudioCapture, Receiver<Vec<u8>>), String> {
    Err("Native system audio capture is only supported on Windows.".to_owned())
}
