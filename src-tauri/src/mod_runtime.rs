mod assets;
mod commands;
mod legacy;
mod profile;
mod protocol;
mod secure_settings;
mod themes;

use serde_json::{Map, Value};
use std::{env, fs, io, path::Path};
use tauri::http::{header::CONTENT_TYPE, Response, StatusCode};
use tauri::AppHandle;

pub(crate) const EQUICORD_REPO: &str = "https://github.com/Equicord/Equicord";
pub(super) const MANAGED_RUNTIME_RELEASE_API_URL: &str =
    "https://api.github.com/repos/Equicord/Equicord/releases/latest";
pub(super) const MANAGED_RUNTIME_REPOSITORY_URL: &str = "https://github.com/Equicord/Equicord";
pub(super) const MANAGED_RUNTIME_RELEASE_OWNER: &str = "Equicord";
pub(super) const MANAGED_RUNTIME_RELEASE_REPO: &str = "Equicord";
pub(super) const MANAGED_RUNTIME_DIR_NAME: &str = "equicord-runtime";
pub(super) const MANAGED_RUNTIME_MANIFEST_NAME: &str = "equirust-runtime.json";
pub(super) const MANAGED_RUNTIME_REFRESH_INTERVAL_MILLIS: i64 = 6 * 60 * 60 * 1000;
pub(super) const MANAGED_RUNTIME_REQUIRED_FILE_NAMES: &[&str] = &["renderer.js", "renderer.css"];
pub(super) const MANAGED_RUNTIME_REQUIRED_ASSETS: &[(&str, &str)] = &[
    ("renderer.js", "renderer.js"),
    ("renderer.css", "renderer.css"),
];
pub(super) const MANAGED_RUNTIME_OPTIONAL_ASSETS: &[(&str, &str)] = &[
    ("renderer.js.map", "renderer.js.map"),
    ("renderer.css.map", "renderer.css.map"),
    ("renderer.js.LEGAL.txt", "renderer.js.LEGAL.txt"),
    ("plugins.json", "plugins.json"),
    ("vencordplugins.json", "vencordplugins.json"),
    ("equicordplugins.json", "equicordplugins.json"),
];
pub(super) const MINIMAL_MOD_PLUGIN_ALLOWLIST: &[&str] = &[
    "BadgeAPI",
    "CommandsAPI",
    "CrashHandler",
    "HeaderBarAPI",
    "MessageAccessoriesAPI",
    "MessageEventsAPI",
    "MessageUpdaterAPI",
    "NewPluginsManager",
    "Settings",
    "UserAreaAPI",
    "UserSettingsAPI",
    "WebKeybinds",
];
pub(super) const PROTECTED_VALUE_MARKER_KEY: &str = "__equirustProtected";
pub(super) const PROTECTED_VALUE_CIPHERTEXT_KEY: &str = "ciphertext";
pub(super) const PROTECTED_VALUE_MARKER: &str = "dpapi-v1";
pub(super) const TRUSTED_GITHUB_DOWNLOAD_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "github-releases.githubusercontent.com",
];

pub(crate) use assets::{
    managed_runtime_required_asset_names, refresh_managed_runtime, renderer_script,
    resolve_runtime_dir, resolve_runtime_resolution, runtime_dir_has_required_assets,
};
pub use legacy::seed_from_legacy_install;
pub use profile::{bridge_seed, resolve_mod_runtime_profile};
pub use protocol::handle_protocol;

pub use commands::VencordFileState;

pub(super) fn write_pretty_json(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    fs::write(path, format!("{json}\n"))
}

pub(super) fn read_text(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

pub(super) fn write_text(path: &Path, value: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, value)
}

pub(super) fn read_json(path: &Path) -> io::Result<Value> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }

    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(io::Error::other)
}

pub(super) fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        "application/json"
    }
}

pub(super) fn protocol_error(status: StatusCode, message: impl ToString) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.to_string().into_bytes())
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

