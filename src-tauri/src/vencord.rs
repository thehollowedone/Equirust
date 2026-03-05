use crate::{
    autostart, discord, file_manager, paths::AppPaths, privacy, settings::Settings,
    store::PersistedStore,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{
    http::{header::CONTENT_TYPE, Response, StatusCode},
    AppHandle, Manager, Runtime, Url,
};

const VENCORD_REPO: &str = "https://github.com/Equicord/Equicord";
const MANAGED_RUNTIME_RELEASE_API_URL: &str =
    "https://api.github.com/repos/Equicord/Equicord/releases/latest";
const MANAGED_RUNTIME_REPOSITORY_URL: &str = "https://github.com/Equicord/Equicord";
const MANAGED_RUNTIME_DIR_NAME: &str = "equicord-runtime";
const MANAGED_RUNTIME_MANIFEST_NAME: &str = "equirust-runtime.json";
const MANAGED_RUNTIME_REFRESH_INTERVAL_MILLIS: i64 = 6 * 60 * 60 * 1000;
const MANAGED_RUNTIME_REQUIRED_ASSETS: &[(&str, &str)] = &[
    ("renderer.js", "renderer.js"),
    ("renderer.css", "renderer.css"),
    ("patcher.js", "patcher.js"),
    ("preload.js", "preload.js"),
];
const MANAGED_RUNTIME_OPTIONAL_ASSETS: &[(&str, &str)] = &[
    ("renderer.js.map", "renderer.js.map"),
    ("renderer.css.map", "renderer.css.map"),
    ("renderer.js.LEGAL.txt", "renderer.js.LEGAL.txt"),
    ("patcher.js.map", "patcher.js.map"),
    ("patcher.js.LEGAL.txt", "patcher.js.LEGAL.txt"),
    ("preload.js.map", "preload.js.map"),
    ("plugins.json", "plugins.json"),
    ("vencordplugins.json", "vencordplugins.json"),
    ("equicordplugins.json", "equicordplugins.json"),
];
const MINIMAL_MOD_PLUGIN_ALLOWLIST: &[&str] = &[
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
const PROTECTED_VALUE_MARKER_KEY: &str = "__equirustProtected";
const PROTECTED_VALUE_CIPHERTEXT_KEY: &str = "ciphertext";
const PROTECTED_VALUE_MARKER: &str = "dpapi-v1";

#[derive(Debug, Clone, Default)]
pub struct ModRuntimeProfile {
    pub minimal: bool,
    pub disabled_plugins: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VencordTheme {
    pub file_name: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub version: Option<String>,
    pub license: Option<String>,
    pub source: Option<String>,
    pub website: Option<String>,
    pub invite: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VencordFileState {
    pub quick_css_revision: i64,
    pub themes_revision: i64,
    pub theme_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedRuntimeManifest {
    version: String,
    source: String,
    asset_family: String,
    checked_at: Option<i64>,
    required_files: Option<BTreeMap<String, String>>,
}

pub fn seed_from_legacy_install(paths: &AppPaths) -> io::Result<()> {
    let legacy_dirs = legacy_equibop_dirs();

    copy_first_missing(
        &paths.settings_file,
        legacy_dirs.iter().map(|dir| dir.join("settings.json")),
    )?;
    copy_first_missing(
        &paths.state_file,
        legacy_dirs.iter().map(|dir| dir.join("state.json")),
    )?;
    copy_first_missing(
        &paths.vencord_settings_file,
        legacy_dirs
            .iter()
            .map(|dir| dir.join("settings").join("settings.json"))
            .chain(
                vencord_legacy_dir()
                    .into_iter()
                    .map(|dir| dir.join("settings").join("settings.json")),
            ),
    )?;
    copy_first_missing(
        &paths.vencord_quickcss_file,
        legacy_dirs
            .iter()
            .map(|dir| dir.join("settings").join("quickCss.css"))
            .chain(
                vencord_legacy_dir()
                    .into_iter()
                    .map(|dir| dir.join("settings").join("quickCss.css")),
            ),
    )?;

    if !has_css_files(&paths.vencord_themes_dir)? {
        for source_dir in legacy_dirs.iter().map(|dir| dir.join("themes")).chain(
            vencord_legacy_dir()
                .into_iter()
                .map(|dir| dir.join("themes")),
        ) {
            copy_theme_dir_if_present(&source_dir, &paths.vencord_themes_dir)?;
            if has_css_files(&paths.vencord_themes_dir)? {
                break;
            }
        }
    }

    Ok(())
}

pub fn bridge_seed(
    app: &AppHandle,
    paths: &AppPaths,
    host_settings: &Settings,
    mod_runtime_profile: &ModRuntimeProfile,
) -> io::Result<Value> {
    let settings = read_secure_vencord_settings(&paths.vencord_settings_file)?;
    let settings = apply_mod_runtime_profile(settings, mod_runtime_profile);
    let quick_css = read_text(&paths.vencord_quickcss_file)?;
    let host_settings = serde_json::to_value(host_settings).map_err(io::Error::other)?;
    let native_autostart_enabled =
        autostart::get_auto_start_status(app.clone()).unwrap_or_else(|err| {
            log::warn!("Failed to read native autostart status: {}", err);
            false
        });

    Ok(Value::Object(Map::from_iter([
        ("settings".into(), settings),
        ("hostSettings".into(), host_settings),
        (
            "nativeAutoStartEnabled".into(),
            Value::Bool(native_autostart_enabled),
        ),
        ("debugBuild".into(), Value::Bool(cfg!(debug_assertions))),
        ("quickCss".into(), Value::String(quick_css)),
        (
            "versions".into(),
            Value::Object(Map::from_iter([
                (
                    "equirust".into(),
                    Value::String(app.package_info().version.to_string()),
                ),
                (
                    "webview".into(),
                    Value::String(
                        tauri::webview_version().unwrap_or_else(|_| "unknown".to_owned()),
                    ),
                ),
                ("tauri".into(), Value::String(tauri::VERSION.to_string())),
                (
                    "platform".into(),
                    Value::String(env::consts::OS.to_string()),
                ),
                ("arch".into(), Value::String(env::consts::ARCH.to_string())),
                ("vencordRepo".into(), Value::String(VENCORD_REPO.into())),
            ])),
        ),
    ])))
}

pub fn resolve_mod_runtime_profile() -> ModRuntimeProfile {
    let mut profile = ModRuntimeProfile {
        minimal: has_truthy_flag("--minimal-mod-runtime", "EQUIRUST_MINIMAL_MOD_RUNTIME"),
        disabled_plugins: BTreeSet::new(),
    };

    for value in collect_arg_values("--disable-vencord-plugin") {
        for plugin_name in value.split(',') {
            let plugin_name = plugin_name.trim();
            if plugin_name.is_empty() {
                continue;
            }

            profile.disabled_plugins.insert(plugin_name.to_owned());
        }
    }

    if let Ok(raw) = env::var("EQUIRUST_DISABLE_VENCORD_PLUGINS") {
        for plugin_name in raw.split(',') {
            let plugin_name = plugin_name.trim();
            if plugin_name.is_empty() {
                continue;
            }

            profile.disabled_plugins.insert(plugin_name.to_owned());
        }
    }

    profile
}

pub fn renderer_script<R: Runtime>(app: &AppHandle<R>) -> io::Result<String> {
    load_runtime_assets(Some(app)).map(|runtime| runtime.renderer_js)
}

pub fn handle_protocol<R: Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let paths = match AppPaths::resolve(ctx.app_handle()) {
        Ok(paths) => paths,
        Err(err) => return protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    let raw_path = request.uri().path().trim_start_matches('/');
    match raw_path {
        path if path.starts_with("themes/") => {
            let file_name = &path["themes/".len()..];
            match safe_theme_path(&paths.vencord_themes_dir, file_name) {
                Some(theme_path) => match fs::read(theme_path) {
                    Ok(contents) => Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "text/css; charset=utf-8")
                        .body(contents)
                        .unwrap_or_else(|err| {
                            protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                        }),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        protocol_error(StatusCode::NOT_FOUND, "theme not found")
                    }
                    Err(err) => protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
                },
                None => protocol_error(StatusCode::BAD_REQUEST, "invalid theme path"),
            }
        }
        "renderer.css.map" | "renderer.js.map" | "preload.js.map" | "patcher.js.map" => {
            let runtime = match resolve_runtime_dir(Some(ctx.app_handle())) {
                Ok(path) => path,
                Err(err) => {
                    return protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
            };

            match fs::read(runtime.join(raw_path)) {
                Ok(contents) => Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, content_type_for(raw_path))
                    .body(contents)
                    .unwrap_or_else(|err| {
                        protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                    }),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    protocol_error(StatusCode::NOT_FOUND, "asset not found")
                }
                Err(err) => protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        _ => protocol_error(StatusCode::NOT_FOUND, "unsupported vencord asset"),
    }
}

#[tauri::command]
pub fn get_vencord_renderer_css(app: AppHandle) -> Result<String, String> {
    let _ = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    load_runtime_assets(Some(&app))
        .map(|runtime| runtime.renderer_css)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_vencord_settings(
    settings: Value,
    app: AppHandle,
    _path: Option<String>,
) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    write_secure_vencord_settings(&paths.vencord_settings_file, &settings).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_vencord_quick_css(css: String, app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    write_text(&paths.vencord_quickcss_file, &css).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn get_vencord_quick_css(app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    read_text(&paths.vencord_quickcss_file).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn get_vencord_themes_list(app: AppHandle) -> Result<Vec<VencordTheme>, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    read_theme_list(&paths.vencord_themes_dir).map_err(|err| err.to_string())
}

#[tauri::command]
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

    fs::copy(&selected_file, &target_path).map_err(|err| err.to_string())?;
    log::info!(
        "Imported theme {} from {}",
        privacy::file_name_for_log(&target_path),
        privacy::file_name_for_log(&selected_file)
    );
    Ok("ok".into())
}

#[tauri::command]
pub fn get_vencord_theme_data(file_name: String, app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let path = safe_theme_path(&paths.vencord_themes_dir, &file_name)
        .ok_or_else(|| "invalid theme path".to_string())?;

    fs::read_to_string(path).map_err(|err| err.to_string())
}

#[tauri::command]
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

#[tauri::command]
pub fn open_vencord_settings_folder(app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    file_manager::open_path(&paths.vencord_settings_dir).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn delete_vencord_theme(file_name: String, app: AppHandle) -> Result<String, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let Some(path) = safe_theme_path(&paths.vencord_themes_dir, &file_name) else {
        return Ok("invalid".into());
    };

    match fs::remove_file(&path) {
        Ok(()) => {
            log::info!("Deleted theme {}", privacy::file_name_for_log(&path));
            Ok("ok".into())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok("missing".into()),
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub fn open_vencord_themes_folder(app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    file_manager::open_path(&paths.vencord_themes_dir).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn open_vencord_quick_css(app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    file_manager::open_path(&paths.vencord_quickcss_file).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn open_external_link(url: String, app: AppHandle) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|err| err.to_string())?;
    discord::route_external_url(&app, &parsed);
    Ok(())
}

struct RuntimeAssets {
    renderer_js: String,
    renderer_css: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RuntimeSource {
    EnvOverride,
    CustomDir,
    ManagedFallback,
}

impl RuntimeSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::EnvOverride => "env-override",
            Self::CustomDir => "custom-dir",
            Self::ManagedFallback => "managed-equicord-cache",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeResolution {
    pub path: PathBuf,
    pub source: RuntimeSource,
}

fn load_runtime_assets<R: Runtime>(app: Option<&AppHandle<R>>) -> io::Result<RuntimeAssets> {
    let runtime_dir = resolve_runtime_dir(app)?;

    Ok(RuntimeAssets {
        renderer_js: fs::read_to_string(runtime_dir.join("renderer.js"))?,
        renderer_css: fs::read_to_string(runtime_dir.join("renderer.css"))?,
    })
}

pub(crate) fn resolve_runtime_dir<R: Runtime>(app: Option<&AppHandle<R>>) -> io::Result<PathBuf> {
    resolve_runtime_resolution(app).map(|runtime| runtime.path)
}

pub(crate) fn resolve_runtime_resolution<R: Runtime>(
    app: Option<&AppHandle<R>>,
) -> io::Result<RuntimeResolution> {
    if let Some(path) = env::var_os("EQUIRUST_VENCORD_DIST_DIR") {
        let path = PathBuf::from(path);
        if path.join("renderer.js").exists() {
            return Ok(RuntimeResolution {
                path,
                source: RuntimeSource::EnvOverride,
            });
        }
    }

    if let Some(store) = app.and_then(|app| app.try_state::<PersistedStore>()) {
        if let Some(path) = file_manager::resolve_custom_runtime_dir(&store) {
            if path.join("renderer.js").exists() {
                return Ok(RuntimeResolution {
                    path,
                    source: RuntimeSource::CustomDir,
                });
            }
        }
    }

    if !has_truthy_flag(
        "--disable-managed-runtime",
        "EQUIRUST_DISABLE_MANAGED_RUNTIME",
    ) {
        if let Some(app) = app {
            match ensure_managed_runtime(app) {
                Ok(path) => {
                    return Ok(RuntimeResolution {
                        path,
                        source: RuntimeSource::ManagedFallback,
                    });
                }
                Err(err) => log::warn!("Managed Equicord runtime refresh failed: {}", err),
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "No compatible runtime was found. Wait for the managed Equicord runtime download or set EQUIRUST_VENCORD_DIST_DIR.",
    ))
}

pub(crate) fn refresh_managed_runtime<R: Runtime>(app: &AppHandle<R>) -> io::Result<PathBuf> {
    ensure_managed_runtime(app)
}

fn ensure_managed_runtime<R: Runtime>(app: &AppHandle<R>) -> io::Result<PathBuf> {
    let paths = AppPaths::resolve(app)?;
    let root = paths.app_cache_dir.join(MANAGED_RUNTIME_DIR_NAME);
    let current_dir = root.join("current");

    let existing_manifest = read_managed_runtime_manifest(&current_dir);
    let current_is_valid = runtime_dir_is_valid(&current_dir);
    let now = now_millis();
    let current_manifest_valid = existing_manifest
        .as_ref()
        .map(|manifest| managed_manifest_matches_files(&current_dir, manifest))
        .unwrap_or(false);

    if current_is_valid
        && current_manifest_valid
        && existing_manifest
            .as_ref()
            .map(|manifest| {
                manifest.asset_family == "desktop-dist"
                    && manifest
                        .checked_at
                        .map(|checked_at| now.saturating_sub(checked_at) < MANAGED_RUNTIME_REFRESH_INTERVAL_MILLIS)
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    {
        return Ok(current_dir);
    }

    let release = match fetch_latest_runtime_release() {
        Ok(release) => release,
        Err(err) => {
            if current_is_valid {
                return Ok(current_dir);
            }
            return Err(err);
        }
    };

    let raw_version = release.tag_name.trim().trim_start_matches('v').to_owned();
    let version = if raw_version.is_empty() || raw_version.eq_ignore_ascii_case("latest") {
        None
    } else {
        Some(raw_version)
    };
    let managed_version = version.unwrap_or_else(|| "latest".to_owned());
    if current_is_valid
        && current_manifest_valid
        && existing_manifest
            .as_ref()
            .map(|manifest| {
                manifest.version == managed_version && manifest.asset_family == "desktop-dist"
            })
            .unwrap_or(false)
    {
        let _ = write_pretty_json(
            &current_dir.join(MANAGED_RUNTIME_MANIFEST_NAME),
            &serde_json::json!(ManagedRuntimeManifest {
                version: managed_version,
                source: MANAGED_RUNTIME_RELEASE_API_URL.to_owned(),
                asset_family: "desktop-dist".to_owned(),
                checked_at: Some(now),
                required_files: compute_required_asset_hashes(&current_dir).ok(),
            }),
        );
        return Ok(current_dir);
    }

    fs::create_dir_all(&root)?;
    let staging_dir = root.join(format!("staging-{managed_version}"));
    if staging_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir);
    }
    fs::create_dir_all(&staging_dir)?;

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .user_agent(discord::standard_http_user_agent())
        .build()
        .map_err(io::Error::other)?;

    for (asset_name, target_name) in MANAGED_RUNTIME_REQUIRED_ASSETS {
        let asset = select_named_asset(&release.assets, asset_name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "Managed runtime asset {asset_name} was not present in the Equicord release"
                ),
            )
        })?;
        download_asset(&client, asset, &staging_dir.join(target_name))?;
    }
    if !runtime_dir_is_valid(&staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Managed runtime staging directory is missing required runtime assets",
        ));
    }
    let required_files = compute_required_asset_hashes(&staging_dir)?;

    for (asset_name, target_name) in MANAGED_RUNTIME_OPTIONAL_ASSETS {
        if let Some(asset) = select_named_asset(&release.assets, asset_name) {
            let _ = download_asset(&client, asset, &staging_dir.join(target_name));
        }
    }

    let managed_version = detect_managed_runtime_build(&staging_dir).unwrap_or(managed_version);

    write_pretty_json(
        &staging_dir.join("package.json"),
        &serde_json::json!({
            "name": "equicord-runtime",
            "version": managed_version,
            "repository": MANAGED_RUNTIME_REPOSITORY_URL,
        }),
    )?;
    write_pretty_json(
        &staging_dir.join(MANAGED_RUNTIME_MANIFEST_NAME),
        &serde_json::json!(ManagedRuntimeManifest {
            version: managed_version,
            source: MANAGED_RUNTIME_RELEASE_API_URL.to_owned(),
            asset_family: "desktop-dist".to_owned(),
            checked_at: Some(now),
            required_files: Some(required_files),
        }),
    )?;

    let backup_dir = root.join("previous");
    if backup_dir.exists() {
        let _ = fs::remove_dir_all(&backup_dir);
    }
    let current_existed = current_dir.exists();
    if current_existed {
        fs::rename(&current_dir, &backup_dir)?;
    }
    match fs::rename(&staging_dir, &current_dir) {
        Ok(()) => {
            if backup_dir.exists() {
                let _ = fs::remove_dir_all(&backup_dir);
            }
        }
        Err(err) => {
            if current_existed && backup_dir.exists() && !current_dir.exists() {
                let _ = fs::rename(&backup_dir, &current_dir);
            }
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(err);
        }
    }
    log::info!(
        "Prepared managed Equicord runtime at {}",
        privacy::file_name_for_log(&current_dir)
    );
    Ok(current_dir)
}

fn fetch_latest_runtime_release() -> io::Result<GithubRelease> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent(discord::standard_http_user_agent())
        .build()
        .map_err(io::Error::other)?
        .get(MANAGED_RUNTIME_RELEASE_API_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(io::Error::other)?
        .json::<GithubRelease>()
        .map_err(io::Error::other)
}

fn select_named_asset<'a>(assets: &'a [GithubAsset], name: &str) -> Option<&'a GithubAsset> {
    assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(name))
}

fn download_asset(
    client: &reqwest::blocking::Client,
    asset: &GithubAsset,
    target_path: &Path,
) -> io::Result<()> {
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(io::Error::other)?
        .bytes()
        .map_err(io::Error::other)?;
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = target_path.with_extension("part");
    fs::write(&temp_path, &bytes)?;
    fs::rename(&temp_path, target_path)
}

fn runtime_dir_is_valid(path: &Path) -> bool {
    MANAGED_RUNTIME_REQUIRED_ASSETS.iter().all(|(_, target_name)| {
        fs::metadata(path.join(target_name))
            .map(|metadata| metadata.is_file() && metadata.len() > 0)
            .unwrap_or(false)
    })
}

fn read_managed_runtime_manifest(path: &Path) -> Option<ManagedRuntimeManifest> {
    let contents = fs::read_to_string(path.join(MANAGED_RUNTIME_MANIFEST_NAME)).ok()?;
    serde_json::from_str(&contents).ok()
}

fn managed_manifest_matches_files(path: &Path, manifest: &ManagedRuntimeManifest) -> bool {
    if manifest.asset_family != "desktop-dist" || !runtime_dir_is_valid(path) {
        return false;
    }

    match manifest.required_files.as_ref() {
        Some(required_files) if !required_files.is_empty() => {
            for (file_name, expected_hash) in required_files {
                let Ok(actual_hash) = sha256_file(&path.join(file_name)) else {
                    return false;
                };
                if actual_hash != *expected_hash {
                    return false;
                }
            }
            true
        }
        _ => true,
    }
}

fn compute_required_asset_hashes(path: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    for (_, target_name) in MANAGED_RUNTIME_REQUIRED_ASSETS {
        hashes.insert((*target_name).to_owned(), sha256_file(&path.join(target_name))?);
    }
    Ok(hashes)
}

fn sha256_file(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};

    let bytes = fs::read(path)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

fn detect_managed_runtime_build(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path.join("renderer.js")).ok()?;
    let line = contents.lines().next()?.trim();
    ["// Vencord ", "// Equicord "]
        .into_iter()
        .find_map(|prefix| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn legacy_equibop_dirs() -> Vec<PathBuf> {
    let Some(app_data) = env::var_os("APPDATA").map(PathBuf::from) else {
        return Vec::new();
    };

    vec![app_data.join("Equibop"), app_data.join("equibop")]
}

fn vencord_legacy_dir() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|dir| dir.join("Vencord"))
}

fn copy_first_missing<I>(target: &Path, sources: I) -> io::Result<()>
where
    I: IntoIterator<Item = PathBuf>,
{
    if target.exists() && fs::metadata(target)?.len() > 0 {
        return Ok(());
    }

    for source in sources {
        if !source.exists() {
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(&source, target)?;
        log::info!(
            "Imported {} from {}",
            privacy::file_name_for_log(target),
            privacy::file_name_for_log(&source)
        );
        break;
    }

    Ok(())
}

fn copy_theme_dir_if_present(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    if !source_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("css") {
            continue;
        }

        let Some(file_name) = path.file_name() else {
            continue;
        };

        let target_path = target_dir.join(file_name);
        if target_path.exists() {
            continue;
        }

        fs::copy(&path, &target_path)?;
        log::info!(
            "Imported theme {}",
            privacy::file_name_for_log(&target_path)
        );
    }

    Ok(())
}

fn has_css_files(dir: &Path) -> io::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("css") {
            return Ok(true);
        }
    }

    Ok(false)
}

