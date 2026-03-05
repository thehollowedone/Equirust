use openh264::{
    encoder::{Encoder as OpenH264Encoder, EncoderConfig, FrameType, RateControlMode},
    formats::{RgbaSliceU8, YUVBuffer, YUVSource},
    OpenH264API, Timestamp,
};

#[cfg(target_os = "windows")]
use std::{
    mem::ManuallyDrop,
    ptr::{null_mut},
};

#[cfg(target_os = "windows")]
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Media::MediaFoundation::{
            eAVEncH264VProfile_High, IMFActivate, IMFMediaBuffer, IMFMediaType, IMFSample,
            IMFTransform, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
            MFSampleExtension_CleanPoint, MFShutdown, MFStartup, MFSTARTUP_NOSOCKET,
            MFVideoFormat_H264, MFVideoFormat_I420, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
            MF_VERSION, MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT,
            MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE,
            MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
            MF_MT_MPEG2_PROFILE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO,
            MF_MT_SUBTYPE, MFTEnumEx, MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
            MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
            MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_INFO,
            MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MFT_CATEGORY_VIDEO_ENCODER,
        },
        System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoderPreference {
    HardwareFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264AnnexB,
    Jpeg,
}

impl VideoCodec {
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264AnnexB => "avc1.42E034",
            VideoCodec::Jpeg => "jpeg",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EncodedVideoFrame {
    pub bytes: Vec<u8>,
    pub keyframe: bool,
    pub timestamp_micros: u64,
}

#[derive(Debug, Clone)]
pub struct EncoderDescriptor {
    pub mode_label: String,
    pub detail_label: Option<String>,
    pub color_mode_label: String,
}

pub struct VideoStreamEncoder {
    inner: VideoStreamEncoderInner,
    descriptor: EncoderDescriptor,
}

enum VideoStreamEncoderInner {
    #[cfg(target_os = "windows")]
    MediaFoundation(MediaFoundationH264Encoder),
    H264(H264FrameEncoder),
    Jpeg(JpegFrameEncoder),
}

impl VideoStreamEncoder {
    pub fn new(width: u32, height: u32, frame_rate: u32) -> Self {
        #[cfg(target_os = "windows")]
        {
            if let Ok(encoder) =
                MediaFoundationH264Encoder::new(width, height, frame_rate, VideoEncoderPreference::HardwareFirst)
            {
                let descriptor = EncoderDescriptor {
                    mode_label: "Windows Hardware H.264".to_owned(),
                    detail_label: Some(encoder.friendly_name().to_owned()),
                    color_mode_label: "SDR-safe".to_owned(),
                };
                return Self {
                    inner: VideoStreamEncoderInner::MediaFoundation(encoder),
                    descriptor,
                };
            }
        }

        let hardware_probe = probe_windows_h264_hardware_encoder();
        match H264FrameEncoder::new(width, height, frame_rate, VideoEncoderPreference::HardwareFirst)
        {
            Ok(encoder) => Self {
                inner: VideoStreamEncoderInner::H264(encoder),
                descriptor: EncoderDescriptor {
                    mode_label: "Software H.264".to_owned(),
                    detail_label: hardware_probe
                        .as_ref()
                        .map(|name| format!("Windows hardware encoder unavailable, using OpenH264. Detected: {name}")),
                    color_mode_label: "SDR-safe".to_owned(),
                },
            },
            Err(err) => {
                log::warn!("Native H.264 encoder unavailable, falling back to JPEG transport: {err}");
                Self {
                    inner: VideoStreamEncoderInner::Jpeg(JpegFrameEncoder::new(frame_rate)),
                    descriptor: EncoderDescriptor {
                        mode_label: "JPEG fallback".to_owned(),
                        detail_label: hardware_probe
                            .as_ref()
                            .map(|name| format!("Windows hardware encoder unavailable, using JPEG fallback. Detected: {name}")),
                        color_mode_label: "SDR-safe".to_owned(),
                    },
                }
            }
        }
    }

