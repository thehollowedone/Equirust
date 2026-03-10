use crate::{file_manager, paths::AppPaths, privacy, store::PersistedStore};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Manager, Runtime};

use super::{
    has_truthy_flag, now_millis, write_pretty_json, MANAGED_RUNTIME_DIR_NAME,
    MANAGED_RUNTIME_MANIFEST_NAME, MANAGED_RUNTIME_OPTIONAL_ASSETS,
    MANAGED_RUNTIME_REFRESH_INTERVAL_MILLIS, MANAGED_RUNTIME_RELEASE_API_URL,
    MANAGED_RUNTIME_RELEASE_OWNER, MANAGED_RUNTIME_RELEASE_REPO, MANAGED_RUNTIME_REPOSITORY_URL,
    MANAGED_RUNTIME_REQUIRED_ASSETS, MANAGED_RUNTIME_REQUIRED_FILE_NAMES,
    TRUSTED_GITHUB_DOWNLOAD_HOSTS,
};

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
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

pub(crate) fn managed_runtime_required_asset_names() -> &'static [&'static str] {
    MANAGED_RUNTIME_REQUIRED_FILE_NAMES
}

pub(crate) fn runtime_dir_has_required_assets(path: &Path) -> bool {
    MANAGED_RUNTIME_REQUIRED_FILE_NAMES.iter().all(|name| {
        fs::metadata(path.join(name))
            .map(|metadata| metadata.is_file() && metadata.len() > 0)
            .unwrap_or(false)
    })
}

pub(crate) fn renderer_script<R: Runtime>(app: &AppHandle<R>) -> io::Result<String> {
    load_runtime_assets(Some(app)).map(|runtime| runtime.renderer_js)
}

