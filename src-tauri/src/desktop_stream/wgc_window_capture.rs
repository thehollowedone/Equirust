#![cfg(windows)]

use super::capture_sources::PreparedFramePixelFormat;
use super::d3d11_device::{create_default_video_device, SharedD3D11Device};
use std::{
    collections::HashMap,
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Condvar, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use windows::{
    core::Interface,
    Foundation::{IClosable, TypedEventHandler},
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem},
        DirectX::DirectXPixelFormat,
    },
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D11::{
                ID3D11Device, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
                },
                IDXGIDevice,
            },
        },
        System::{
            Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
            },
        },
    },
};

// ── Reinhard tonemapping LUT ──────────────────────────────────────────────────
//
// Maps every possible IEEE 754 half-precision bit pattern (65536 entries) to a
// tonemapped, sRGB-gamma-corrected u8 output value.  Built once at first use
// and reused for every captured frame, so per-pixel cost is a single array
// lookup — no floating-point arithmetic on the hot path.
//
// Tone curve: global Reinhard  t/(1+t)  then sRGB EOTF inverse.
// On SDR content (values ≤ 1.0 scRGB), Reinhard changes the value by ≤ 0.5 LSB
// at u8 precision, so SDR captures look identical to the BGRA path.

pub(crate) fn build_tonemap_lut() -> Box<[u8; 65536]> {
    let mut lut = Box::new([0u8; 65536]);
    for bits in 0u32..=65535 {
        let linear = f16_bits_to_f32(bits as u16);
        lut[bits as usize] = linear_to_tonemapped_srgb(linear);
    }
    lut
}

pub(crate) fn tonemap_lut() -> &'static [u8; 65536] {
    static LUT: OnceLock<Box<[u8; 65536]>> = OnceLock::new();
    LUT.get_or_init(build_tonemap_lut)
}

fn f16_bits_to_f32(bits: u16) -> f32 {
    let e = (bits >> 10) & 0x1f;
    let m = (bits & 0x3ff) as u32;
    let s = (bits >> 15) as u32;
    let v = if e == 0 {
        // subnormal
        m as f32 * (1.0f32 / (1u32 << 24) as f32)
    } else if e < 31 {
        // normal: adjust exponent bias (15 → 127)
        f32::from_bits(((e as u32 + 112) << 23) | (m << 13))
    } else if m == 0 {
        f32::INFINITY
    } else {
        f32::NAN
    };
    if s != 0 {
        -v
    } else {
        v
    }
}