pub(super) fn has_truthy_flag(arg_name: &str, env_name: &str) -> bool {
    env::args().any(|arg| arg == arg_name)
        || matches!(
            env::var(env_name).as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

pub(super) fn collect_arg_values(flag_name: &str) -> Vec<String> {
    let mut values = Vec::new();
    let args = env::args().collect::<Vec<_>>();
    let mut index = 0;

    while index < args.len() {
        let current = &args[index];
        if current == flag_name {
            if let Some(value) = args.get(index + 1) {
                values.push(value.clone());
                index += 2;
                continue;
            }
        } else if let Some(value) = current.strip_prefix(&format!("{flag_name}=")) {
            values.push(value.to_owned());
        }

        index += 1;
    }

    values
}

pub(super) fn join_names(values: &[String]) -> String {
    if values.is_empty() {
        return "-".into();
    }

    values.join(",")
}

pub(super) fn compat_disabled_plugins() -> &'static [&'static str] {
    &[]
}

pub(super) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[tauri::command]
pub fn get_vencord_renderer_css(app: AppHandle) -> Result<String, String> {
    commands::get_vencord_renderer_css(app)
}

#[tauri::command]
pub fn set_vencord_settings(
    settings: Value,
    app: AppHandle,
    path: Option<String>,
) -> Result<(), String> {
    commands::set_vencord_settings(settings, app, path)
}

#[tauri::command]
pub fn set_vencord_quick_css(css: String, app: AppHandle) -> Result<(), String> {
    commands::set_vencord_quick_css(css, app)
}

#[tauri::command]
pub fn get_vencord_quick_css(app: AppHandle) -> Result<String, String> {
    commands::get_vencord_quick_css(app)
}

#[tauri::command]
pub fn get_vencord_themes_list(app: AppHandle) -> Result<Vec<themes::VencordTheme>, String> {
    commands::get_vencord_themes_list(app)
}

#[tauri::command]
pub fn get_vencord_theme_entries(app: AppHandle) -> Result<Vec<themes::VencordThemeEntry>, String> {
    commands::get_vencord_theme_entries(app)
}

#[tauri::command]
pub fn upload_vencord_theme(app: AppHandle) -> Result<String, String> {
    commands::upload_vencord_theme(app)
}

#[tauri::command]
pub fn set_vencord_theme_data(
    file_name: String,
    content: String,
    app: AppHandle,
) -> Result<(), String> {
    commands::set_vencord_theme_data(file_name, content, app)
}

#[tauri::command]
pub fn get_vencord_theme_data(file_name: String, app: AppHandle) -> Result<String, String> {
    commands::get_vencord_theme_data(file_name, app)
}

#[tauri::command]
pub fn get_vencord_file_state(app: AppHandle) -> Result<VencordFileState, String> {
    commands::get_vencord_file_state(app)
}

#[tauri::command]
pub fn open_vencord_settings_folder(app: AppHandle) -> Result<(), String> {
    commands::open_vencord_settings_folder(app)
}

#[tauri::command]
pub fn get_vencord_settings_dir(app: AppHandle) -> Result<String, String> {
    commands::get_vencord_settings_dir(app)
}

#[tauri::command]
pub fn delete_vencord_theme(file_name: String, app: AppHandle) -> Result<String, String> {
    commands::delete_vencord_theme(file_name, app)
}

#[tauri::command]
pub fn open_vencord_themes_folder(app: AppHandle) -> Result<(), String> {
    commands::open_vencord_themes_folder(app)
}

#[tauri::command]
pub fn get_vencord_themes_dir(app: AppHandle) -> Result<String, String> {
    commands::get_vencord_themes_dir(app)
}

#[tauri::command]
pub fn open_vencord_quick_css(app: AppHandle) -> Result<(), String> {
    commands::open_vencord_quick_css(app)
}

#[tauri::command]
pub fn open_external_link(url: String, app: AppHandle) -> Result<(), String> {
    commands::open_external_link(url, app)
}
