#![cfg(windows)]

use super::capture_sources::{CaptureFrameMode, PreparedFramePixelFormat};
use super::d3d11_device::{
    create_video_device_for_monitor, find_output_for_monitor, SharedD3D11Device,
};
use std::sync::Arc;
use windows::{
    core::Interface,
    Win32::Graphics::{
        Direct3D11::{
            ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
            D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
        },
        Dxgi::{
            Common::{
                DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
            },
            IDXGIOutput1, IDXGIOutput6, IDXGIOutputDuplication, IDXGIResource,
            DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
        },
        Gdi::HMONITOR,
    },
};

pub enum DxgiFrameResult {
    Frame {
        width: u32,
        height: u32,
        bgra: Vec<u8>,
    },
    Texture {
        width: u32,
        height: u32,
        texture: ID3D11Texture2D,
        pixel_format: PreparedFramePixelFormat,
    },
    Timeout,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DxgiCaptureFormat {
    Bgra,
    Float,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DxgiTextureMode {
    Bgra,
    Hdr,
}

pub struct DxgiScreenCapture {
    shared_device: Arc<SharedD3D11Device>,
    output1: IDXGIOutput1,
    duplication: IDXGIOutputDuplication,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    copy_texture: Option<(ID3D11Texture2D, u32, u32)>,
    format: DxgiCaptureFormat,
    texture_mode: Option<DxgiTextureMode>,
}

unsafe impl Send for DxgiScreenCapture {}

impl DxgiScreenCapture {
    pub fn new(
        monitor: HMONITOR,
        shared_device: Option<Arc<SharedD3D11Device>>,
        frame_mode: CaptureFrameMode,
    ) -> Result<Self, String> {
        unsafe { Self::init(monitor, shared_device, frame_mode) }
            .map_err(|e| format!("DXGI screen capture init failed: {e}"))
    }

    unsafe fn init(
        monitor: HMONITOR,
        shared_device: Option<Arc<SharedD3D11Device>>,
        frame_mode: CaptureFrameMode,
    ) -> windows::core::Result<Self> {
        let (_, output1) = find_output_for_monitor(monitor).map_err(|err| {
            windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), err)
        })?;
        let shared_device = match shared_device {
            Some(device) => device,
            None => create_video_device_for_monitor(monitor).map_err(|err| {
                windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), err)
            })?,
        };

        let texture_mode = match frame_mode {
            CaptureFrameMode::TargetRgbaFrame | CaptureFrameMode::SourceBgraFrame => None,
            CaptureFrameMode::SourceBgraTextureFrame => Some(DxgiTextureMode::Bgra),
            CaptureFrameMode::SourceHdrTextureFrame => Some(DxgiTextureMode::Hdr),
        };

        let (duplication, format) = match texture_mode {
            Some(DxgiTextureMode::Bgra) => (
                output1.DuplicateOutput(shared_device.device())?,
                DxgiCaptureFormat::Bgra,
            ),
            Some(DxgiTextureMode::Hdr) | None => {
                match create_float_duplication(&output1, shared_device.device()) {
                    Ok(dup) => {
                        log::debug!("DXGI screen capture using R16G16B16A16_FLOAT (HDR-ready)");
                        (dup, DxgiCaptureFormat::Float)
                    }
                    Err(e) => {
                        log::debug!("DXGI float duplication unavailable ({e}), using BGRA");
                        (
                            output1.DuplicateOutput(shared_device.device())?,
                            DxgiCaptureFormat::Bgra,
                        )
                    }
                }
            }
        };

        Ok(Self {
            shared_device,
            output1,
            duplication,
            staging: None,
            copy_texture: None,
            format,
            texture_mode,
        })
    }

    fn recreate_duplication(&mut self) -> Result<(), String> {
        let duplication = unsafe {
            match self.texture_mode {
                Some(DxgiTextureMode::Bgra) => {
                    self.output1.DuplicateOutput(self.shared_device.device())
                }
                Some(DxgiTextureMode::Hdr) | None => match self.format {
                    DxgiCaptureFormat::Float => {
                        create_float_duplication(&self.output1, self.shared_device.device())
                            .or_else(|_| self.output1.DuplicateOutput(self.shared_device.device()))
                    }
                    DxgiCaptureFormat::Bgra => {
                        self.output1.DuplicateOutput(self.shared_device.device())
                    }
                },
            }
        }
        .map_err(|e| format!("DXGI recreate duplication failed: {e}"))?;

        self.duplication = duplication;
        self.staging = None;
        self.copy_texture = None;
        Ok(())
    }

    pub fn acquire_frame(&mut self, timeout_ms: u32) -> Result<DxgiFrameResult, String> {
        for _ in 0..3 {
            let result = unsafe { self.try_acquire_frame(timeout_ms) };
            match result {
                Ok(frame) => return Ok(frame),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    self.recreate_duplication()?;
                }
                Err(e) => return Err(format!("DXGI AcquireNextFrame: {e}")),
            }
        }
        Err("DXGI access lost repeatedly".to_owned())
    }

    unsafe fn try_acquire_frame(
        &mut self,
        timeout_ms: u32,
    ) -> windows::core::Result<DxgiFrameResult> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        match self
            .duplication
            .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                return Ok(DxgiFrameResult::Timeout);
            }
            Err(e) => return Err(e),
        }

        let texture: ID3D11Texture2D = resource.unwrap().cast()?;

        if self.texture_mode.is_some() {
            let (copy_texture, width, height) = self.get_or_create_copy_texture(&texture)?;
            {
                let _lock = self.shared_device.lock();
                self.shared_device
                    .context()
                    .CopyResource(&copy_texture, &texture);
            }
            self.duplication.ReleaseFrame()?;

            return Ok(DxgiFrameResult::Texture {
                width,
                height,
                texture: copy_texture,
                pixel_format: match self.format {
                    DxgiCaptureFormat::Bgra => PreparedFramePixelFormat::Bgra,
                    DxgiCaptureFormat::Float => PreparedFramePixelFormat::Rgba16FloatTexture,
                },
            });
        }

        let (staging, width, height) = self.get_or_create_staging(&texture)?;

        {
            let _lock = self.shared_device.lock();
            self.shared_device
                .context()
                .CopyResource(&staging, &texture);
        }
        self.duplication.ReleaseFrame()?;

        let mut mapped = windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
        {
            let _lock = self.shared_device.lock();
            self.shared_device
                .context()
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }

        let row_pitch = mapped.RowPitch as usize;
        let bgra = match self.format {
            DxgiCaptureFormat::Bgra => {
                let bytes_per_row = width as usize * 4;
                let src = std::slice::from_raw_parts(
                    mapped.pData as *const u8,
                    row_pitch * height as usize,
                );
                let mut out = Vec::with_capacity(bytes_per_row * height as usize);
                for row in 0..height as usize {
                    out.extend_from_slice(&src[row * row_pitch..row * row_pitch + bytes_per_row]);
                }
                out
            }
            DxgiCaptureFormat::Float => {
                let bytes_per_row_f16 = width as usize * 8;
                let src = std::slice::from_raw_parts(
                    mapped.pData as *const u8,
                    row_pitch * height as usize,
                );
                let lut = super::wgc_window_capture::tonemap_lut();
                let mut out = Vec::with_capacity(width as usize * height as usize * 4);
                for row in 0..height as usize {
                    let row_data = &src[row * row_pitch..row * row_pitch + bytes_per_row_f16];
                    for px in 0..width as usize {
                        let base = px * 8;
                        let r_bits = u16::from_le_bytes([row_data[base], row_data[base + 1]]);
                        let g_bits = u16::from_le_bytes([row_data[base + 2], row_data[base + 3]]);
                        let b_bits = u16::from_le_bytes([row_data[base + 4], row_data[base + 5]]);
                        out.push(lut[b_bits as usize]);
                        out.push(lut[g_bits as usize]);
                        out.push(lut[r_bits as usize]);
                        out.push(255);
                    }
                }
                out
            }
        };

        {
            let _lock = self.shared_device.lock();
            self.shared_device.context().Unmap(&staging, 0);
        }

        Ok(DxgiFrameResult::Frame {
            width,
            height,
            bgra,
        })
    }

    unsafe fn get_or_create_staging(
        &mut self,
        source: &ID3D11Texture2D,
    ) -> windows::core::Result<(ID3D11Texture2D, u32, u32)> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        source.GetDesc(&mut desc);
        let (w, h) = (desc.Width, desc.Height);

        if let Some((ref tex, sw, sh)) = self.staging {
            if sw == w && sh == h {
                return Ok((tex.clone(), w, h));
            }
        }

        let staging_fmt = match self.format {
            DxgiCaptureFormat::Bgra => DXGI_FORMAT_B8G8R8A8_UNORM,
            DxgiCaptureFormat::Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
        };
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: staging_fmt,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        self.shared_device
            .device()
            .CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
        let staging = staging.unwrap();
        self.staging = Some((staging.clone(), w, h));
        Ok((staging, w, h))
    }

    unsafe fn get_or_create_copy_texture(
        &mut self,
        source: &ID3D11Texture2D,
    ) -> windows::core::Result<(ID3D11Texture2D, u32, u32)> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        source.GetDesc(&mut desc);
        let (w, h) = (desc.Width, desc.Height);

        if let Some((ref tex, sw, sh)) = self.copy_texture {
            if sw == w && sh == h {
                return Ok((tex.clone(), w, h));
            }
        }

        let copy_desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: match self.format {
                DxgiCaptureFormat::Bgra => DXGI_FORMAT_B8G8R8A8_UNORM,
                DxgiCaptureFormat::Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut copy_texture = None;
        self.shared_device
            .device()
            .CreateTexture2D(&copy_desc, None, Some(&mut copy_texture))?;
        let copy_texture = copy_texture.unwrap();
        self.copy_texture = Some((copy_texture.clone(), w, h));
        Ok((copy_texture, w, h))
    }
}

unsafe fn create_float_duplication(
    output1: &IDXGIOutput1,
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
) -> windows::core::Result<IDXGIOutputDuplication> {
    let output6: IDXGIOutput6 = output1.cast()?;
    output6.DuplicateOutput1(device, 0, &[DXGI_FORMAT_R16G16B16A16_FLOAT])
}