pub(super) fn renderer_stylesheet<R: Runtime>(app: &AppHandle<R>) -> io::Result<String> {
    load_runtime_assets(Some(app)).map(|runtime| runtime.renderer_css)
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
        if runtime_dir_has_required_assets(&path) {
            return Ok(RuntimeResolution {
                path,
                source: RuntimeSource::EnvOverride,
            });
        }
    }

    if let Some(store) = app.and_then(|app| app.try_state::<PersistedStore>()) {
        if let Some(path) = file_manager::resolve_custom_runtime_dir(&store) {
            if runtime_dir_has_required_assets(&path) {
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
        "No compatible runtime was found. Wait for the managed Equicord runtime download or set EQUIRUST_VENCORD_DIST_DIR to a folder containing renderer.js and renderer.css.",
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
    let current_is_valid = runtime_dir_has_required_assets(&current_dir);
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
                        .map(|checked_at| {
                            now.saturating_sub(checked_at) < MANAGED_RUNTIME_REFRESH_INTERVAL_MILLIS
                        })
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
        .user_agent(crate::browser_runtime::standard_http_user_agent())
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
    if !runtime_dir_has_required_assets(&staging_dir) {
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
        .user_agent(crate::browser_runtime::standard_http_user_agent())
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
    if !is_trusted_runtime_asset_url(&asset.browser_download_url) {
        return Err(io::Error::other(format!(
            "Managed runtime rejected untrusted asset URL for {}",
            asset.name
        )));
    }

    let expected_digest = expected_asset_sha256(asset)?;
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(io::Error::other)?;
    if !is_trusted_release_response_url(response.url()) {
        return Err(io::Error::other(format!(
            "Managed runtime rejected unexpected download host for {}",
            asset.name
        )));
    }
    let bytes = response.bytes().map_err(io::Error::other)?;
    let actual_digest = sha256_bytes(bytes.as_ref());
    if !actual_digest.eq_ignore_ascii_case(expected_digest) {
        return Err(io::Error::other(format!(
            "Managed runtime digest verification failed for {}",
            asset.name
        )));
    }
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = target_path.with_extension("part");
    fs::write(&temp_path, &bytes)?;
    fs::rename(&temp_path, target_path)
}

fn read_managed_runtime_manifest(path: &Path) -> Option<ManagedRuntimeManifest> {
    let contents = fs::read_to_string(path.join(MANAGED_RUNTIME_MANIFEST_NAME)).ok()?;
    serde_json::from_str(&contents).ok()
}

fn managed_manifest_matches_files(path: &Path, manifest: &ManagedRuntimeManifest) -> bool {
    if manifest.asset_family != "desktop-dist" || !runtime_dir_has_required_assets(path) {
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
    for file_name in MANAGED_RUNTIME_REQUIRED_FILE_NAMES {
        hashes.insert((*file_name).to_owned(), sha256_file(&path.join(file_name))?);
    }
    Ok(hashes)
}

fn sha256_file(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};

    let bytes = fs::read(path)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

fn expected_asset_sha256(asset: &GithubAsset) -> io::Result<&str> {
    let digest = asset.digest.as_deref().ok_or_else(|| {
        io::Error::other(format!(
            "Managed runtime asset is missing a SHA-256 digest: {}",
            asset.name
        ))
    })?;
    let digest = digest
        .strip_prefix("sha256:")
        .or_else(|| digest.strip_prefix("SHA256:"))
        .ok_or_else(|| {
            io::Error::other(format!(
                "Managed runtime asset digest is not SHA-256: {}",
                asset.name
            ))
        })?;
    if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(io::Error::other(format!(
            "Managed runtime asset digest is malformed: {}",
            asset.name
        )));
    }
    Ok(digest)
}

fn is_trusted_runtime_asset_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    if host != "github.com" {
        return false;
    }

    parsed.path().starts_with(&format!(
        "/{}/{}/releases/download/",
        MANAGED_RUNTIME_RELEASE_OWNER, MANAGED_RUNTIME_RELEASE_REPO
    ))
}

fn is_trusted_release_response_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }

    let Some(host) = url.host_str() else {
        return false;
    };
    TRUSTED_GITHUB_DOWNLOAD_HOSTS
        .iter()
        .any(|entry| host.eq_ignore_ascii_case(entry))
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

#[cfg(test)]
mod tests {
    use super::{resolve_runtime_resolution, runtime_dir_has_required_assets, RuntimeSource};
    use std::{
        env,
        ffi::OsString,
        fs,
        path::PathBuf,
        sync::{Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        env::temp_dir().join(format!("equirust-{name}-{stamp}"))
    }

    fn write_runtime_files(dir: &PathBuf, include_css: bool) {
        fs::create_dir_all(dir).expect("create temp runtime dir");
        fs::write(dir.join("renderer.js"), "// Equicord test\n").expect("write renderer.js");
        if include_css {
            fs::write(dir.join("renderer.css"), "body{}\n").expect("write renderer.css");
        }
    }

    #[test]
    fn runtime_dir_requires_both_renderer_assets() {
        let dir = unique_temp_dir("runtime-assets");
        write_runtime_files(&dir, false);
        assert!(!runtime_dir_has_required_assets(&dir));

        fs::write(dir.join("renderer.css"), "body{}\n").expect("write renderer.css");
        assert!(runtime_dir_has_required_assets(&dir));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_runtime_resolution_uses_valid_env_override() {
        let _guard = env_lock().lock().expect("env mutex poisoned");
        let previous = env::var_os("EQUIRUST_VENCORD_DIST_DIR");
        let dir = unique_temp_dir("env-override");
        write_runtime_files(&dir, true);
        env::set_var("EQUIRUST_VENCORD_DIST_DIR", &dir);

        let resolution = resolve_runtime_resolution::<tauri::Wry>(None).expect("resolve runtime");
        assert_eq!(resolution.path, dir);
        assert!(matches!(resolution.source, RuntimeSource::EnvOverride));

        match previous {
            Some(value) => env::set_var("EQUIRUST_VENCORD_DIST_DIR", value),
            None => env::remove_var("EQUIRUST_VENCORD_DIST_DIR"),
        }
        let _ = fs::remove_dir_all(&resolution.path);
    }

    #[test]
    fn resolve_runtime_resolution_rejects_invalid_env_override() {
        let _guard = env_lock().lock().expect("env mutex poisoned");
        let previous: Option<OsString> = env::var_os("EQUIRUST_VENCORD_DIST_DIR");
        let dir = unique_temp_dir("invalid-env-override");
        write_runtime_files(&dir, false);
        env::set_var("EQUIRUST_VENCORD_DIST_DIR", &dir);

        let result = resolve_runtime_resolution::<tauri::Wry>(None);
        assert!(result.is_err());

        match previous {
            Some(value) => env::set_var("EQUIRUST_VENCORD_DIST_DIR", value),
            None => env::remove_var("EQUIRUST_VENCORD_DIST_DIR"),
        }
        let _ = fs::remove_dir_all(dir);
    }
}