    pub fn codec(&self) -> VideoCodec {
        match self.inner {
            #[cfg(target_os = "windows")]
            VideoStreamEncoderInner::MediaFoundation(_) => VideoCodec::H264AnnexB,
            VideoStreamEncoderInner::H264(_) => VideoCodec::H264AnnexB,
            VideoStreamEncoderInner::Jpeg(_) => VideoCodec::Jpeg,
        }
    }

    pub fn descriptor(&self) -> &EncoderDescriptor {
        &self.descriptor
    }

    pub fn encode_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<EncodedVideoFrame, String> {
        match &mut self.inner {
            #[cfg(target_os = "windows")]
            VideoStreamEncoderInner::MediaFoundation(encoder) => encoder.encode_rgba(width, height, rgba),
            VideoStreamEncoderInner::H264(encoder) => encoder.encode_rgba(width, height, rgba),
            VideoStreamEncoderInner::Jpeg(encoder) => encoder.encode_rgba(width, height, rgba),
        }
    }
}

pub fn describe_preferred_encoder(width: u32, height: u32, frame_rate: u32) -> (VideoCodec, EncoderDescriptor) {
    #[cfg(target_os = "windows")]
    if let Some(name) = probe_windows_h264_hardware_encoder() {
        return (
            VideoCodec::H264AnnexB,
            EncoderDescriptor {
                mode_label: "Windows Hardware H.264".to_owned(),
                detail_label: Some(name),
                color_mode_label: "SDR-safe".to_owned(),
            },
        );
    }

    if H264FrameEncoder::new(width, height, frame_rate, VideoEncoderPreference::HardwareFirst).is_ok() {
        (
            VideoCodec::H264AnnexB,
            EncoderDescriptor {
                mode_label: "Software H.264".to_owned(),
                detail_label: None,
                color_mode_label: "SDR-safe".to_owned(),
            },
        )
    } else {
        (
            VideoCodec::Jpeg,
            EncoderDescriptor {
                mode_label: "JPEG fallback".to_owned(),
                detail_label: None,
                color_mode_label: "SDR-safe".to_owned(),
            },
        )
    }
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MfInputFormat {
    Nv12,
    I420,
}

#[cfg(target_os = "windows")]
impl MfInputFormat {
    fn guid(self) -> windows::core::GUID {
        match self {
            MfInputFormat::Nv12 => MFVideoFormat_NV12,
            MfInputFormat::I420 => MFVideoFormat_I420,
        }
    }
}

#[cfg(target_os = "windows")]
struct MediaFoundationRuntime {
    com_initialized: bool,
    mf_started: bool,
}

#[cfg(target_os = "windows")]
impl MediaFoundationRuntime {
    fn start() -> Result<Self, String> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            let com_initialized = if hr.is_ok() {
                true
            } else if hr == RPC_E_CHANGED_MODE {
                false
            } else {
                return Err(format!("CoInitializeEx failed: {hr:?}"));
            };

            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)
                .map_err(|err| format!("MFStartup failed: {err}"))?;

            Ok(Self {
                com_initialized,
                mf_started: true,
            })
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for MediaFoundationRuntime {
    fn drop(&mut self) {
        unsafe {
            if self.mf_started {
                let _ = MFShutdown();
            }
            if self.com_initialized {
                CoUninitialize();
            }
        }
    }
}

#[cfg(target_os = "windows")]
struct MediaFoundationH264Encoder {
    _runtime: MediaFoundationRuntime,
    transform: IMFTransform,
    input_stream_id: u32,
    output_stream_id: u32,
    output_stream_info: MFT_OUTPUT_STREAM_INFO,
    input_format: MfInputFormat,
    frame_duration_hns: i64,
    frame_index: u64,
    sequence_header_annexb: Vec<u8>,
    friendly_name: String,
}

#[cfg(target_os = "windows")]
impl MediaFoundationH264Encoder {
    fn new(
        width: u32,
        height: u32,
        frame_rate: u32,
        _preference: VideoEncoderPreference,
    ) -> Result<Self, String> {
        let runtime = MediaFoundationRuntime::start()?;
        let (transform, friendly_name) = activate_hardware_h264_encoder()?;
        let input_stream_id = 0u32;
        let output_stream_id = 0u32;

        let output_type = create_output_media_type(width, height, frame_rate)?;
        unsafe {
            transform
                .SetOutputType(output_stream_id, &output_type, 0)
                .map_err(|err| format!("SetOutputType failed: {err}"))?;
        }

        let input_format = configure_input_media_type(&transform, input_stream_id, width, height, frame_rate)?;
        let output_stream_info = unsafe { transform.GetOutputStreamInfo(output_stream_id) }
            .map_err(|err| format!("GetOutputStreamInfo failed: {err}"))?;

        unsafe {
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(|err| format!("MFT begin streaming failed: {err}"))?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(|err| format!("MFT start stream failed: {err}"))?;
        }

        let sequence_header_annexb =
            read_sequence_header_annexb(&transform, output_stream_id).unwrap_or_default();

        Ok(Self {
            _runtime: runtime,
            transform,
            input_stream_id,
            output_stream_id,
            output_stream_info,
            input_format,
            frame_duration_hns: ((1_000_000u64 / frame_rate.max(1) as u64).max(1) * 10) as i64,
            frame_index: 0,
            sequence_header_annexb,
            friendly_name,
        })
    }

    fn friendly_name(&self) -> &str {
        &self.friendly_name
    }

    fn encode_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<EncodedVideoFrame, String> {
        let even_width = normalize_even_dimension(width);
        let even_height = normalize_even_dimension(height);
        if rgba.len() != (even_width as usize) * (even_height as usize) * 4 {
            return Err("Windows hardware encoder received an invalid RGBA frame".to_owned());
        }

        let rgba_source = RgbaSliceU8::new(rgba, (even_width as usize, even_height as usize));
        let yuv = YUVBuffer::from_rgb_source(rgba_source);
        let input_sample = create_input_sample(
            &yuv,
            self.input_format,
            self.frame_index as i64 * self.frame_duration_hns,
            self.frame_duration_hns,
        )?;

        unsafe {
            match self.transform.ProcessInput(self.input_stream_id, &input_sample, 0) {
                Ok(()) => {}
                Err(err) if err.code() == MF_E_NOTACCEPTING => {
                    let _ = self.collect_output(false)?;
                    self.transform
                        .ProcessInput(self.input_stream_id, &input_sample, 0)
                        .map_err(|inner| format!("Windows hardware encoder ProcessInput failed: {inner}"))?;
                }
                Err(err) => {
                    return Err(format!("Windows hardware encoder ProcessInput failed: {err}"));
                }
            }
        }

        let (bytes, keyframe) = self.collect_output(true)?;
        let encoded = EncodedVideoFrame {
            bytes,
            keyframe,
            timestamp_micros: (self.frame_index as i64 * self.frame_duration_hns / 10) as u64,
        };
        self.frame_index = self.frame_index.saturating_add(1);
        Ok(encoded)
    }

    fn collect_output(&mut self, require_bytes: bool) -> Result<(Vec<u8>, bool), String> {
        let mut bytes = Vec::new();
        let mut keyframe = false;

        loop {
            let sample = if needs_caller_allocated_sample(self.output_stream_info.dwFlags) {
                Some(create_output_sample(self.output_stream_info.cbSize.max(1_048_576))?)
            } else {
                None
            };

            let mut output_buffers = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: self.output_stream_id,
                pSample: ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            }];
            let mut status = 0u32;

            let process_result = unsafe { self.transform.ProcessOutput(0, &mut output_buffers, &mut status) };
            match process_result {
                Ok(()) => {
                    let sample = unsafe { ManuallyDrop::take(&mut output_buffers[0].pSample) };
                    let events = unsafe { ManuallyDrop::take(&mut output_buffers[0].pEvents) };
                    drop(events);

                    if let Some(sample) = sample {
                        let sample_keyframe =
                            unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0 };
                        let sample_bytes = read_sample_bytes(&sample)?;
                        let annexb = self.convert_output_to_annexb(&sample_bytes, sample_keyframe)?;
                        if sample_keyframe {
                            keyframe = true;
                        }
                        if !annexb.is_empty() {
                            bytes.extend_from_slice(&annexb);
                        }
                    }
                }
                Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.sequence_header_annexb =
                        read_sequence_header_annexb(&self.transform, self.output_stream_id)
                            .unwrap_or_default();
                    continue;
                }
                Err(err) => {
                    return Err(format!("Windows hardware encoder ProcessOutput failed: {err}"));
                }
            }
        }

        if require_bytes && bytes.is_empty() {
            return Err("Windows hardware encoder did not produce an output frame".to_owned());
        }

        Ok((bytes, keyframe))
    }

    fn convert_output_to_annexb(&self, sample_bytes: &[u8], keyframe: bool) -> Result<Vec<u8>, String> {
        if sample_bytes.is_empty() {
            return Ok(Vec::new());
        }

        let mut annexb = if looks_like_annexb(sample_bytes) {
            sample_bytes.to_vec()
        } else {
            avcc_payload_to_annexb(sample_bytes, &self.sequence_header_annexb)?
        };

        if keyframe
            && !self.sequence_header_annexb.is_empty()
            && !annexb_starts_with_header(&annexb, &self.sequence_header_annexb)
        {
            let mut with_header = self.sequence_header_annexb.clone();
            with_header.extend_from_slice(&annexb);
            annexb = with_header;
        }

        Ok(annexb)
    }
}

