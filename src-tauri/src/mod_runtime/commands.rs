use crate::{discord, file_manager, paths::AppPaths, privacy};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Url};

use super::{
    assets, read_text,
    secure_settings::write_secure_vencord_settings,
    themes::{
        compute_theme_revision, file_revision, read_theme_entries, read_theme_list,
        safe_theme_path, VencordTheme, VencordThemeEntry,
    },
    write_text,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VencordFileState {
    pub quick_css_revision: i64,
    pub themes_revision: i64,
    pub theme_count: usize,
}

pub fn get_vencord_renderer_css(app: AppHandle) -> Result<String, String> {
    let _ = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    assets::renderer_stylesheet(&app).map_err(|err| err.to_string())
}

pub fn set_vencord_settings(
    settings: Value,
    app: AppHandle,
    _path: Option<String>,
) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    write_secure_vencord_settings(&paths.vencord_settings_file, &settings)
        .map_err(|err| err.to_string())
}

pub fn set_vencord_quick_css(css: String, app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    write_text(&paths.vencord_quickcss_file, &css).map_err(|err| err.to_string())
}

pub fn get_vencord_quick_css(app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    read_text(&paths.vencord_quickcss_file).map_err(|err| err.to_string())
}

pub fn get_vencord_themes_list(app: AppHandle) -> Result<Vec<VencordTheme>, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    read_theme_list(&paths.vencord_themes_dir).map_err(|err| err.to_string())
}

pub fn get_vencord_theme_entries(app: AppHandle) -> Result<Vec<VencordThemeEntry>, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    read_theme_entries(&paths.vencord_themes_dir).map_err(|err| err.to_string())
}

pub fn upload_vencord_theme(app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let Some(selected_file) = rfd::FileDialog::new()
        .set_title("Import a Vencord theme")
        .add_filter("CSS Themes", &["css"])
        .pick_file()
    else {
        return Ok("cancelled".into());
    };

    let Some(file_name) = selected_file.file_name().and_then(|value| value.to_str()) else {
        return Ok("invalid".into());
    };
    let Some(target_path) = safe_theme_path(&paths.vencord_themes_dir, file_name) else {
        return Ok("invalid".into());
    };

    std::fs::copy(&selected_file, &target_path).map_err(|err| err.to_string())?;
    log::info!(
        "Imported theme {} from {}",
        privacy::file_name_for_log(&target_path),
        privacy::file_name_for_log(&selected_file)
    );
    Ok("ok".into())
}

pub fn set_vencord_theme_data(
    file_name: String,
    content: String,
    app: AppHandle,
) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let path = safe_theme_path(&paths.vencord_themes_dir, &file_name)
        .ok_or_else(|| "invalid theme path".to_owned())?;
    write_text(&path, &content).map_err(|err| err.to_string())
}

pub fn get_vencord_theme_data(file_name: String, app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let path = safe_theme_path(&paths.vencord_themes_dir, &file_name)
        .ok_or_else(|| "invalid theme path".to_string())?;

    std::fs::read_to_string(path).map_err(|err| err.to_string())
}

pub fn get_vencord_file_state(app: AppHandle) -> Result<VencordFileState, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let (themes_revision, theme_count) =
        compute_theme_revision(&paths.vencord_themes_dir).map_err(|err| err.to_string())?;

    Ok(VencordFileState {
        quick_css_revision: file_revision(&paths.vencord_quickcss_file)
            .map_err(|err| err.to_string())?,
        themes_revision,
        theme_count,
    })
}

pub fn open_vencord_settings_folder(app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    file_manager::open_path(&paths.vencord_settings_dir).map_err(|err| err.to_string())
}

pub fn get_vencord_settings_dir(app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    Ok(paths.vencord_settings_dir.display().to_string())
}

pub fn delete_vencord_theme(file_name: String, app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let Some(path) = safe_theme_path(&paths.vencord_themes_dir, &file_name) else {
        return Ok("invalid".into());
    };

    match std::fs::remove_file(&path) {
        Ok(()) => {
            log::info!("Deleted theme {}", privacy::file_name_for_log(&path));
            Ok("ok".into())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok("missing".into()),
        Err(err) => Err(err.to_string()),
    }
}

pub fn open_vencord_themes_folder(app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    file_manager::open_path(&paths.vencord_themes_dir).map_err(|err| err.to_string())
}

pub fn get_vencord_themes_dir(app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    Ok(paths.vencord_themes_dir.display().to_string())
}

pub fn open_vencord_quick_css(app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    file_manager::open_path(&paths.vencord_quickcss_file).map_err(|err| err.to_string())
}

pub fn open_external_link(url: String, app: AppHandle) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|err| err.to_string())?;
    discord::route_external_url(&app, &parsed);
    Ok(())
}