fn linear_to_tonemapped_srgb(linear: f32) -> u8 {
    if !linear.is_finite() || linear <= 0.0 {
        return 0;
    }
    // Global Reinhard: compresses HDR highlights into SDR range.
    let t = linear / (1.0 + linear);
    // sRGB EOTF inverse (IEC 61966-2-1).
    let gamma = if t <= 0.003_130_8 {
        t * 12.92
    } else {
        1.055 * t.powf(1.0 / 2.4) - 0.055
    };
    (gamma.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

// ─────────────────────────────────────────────────────────────────────────────

pub enum WgcFrameResult {
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
    Error(String),
}

enum WgcCapturedFrame {
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
}

pub struct WgcWindowCapture {
    frame_rx: Receiver<Result<WgcCapturedFrame, String>>,
    stop_tx: SyncSender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl WgcWindowCapture {
    pub fn start(hwnd_value: u32) -> Result<Self, String> {
        Self::start_internal(hwnd_value, None, false, false)
    }

    pub fn start_with_device(
        hwnd_value: u32,
        shared_device: Option<Arc<SharedD3D11Device>>,
        hdr_texture: bool,
    ) -> Result<Self, String> {
        Self::start_internal(hwnd_value, shared_device, true, hdr_texture)
    }

    fn start_internal(
        hwnd_value: u32,
        shared_device: Option<Arc<SharedD3D11Device>>,
        direct_texture: bool,
        hdr_texture: bool,
    ) -> Result<Self, String> {
        // Initialise the LUT eagerly on the calling thread so the first
        // captured frame doesn't pay the build cost (~0.5 ms).
        if !direct_texture {
            let _ = tonemap_lut();
        }

        let (frame_tx, frame_rx) = sync_channel::<Result<WgcCapturedFrame, String>>(1);
        let (stop_tx, stop_rx) = sync_channel::<()>(1);

        let frame_ready = Arc::new((Mutex::new(false), Condvar::new()));
        let frame_ready_thread = Arc::clone(&frame_ready);

        let join_handle = thread::Builder::new()
            .name(format!("WgcWindowCapture-{hwnd_value}"))
            .spawn(move || unsafe {
                let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                let com_initialized = hr.is_ok();

                match run_wgc_capture(
                    hwnd_value,
                    shared_device,
                    direct_texture,
                    hdr_texture,
                    &frame_tx,
                    &stop_rx,
                    frame_ready_thread,
                ) {
                    Ok(()) => {}
                    Err(e) => {
                        let _ = frame_tx.try_send(Err(e));
                    }
                }

                if com_initialized {
                    CoUninitialize();
                }
            })
            .map_err(|err| err.to_string())?;

        Ok(Self {
            frame_rx,
            stop_tx,
            join_handle: Some(join_handle),
        })
    }

    pub fn acquire_frame(&mut self, timeout_ms: u32) -> WgcFrameResult {
        match self
            .frame_rx
            .recv_timeout(Duration::from_millis(u64::from(timeout_ms)))
        {
            Ok(Ok(WgcCapturedFrame::Frame {
                width,
                height,
                bgra,
            })) => WgcFrameResult::Frame {
                width,
                height,
                bgra,
            },
            Ok(Ok(WgcCapturedFrame::Texture {
                width,
                height,
                texture,
                pixel_format,
            })) => WgcFrameResult::Texture {
                width,
                height,
                texture,
                pixel_format,
            },
            Ok(Err(msg)) => WgcFrameResult::Error(msg),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => WgcFrameResult::Timeout,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                WgcFrameResult::Error("WGC capture thread disconnected".to_owned())
            }
        }
    }

    pub fn stop(&mut self) {
        let _ = self.stop_tx.try_send(());
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for WgcWindowCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe fn run_wgc_capture(
    hwnd_value: u32,
    shared_device: Option<Arc<SharedD3D11Device>>,
    direct_texture: bool,
    hdr_texture: bool,
    frame_tx: &SyncSender<Result<WgcCapturedFrame, String>>,
    stop_rx: &Receiver<()>,
    frame_ready: Arc<(Mutex<bool>, Condvar)>,
) -> Result<(), String> {
    let shared_device = match shared_device {
        Some(device) => device,
        None => create_default_video_device()?,
    };

    let dxgi_device: IDXGIDevice = shared_device
        .device()
        .cast()
        .map_err(|e| format!("D3D11→IDXGIDevice: {e}"))?;
    let winrt_device_inspect = CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)
        .map_err(|e| format!("CreateDirect3D11DeviceFromDXGIDevice: {e}"))?;
    let winrt_device: windows::Graphics::DirectX::Direct3D11::IDirect3DDevice =
        winrt_device_inspect
            .cast()
            .map_err(|e| format!("WinRT D3D device cast: {e}"))?;

    let item = create_capture_item(HWND(hwnd_value as _))?;
    let item_size = item
        .Size()
        .map_err(|e| format!("GraphicsCaptureItem::Size: {e}"))?;

    let capture_format = if direct_texture && !hdr_texture {
        DirectXPixelFormat::B8G8R8A8UIntNormalized
    } else {
        // Capture in scRGB (R16G16B16A16_FLOAT, linear light). This preserves
        // HDR luminance so we can apply Reinhard tonemapping ourselves,
        // preventing the blown-out look that results from WGC's default SDR clipping.
        DirectXPixelFormat::R16G16B16A16Float
    };

    let frame_pool =
        Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, capture_format, 2, item_size)
            .map_err(|e| format!("Direct3D11CaptureFramePool::CreateFreeThreaded: {e}"))?;

    let frame_ready_handler = Arc::clone(&frame_ready);
    frame_pool
        .FrameArrived(&TypedEventHandler::new(move |_, _| {
            let (lock, cvar) = &*frame_ready_handler;
            let mut ready = lock.lock().unwrap_or_else(|p| p.into_inner());
            *ready = true;
            cvar.notify_one();
            Ok(())
        }))
        .map_err(|e| format!("FrameArrived registration: {e}"))?;

    let session = frame_pool
        .CreateCaptureSession(&item)
        .map_err(|e| format!("CreateCaptureSession: {e}"))?;

    let _ = session.SetIsBorderRequired(false);
    session
        .StartCapture()
        .map_err(|e| format!("StartCapture: {e}"))?;

    let mut staging: Option<(ID3D11Texture2D, u32, u32)> = None;
    let mut copy_texture: Option<(ID3D11Texture2D, u32, u32)> = None;
    let mut pool_size = item_size;
    let lut = tonemap_lut();

    loop {
        match stop_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        let was_ready = {
            let (lock, cvar) = &*frame_ready;
            let guard = lock.lock().unwrap_or_else(|p| p.into_inner());
            let (mut guard, _) = cvar
                .wait_timeout(guard, Duration::from_millis(100))
                .unwrap_or_else(|p| p.into_inner());
            let ready = *guard;
            *guard = false;
            ready
        };

        if !was_ready {
            continue;
        }

        // Drain all pending frames, keep only the latest one.
        let mut latest: Option<WgcCapturedFrame> = None;
        loop {
            let frame = match frame_pool.TryGetNextFrame() {
                Ok(f) => f,
                Err(_) => break,
            };

            // If the window was resized, recreate the frame pool so subsequent
            // frames match the new dimensions.
            if let Ok(content_size) = frame.ContentSize() {
                if content_size.Width != pool_size.Width || content_size.Height != pool_size.Height
                {
                    pool_size = content_size;
                    let _ = frame_pool.Recreate(&winrt_device, capture_format, 2, pool_size);
                    // This frame may have old dimensions; drop it and wait for
                    // the next one at the correct size.
                    drop(frame);
                    break;
                }
            }

            let result: Result<WgcCapturedFrame, String> = (|| {
                let surface = frame.Surface().map_err(|e| format!("frame Surface: {e}"))?;
                let access: IDirect3DDxgiInterfaceAccess = surface
                    .cast()
                    .map_err(|e| format!("Surface→IDirect3DDxgiInterfaceAccess: {e}"))?;
                let texture: ID3D11Texture2D = access
                    .GetInterface()
                    .map_err(|e| format!("GetInterface→ID3D11Texture2D: {e}"))?;

                if direct_texture {
                    let (copy_tex, w, h) = get_or_create_copy_texture(
                        shared_device.device(),
                        &mut copy_texture,
                        &texture,
                        hdr_texture,
                    )?;
                    {
                        let _lock = shared_device.lock();
                        shared_device.context().CopyResource(&copy_tex, &texture);
                    }
                    drop(frame);
                    return Ok(WgcCapturedFrame::Texture {
                        width: w,
                        height: h,
                        texture: copy_tex,
                        pixel_format: if hdr_texture {
                            PreparedFramePixelFormat::Rgba16FloatTexture
                        } else {
                            PreparedFramePixelFormat::Bgra
                        },
                    });
                }

                let (staging_tex, w, h) =
                    get_or_create_staging(shared_device.device(), &mut staging, &texture)?;

                {
                    let _lock = shared_device.lock();
                    shared_device.context().CopyResource(&staging_tex, &texture);
                }
                drop(frame); // release pool slot before Map stalls

                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                {
                    let _lock = shared_device.lock();
                    shared_device
                        .context()
                        .Map(&staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                        .map_err(|e| format!("ID3D11DeviceContext::Map: {e}"))?;
                }

                // R16G16B16A16_FLOAT layout: R, G, B, A each as 2-byte LE f16.
                // RowPitch may include padding — always step by RowPitch, not
                // by pixel width × 8.
                let row_pitch = mapped.RowPitch as usize;
                let bytes_per_row = w as usize * 8; // 4 channels × 2 bytes/channel
                let src =
                    std::slice::from_raw_parts(mapped.pData as *const u8, row_pitch * h as usize);

                // Output is BGRA u8 — 4 bytes/pixel.
                let mut bgra = Vec::with_capacity(w as usize * h as usize * 4);
                for row in 0..h as usize {
                    let row_start = row * row_pitch;
                    let row_data = &src[row_start..row_start + bytes_per_row];
                    for px in 0..w as usize {
                        let base = px * 8;
                        let r_bits = u16::from_le_bytes([row_data[base], row_data[base + 1]]);
                        let g_bits = u16::from_le_bytes([row_data[base + 2], row_data[base + 3]]);
                        let b_bits = u16::from_le_bytes([row_data[base + 4], row_data[base + 5]]);
                        // A channel is discarded — screenshare is always opaque.
                        bgra.push(lut[b_bits as usize]);
                        bgra.push(lut[g_bits as usize]);
                        bgra.push(lut[r_bits as usize]);
                        bgra.push(255);
                    }
                }

                {
                    let _lock = shared_device.lock();
                    shared_device.context().Unmap(&staging_tex, 0);
                }
                Ok(WgcCapturedFrame::Frame {
                    width: w,
                    height: h,
                    bgra,
                })
            })();

            match result {
                Ok(frame_data) => latest = Some(frame_data),
                Err(e) => {
                    if matches!(
                        frame_tx.try_send(Err(e)),
                        Err(TrySendError::Disconnected(_))
                    ) {
                        break;
                    }
                    break;
                }
            }
        }

        if let Some(frame_data) = latest {
            if matches!(
                frame_tx.try_send(Ok(frame_data)),
                Err(TrySendError::Disconnected(_))
            ) {
                break;
            }
        }
    }

    let _ = session.Close();
    let closable: Option<IClosable> = frame_pool.cast().ok();
    if let Some(c) = closable {
        let _ = c.Close();
    }

    Ok(())
}

fn create_capture_item(hwnd: HWND) -> Result<GraphicsCaptureItem, String> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| format!("IGraphicsCaptureItemInterop factory: {e}"))?;
    unsafe {
        interop
            .CreateForWindow(hwnd)
            .map_err(|e| format!("CreateForWindow: {e}"))
    }
}

unsafe fn get_or_create_staging(
    device: &ID3D11Device,
    staging: &mut Option<(ID3D11Texture2D, u32, u32)>,
    source: &ID3D11Texture2D,
) -> Result<(ID3D11Texture2D, u32, u32), String> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    source.GetDesc(&mut desc);
    let (w, h) = (desc.Width, desc.Height);

    if let Some((ref tex, sw, sh)) = *staging {
        if sw == w && sh == h {
            return Ok((tex.clone(), w, h));
        }
    }

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut tex: Option<ID3D11Texture2D> = None;
    device
        .CreateTexture2D(&staging_desc, None, Some(&mut tex))
        .map_err(|e| format!("CreateTexture2D (staging): {e}"))?;
    let tex = tex.unwrap();
    *staging = Some((tex.clone(), w, h));
    Ok((tex, w, h))
}

