use crate::{
    discord, privacy,
    settings::UpdaterState,
    store::{PersistedStore, StoreSnapshot},
    vencord,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, State as TauriState};

const DEFAULT_HOST_RELEASES_API_URL: &str =
    "https://api.github.com/repos/thehollowedone/equirust/releases/latest";
const DEFAULT_HOST_RELEASES_PAGE_URL: &str = "https://github.com/thehollowedone/equirust/releases";
const HOST_RELEASES_API_URL: Option<&str> = option_env!("EQUIRUST_HOST_RELEASES_API_URL");
const HOST_RELEASES_PAGE_URL: Option<&str> = option_env!("EQUIRUST_HOST_RELEASES_PAGE_URL");
const RUNTIME_RELEASES_API_URL: &str =
    "https://api.github.com/repos/Equicord/Equicord/releases/latest";
const RUNTIME_RELEASES_PAGE_URL: &str = "https://github.com/Equicord/Equicord/releases/latest";
const RUNTIME_DOWNLOAD_ASSET_NAMES: &[&str] =
    &["renderer.js", "renderer.css", "patcher.js", "preload.js"];
const SNOOZE_MILLIS: i64 = 24 * 60 * 60 * 1000;
const LINKED_RUNTIME_PACKAGE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime-package.json"
));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub track: String,
    pub configured: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_name: Option<String>,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
    pub release_url: Option<String>,
    pub download_url: Option<String>,
    pub update_available: bool,
    pub ignored: bool,
    pub snoozed: bool,
    pub should_prompt: bool,
    pub checked_at: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostUpdateDownloadState {
    pub phase: String,
    pub percent: f64,
    pub asset_name: Option<String>,
    pub destination: Option<String>,
    pub error: Option<String>,
}