struct H264FrameEncoder {
    encoder: OpenH264Encoder,
    frame_duration_micros: u64,
    frame_index: u64,
    intra_interval_frames: u64,
}

impl H264FrameEncoder {
    fn new(
        width: u32,
        height: u32,
        frame_rate: u32,
        _preference: VideoEncoderPreference,
    ) -> Result<Self, String> {
        let config = EncoderConfig::new()
            .set_bitrate_bps(compute_target_bitrate_bps(width, height, frame_rate))
            .max_frame_rate(frame_rate.max(1) as f32)
            .rate_control_mode(RateControlMode::Bitrate)
            .debug(false);
        let encoder = OpenH264Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|err| err.to_string())?;

        Ok(Self {
            encoder,
            frame_duration_micros: (1_000_000u64 / frame_rate.max(1) as u64).max(1),
            frame_index: 0,
            intra_interval_frames: u64::from(frame_rate.max(1)).saturating_mul(2),
        })
    }

    fn encode_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<EncodedVideoFrame, String> {
        let even_width = normalize_even_dimension(width);
        let even_height = normalize_even_dimension(height);
        if rgba.len() != (even_width as usize) * (even_height as usize) * 4 {
            return Err("Native H.264 encoder received an invalid RGBA frame".to_owned());
        }

        if self.frame_index != 0
            && self.intra_interval_frames > 0
            && self.frame_index % self.intra_interval_frames == 0
        {
            self.encoder.force_intra_frame();
        }

        let rgba_source = RgbaSliceU8::new(rgba, (even_width as usize, even_height as usize));
        let yuv = YUVBuffer::from_rgb_source(rgba_source);
        let timestamp_millis =
            self.frame_index.saturating_mul(self.frame_duration_micros).saturating_div(1_000);
        let bitstream = self
            .encoder
            .encode_at(&yuv, Timestamp::from_millis(timestamp_millis))
            .map_err(|err| err.to_string())?;
        let frame_type = bitstream.frame_type();
        let bytes = bitstream.to_vec();
        let encoded = EncodedVideoFrame {
            bytes,
            keyframe: matches!(frame_type, FrameType::IDR | FrameType::I),
            timestamp_micros: self.frame_index.saturating_mul(self.frame_duration_micros),
        };
        self.frame_index = self.frame_index.saturating_add(1);
        Ok(encoded)
    }
}

