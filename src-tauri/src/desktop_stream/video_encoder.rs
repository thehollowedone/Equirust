use super::capture_sources::{CaptureFrameMode, PreparedFramePixelFormat, PreparedVideoFrame};
#[cfg(target_os = "windows")]
use super::d3d11_device::SharedD3D11Device;
use openh264::{
    encoder::{Encoder as OpenH264Encoder, EncoderConfig, FrameType, RateControlMode},
    formats::{RgbaSliceU8, YUVBuffer},
    OpenH264API, Timestamp,
};

#[cfg(target_os = "windows")]
use std::{mem::ManuallyDrop, ptr::null_mut, sync::Arc};

#[cfg(target_os = "windows")]
use windows::{
    core::{Interface, PWSTR},
    Win32::{
        Foundation::{HMODULE, RECT, RPC_E_CHANGED_MODE},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
                ID3D11VideoContext, ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor,
                ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorEnumerator1,
                ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                D3D11_SDK_VERSION, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_DEFAULT, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0,
                D3D11_VIDEO_COLOR_RGBA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
                D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
                D3D11_VPOV_DIMENSION_TEXTURE2D,
            },
            Dxgi::Common::{
                DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_FORMAT_NV12, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
            },
        },
        Media::MediaFoundation::{
            eAVEncH264VProfile_High, CODECAPI_AVEncVideoForceKeyFrame, ICodecAPI, IMFActivate,
            IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaType, IMFSample, IMFTransform,
            MFCreateDXGIDeviceManager, MFCreateDXGISurfaceBuffer, MFCreateMediaType,
            MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSampleExtension_CleanPoint,
            MFShutdown, MFStartup, MFTEnumEx, MFT_FRIENDLY_NAME_Attribute, MFVideoFormat_H264,
            MFVideoFormat_I420, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
            MFSTARTUP_NOSOCKET, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE,
            MFT_ENUM_FLAG_SORTANDFILTER, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER,
            MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_INFO,
            MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MF_E_NOTACCEPTING,
            MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
            MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
            MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE,
            MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_SA_D3D11_AWARE,
            MF_VERSION,
        },
        System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED},
        System::Variant::{VARIANT, VT_UI4},
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
    #[allow(dead_code)]
    pub fn new(width: u32, height: u32, frame_rate: u32) -> Self {
        #[cfg(target_os = "windows")]
        {
            return Self::new_with_shared_device(width, height, frame_rate, None);
        }

        #[cfg(not(target_os = "windows"))]
        {
            Self::new_with_shared_device(width, height, frame_rate)
        }
    }

    #[cfg(target_os = "windows")]
    pub fn new_with_shared_device(
        width: u32,
        height: u32,
        frame_rate: u32,
        shared_device: Option<Arc<SharedD3D11Device>>,
    ) -> Self {
        #[cfg(target_os = "windows")]
        {
            if let Ok(encoder) = MediaFoundationH264Encoder::new(
                width,
                height,
                frame_rate,
                VideoEncoderPreference::HardwareFirst,
                shared_device.clone(),
            ) {
                let descriptor = EncoderDescriptor {
                    mode_label: "Windows Hardware H.264".to_owned(),
                    detail_label: Some(format!(
                        "{} ({})",
                        encoder.friendly_name(),
                        encoder.input_path_label()
                    )),
                    color_mode_label: encoder.color_pipeline_label().to_owned(),
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

    #[cfg(not(target_os = "windows"))]
    fn new_with_shared_device(width: u32, height: u32, frame_rate: u32) -> Self {
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

    pub fn capture_frame_mode(&self) -> CaptureFrameMode {
        match &self.inner {
            #[cfg(target_os = "windows")]
            VideoStreamEncoderInner::MediaFoundation(encoder) => encoder.capture_frame_mode(),
            VideoStreamEncoderInner::H264(_) | VideoStreamEncoderInner::Jpeg(_) => {
                CaptureFrameMode::TargetRgbaFrame
            }
        }
    }

    pub fn encode_frame(
        &mut self,
        frame: &PreparedVideoFrame,
    ) -> Result<EncodedVideoFrame, String> {
        match &mut self.inner {
            #[cfg(target_os = "windows")]
            VideoStreamEncoderInner::MediaFoundation(encoder) => encoder.encode_frame(frame),
            VideoStreamEncoderInner::H264(encoder) => encoder.encode_frame(frame),
            VideoStreamEncoderInner::Jpeg(encoder) => encoder.encode_frame(frame),
        }
    }

    pub fn request_keyframe(&mut self) -> Result<(), String> {
        match &mut self.inner {
            #[cfg(target_os = "windows")]
            VideoStreamEncoderInner::MediaFoundation(encoder) => encoder.request_keyframe(),
            VideoStreamEncoderInner::H264(encoder) => {
                encoder.request_keyframe();
                Ok(())
            }
            // Every JPEG frame is independently decodable already.
            VideoStreamEncoderInner::Jpeg(_) => Ok(()),
        }
    }
}

pub fn describe_preferred_encoder(
    width: u32,
    height: u32,
    frame_rate: u32,
) -> (VideoCodec, EncoderDescriptor) {
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

    if H264FrameEncoder::new(
        width,
        height,
        frame_rate,
        VideoEncoderPreference::HardwareFirst,
    )
    .is_ok()
    {
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
enum MediaFoundationInputPath {
    Cpu(MfInputFormat),
    Gpu(MfGpuUploadPipeline),
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuProcessorInputFormat {
    Bgra8,
    HdrScRgb16Float,
}

#[cfg(target_os = "windows")]
impl GpuProcessorInputFormat {
    fn input_color_space(self) -> windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_TYPE {
        match self {
            GpuProcessorInputFormat::Bgra8 => DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            GpuProcessorInputFormat::HdrScRgb16Float => DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
        }
    }
}

#[cfg(target_os = "windows")]
struct MfGpuUploadPipeline {
    shared_device: Option<Arc<SharedD3D11Device>>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    _device_manager: IMFDXGIDeviceManager,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    video_context1: Option<ID3D11VideoContext1>,
    target_width: u32,
    target_height: u32,
    frame_rate: u32,
    supports_hdr_tonemap: bool,
    output_texture: ID3D11Texture2D,
    processor_resources: Option<GpuProcessorResources>,
}

#[cfg(target_os = "windows")]
struct GpuProcessorResources {
    source_width: u32,
    source_height: u32,
    input_format: GpuProcessorInputFormat,
    _processor_enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    input_texture: Option<ID3D11Texture2D>,
    input_view: Option<ID3D11VideoProcessorInputView>,
    output_view: ID3D11VideoProcessorOutputView,
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
    input_path: MediaFoundationInputPath,
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
        shared_device: Option<Arc<SharedD3D11Device>>,
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

        let gpu_pipeline =
            try_create_gpu_upload_pipeline(&transform, width, height, frame_rate, shared_device)
                .inspect_err(|err| {
                    log::debug!("Windows hardware encoder GPU upload path unavailable: {err}");
                })
                .ok();
        let input_format =
            configure_input_media_type(&transform, input_stream_id, width, height, frame_rate)?;
        let input_path = match (gpu_pipeline, input_format) {
            (Some(pipeline), MfInputFormat::Nv12) => MediaFoundationInputPath::Gpu(pipeline),
            (_, format) => MediaFoundationInputPath::Cpu(format),
        };
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
            input_path,
            frame_duration_hns: ((1_000_000u64 / frame_rate.max(1) as u64).max(1) * 10) as i64,
            frame_index: 0,
            sequence_header_annexb,
            friendly_name,
        })
    }

    fn friendly_name(&self) -> &str {
        &self.friendly_name
    }

    fn input_path_label(&self) -> &'static str {
        match &self.input_path {
            MediaFoundationInputPath::Cpu(_) => "CPU input",
            MediaFoundationInputPath::Gpu(pipeline) => {
                if pipeline.uses_shared_capture_textures() {
                    "Shared GPU direct"
                } else {
                    "GPU upload"
                }
            }
        }
    }

    fn color_pipeline_label(&self) -> &'static str {
        match &self.input_path {
            MediaFoundationInputPath::Gpu(pipeline) if pipeline.supports_hdr_tonemap() => {
                "HDR->SDR GPU tonemap"
            }
            _ => "SDR-safe",
        }
    }

    fn capture_frame_mode(&self) -> CaptureFrameMode {
        match &self.input_path {
            MediaFoundationInputPath::Cpu(_) => CaptureFrameMode::TargetRgbaFrame,
            MediaFoundationInputPath::Gpu(pipeline) => {
                if pipeline.uses_shared_capture_textures() {
                    if pipeline.supports_hdr_tonemap() {
                        CaptureFrameMode::SourceHdrTextureFrame
                    } else {
                        CaptureFrameMode::SourceBgraTextureFrame
                    }
                } else {
                    CaptureFrameMode::SourceBgraFrame
                }
            }
        }
    }

    fn encode_frame(&mut self, frame: &PreparedVideoFrame) -> Result<EncodedVideoFrame, String> {
        let sample_time_hns = self.frame_index as i64 * self.frame_duration_hns;
        let input_sample = match &mut self.input_path {
            MediaFoundationInputPath::Cpu(input_format) => create_cpu_input_sample(
                frame,
                *input_format,
                sample_time_hns,
                self.frame_duration_hns,
            )?,
            MediaFoundationInputPath::Gpu(pipeline) => {
                pipeline.create_input_sample(frame, sample_time_hns, self.frame_duration_hns)?
            }
        };

        self.process_input_sample(&input_sample)?;
        let (bytes, keyframe) = self.collect_output(true)?;
        let encoded = EncodedVideoFrame {
            bytes,
            keyframe,
            timestamp_micros: (sample_time_hns / 10) as u64,
        };
        self.frame_index = self.frame_index.saturating_add(1);
        Ok(encoded)
    }

    fn process_input_sample(&mut self, input_sample: &IMFSample) -> Result<(), String> {
        unsafe {
            match self
                .transform
                .ProcessInput(self.input_stream_id, input_sample, 0)
            {
                Ok(()) => Ok(()),
                Err(err) if err.code() == MF_E_NOTACCEPTING => {
                    let _ = self.collect_output(false)?;
                    self.transform
                        .ProcessInput(self.input_stream_id, input_sample, 0)
                        .map_err(|inner| {
                            format!("Windows hardware encoder ProcessInput failed: {inner}")
                        })
                }
                Err(err) => Err(format!(
                    "Windows hardware encoder ProcessInput failed: {err}"
                )),
            }
        }
    }

    fn request_keyframe(&mut self) -> Result<(), String> {
        request_media_foundation_keyframe(&self.transform)
    }

    fn collect_output(&mut self, require_bytes: bool) -> Result<(Vec<u8>, bool), String> {
        let mut bytes = Vec::new();
        let mut keyframe = false;

        loop {
            let sample = if needs_caller_allocated_sample(self.output_stream_info.dwFlags) {
                Some(create_output_sample(
                    self.output_stream_info.cbSize.max(1_048_576),
                )?)
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

            let process_result = unsafe {
                self.transform
                    .ProcessOutput(0, &mut output_buffers, &mut status)
            };
            match process_result {
                Ok(()) => {
                    let sample = unsafe { ManuallyDrop::take(&mut output_buffers[0].pSample) };
                    let events = unsafe { ManuallyDrop::take(&mut output_buffers[0].pEvents) };
                    drop(events);

                    if let Some(sample) = sample {
                        let sample_keyframe = unsafe {
                            sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0
                        };
                        let sample_bytes = read_sample_bytes(&sample)?;
                        let annexb =
                            self.convert_output_to_annexb(&sample_bytes, sample_keyframe)?;
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
                    return Err(format!(
                        "Windows hardware encoder ProcessOutput failed: {err}"
                    ));
                }
            }
        }

        if require_bytes && bytes.is_empty() {
            return Err("Windows hardware encoder did not produce an output frame".to_owned());
        }

        Ok((bytes, keyframe))
    }

    fn convert_output_to_annexb(
        &self,
        sample_bytes: &[u8],
        keyframe: bool,
    ) -> Result<Vec<u8>, String> {
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

#[cfg(target_os = "windows")]
fn create_cpu_input_sample(
    frame: &PreparedVideoFrame,
    input_format: MfInputFormat,
    sample_time_hns: i64,
    sample_duration_hns: i64,
) -> Result<IMFSample, String> {
    let even_width = normalize_even_dimension(frame.width);
    let even_height = normalize_even_dimension(frame.height);
    let expected_len = (even_width as usize)
        .saturating_mul(even_height as usize)
        .saturating_mul(4);

    let rgba_storage;
    let rgba = match frame.pixel_format {
        PreparedFramePixelFormat::Rgba => frame.pixels.as_slice(),
        PreparedFramePixelFormat::Bgra => {
            rgba_storage = bgra_to_rgba_copy(&frame.pixels);
            rgba_storage.as_slice()
        }
        PreparedFramePixelFormat::Rgba16FloatTexture => {
            return Err(
                "Windows hardware encoder CPU path does not accept HDR GPU textures".to_owned(),
            )
        }
    };
    if rgba.len() != expected_len {
        return Err("Windows hardware encoder received an invalid RGBA frame".to_owned());
    }

    let yuv_bytes = rgba_to_yuv_direct(
        rgba,
        even_width as usize,
        even_height as usize,
        input_format,
    );
    create_input_sample_from_bytes(&yuv_bytes, sample_time_hns, sample_duration_hns)
}

#[cfg(target_os = "windows")]
fn bgra_to_rgba_copy(bytes: &[u8]) -> Vec<u8> {
    let mut rgba = bytes.to_vec();
    for chunk in rgba.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
    rgba
}

#[cfg(target_os = "windows")]
fn try_create_gpu_upload_pipeline(
    transform: &IMFTransform,
    target_width: u32,
    target_height: u32,
    frame_rate: u32,
    shared_device: Option<Arc<SharedD3D11Device>>,
) -> Result<MfGpuUploadPipeline, String> {
    if !transform_supports_d3d11(transform)? {
        return Err("Encoder did not advertise D3D11-aware input".to_owned());
    }

    let (device, context) = match shared_device.as_ref() {
        Some(device) => (device.device().clone(), device.context().clone()),
        None => create_gpu_encoder_device()?,
    };
    let mut reset_token = 0u32;
    let mut device_manager = None;
    unsafe {
        MFCreateDXGIDeviceManager(&mut reset_token, &mut device_manager)
            .map_err(|err| format!("MFCreateDXGIDeviceManager failed: {err}"))?;
    }
    let device_manager = device_manager
        .ok_or_else(|| "MFCreateDXGIDeviceManager returned no device manager".to_owned())?;
    unsafe {
        device_manager
            .ResetDevice(&device, reset_token)
            .map_err(|err| format!("IMFDXGIDeviceManager::ResetDevice failed: {err}"))?;
        transform
            .ProcessMessage(
                MFT_MESSAGE_SET_D3D_MANAGER,
                Interface::as_raw(&device_manager) as usize,
            )
            .map_err(|err| format!("MFT_MESSAGE_SET_D3D_MANAGER failed: {err}"))?;
    }

    MfGpuUploadPipeline::new(
        shared_device,
        device,
        context,
        device_manager,
        target_width,
        target_height,
        frame_rate,
    )
}

#[cfg(target_os = "windows")]
fn transform_supports_d3d11(transform: &IMFTransform) -> Result<bool, String> {
    let attributes = unsafe { transform.GetAttributes() }
        .map_err(|err| format!("IMFTransform::GetAttributes failed: {err}"))?;
    let aware = unsafe { attributes.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) };
    Ok(aware != 0)
}

#[cfg(target_os = "windows")]
fn create_gpu_encoder_device() -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    let mut device = None;
    let mut context = None;
    let mut feature_level = D3D_FEATURE_LEVEL::default();
    let levels = [D3D_FEATURE_LEVEL_11_0];
    let flags = windows::Win32::Graphics::Direct3D11::D3D11_CREATE_DEVICE_FLAG(
        D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 | D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0,
    );

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut feature_level),
            Some(&mut context),
        )
        .map_err(|err| format!("D3D11CreateDevice (encoder) failed: {err}"))?;
    }

    Ok((
        device.ok_or_else(|| "D3D11 encoder device was not created".to_owned())?,
        context.ok_or_else(|| "D3D11 encoder context was not created".to_owned())?,
    ))
}

#[cfg(target_os = "windows")]
impl MfGpuUploadPipeline {
    fn new(
        shared_device: Option<Arc<SharedD3D11Device>>,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        device_manager: IMFDXGIDeviceManager,
        target_width: u32,
        target_height: u32,
        frame_rate: u32,
    ) -> Result<Self, String> {
        let video_device: ID3D11VideoDevice = device
            .cast()
            .map_err(|err| format!("D3D11 encoder video device cast failed: {err}"))?;
        let video_context: ID3D11VideoContext = context
            .cast()
            .map_err(|err| format!("D3D11 encoder video context cast failed: {err}"))?;
        let video_context1 = video_context.cast().ok();
        let supports_hdr_tonemap = if shared_device.is_some() {
            probe_hdr_tonemap_support(
                &video_device,
                target_width,
                target_height,
                frame_rate,
                video_context1.is_some(),
            )
            .unwrap_or(false)
        } else {
            false
        };
        let output_texture = create_nv12_output_texture(&device, target_width, target_height)?;

        Ok(Self {
            shared_device,
            device,
            context,
            _device_manager: device_manager,
            video_device,
            video_context,
            video_context1,
            target_width,
            target_height,
            frame_rate,
            supports_hdr_tonemap,
            output_texture,
            processor_resources: None,
        })
    }

    fn uses_shared_capture_textures(&self) -> bool {
        self.shared_device.is_some()
    }

    fn supports_hdr_tonemap(&self) -> bool {
        self.supports_hdr_tonemap
    }

    fn create_input_sample(
        &mut self,
        frame: &PreparedVideoFrame,
        sample_time_hns: i64,
        sample_duration_hns: i64,
    ) -> Result<IMFSample, String> {
        let input_format = match frame.pixel_format {
            PreparedFramePixelFormat::Bgra => GpuProcessorInputFormat::Bgra8,
            PreparedFramePixelFormat::Rgba16FloatTexture => {
                GpuProcessorInputFormat::HdrScRgb16Float
            }
            PreparedFramePixelFormat::Rgba => {
                return Err("Windows GPU upload path expected a BGRA or HDR GPU frame".to_owned())
            }
        };
        self.ensure_processor_resources(frame.width, frame.height, input_format)?;
        let resources = self
            .processor_resources
            .as_ref()
            .ok_or_else(|| "Windows GPU upload path was missing processor resources".to_owned())?;
        let input_view = if let Some(texture) = frame.gpu_texture.as_ref().filter(|_| {
            matches!(
                frame.pixel_format,
                PreparedFramePixelFormat::Bgra | PreparedFramePixelFormat::Rgba16FloatTexture
            )
        }) {
            create_video_processor_input_view(
                &self.video_device,
                texture,
                &resources._processor_enumerator,
            )?
        } else {
            if input_format != GpuProcessorInputFormat::Bgra8 {
                return Err(
                    "Windows GPU upload path expected an HDR GPU texture or BGRA frame".to_owned(),
                );
            }
            let expected_len = (frame.width as usize)
                .saturating_mul(frame.height as usize)
                .saturating_mul(4);
            if frame.pixels.len() != expected_len {
                return Err("Windows GPU upload path received an invalid BGRA frame".to_owned());
            }

            let input_texture = resources.input_texture.as_ref().ok_or_else(|| {
                "Windows GPU upload path was missing a BGRA upload texture".to_owned()
            })?;
            let _lock = self.shared_device.as_ref().map(|device| device.lock());
            unsafe {
                self.context.UpdateSubresource(
                    input_texture,
                    0,
                    None,
                    frame.pixels.as_ptr().cast(),
                    frame.width.saturating_mul(4),
                    0,
                );
            }
            resources
                .input_view
                .as_ref()
                .cloned()
                .ok_or_else(|| "Windows GPU upload path was missing a BGRA input view".to_owned())?
        };

        unsafe {
            let source_rect = RECT {
                left: 0,
                top: 0,
                right: frame.width as i32,
                bottom: frame.height as i32,
            };
            let dest_rect = compute_letterbox_rect(
                frame.width,
                frame.height,
                self.target_width,
                self.target_height,
            );
            let background = black_video_color();

            self.video_context.VideoProcessorSetStreamFrameFormat(
                &resources.processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            self.video_context.VideoProcessorSetOutputBackgroundColor(
                &resources.processor,
                false,
                &background,
            );
            self.video_context.VideoProcessorSetStreamSourceRect(
                &resources.processor,
                0,
                true,
                Some(&source_rect),
            );
            self.video_context.VideoProcessorSetOutputTargetRect(
                &resources.processor,
                true,
                Some(&dest_rect),
            );
            configure_video_processor_color_spaces(
                self.video_context1.as_ref(),
                &resources.processor,
                input_format,
            );

            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM::default();
            stream.Enable = true.into();
            stream.OutputIndex = 0;
            stream.InputFrameOrField = 0;
            stream.PastFrames = 0;
            stream.FutureFrames = 0;
            stream.pInputSurface = ManuallyDrop::new(Some(input_view));

            let _lock = self.shared_device.as_ref().map(|device| device.lock());
            self.video_context
                .VideoProcessorBlt(&resources.processor, &resources.output_view, 0, &[stream])
                .map_err(|err| format!("D3D11 video processor blit failed: {err}"))?;
        }

        create_input_sample_from_texture(&self.output_texture, sample_time_hns, sample_duration_hns)
    }

    fn ensure_processor_resources(
        &mut self,
        source_width: u32,
        source_height: u32,
        input_format: GpuProcessorInputFormat,
    ) -> Result<(), String> {
        let needs_refresh = self
            .processor_resources
            .as_ref()
            .map(|resources| {
                resources.source_width != source_width
                    || resources.source_height != source_height
                    || resources.input_format != input_format
            })
            .unwrap_or(true);
        if !needs_refresh {
            return Ok(());
        }

        self.processor_resources = Some(create_gpu_processor_resources(
            &self.device,
            &self.video_device,
            &self.output_texture,
            source_width,
            source_height,
            self.target_width,
            self.target_height,
            self.frame_rate,
            input_format,
        )?);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn create_nv12_output_texture(
    device: &ID3D11Device,
    target_width: u32,
    target_height: u32,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: normalize_even_dimension(target_width),
        Height: normalize_even_dimension(target_height),
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe {
        device
            .CreateTexture2D(&desc, None, Some(&mut texture))
            .map_err(|err| format!("CreateTexture2D (NV12 output) failed: {err}"))?;
    }
    texture.ok_or_else(|| "NV12 output texture was not created".to_owned())
}

#[cfg(target_os = "windows")]
fn create_gpu_processor_resources(
    device: &ID3D11Device,
    video_device: &ID3D11VideoDevice,
    output_texture: &ID3D11Texture2D,
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
    frame_rate: u32,
    input_format: GpuProcessorInputFormat,
) -> Result<GpuProcessorResources, String> {
    let input_texture = if input_format == GpuProcessorInputFormat::Bgra8 {
        let input_desc = D3D11_TEXTURE2D_DESC {
            Width: source_width.max(1),
            Height: source_height.max(1),
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut input_texture = None;
        unsafe {
            device
                .CreateTexture2D(&input_desc, None, Some(&mut input_texture))
                .map_err(|err| format!("CreateTexture2D (BGRA upload) failed: {err}"))?;
        }
        let input_texture =
            input_texture.ok_or_else(|| "BGRA upload texture was not created".to_owned())?;
        Some(input_texture)
    } else {
        None
    };

    let content_desc = build_video_processor_content_desc(
        source_width,
        source_height,
        target_width,
        target_height,
        frame_rate,
    );
    let processor_enumerator = unsafe {
        video_device
            .CreateVideoProcessorEnumerator(&content_desc)
            .map_err(|err| format!("CreateVideoProcessorEnumerator failed: {err}"))?
    };
    let processor = unsafe {
        video_device
            .CreateVideoProcessor(&processor_enumerator, 0)
            .map_err(|err| format!("CreateVideoProcessor failed: {err}"))?
    };

    let input_view = match input_texture.as_ref() {
        Some(input_texture) => Some(create_video_processor_input_view(
            video_device,
            input_texture,
            &processor_enumerator,
        )?),
        None => None,
    };

    let output_view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
        ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
        },
    };
    let mut output_view = None;
    unsafe {
        video_device
            .CreateVideoProcessorOutputView(
                output_texture,
                &processor_enumerator,
                &output_view_desc,
                Some(&mut output_view),
            )
            .map_err(|err| format!("CreateVideoProcessorOutputView failed: {err}"))?;
    }
    let output_view =
        output_view.ok_or_else(|| "Video processor output view was not created".to_owned())?;

    Ok(GpuProcessorResources {
        source_width,
        source_height,
        input_format,
        _processor_enumerator: processor_enumerator,
        processor,
        input_texture,
        input_view,
        output_view,
    })
}

#[cfg(target_os = "windows")]
fn build_video_processor_content_desc(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
    frame_rate: u32,
) -> D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: DXGI_RATIONAL {
            Numerator: frame_rate.max(1),
            Denominator: 1,
        },
        InputWidth: source_width.max(1),
        InputHeight: source_height.max(1),
        OutputFrameRate: DXGI_RATIONAL {
            Numerator: frame_rate.max(1),
            Denominator: 1,
        },
        OutputWidth: normalize_even_dimension(target_width),
        OutputHeight: normalize_even_dimension(target_height),
        Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
    }
}

#[cfg(target_os = "windows")]
fn probe_hdr_tonemap_support(
    video_device: &ID3D11VideoDevice,
    target_width: u32,
    target_height: u32,
    frame_rate: u32,
    has_video_context1: bool,
) -> Result<bool, String> {
    if !has_video_context1 {
        return Ok(false);
    }

    let content_desc = build_video_processor_content_desc(
        target_width,
        target_height,
        target_width,
        target_height,
        frame_rate,
    );
    let processor_enumerator = unsafe {
        video_device
            .CreateVideoProcessorEnumerator(&content_desc)
            .map_err(|err| format!("CreateVideoProcessorEnumerator failed: {err}"))?
    };
    let enumerator1: ID3D11VideoProcessorEnumerator1 = match processor_enumerator.cast() {
        Ok(enumerator1) => enumerator1,
        Err(_) => return Ok(false),
    };

    let supported = unsafe {
        enumerator1
            .CheckVideoProcessorFormatConversion(
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
                DXGI_FORMAT_NV12,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            )
            .map_err(|err| format!("CheckVideoProcessorFormatConversion failed: {err}"))?
    };
    Ok(supported.as_bool())
}

#[cfg(target_os = "windows")]
fn configure_video_processor_color_spaces(
    video_context1: Option<&ID3D11VideoContext1>,
    processor: &ID3D11VideoProcessor,
    input_format: GpuProcessorInputFormat,
) {
    if let Some(video_context1) = video_context1 {
        unsafe {
            video_context1.VideoProcessorSetStreamColorSpace1(
                processor,
                0,
                input_format.input_color_space(),
            );
            video_context1.VideoProcessorSetOutputColorSpace1(
                processor,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
        }
    }
}

#[cfg(target_os = "windows")]
fn create_video_processor_input_view(
    video_device: &ID3D11VideoDevice,
    texture: &ID3D11Texture2D,
    processor_enumerator: &ID3D11VideoProcessorEnumerator,
) -> Result<ID3D11VideoProcessorInputView, String> {
    let input_view_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
        FourCC: 0,
        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_VPIV {
                MipSlice: 0,
                ArraySlice: 0,
            },
        },
    };
    let mut input_view = None;
    unsafe {
        video_device
            .CreateVideoProcessorInputView(
                texture,
                processor_enumerator,
                &input_view_desc,
                Some(&mut input_view),
            )
            .map_err(|err| format!("CreateVideoProcessorInputView failed: {err}"))?;
    }
    input_view.ok_or_else(|| "Video processor input view was not created".to_owned())
}