fn read_json(path: &Path) -> io::Result<Value> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }

    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(io::Error::other)
}

fn read_secure_vencord_settings(path: &Path) -> io::Result<Value> {
    let mut raw = read_json(path)?;
    decrypt_sensitive_values(&mut raw);

    let mut protected_copy = raw.clone();
    protect_sensitive_values(None, &mut protected_copy);
    if protected_copy != read_json(path)? {
        write_pretty_json(path, &protected_copy)?;
    }

    Ok(raw)
}

fn write_secure_vencord_settings(path: &Path, settings: &Value) -> io::Result<()> {
    let mut protected = settings.clone();
    prune_empty_sensitive_values(&mut protected);
    protect_sensitive_values(None, &mut protected);
    write_pretty_json(path, &protected)
}

fn prune_empty_sensitive_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if is_protected_value_object(map) {
                return;
            }

            for child in map.values_mut() {
                prune_empty_sensitive_values(child);
            }

            map.retain(|key, child| {
                if is_sensitive_setting_key(key) && is_empty_secret_value(child) {
                    return false;
                }
                true
            });
        }
        Value::Array(values) => {
            for child in values.iter_mut() {
                prune_empty_sensitive_values(child);
            }
        }
        _ => {}
    }
}

fn is_empty_secret_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