struct JpegFrameEncoder {
    frame_duration_micros: u64,
    frame_index: u64,
}

impl JpegFrameEncoder {
    fn new(frame_rate: u32) -> Self {
        Self {
            frame_duration_micros: (1_000_000u64 / frame_rate.max(1) as u64).max(1),
            frame_index: 0,
        }
    }

    fn encode_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<EncodedVideoFrame, String> {
        let bytes = super::source::encode_rgba_frame_jpeg(width, height, rgba.to_vec(), width, height, 78)?;
        let encoded = EncodedVideoFrame {
            bytes,
            keyframe: true,
            timestamp_micros: self.frame_index.saturating_mul(self.frame_duration_micros),
        };
        self.frame_index = self.frame_index.saturating_add(1);
        Ok(encoded)
    }
}

#[cfg(target_os = "windows")]
fn probe_windows_h264_hardware_encoder() -> Option<String> {
    enumerate_windows_h264_hardware_encoders().ok()?.into_iter().next()
}

#[cfg(not(target_os = "windows"))]
fn probe_windows_h264_hardware_encoder() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
fn enumerate_windows_h264_hardware_encoders() -> Result<Vec<String>, String> {
    let runtime = MediaFoundationRuntime::start()?;
    let _keep_runtime_alive = runtime;
    let mut activates = null_mut();
    let mut count = 0u32;
    let output_type = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };

    unsafe {
        let flags: MFT_ENUM_FLAG = MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            None,
            Some(&output_type),
            &mut activates,
            &mut count,
        )
        .map_err(|err| format!("MFTEnumEx failed: {err}"))?;

        let mut names = Vec::new();
        if !activates.is_null() && count > 0 {
            let slice = std::slice::from_raw_parts_mut(activates, count as usize);
            for activate in slice.iter_mut() {
                if let Some(activate) = activate.as_ref() {
                    if let Some(name) = get_activation_name(activate) {
                        names.push(name);
                    }
                }
            }
            CoTaskMemFree(Some(activates.cast()));
        }
        Ok(names)
    }
}