unsafe fn get_or_create_copy_texture(
    device: &ID3D11Device,
    copy_texture: &mut Option<(ID3D11Texture2D, u32, u32)>,
    source: &ID3D11Texture2D,
    hdr_texture: bool,
) -> Result<(ID3D11Texture2D, u32, u32), String> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    source.GetDesc(&mut desc);
    let (w, h) = (desc.Width, desc.Height);

    if let Some((ref tex, sw, sh)) = *copy_texture {
        if sw == w && sh == h {
            return Ok((tex.clone(), w, h));
        }
    }

    let copy_desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: if hdr_texture {
            DXGI_FORMAT_R16G16B16A16_FLOAT
        } else {
            DXGI_FORMAT_B8G8R8A8_UNORM
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
    let mut tex: Option<ID3D11Texture2D> = None;
    device
        .CreateTexture2D(&copy_desc, None, Some(&mut tex))
        .map_err(|e| format!("CreateTexture2D (copy): {e}"))?;
    let tex = tex.unwrap();
    *copy_texture = Some((tex.clone(), w, h));
    Ok((tex, w, h))
}

// ── Pre-warm cache ────────────────────────────────────────────────────────────

struct PrewarmEntry {
    capture: WgcWindowCapture,
    created_at: Instant,
}

const PREWARM_TTL: Duration = Duration::from_secs(30);

fn prewarm_cache() -> &'static Mutex<HashMap<u32, PrewarmEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, PrewarmEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn prewarm_window_captures(hwnd_values: Vec<u32>) {
    let mut guard = prewarm_cache().lock().unwrap_or_else(|p| p.into_inner());

    guard.retain(|_, entry| entry.created_at.elapsed() < PREWARM_TTL);

    for hwnd_value in hwnd_values {
        if guard.contains_key(&hwnd_value) {
            continue;
        }
        match WgcWindowCapture::start(hwnd_value) {
            Ok(capture) => {
                guard.insert(
                    hwnd_value,
                    PrewarmEntry {
                        capture,
                        created_at: Instant::now(),
                    },
                );
            }
            Err(e) => {
                log::debug!("WGC prewarm failed hwnd={hwnd_value}: {e}");
            }
        }
    }
}

pub fn take_prewarmed_capture(hwnd_value: u32) -> Option<WgcWindowCapture> {
    let mut guard = prewarm_cache().lock().unwrap_or_else(|p| p.into_inner());
    let entry = guard.remove(&hwnd_value)?;
    if entry.created_at.elapsed() < PREWARM_TTL {
        Some(entry.capture)
    } else {
        None
    }
}