fn protect_sensitive_values(current_key: Option<&str>, value: &mut Value) {
    match value {
        Value::Object(map) => {
            if is_protected_value_object(map) {
                return;
            }

            for (key, child) in map.iter_mut() {
                protect_sensitive_values(Some(key), child);
            }
        }
        Value::Array(values) => {
            for child in values.iter_mut() {
                protect_sensitive_values(current_key, child);
            }
        }
        Value::String(text) => {
            let Some(key) = current_key else {
                return;
            };
            if !is_sensitive_setting_key(key) || text.trim().is_empty() {
                return;
            }

            match protect_secret_value(text) {
                Ok(ciphertext) => {
                    *value = Value::Object(Map::from_iter([
                        (
                            PROTECTED_VALUE_MARKER_KEY.to_owned(),
                            Value::String(PROTECTED_VALUE_MARKER.to_owned()),
                        ),
                        (
                            PROTECTED_VALUE_CIPHERTEXT_KEY.to_owned(),
                            Value::String(ciphertext),
                        ),
                    ]));
                }
                Err(err) => {
                    log::warn!(
                        "Failed to protect sensitive setting key {}: {}",
                        key,
                        err
                    );
                }
            }
        }
        _ => {}
    }
}

fn decrypt_sensitive_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(ciphertext) = extract_protected_ciphertext(map) {
                match unprotect_secret_value(ciphertext) {
                    Ok(plaintext) => {
                        *value = Value::String(plaintext);
                    }
                    Err(err) => {
                        log::warn!("Failed to decrypt protected Vencord setting: {}", err);
                        *value = Value::String(String::new());
                    }
                }
                return;
            }

            for child in map.values_mut() {
                decrypt_sensitive_values(child);
            }
        }
        Value::Array(values) => {
            for child in values.iter_mut() {
                decrypt_sensitive_values(child);
            }
        }
        _ => {}
    }
}