#[cfg(target_os = "windows")]
fn activate_hardware_h264_encoder() -> Result<(IMFTransform, String), String> {
    let mut activates = null_mut();
    let mut count = 0u32;
    let output_type = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };

    unsafe {
        let flags: MFT_ENUM_FLAG = MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            None,
            Some(&output_type),
            &mut activates,
            &mut count,
        )
        .map_err(|err| format!("MFTEnumEx failed: {err}"))?;

        if activates.is_null() || count == 0 {
            return Err("Windows hardware H.264 encoder was not found".to_owned());
        }

        let slice = std::slice::from_raw_parts_mut(activates, count as usize);
        let mut activated = None;
        for activate in slice.iter_mut() {
            if let Some(activate) = activate.take() {
                let name = get_activation_name(&activate).unwrap_or_else(|| "Windows H.264 Encoder".to_owned());
                match activate.ActivateObject::<IMFTransform>() {
                    Ok(transform) => {
                        activated = Some((transform, name));
                        break;
                    }
                    Err(err) => {
                        log::warn!("Failed to activate Windows hardware H.264 encoder '{name}': {err}");
                    }
                }
            }
        }

        CoTaskMemFree(Some(activates.cast()));
        activated.ok_or_else(|| "Windows hardware H.264 encoder could not be activated".to_owned())
    }
}

#[cfg(target_os = "windows")]
fn get_activation_name(activate: &IMFActivate) -> Option<String> {
    unsafe {
        let mut name_ptr = PWSTR::null();
        let mut length = 0u32;
        let result = activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut name_ptr, &mut length);
        if result.is_err() {
            return None;
        }
        let name = name_ptr
            .to_string()
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        CoTaskMemFree(Some(name_ptr.0.cast()));
        name
    }
}

#[cfg(target_os = "windows")]
fn create_output_media_type(width: u32, height: u32, frame_rate: u32) -> Result<IMFMediaType, String> {
    unsafe {
        let media_type = MFCreateMediaType().map_err(|err| err.to_string())?;
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|err| err.to_string())?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(|err| err.to_string())?;
        media_type
            .SetUINT32(&MF_MT_AVG_BITRATE, compute_target_bitrate_bps(width, height, frame_rate))
            .map_err(|err| err.to_string())?;
        media_type
            .SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(width, height))
            .map_err(|err| err.to_string())?;
        media_type
            .SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(frame_rate.max(1), 1))
            .map_err(|err| err.to_string())?;
        media_type
            .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))
            .map_err(|err| err.to_string())?;
        media_type
            .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(|err| err.to_string())?;
        media_type
            .SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
            .map_err(|err| err.to_string())?;
        media_type
            .SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 0)
            .map_err(|err| err.to_string())?;
        Ok(media_type)
    }
}