pub struct RuntimeState {
    inner: Mutex<HostUpdateDownloadState>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HostUpdateDownloadState {
                phase: "idle".to_owned(),
                percent: 0.0,
                asset_name: None,
                destination: None,
                error: None,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateTrack {
    Host,
    Runtime,
}

#[derive(Debug, Clone, Copy)]
struct ReleaseSource {
    api_url: Option<&'static str>,
    page_url: Option<&'static str>,
}

#[tauri::command]
pub fn get_host_update_status(
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    Ok(resolve_status(UpdateTrack::Host, &app, &store.snapshot()))
}

#[tauri::command]
pub fn get_runtime_update_status(
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    Ok(resolve_status(
        UpdateTrack::Runtime,
        &app,
        &store.snapshot(),
    ))
}

#[tauri::command]
pub fn open_host_update(
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<(), String> {
    open_update_target(UpdateTrack::Host, &app, &store.snapshot())
}

#[tauri::command]
pub fn open_runtime_update(
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<(), String> {
    open_update_target(UpdateTrack::Runtime, &app, &store.snapshot())
}

#[tauri::command]
pub fn install_runtime_update(app: AppHandle) -> Result<(), String> {
    vencord::refresh_managed_runtime(&app).map_err(|err| err.to_string())?;
    log::info!("Installed managed Equicord runtime update; restarting Equirust");
    app.restart();
}

#[tauri::command]
pub fn get_host_update_download_state(
    runtime_state: TauriState<'_, RuntimeState>,
) -> Result<HostUpdateDownloadState, String> {
    Ok(runtime_state.snapshot())
}

#[tauri::command]
pub fn install_host_update(
    app: AppHandle,
    runtime_state: TauriState<'_, RuntimeState>,
) -> Result<(), String> {
    let source = release_source(UpdateTrack::Host);
    let api_url = source
        .api_url
        .ok_or_else(|| "Equirust host updates are not configured yet".to_owned())?;
    let release = fetch_latest_release(api_url)?;
    let asset = select_download_asset(&release.assets)
        .ok_or_else(|| "No compatible downloadable asset was found for this platform".to_owned())?;
    let download_dir = app
        .path()
        .app_cache_dir()
        .map_err(|err| err.to_string())?
        .join("updates");
    let target_path = download_dir.join(&asset.name);

    {
        let mut state = runtime_state
            .inner
            .lock()
            .expect("updater runtime mutex poisoned");
        if state.phase == "downloading" {
            return Err("An update download is already in progress".to_owned());
        }

        *state = HostUpdateDownloadState {
            phase: "downloading".to_owned(),
            percent: 0.0,
            asset_name: Some(asset.name.clone()),
            destination: Some(target_path.display().to_string()),
            error: None,
        };
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        let runtime_state = app_handle.state::<RuntimeState>();
        if let Err(err) =
            download_and_launch_update(&app_handle, &runtime_state, &asset, &target_path)
        {
            runtime_state.set_error(err);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn snooze_host_update(
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    snooze_update(UpdateTrack::Host, &app, &store)
}

#[tauri::command]
pub fn ignore_host_update(
    version: String,
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    ignore_update(UpdateTrack::Host, version, &app, &store)
}

#[tauri::command]
pub fn snooze_runtime_update(
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    snooze_update(UpdateTrack::Runtime, &app, &store)
}

#[tauri::command]
pub fn ignore_runtime_update(
    version: String,
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    ignore_update(UpdateTrack::Runtime, version, &app, &store)
}

fn snooze_update(
    track: UpdateTrack,
    app: &AppHandle,
    store: &TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    store
        .update_state(|state| {
            let updater = updater_state_mut(state, track);
            updater.get_or_insert_default().snooze_until = Some(now_millis() + SNOOZE_MILLIS);
        })
        .map_err(|err| err.to_string())?;

    Ok(resolve_status(track, app, &store.snapshot()))
}

fn ignore_update(
    track: UpdateTrack,
    version: String,
    app: &AppHandle,
    store: &TauriState<'_, PersistedStore>,
) -> Result<UpdateStatus, String> {
    store
        .update_state(|state| {
            let updater = updater_state_mut(state, track);
            updater.get_or_insert_default().ignored_version = Some(version.clone());
        })
        .map_err(|err| err.to_string())?;

    Ok(resolve_status(track, app, &store.snapshot()))
}

fn open_update_target(
    track: UpdateTrack,
    app: &AppHandle,
    snapshot: &StoreSnapshot,
) -> Result<(), String> {
    let status = resolve_status(track, app, snapshot);
    let target = match track {
        UpdateTrack::Host => status
            .download_url
            .as_deref()
            .or(status.release_url.as_deref()),
        UpdateTrack::Runtime => status
            .release_url
            .as_deref()
            .or(status.download_url.as_deref()),
    }
    .ok_or_else(|| match track {
        UpdateTrack::Host => "Equirust host updates are not configured yet".to_owned(),
        UpdateTrack::Runtime => "No Equicord runtime release link is available".to_owned(),
    })?;

    webbrowser::open(target)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn resolve_status(track: UpdateTrack, app: &AppHandle, snapshot: &StoreSnapshot) -> UpdateStatus {
    let checked_at = now_millis();
    let current_version = current_version_for(track, app);
    let track_name = track.label().to_owned();
    let source = release_source(track);

    let Some(api_url) = source.api_url else {
        return UpdateStatus {
            track: track_name,
            configured: false,
            current_version,
            latest_version: None,
            release_name: None,
            release_notes: None,
            published_at: None,
            release_url: source.page_url.map(str::to_owned),
            download_url: None,
            update_available: false,
            ignored: false,
            snoozed: false,
            should_prompt: false,
            checked_at,
            error: None,
        };
    };

    match fetch_track_release(track, api_url) {
        Ok((release, latest_version, download_url)) => {
            let update_available = latest_version
                .as_deref()
                .map(|latest| is_newer_version(&current_version, latest))
                .unwrap_or(false);
            let ignored = updater_state(snapshot, track)
                .and_then(|state| state.ignored_version.as_deref())
                == latest_version.as_deref();
            let snoozed = updater_state(snapshot, track)
                .and_then(|state| state.snooze_until)
                .unwrap_or_default()
                > checked_at;

            UpdateStatus {
                track: track_name,
                configured: true,
                current_version,
                latest_version,
                release_name: release
                    .name
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| normalize_version_string(&release.tag_name)),
                release_notes: release.body.filter(|value| !value.trim().is_empty()),
                published_at: release.published_at,
                release_url: Some(release.html_url),
                download_url,
                update_available,
                ignored,
                snoozed,
                should_prompt: update_available && !ignored && !snoozed,
                checked_at,
                error: None,
            }
        }
        Err(err) => UpdateStatus {
            track: track_name,
            configured: true,
            current_version,
            latest_version: None,
            release_name: None,
            release_notes: None,
            published_at: None,
            release_url: source.page_url.map(str::to_owned),
            download_url: None,
            update_available: false,
            ignored: false,
            snoozed: false,
            should_prompt: false,
            checked_at,
            error: Some(err),
        },
    }
}

fn fetch_track_release(
    track: UpdateTrack,
    api_url: &str,
) -> Result<(GithubRelease, Option<String>, Option<String>), String> {
    let release = fetch_latest_release(api_url)?;
    let latest_version = match track {
        UpdateTrack::Host => normalize_version_string(&release.tag_name),
        UpdateTrack::Runtime => fetch_runtime_release_version(&release)?,
    };
    let download_url = match track {
        UpdateTrack::Host => select_download_url(&release.assets),
        UpdateTrack::Runtime => {
            select_named_asset_url(&release.assets, RUNTIME_DOWNLOAD_ASSET_NAMES)
        }
    };

    Ok((release, latest_version, download_url))
}

fn current_version_for(track: UpdateTrack, app: &AppHandle) -> String {
    match track {
        UpdateTrack::Host => app.package_info().version.to_string(),
        UpdateTrack::Runtime => detect_runtime_version(app),
    }
}

fn fetch_runtime_release_version(release: &GithubRelease) -> Result<Option<String>, String> {
    if let Some(version) = normalize_version_string(&release.tag_name) {
        return Ok(Some(version));
    }

    let Some(renderer_asset) = select_named_asset(&release.assets, &["renderer.js"]) else {
        return Ok(None);
    };

    read_renderer_build_from_url(&renderer_asset.browser_download_url).map(Some)
}

fn detect_runtime_version(app: &AppHandle) -> String {
    runtime_dir_package_version(app)
        .or_else(linked_runtime_version)
        .or_else(|| runtime_renderer_build(app))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn runtime_dir_package_version(app: &AppHandle) -> Option<String> {
    let runtime_dir = vencord::resolve_runtime_dir(Some(app)).ok()?;

    [runtime_dir.join("package.json")]
        .into_iter()
        .find_map(|path| read_version_from_json_file(&path))
}

fn runtime_renderer_build(app: &AppHandle) -> Option<String> {
    let runtime_dir = vencord::resolve_runtime_dir(Some(app)).ok()?;

    ["equibopRenderer.js", "renderer.js"]
        .into_iter()
        .map(|name| runtime_dir.join(name))
        .find_map(|path| read_renderer_build_from_file(&path))
}

fn linked_runtime_version() -> Option<String> {
    parse_runtime_version_from_json(LINKED_RUNTIME_PACKAGE_JSON)
}

fn read_version_from_json_file(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    parse_runtime_version_from_json(&contents)
}

fn parse_runtime_version_from_json(contents: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(contents).ok()?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn read_renderer_build_from_file(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut line = String::new();
    let mut reader = BufReader::new(file);
    let read = reader.read_line(&mut line).ok()?;
    if read == 0 {
        return None;
    }

    parse_renderer_build_line(&line)
}

fn read_renderer_build_from_url(url: &str) -> Result<String, String> {
    let client = build_client(Duration::from_secs(5), Duration::from_secs(30))?;
    let mut response = client
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            discord::standard_http_user_agent(),
        )
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|err| err.to_string())?;
    let mut line = String::new();
    let mut reader = BufReader::new(&mut response);
    let read = reader.read_line(&mut line).map_err(|err| err.to_string())?;
    if read == 0 {
        return Err("Runtime renderer asset was empty".to_owned());
    }

    parse_renderer_build_line(&line)
        .ok_or_else(|| "Runtime renderer asset did not expose a build identifier".to_owned())
}

fn parse_renderer_build_line(line: &str) -> Option<String> {
    let line = line.trim();
    ["// Vencord ", "// Equicord "]
        .into_iter()
        .find_map(|prefix| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn release_source(track: UpdateTrack) -> ReleaseSource {
    match track {
        UpdateTrack::Host => ReleaseSource {
            api_url: Some(
                HOST_RELEASES_API_URL
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(DEFAULT_HOST_RELEASES_API_URL),
            ),
            page_url: Some(
                HOST_RELEASES_PAGE_URL
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(DEFAULT_HOST_RELEASES_PAGE_URL),
            ),
        },
        UpdateTrack::Runtime => ReleaseSource {
            api_url: Some(RUNTIME_RELEASES_API_URL),
            page_url: Some(RUNTIME_RELEASES_PAGE_URL),
        },
    }
}

fn fetch_latest_release(api_url: &str) -> Result<GithubRelease, String> {
    let client = build_client(Duration::from_secs(5), Duration::from_secs(10))?;

    client
        .get(api_url)
        .header(
            reqwest::header::USER_AGENT,
            discord::standard_http_user_agent(),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|err| err.to_string())?
        .json::<GithubRelease>()
        .map_err(|err| err.to_string())
}

fn build_client(
    connect_timeout: Duration,
    timeout: Duration,
) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(connect_timeout)
        .timeout(timeout)
        .user_agent(discord::standard_http_user_agent())
        .build()
        .map_err(|err| err.to_string())
}

fn select_download_url(assets: &[GithubAsset]) -> Option<String> {
    select_download_asset(assets).map(|asset| asset.browser_download_url)
}

fn select_download_asset(assets: &[GithubAsset]) -> Option<GithubAsset> {
    let preferred_suffixes: &[&str] = if cfg!(target_os = "windows") {
        &[".msi", ".exe", ".msix", ".zip"]
    } else if cfg!(target_os = "macos") {
        &[".dmg", ".pkg", ".zip", ".tar.gz"]
    } else {
        &[".AppImage", ".deb", ".rpm", ".tar.gz", ".zip"]
    };

    for suffix in preferred_suffixes {
        if let Some(asset) = assets
            .iter()
            .find(|asset| asset.name.ends_with(suffix) && !asset.name.ends_with(".blockmap"))
        {
            return Some(asset.clone());
        }
    }

    assets
        .iter()
        .find(|asset| !asset.name.ends_with(".blockmap"))
        .cloned()
}

fn select_named_asset<'a>(assets: &'a [GithubAsset], names: &[&str]) -> Option<&'a GithubAsset> {
    names.iter().find_map(|name| {
        assets
            .iter()
            .find(|asset| asset.name.eq_ignore_ascii_case(name))
    })
}

fn select_named_asset_url(assets: &[GithubAsset], names: &[&str]) -> Option<String> {
    select_named_asset(assets, names).map(|asset| asset.browser_download_url.clone())
}

fn normalize_version_string(raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_start_matches('v');
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("latest") {
        None
    } else {
        Some(normalized.to_owned())
    }
}

fn is_newer_version(current: &str, latest: &str) -> bool {
    let current = current.trim().trim_start_matches('v');
    let latest = latest.trim().trim_start_matches('v');

    match (Version::parse(current), Version::parse(latest)) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => latest != current,
    }
}

fn updater_state<'a>(snapshot: &'a StoreSnapshot, track: UpdateTrack) -> Option<&'a UpdaterState> {
    match track {
        UpdateTrack::Host => snapshot.state.host_updater.as_ref(),
        UpdateTrack::Runtime => snapshot.state.runtime_updater.as_ref(),
    }
}

fn updater_state_mut(
    state: &mut crate::settings::PersistedState,
    track: UpdateTrack,
) -> &mut Option<UpdaterState> {
    match track {
        UpdateTrack::Host => &mut state.host_updater,
        UpdateTrack::Runtime => &mut state.runtime_updater,
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl UpdateTrack {
    fn label(self) -> &'static str {
        match self {
            UpdateTrack::Host => "host",
            UpdateTrack::Runtime => "runtime",
        }
    }
}

impl RuntimeState {
    fn snapshot(&self) -> HostUpdateDownloadState {
        self.inner
            .lock()
            .expect("updater runtime mutex poisoned")
            .clone()
    }

    fn set_error(&self, error: String) {
        let mut state = self.inner.lock().expect("updater runtime mutex poisoned");
        state.phase = "error".to_owned();
        state.error = Some(error);
    }

    fn set_progress(&self, phase: &str, percent: f64) {
        let mut state = self.inner.lock().expect("updater runtime mutex poisoned");
        state.phase = phase.to_owned();
        state.percent = percent;
        state.error = None;
    }
}

fn download_and_launch_update(
    app: &AppHandle,
    runtime_state: &RuntimeState,
    asset: &GithubAsset,
    target_path: &Path,
) -> Result<(), String> {
    fs::create_dir_all(
        target_path
            .parent()
            .ok_or_else(|| "Updater target path has no parent directory".to_owned())?,
    )
    .map_err(|err| err.to_string())?;

    let client = build_client(Duration::from_secs(5), Duration::from_secs(600))?;
    let mut response = client
        .get(&asset.browser_download_url)
        .header(
            reqwest::header::USER_AGENT,
            discord::standard_http_user_agent(),
        )
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|err| err.to_string())?;

    let total = response.content_length().unwrap_or_default();
    let mut file = File::create(target_path).map_err(|err| err.to_string())?;
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = response.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])
            .map_err(|err| err.to_string())?;
        downloaded = downloaded.saturating_add(read as u64);

        let percent = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        runtime_state.set_progress("downloading", percent.min(100.0));
    }

    runtime_state.set_progress("launching", 100.0);
    launch_installer(target_path)?;
    runtime_state.set_progress("launched", 100.0);
    log::info!(
        "Launched host update installer {}",
        privacy::file_name_for_log(target_path)
    );
    app.exit(0);
    Ok(())
}

fn launch_installer(target_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &target_path.display().to_string()])
            .spawn()
            .map_err(|err| err.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(target_path)
            .spawn()
            .map_err(|err| err.to_string())?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(target_path)
            .spawn()
            .map_err(|err| err.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("Updater install launching is unsupported on this platform".to_owned())
}