fn is_protected_value_object(map: &Map<String, Value>) -> bool {
    matches!(
        (
            map.get(PROTECTED_VALUE_MARKER_KEY).and_then(Value::as_str),
            map.get(PROTECTED_VALUE_CIPHERTEXT_KEY).and_then(Value::as_str),
        ),
        (Some(PROTECTED_VALUE_MARKER), Some(_))
    )
}

fn extract_protected_ciphertext(map: &Map<String, Value>) -> Option<&str> {
    match (
        map.get(PROTECTED_VALUE_MARKER_KEY).and_then(Value::as_str),
        map.get(PROTECTED_VALUE_CIPHERTEXT_KEY).and_then(Value::as_str),
    ) {
        (Some(PROTECTED_VALUE_MARKER), Some(ciphertext)) => Some(ciphertext),
        _ => None,
    }
}

fn is_sensitive_setting_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() {
        return false;
    }

    if matches!(
        key.as_str(),
        "keyboard" | "hotkey" | "keybind" | "keybinds" | "shortcut" | "shortcuts"
    ) {
        return false;
    }

    if matches!(
        key.as_str(),
        "token"
            | "secret"
            | "password"
            | "passphrase"
            | "apikey"
            | "api_key"
            | "clientsecret"
            | "client_secret"
            | "authorization"
            | "auth"
            | "oauth"
            | "webhook"
            | "key"
    ) {
        return true;
    }

    [
        "token",
        "secret",
        "password",
        "passphrase",
        "apikey",
        "api_key",
        "api-key",
        "clientsecret",
        "client_secret",
        "authorization",
        "oauth",
        "webhook",
    ]
    .iter()
    .any(|needle| {
        key.ends_with(needle)
            || key.contains(&format!("_{needle}"))
            || key.contains(&format!("-{needle}"))
    })
}