#[cfg(target_os = "windows")]
fn configure_input_media_type(
    transform: &IMFTransform,
    input_stream_id: u32,
    width: u32,
    height: u32,
    frame_rate: u32,
) -> Result<MfInputFormat, String> {
    for format in [MfInputFormat::Nv12, MfInputFormat::I420] {
        let input_type = unsafe { MFCreateMediaType() }.map_err(|err| err.to_string())?;
        unsafe {
            input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|err| err.to_string())?;
            input_type.SetGUID(&MF_MT_SUBTYPE, &format.guid()).map_err(|err| err.to_string())?;
            input_type
                .SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(width, height))
                .map_err(|err| err.to_string())?;
            input_type
                .SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(frame_rate.max(1), 1))
                .map_err(|err| err.to_string())?;
            input_type
                .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))
                .map_err(|err| err.to_string())?;
            input_type
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(|err| err.to_string())?;
            input_type
                .SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)
                .map_err(|err| err.to_string())?;
            if transform.SetInputType(input_stream_id, &input_type, 0).is_ok() {
                return Ok(format);
            }
        }
    }

    Err("Windows hardware H.264 encoder did not accept NV12 or I420 input".to_owned())
}

#[cfg(target_os = "windows")]
fn create_input_sample(
    yuv: &YUVBuffer,
    format: MfInputFormat,
    sample_time_hns: i64,
    sample_duration_hns: i64,
) -> Result<IMFSample, String> {
    let bytes = match format {
        MfInputFormat::Nv12 => pack_yuvbuffer_as_nv12(yuv),
        MfInputFormat::I420 => pack_yuvbuffer_as_i420(yuv),
    };

    unsafe {
        let buffer = MFCreateMemoryBuffer(bytes.len() as u32).map_err(|err| err.to_string())?;
        copy_bytes_into_media_buffer(&buffer, &bytes)?;
        let sample = MFCreateSample().map_err(|err| err.to_string())?;
        sample.AddBuffer(&buffer).map_err(|err| err.to_string())?;
        sample.SetSampleTime(sample_time_hns).map_err(|err| err.to_string())?;
        sample
            .SetSampleDuration(sample_duration_hns)
            .map_err(|err| err.to_string())?;
        Ok(sample)
    }
}

#[cfg(target_os = "windows")]
fn create_output_sample(buffer_size: u32) -> Result<IMFSample, String> {
    unsafe {
        let sample = MFCreateSample().map_err(|err| err.to_string())?;
        let buffer = MFCreateMemoryBuffer(buffer_size.max(1)).map_err(|err| err.to_string())?;
        sample.AddBuffer(&buffer).map_err(|err| err.to_string())?;
        Ok(sample)
    }
}

#[cfg(target_os = "windows")]
fn copy_bytes_into_media_buffer(buffer: &IMFMediaBuffer, bytes: &[u8]) -> Result<(), String> {
    unsafe {
        let mut raw_ptr = null_mut();
        let mut max_len = 0u32;
        let mut current_len = 0u32;
        buffer
            .Lock(&mut raw_ptr, Some(&mut max_len), Some(&mut current_len))
            .map_err(|err| err.to_string())?;
        if max_len < bytes.len() as u32 || raw_ptr.is_null() {
            let _ = buffer.Unlock();
            return Err("Media Foundation buffer was smaller than the input frame".to_owned());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), raw_ptr, bytes.len());
        buffer
            .SetCurrentLength(bytes.len() as u32)
            .map_err(|err| err.to_string())?;
        buffer.Unlock().map_err(|err| err.to_string())?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn read_sample_bytes(sample: &IMFSample) -> Result<Vec<u8>, String> {
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().map_err(|err| err.to_string())?;
        let mut raw_ptr = null_mut();
        let mut max_len = 0u32;
        let mut current_len = 0u32;
        buffer
            .Lock(&mut raw_ptr, Some(&mut max_len), Some(&mut current_len))
            .map_err(|err| err.to_string())?;
        if raw_ptr.is_null() {
            let _ = buffer.Unlock();
            return Err("Media Foundation output buffer was null".to_owned());
        }
        let bytes = std::slice::from_raw_parts(raw_ptr, current_len as usize).to_vec();
        buffer.Unlock().map_err(|err| err.to_string())?;
        Ok(bytes)
    }
}

