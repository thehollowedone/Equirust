use crate::{settings::Settings, store::PersistedStore};
use tauri::{AppHandle, State as TauriState};

#[tauri::command]
pub fn get_auto_start_status(app: AppHandle) -> Result<bool, String> {
    is_enabled(&app)
}

#[tauri::command]
pub fn set_auto_start_enabled(
    enabled: bool,
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<bool, String> {
    let snapshot = store.snapshot();
    set_enabled(&app, enabled, &snapshot.settings)?;
    Ok(enabled)
}

pub fn sync(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    if is_enabled(app)? {
        set_enabled(app, true, settings)?;
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn is_enabled(app: &AppHandle) -> Result<bool, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let run_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(run_key_path())
        .map_err(|err| err.to_string())?;

    match run_key.get_value::<String, _>(run_value_name(app)) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(not(target_os = "windows"))]
fn is_enabled(_app: &AppHandle) -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_os = "windows")]
fn set_enabled(app: &AppHandle, enabled: bool, settings: &Settings) -> Result<(), String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let (run_key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(run_key_path())
        .map_err(|err| err.to_string())?;

    if enabled {
        let command = launch_command(settings).map_err(|err| err.to_string())?;
        run_key
            .set_value(run_value_name(app), &command)
            .map_err(|err| err.to_string())?;
        log::info!(
            "Enabled Windows autostart start_minimized={}",
            settings.auto_start_minimized == Some(true)
        );
    } else {
        match run_key.delete_value(run_value_name(app)) {
            Ok(_) => {
                log::info!("Disabled Windows autostart");
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.to_string()),
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_enabled(_app: &AppHandle, enabled: bool, _settings: &Settings) -> Result<(), String> {
    if enabled {
        Err("native autostart is not implemented on this platform yet".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn launch_command(settings: &Settings) -> std::io::Result<String> {
    let exe = std::env::current_exe()?;
    let mut command = format!("\"{}\"", exe.display());
    if settings.auto_start_minimized == Some(true) {
        command.push_str(" --start-minimized");
    }
    Ok(command)
}

#[cfg(target_os = "windows")]
fn run_key_path() -> &'static str {
    r"Software\Microsoft\Windows\CurrentVersion\Run"
}

#[cfg(target_os = "windows")]
fn run_value_name(app: &AppHandle) -> &str {
    &app.package_info().name
}