#[cfg(target_os = "windows")]
fn protect_secret_value(value: &str) -> io::Result<String> {
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use windows::Win32::Foundation::{HLOCAL, LocalFree};

    let mut input = value.as_bytes().to_vec();
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_mut_ptr(),
    };
    let mut output_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &input_blob,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output_blob,
        )
        .map_err(|err| io::Error::other(format!("CryptProtectData failed: {err}")))?;

        let bytes = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let _ = LocalFree(Some(HLOCAL(output_blob.pbData.cast())));
        Ok(encoded)
    }
}

#[cfg(not(target_os = "windows"))]
fn protect_secret_value(_value: &str) -> io::Result<String> {
    Err(io::Error::other(
        "secret encryption is not available on this platform",
    ))
}

#[cfg(target_os = "windows")]
fn unprotect_secret_value(value: &str) -> io::Result<String> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use windows::Win32::Foundation::{HLOCAL, LocalFree};

    let mut ciphertext = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(io::Error::other)?;
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_mut_ptr(),
    };
    let mut output_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(
            &input_blob,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output_blob,
        )
        .map_err(|err| io::Error::other(format!("CryptUnprotectData failed: {err}")))?;

        let bytes = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
        let plaintext = String::from_utf8(bytes.to_vec()).map_err(io::Error::other)?;
        let _ = LocalFree(Some(HLOCAL(output_blob.pbData.cast())));
        Ok(plaintext)
    }
}