#[cfg(target_os = "windows")]
fn read_sequence_header_annexb(transform: &IMFTransform, output_stream_id: u32) -> Result<Vec<u8>, String> {
    unsafe {
        let output_type = transform
            .GetOutputCurrentType(output_stream_id)
            .map_err(|err| err.to_string())?;
        let mut blob_ptr = null_mut();
        let mut blob_size = 0u32;
        output_type
            .GetAllocatedBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut blob_ptr, &mut blob_size)
            .map_err(|err| err.to_string())?;
        let blob = std::slice::from_raw_parts(blob_ptr, blob_size as usize).to_vec();
        CoTaskMemFree(Some(blob_ptr.cast()));
        if blob.is_empty() {
            Ok(Vec::new())
        } else if looks_like_annexb(&blob) {
            Ok(blob)
        } else {
            avcc_header_to_annexb(&blob)
        }
    }
}

#[cfg(target_os = "windows")]
fn needs_caller_allocated_sample(flags: u32) -> bool {
    let provides_samples = flags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
    let can_provide_samples = flags & MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32 != 0;
    !(provides_samples || can_provide_samples)
}

#[cfg(target_os = "windows")]
fn pack_yuvbuffer_as_i420(yuv: &YUVBuffer) -> Vec<u8> {
    let (width, height) = yuv.dimensions();
    let (y_stride, u_stride, v_stride) = yuv.strides();
    let mut bytes = Vec::with_capacity(width * height * 3 / 2);
    copy_plane(&mut bytes, yuv.y(), width, height, y_stride);
    copy_plane(&mut bytes, yuv.u(), width / 2, height / 2, u_stride);
    copy_plane(&mut bytes, yuv.v(), width / 2, height / 2, v_stride);
    bytes
}

#[cfg(target_os = "windows")]
fn pack_yuvbuffer_as_nv12(yuv: &YUVBuffer) -> Vec<u8> {
    let (width, height) = yuv.dimensions();
    let (y_stride, u_stride, v_stride) = yuv.strides();
    let mut bytes = Vec::with_capacity(width * height * 3 / 2);
    copy_plane(&mut bytes, yuv.y(), width, height, y_stride);

    let chroma_width = width / 2;
    let chroma_height = height / 2;
    let u_plane = yuv.u();
    let v_plane = yuv.v();
    for row in 0..chroma_height {
        let u_offset = row * u_stride;
        let v_offset = row * v_stride;
        for column in 0..chroma_width {
            bytes.push(u_plane[u_offset + column]);
            bytes.push(v_plane[v_offset + column]);
        }
    }

    bytes
}

#[cfg(target_os = "windows")]
fn copy_plane(target: &mut Vec<u8>, plane: &[u8], width: usize, height: usize, stride: usize) {
    for row in 0..height {
        let offset = row * stride;
        target.extend_from_slice(&plane[offset..offset + width]);
    }
}

