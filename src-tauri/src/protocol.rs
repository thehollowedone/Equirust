use crate::{discord, privacy, tray};
use tauri::{AppHandle, Url};
use tauri_plugin_deep_link::DeepLinkExt;

pub fn init(app: &AppHandle) -> Result<(), String> {
    let deep_link = app.deep_link();

    if let Err(err) = deep_link.register_all() {
        log::warn!("Failed to register deep link protocols: {err}");
    }

    if let Ok(Some(urls)) = deep_link.get_current() {
        for url in urls {
            if let Err(err) = handle_open_url(app, &url) {
                log::warn!(
                    "Failed to handle startup deep link {}: {err}",
                    privacy::sanitize_url_for_log(url.as_str())
                );
            }
        }
    }

    let app_handle = app.clone();
    deep_link.on_open_url(move |event| {
        for url in event.urls() {
            if let Err(err) = handle_open_url(&app_handle, &url) {
                log::warn!(
                    "Failed to handle deep link {}: {err}",
                    privacy::sanitize_url_for_log(url.as_str())
                );
            }
        }
    });

    Ok(())
}

fn handle_open_url(app: &AppHandle, url: &Url) -> Result<(), String> {
    if url.scheme() != "discord" {
        return Ok(());
    }

    log::info!(
        "Handling deep link {}",
        privacy::sanitize_url_for_log(url.as_str())
    );
    tray::restore_main_window(app)?;
    discord::navigate_main_window_to_route(app, Some(discord::route_from_url(url)?))?;
    Ok(())
}