#[cfg(not(target_os = "windows"))]
fn unprotect_secret_value(_value: &str) -> io::Result<String> {
    Err(io::Error::other(
        "secret decryption is not available on this platform",
    ))
}

fn apply_mod_runtime_profile(mut settings: Value, profile: &ModRuntimeProfile) -> Value {
    let Some(plugins) = settings.get_mut("plugins").and_then(Value::as_object_mut) else {
        return settings;
    };

    let disabled_plugin_names = profile
        .disabled_plugins
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let compat_disabled_plugin_names = compat_disabled_plugins()
        .into_iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut enabled_before = Vec::new();
    let mut enabled_after = Vec::new();
    let mut disabled_by_profile = Vec::new();

    for (plugin_name, plugin_settings) in plugins.iter_mut() {
        let Some(plugin_settings) = plugin_settings.as_object_mut() else {
            continue;
        };

        let was_enabled = plugin_settings
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !was_enabled {
            continue;
        }

        enabled_before.push(plugin_name.clone());

        let explicitly_disabled = disabled_plugin_names.contains(&plugin_name.to_ascii_lowercase());
        let disabled_by_minimal =
            profile.minimal && !MINIMAL_MOD_PLUGIN_ALLOWLIST.contains(&plugin_name.as_str());
        let disabled_by_compat =
            compat_disabled_plugin_names.contains(&plugin_name.to_ascii_lowercase());
        if explicitly_disabled || disabled_by_minimal || disabled_by_compat {
            plugin_settings.insert("enabled".into(), Value::Bool(false));
            disabled_by_profile.push(plugin_name.clone());
            continue;
        }

        enabled_after.push(plugin_name.clone());
    }

    if profile.minimal || !disabled_by_profile.is_empty() {
        log::info!(
            "Vencord mod profile applied: minimal={} enabled_before={} enabled_after={} disabled={}",
            profile.minimal,
            join_names(&enabled_before),
            join_names(&enabled_after),
            join_names(&disabled_by_profile)
        );
    }

    settings
}