#[cfg(target_os = "windows")]
fn avcc_header_to_annexb(blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < 7 {
        return Err("Media Foundation sequence header was too short".to_owned());
    }

    let mut cursor = 5usize;
    let num_sps = (blob[cursor] & 0x1f) as usize;
    cursor += 1;
    let mut annexb = Vec::new();

    for _ in 0..num_sps {
        if cursor + 2 > blob.len() {
            return Err("Media Foundation sequence header was truncated".to_owned());
        }
        let length = u16::from_be_bytes([blob[cursor], blob[cursor + 1]]) as usize;
        cursor += 2;
        if cursor + length > blob.len() {
            return Err("Media Foundation SPS blob was truncated".to_owned());
        }
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&blob[cursor..cursor + length]);
        cursor += length;
    }

    if cursor >= blob.len() {
        return Ok(annexb);
    }

    let num_pps = blob[cursor] as usize;
    cursor += 1;
    for _ in 0..num_pps {
        if cursor + 2 > blob.len() {
            return Err("Media Foundation PPS header was truncated".to_owned());
        }
        let length = u16::from_be_bytes([blob[cursor], blob[cursor + 1]]) as usize;
        cursor += 2;
        if cursor + length > blob.len() {
            return Err("Media Foundation PPS blob was truncated".to_owned());
        }
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&blob[cursor..cursor + length]);
        cursor += length;
    }

    Ok(annexb)
}

#[cfg(target_os = "windows")]
fn avcc_payload_to_annexb(sample: &[u8], _sequence_header_annexb: &[u8]) -> Result<Vec<u8>, String> {
    let length_size = 4usize;
    let mut cursor = 0usize;
    let mut annexb = Vec::with_capacity(sample.len().saturating_add(64));

    while cursor + length_size <= sample.len() {
        let length = u32::from_be_bytes([
            sample[cursor],
            sample[cursor + 1],
            sample[cursor + 2],
            sample[cursor + 3],
        ]) as usize;
        cursor += length_size;
        if length == 0 {
            continue;
        }
        if cursor + length > sample.len() {
            return Err("Media Foundation AVC payload was truncated".to_owned());
        }
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&sample[cursor..cursor + length]);
        cursor += length;
    }

    if annexb.is_empty() {
        return Err("Media Foundation AVC payload did not contain any NAL units".to_owned());
    }

    Ok(annexb)
}

#[cfg(target_os = "windows")]
fn looks_like_annexb(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0, 0, 0, 1]) || bytes.starts_with(&[0, 0, 1])
}

#[cfg(target_os = "windows")]
fn annexb_starts_with_header(bytes: &[u8], header: &[u8]) -> bool {
    !header.is_empty() && bytes.starts_with(header)
}

#[cfg(target_os = "windows")]
fn pack_u32_pair(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn compute_target_bitrate_bps(width: u32, height: u32, frame_rate: u32) -> u32 {
    let pixels_per_frame = width.saturating_mul(height).max(1);
    let motion_factor = if frame_rate >= 60 { 2 } else { 1 };

    match pixels_per_frame {
        0..=921_600 => 4_000_000u32.saturating_mul(motion_factor),
        921_601..=2_073_600 => {
            if frame_rate >= 60 {
                10_000_000
            } else {
                6_000_000
            }
        }
        2_073_601..=3_686_400 => {
            if frame_rate >= 60 {
                18_000_000
            } else {
                12_000_000
            }
        }
        _ => {
            if frame_rate >= 60 {
                35_000_000
            } else {
                24_000_000
            }
        }
    }
}

fn normalize_even_dimension(value: u32) -> u32 {
    let adjusted = value.max(2);
    if adjusted % 2 == 0 {
        adjusted
    } else {
        adjusted.saturating_sub(1).max(2)
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_target_bitrate_bps, normalize_even_dimension, VideoCodec};

    #[test]
    fn keeps_video_dimensions_even() {
        assert_eq!(normalize_even_dimension(1), 2);
        assert_eq!(normalize_even_dimension(721), 720);
        assert_eq!(normalize_even_dimension(1440), 1440);
    }

    #[test]
    fn chooses_expected_bitrate_tiers() {
        assert_eq!(compute_target_bitrate_bps(1280, 720, 30), 4_000_000);
        assert_eq!(compute_target_bitrate_bps(1920, 1080, 60), 10_000_000);
        assert_eq!(compute_target_bitrate_bps(2560, 1440, 60), 18_000_000);
    }

    #[test]
    fn exposes_expected_codec_label() {
        assert_eq!(VideoCodec::H264AnnexB.label(), "avc1.42E034");
        assert_eq!(VideoCodec::Jpeg.label(), "jpeg");
    }
}
