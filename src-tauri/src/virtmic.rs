use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtmicListOk {
    pub ok: bool,
    pub targets: Vec<BTreeMap<String, String>>,
    pub has_pipewire_pulse: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtmicListErr {
    pub ok: bool,
    pub is_glib_cxx_outdated: bool,
    pub has_pipewire_pulse: bool,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum VirtmicListResponse {
    Ok(VirtmicListOk),
    Err(VirtmicListErr),
}

#[tauri::command]
pub fn virtmic_list() -> Result<VirtmicListResponse, String> {
    #[cfg(target_os = "linux")]
    {
        return Ok(VirtmicListResponse::Err(VirtmicListErr {
            ok: false,
            is_glib_cxx_outdated: false,
            has_pipewire_pulse: detect_pipewire_pulse(),
            error: "Rust-native venmic parity is not implemented yet.".to_owned(),
        }));
    }

    #[cfg(not(target_os = "linux"))]
    {
        Ok(VirtmicListResponse::Err(VirtmicListErr {
            ok: false,
            is_glib_cxx_outdated: false,
            has_pipewire_pulse: false,
            error: "Virtual microphone support is only available on Linux.".to_owned(),
        }))
    }
}

#[tauri::command]
pub fn virtmic_start(_include: Vec<BTreeMap<String, String>>) -> Result<(), String> {
    Err("Rust-native virtual microphone routing is not implemented yet".to_owned())
}

#[tauri::command]
pub fn virtmic_start_system(_exclude: Vec<BTreeMap<String, String>>) -> Result<(), String> {
    Err("Rust-native virtual microphone routing is not implemented yet".to_owned())
}

#[tauri::command]
pub fn virtmic_stop() -> Result<(), String> {
    Err("Rust-native virtual microphone routing is not implemented yet".to_owned())
}

#[cfg(target_os = "linux")]
fn detect_pipewire_pulse() -> bool {
    std::process::Command::new("sh")
        .arg("-lc")
        .arg("command -v pipewire-pulse >/dev/null 2>&1 || command -v pw-cli >/dev/null 2>&1")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
