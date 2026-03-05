use tauri::{AppHandle, Emitter, Manager};

pub const TOGGLE_MUTE_EVENT: &str = "equirust:voice-toggle-mute";
pub const TOGGLE_DEAFEN_EVENT: &str = "equirust:voice-toggle-deafen";

pub fn handle_second_instance(app: &AppHandle, args: &[String]) -> Result<bool, String> {
    let event = if args.iter().any(|arg| arg == "--toggle-mic") {
        Some(TOGGLE_MUTE_EVENT)
    } else if args.iter().any(|arg| arg == "--toggle-deafen") {
        Some(TOGGLE_DEAFEN_EVENT)
    } else {
        None
    };

    let Some(event) = event else {
        return Ok(false);
    };

    let Some(window) = app.get_webview_window("main") else {
        return Err("main window is not available for voice toggle dispatch".to_owned());
    };

    window.emit(event, ()).map_err(|err| err.to_string())?;
    Ok(true)
}
