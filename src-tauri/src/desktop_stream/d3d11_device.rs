#![cfg(windows)]

use std::sync::Arc;

use windows::{
    core::Interface,
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL,
                D3D_FEATURE_LEVEL_11_0,
            },
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                D3D11_SDK_VERSION,
            },
            Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1},
            Gdi::HMONITOR,
        },
    },
};

pub struct SharedD3D11Device {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    multithread: ID3D11Multithread,
}

pub struct D3D11MultithreadGuard<'a> {
    multithread: &'a ID3D11Multithread,
}

impl Drop for D3D11MultithreadGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.multithread.Leave();
        }
    }
}

impl SharedD3D11Device {
    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    pub fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }

    pub fn lock(&self) -> D3D11MultithreadGuard<'_> {
        unsafe {
            self.multithread.Enter();
        }
        D3D11MultithreadGuard {
            multithread: &self.multithread,
        }
    }
}

pub fn create_default_video_device() -> Result<Arc<SharedD3D11Device>, String> {
    create_video_device_internal(None)
}

pub fn create_video_device_for_monitor(
    monitor: HMONITOR,
) -> Result<Arc<SharedD3D11Device>, String> {
    let adapter = find_adapter_for_monitor(monitor)?;
    create_video_device_internal(Some(&adapter))
}

fn create_video_device_internal(
    adapter: Option<&IDXGIAdapter1>,
) -> Result<Arc<SharedD3D11Device>, String> {
    let mut device = None;
    let mut context = None;
    let mut feature_level = D3D_FEATURE_LEVEL::default();
    let levels = [D3D_FEATURE_LEVEL_11_0];
    let flags = windows::Win32::Graphics::Direct3D11::D3D11_CREATE_DEVICE_FLAG(
        D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 | D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0,
    );

    let dxgi_adapter: Option<IDXGIAdapter> = adapter
        .map(|entry| {
            entry
                .cast()
                .map_err(|err| format!("IDXGIAdapter cast failed: {err}"))
        })
        .transpose()?;

    unsafe {
        D3D11CreateDevice(
            dxgi_adapter.as_ref(),
            if dxgi_adapter.is_some() {
                D3D_DRIVER_TYPE_UNKNOWN
            } else {
                D3D_DRIVER_TYPE_HARDWARE
            },
            HMODULE::default(),
            flags,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut feature_level),
            Some(&mut context),
        )
        .map_err(|err| format!("D3D11CreateDevice failed: {err}"))?;
    }

    let device = device.ok_or_else(|| "D3D11 device was not created".to_owned())?;
    let context = context.ok_or_else(|| "D3D11 device context was not created".to_owned())?;
    let multithread: ID3D11Multithread = context
        .cast()
        .map_err(|err| format!("ID3D11Multithread cast failed: {err}"))?;
    unsafe {
        let _ = multithread.SetMultithreadProtected(true);
    }

    Ok(Arc::new(SharedD3D11Device {
        device,
        context,
        multithread,
    }))
}

pub fn find_output_for_monitor(monitor: HMONITOR) -> Result<(IDXGIAdapter1, IDXGIOutput1), String> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|err| format!("CreateDXGIFactory1 failed: {err}"))?;
    let mut adapter_index = 0u32;
    loop {
        let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapters1(adapter_index) }
            .map_err(|err| format!("EnumAdapters1 failed: {err}"))?;
        let mut output_index = 0u32;
        loop {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(_) => break,
            };
            let desc = unsafe { output.GetDesc() }
                .map_err(|err| format!("IDXGIOutput::GetDesc failed: {err}"))?;
            if desc.Monitor == monitor {
                let output1: IDXGIOutput1 = output
                    .cast()
                    .map_err(|err| format!("IDXGIOutput1 cast failed: {err}"))?;
                return Ok((adapter, output1));
            }
            output_index = output_index.saturating_add(1);
        }
        adapter_index = adapter_index.saturating_add(1);
    }
}

fn find_adapter_for_monitor(monitor: HMONITOR) -> Result<IDXGIAdapter1, String> {
    find_output_for_monitor(monitor).map(|(adapter, _)| adapter)
}
