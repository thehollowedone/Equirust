use crate::store::PersistedStore;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    app_name: String,
    app_version: String,
    package_version: String,
    target_triple: String,
    family: String,
    os: String,
    arch: String,
    portable_mode: bool,
    data_dir: String,
    settings_file: String,
    state_file: String,
    settings_exists: bool,
    state_exists: bool,
    app_data_dir: String,
    app_cache_dir: String,
    app_log_dir: String,
}

#[tauri::command]
pub fn run_doctor(
    app: AppHandle,
    store: State<'_, PersistedStore>,
) -> Result<DoctorReport, String> {
    let snapshot = store.snapshot();

    Ok(DoctorReport {
        app_name: app.package_info().name.clone(),
        app_version: app.package_info().version.to_string(),
        package_version: env!("CARGO_PKG_VERSION").to_owned(),
        target_triple: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        family: std::env::consts::FAMILY.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        portable_mode: snapshot.paths.portable,
        data_dir: snapshot.paths.data_dir.display().to_string(),
        settings_file: snapshot.paths.settings_file.display().to_string(),
        state_file: snapshot.paths.state_file.display().to_string(),
        settings_exists: snapshot.paths.settings_file.exists(),
        state_exists: snapshot.paths.state_file.exists(),
        app_data_dir: app
            .path()
            .app_data_dir()
            .map_err(|err| err.to_string())?
            .display()
            .to_string(),
        app_cache_dir: app
            .path()
            .app_cache_dir()
            .map_err(|err| err.to_string())?
            .display()
            .to_string(),
        app_log_dir: app
            .path()
            .app_log_dir()
            .map_err(|err| err.to_string())?
            .display()
            .to_string(),
    })
}