fn write_pretty_json(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    fs::write(path, format!("{json}\n"))
}

fn read_text(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

fn write_text(path: &Path, value: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, value)
}

fn read_theme_list(dir: &Path) -> io::Result<Vec<VencordTheme>> {
    let mut themes = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("css") {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        let contents = fs::read_to_string(&path)?;
        themes.push(parse_theme_metadata(&file_name, &contents));
    }

    themes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(themes)
}

fn file_revision(path: &Path) -> io::Result<i64> {
    match fs::metadata(path) {
        Ok(metadata) => modified_millis(&metadata),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(-1),
        Err(err) => Err(err),
    }
}

fn compute_theme_revision(dir: &Path) -> io::Result<(i64, usize)> {
    let mut latest_revision = -1_i64;
    let mut count = 0_usize;

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("css") {
            continue;
        }

        count += 1;
        let revision = modified_millis(&entry.metadata()?)?;
        latest_revision = latest_revision.max(revision);
    }

    Ok((latest_revision, count))
}

fn modified_millis(metadata: &fs::Metadata) -> io::Result<i64> {
    let modified = metadata.modified()?;
    let duration = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(duration.as_millis() as i64)
}

fn parse_theme_metadata(file_name: &str, contents: &str) -> VencordTheme {
    let mut theme = VencordTheme {
        file_name: file_name.to_string(),
        name: file_name.trim_end_matches(".css").to_string(),
        author: "Unknown Author".into(),
        description: "A Discord theme.".into(),
        version: None,
        license: None,
        source: None,
        website: None,
        invite: None,
    };

    let Some(start) = contents.find("/**") else {
        return theme;
    };
    let Some(end) = contents[start + 3..].find("*/") else {
        return theme;
    };
    let block = &contents[start + 3..start + 3 + end];

    let mut current_key = String::new();
    let mut current_value = String::new();
    let flush = |key: &str, value: &str, theme: &mut VencordTheme| {
        let value = value.trim();
        if value.is_empty() {
            return;
        }

        match key {
            "name" => theme.name = value.to_string(),
            "author" => theme.author = value.to_string(),
            "description" => theme.description = value.to_string(),
            "version" => theme.version = Some(value.to_string()),
            "license" => theme.license = Some(value.to_string()),
            "source" => theme.source = Some(value.to_string()),
            "website" => theme.website = Some(value.to_string()),
            "invite" => theme.invite = Some(value.to_string()),
            _ => {}
        }
    };

    for raw_line in block.lines() {
        let line = raw_line.trim().trim_start_matches('*').trim();
        if let Some(stripped) = line.strip_prefix('@') {
            flush(&current_key, &current_value, &mut theme);
            if let Some((key, value)) = stripped.split_once(' ') {
                current_key = key.to_string();
                current_value = value.trim().to_string();
            } else {
                current_key = stripped.to_string();
                current_value.clear();
            }
        } else if !line.is_empty() {
            if !current_value.is_empty() {
                current_value.push('\n');
            }
            current_value.push_str(line);
        }
    }

    flush(&current_key, &current_value, &mut theme);
    theme
}