#[cfg(target_os = "windows")]
fn create_input_sample_from_texture(
    texture: &ID3D11Texture2D,
    sample_time_hns: i64,
    sample_duration_hns: i64,
) -> Result<IMFSample, String> {
    unsafe {
        let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, texture, 0, false)
            .map_err(|err| format!("MFCreateDXGISurfaceBuffer failed: {err}"))?;
        let sample = MFCreateSample().map_err(|err| err.to_string())?;
        sample.AddBuffer(&buffer).map_err(|err| err.to_string())?;
        sample
            .SetSampleTime(sample_time_hns)
            .map_err(|err| err.to_string())?;
        sample
            .SetSampleDuration(sample_duration_hns)
            .map_err(|err| err.to_string())?;
        Ok(sample)
    }
}

#[cfg(target_os = "windows")]
fn black_video_color() -> D3D11_VIDEO_COLOR {
    D3D11_VIDEO_COLOR {
        Anonymous: D3D11_VIDEO_COLOR_0 {
            RGBA: D3D11_VIDEO_COLOR_RGBA {
                R: 0.0,
                G: 0.0,
                B: 0.0,
                A: 1.0,
            },
        },
    }
}

#[cfg(target_os = "windows")]
fn compute_letterbox_rect(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> RECT {
    let width_scale = target_width as f32 / source_width.max(1) as f32;
    let height_scale = target_height as f32 / source_height.max(1) as f32;
    let scale = width_scale.min(height_scale).max(0.000_1);
    let scaled_width = ((source_width as f32 * scale).round() as i32).clamp(1, target_width as i32);
    let scaled_height =
        ((source_height as f32 * scale).round() as i32).clamp(1, target_height as i32);
    let left = ((target_width as i32 - scaled_width) / 2).max(0);
    let top = ((target_height as i32 - scaled_height) / 2).max(0);

    RECT {
        left,
        top,
        right: left + scaled_width,
        bottom: top + scaled_height,
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
            .enable_skip_frame(false)
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

    fn encode_frame(&mut self, frame: &PreparedVideoFrame) -> Result<EncodedVideoFrame, String> {
        if frame.pixel_format != PreparedFramePixelFormat::Rgba {
            return Err("Native H.264 encoder expected an RGBA frame".to_owned());
        }
        let width = frame.width;
        let height = frame.height;
        let even_width = normalize_even_dimension(width);
        let even_height = normalize_even_dimension(height);
        if frame.pixels.len() != (even_width as usize) * (even_height as usize) * 4 {
            return Err("Native H.264 encoder received an invalid RGBA frame".to_owned());
        }

        if self.frame_index != 0
            && self.intra_interval_frames > 0
            && self.frame_index % self.intra_interval_frames == 0
        {
            self.encoder.force_intra_frame();
        }

        let rgba_source = RgbaSliceU8::new(
            frame.pixels.as_slice(),
            (even_width as usize, even_height as usize),
        );
        let yuv = YUVBuffer::from_rgb_source(rgba_source);
        let timestamp_millis = self
            .frame_index
            .saturating_mul(self.frame_duration_micros)
            .saturating_div(1_000);
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

    fn request_keyframe(&mut self) {
        self.encoder.force_intra_frame();
    }
}

#[cfg(target_os = "windows")]
fn request_media_foundation_keyframe(transform: &IMFTransform) -> Result<(), String> {
    let codec_api: ICodecAPI = transform
        .cast()
        .map_err(|err| format!("Windows hardware encoder does not expose ICodecAPI: {err}"))?;
    let mut value = VARIANT::default();
    unsafe {
        let inner = &mut *value.Anonymous.Anonymous;
        inner.vt = VT_UI4;
        inner.Anonymous.ulVal = 1;
        codec_api
            .SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &value)
            .map_err(|err| format!("Windows hardware encoder force keyframe failed: {err}"))?;
    }
    Ok(())
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

    fn encode_frame(&mut self, frame: &PreparedVideoFrame) -> Result<EncodedVideoFrame, String> {
        if frame.pixel_format != PreparedFramePixelFormat::Rgba {
            return Err("Native JPEG encoder expected an RGBA frame".to_owned());
        }
        let bytes = super::capture_sources::encode_rgba_frame_jpeg(
            frame.width,
            frame.height,
            frame.pixels.clone(),
            frame.width,
            frame.height,
            78,
        )?;
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
    enumerate_windows_h264_hardware_encoders()
        .ok()?
        .into_iter()
        .next()
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
                let name = get_activation_name(&activate)
                    .unwrap_or_else(|| "Windows H.264 Encoder".to_owned());
                match activate.ActivateObject::<IMFTransform>() {
                    Ok(transform) => {
                        activated = Some((transform, name));
                        break;
                    }
                    Err(err) => {
                        log::warn!(
                            "Failed to activate Windows hardware H.264 encoder '{name}': {err}"
                        );
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
        let result =
            activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut name_ptr, &mut length);
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
fn create_output_media_type(
    width: u32,
    height: u32,
    frame_rate: u32,
) -> Result<IMFMediaType, String> {
    unsafe {
        let media_type = MFCreateMediaType().map_err(|err| err.to_string())?;
        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|err| err.to_string())?;
        media_type
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
            .map_err(|err| err.to_string())?;
        media_type
            .SetUINT32(
                &MF_MT_AVG_BITRATE,
                compute_target_bitrate_bps(width, height, frame_rate),
            )
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
            input_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|err| err.to_string())?;
            input_type
                .SetGUID(&MF_MT_SUBTYPE, &format.guid())
                .map_err(|err| err.to_string())?;
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
            if transform
                .SetInputType(input_stream_id, &input_type, 0)
                .is_ok()
            {
                return Ok(format);
            }
        }
    }

    Err("Windows hardware H.264 encoder did not accept NV12 or I420 input".to_owned())
}

#[cfg(target_os = "windows")]
fn rgba_to_yuv_direct(rgba: &[u8], width: usize, height: usize, format: MfInputFormat) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 3 / 2);

    // Y plane — BT.601 limited range: Y = ((66R + 129G + 25B + 128) >> 8) + 16
    for row in 0..height {
        for col in 0..width {
            let i = (row * width + col) * 4;
            let r = rgba[i] as i32;
            let g = rgba[i + 1] as i32;
            let b = rgba[i + 2] as i32;
            out.push((((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(0, 255) as u8);
        }
    }

    let chroma_h = height / 2;
    let chroma_w = width / 2;

    match format {
        MfInputFormat::Nv12 => {
            for row in 0..chroma_h {
                for col in 0..chroma_w {
                    let (cb, cr) = avg_chroma_2x2(rgba, width, row * 2, col * 2);
                    out.push(cb);
                    out.push(cr);
                }
            }
        }
        MfInputFormat::I420 => {
            let mut cb_plane = Vec::with_capacity(chroma_w * chroma_h);
            let mut cr_plane = Vec::with_capacity(chroma_w * chroma_h);
            for row in 0..chroma_h {
                for col in 0..chroma_w {
                    let (cb, cr) = avg_chroma_2x2(rgba, width, row * 2, col * 2);
                    cb_plane.push(cb);
                    cr_plane.push(cr);
                }
            }
            out.extend_from_slice(&cb_plane);
            out.extend_from_slice(&cr_plane);
        }
    }

    out
}

#[cfg(target_os = "windows")]
fn avg_chroma_2x2(rgba: &[u8], width: usize, y: usize, x: usize) -> (u8, u8) {
    let i00 = (y * width + x) * 4;
    let i01 = (y * width + x + 1) * 4;
    let i10 = ((y + 1) * width + x) * 4;
    let i11 = ((y + 1) * width + x + 1) * 4;
    let r = (rgba[i00] as i32 + rgba[i01] as i32 + rgba[i10] as i32 + rgba[i11] as i32) >> 2;
    let g =
        (rgba[i00 + 1] as i32 + rgba[i01 + 1] as i32 + rgba[i10 + 1] as i32 + rgba[i11 + 1] as i32)
            >> 2;
    let b =
        (rgba[i00 + 2] as i32 + rgba[i01 + 2] as i32 + rgba[i10 + 2] as i32 + rgba[i11 + 2] as i32)
            >> 2;
    let cb = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
    let cr = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
    (cb, cr)
}

#[cfg(target_os = "windows")]
fn create_input_sample_from_bytes(
    bytes: &[u8],
    sample_time_hns: i64,
    sample_duration_hns: i64,
) -> Result<IMFSample, String> {
    unsafe {
        let buffer = MFCreateMemoryBuffer(bytes.len() as u32).map_err(|err| err.to_string())?;
        copy_bytes_into_media_buffer(&buffer, bytes)?;
        let sample = MFCreateSample().map_err(|err| err.to_string())?;
        sample.AddBuffer(&buffer).map_err(|err| err.to_string())?;
        sample
            .SetSampleTime(sample_time_hns)
            .map_err(|err| err.to_string())?;
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
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|err| err.to_string())?;
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
fn read_sequence_header_annexb(
    transform: &IMFTransform,
    output_stream_id: u32,
) -> Result<Vec<u8>, String> {
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
fn avcc_payload_to_annexb(
    sample: &[u8],
    _sequence_header_annexb: &[u8],
) -> Result<Vec<u8>, String> {
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
        0..=921_600 => {
            if frame_rate >= 60 {
                9_000_000
            } else {
                6_000_000u32.saturating_mul(motion_factor)
            }
        }
        921_601..=2_073_600 => {
            if frame_rate >= 60 {
                14_000_000
            } else {
                8_000_000
            }
        }
        2_073_601..=3_686_400 => {
            if frame_rate >= 60 {
                22_000_000
            } else {
                14_000_000
            }
        }
        _ => {
            if frame_rate >= 60 {
                40_000_000
            } else {
                28_000_000
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
        assert_eq!(compute_target_bitrate_bps(1280, 720, 30), 6_000_000);
        assert_eq!(compute_target_bitrate_bps(1920, 1080, 60), 14_000_000);
        assert_eq!(compute_target_bitrate_bps(2560, 1440, 60), 22_000_000);
    }

    #[test]
    fn exposes_expected_codec_label() {
        assert_eq!(VideoCodec::H264AnnexB.label(), "avc1.42E034");
        assert_eq!(VideoCodec::Jpeg.label(), "jpeg");
    }
}