fn safe_theme_path(dir: &Path, file_name: &str) -> Option<PathBuf> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || !file_name.ends_with(".css")
    {
        return None;
    }

    Some(dir.join(file_name))
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        "application/json"
    }
}

fn protocol_error(status: StatusCode, message: impl ToString) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.to_string().into_bytes())
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

fn has_truthy_flag(arg_name: &str, env_name: &str) -> bool {
    env::args().any(|arg| arg == arg_name)
        || matches!(
            env::var(env_name).as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

fn collect_arg_values(flag_name: &str) -> Vec<String> {
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

fn join_names(values: &[String]) -> String {
    if values.is_empty() {
        return "-".into();
    }

    values.join(",")
}

fn compat_disabled_plugins() -> &'static [&'static str] {
    &[]
}

#[cfg(test)]
mod tests {
    use super::{
        extract_protected_ciphertext, is_protected_value_object, is_sensitive_setting_key,
        prune_empty_sensitive_values,
        PROTECTED_VALUE_CIPHERTEXT_KEY, PROTECTED_VALUE_MARKER, PROTECTED_VALUE_MARKER_KEY,
    };
    use serde_json::{json, Map, Value};

    #[test]
    fn detects_sensitive_setting_keys() {
        assert!(is_sensitive_setting_key("token"));
        assert!(is_sensitive_setting_key("discordAuthToken"));
        assert!(is_sensitive_setting_key("api_key"));
        assert!(is_sensitive_setting_key("clientSecret"));
        assert!(!is_sensitive_setting_key("keyboard"));
        assert!(!is_sensitive_setting_key("keybind"));
    }

    #[test]
    fn detects_protected_marker_object() {
        let map = Map::from_iter([
            (
                PROTECTED_VALUE_MARKER_KEY.to_owned(),
                Value::String(PROTECTED_VALUE_MARKER.to_owned()),
            ),
            (
                PROTECTED_VALUE_CIPHERTEXT_KEY.to_owned(),
                Value::String("abc".to_owned()),
            ),
        ]);

        assert!(is_protected_value_object(&map));
        assert_eq!(extract_protected_ciphertext(&map), Some("abc"));
    }

    #[test]
    fn removes_empty_sensitive_values_before_write() {
        let mut value = json!({
            "plugins": {
                "Example": {
                    "token": "",
                    "apiKey": "abc123",
                    "password": null,
                    "safeField": ""
                }
            }
        });

        prune_empty_sensitive_values(&mut value);
        let settings = &value["plugins"]["Example"];
        assert!(settings.get("token").is_none());
        assert!(settings.get("password").is_none());
        assert_eq!(settings.get("apiKey").and_then(Value::as_str), Some("abc123"));
        assert_eq!(settings.get("safeField").and_then(Value::as_str), Some(""));
    }
}
