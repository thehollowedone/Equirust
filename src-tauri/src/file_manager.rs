use crate::{mod_runtime, paths::AppPaths, store::PersistedStore, tray};
use serde::Serialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, State};

const CUSTOMIZABLE_ASSETS: &[&str] = &[
    "splash",
    "tray",
    "trayUnread",
    "traySpeaking",
    "trayIdle",
    "trayMuted",
    "trayDeafened",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileManagerState {
    pub using_custom_vencord_dir: bool,
    pub custom_vencord_dir: Option<String>,
    pub active_runtime_dir: Option<String>,
    pub runtime_source: Option<String>,
    pub user_assets_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAssetData {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[tauri::command]
pub fn get_file_manager_state(
    app: AppHandle,
    store: State<'_, PersistedStore>,
) -> Result<FileManagerState, String> {
    let snapshot = store.snapshot();
    let runtime_resolution = mod_runtime::resolve_runtime_resolution(Some(&app)).ok();
    Ok(FileManagerState {
        using_custom_vencord_dir: snapshot.state.equicord_dir.is_some(),
        custom_vencord_dir: snapshot.state.equicord_dir,
        active_runtime_dir: runtime_resolution
            .as_ref()
            .map(|runtime| runtime.path.display().to_string()),
        runtime_source: runtime_resolution
            .as_ref()
            .map(|runtime| runtime.source.as_str().to_owned()),
        user_assets_dir: snapshot.paths.user_assets_dir.display().to_string(),
    })
}

#[tauri::command]
pub fn show_custom_vencord_dir(store: State<'_, PersistedStore>) -> Result<(), String> {
    let snapshot = store.snapshot();
    let Some(path) = snapshot.state.equicord_dir else {
        return Ok(());
    };

    open_path(Path::new(&path)).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn open_user_assets_folder(app: AppHandle) -> Result<(), String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    open_path(&paths.user_assets_dir).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn select_vencord_dir(
    reset: bool,
    app: AppHandle,
    store: State<'_, PersistedStore>,
) -> Result<String, String> {
    if reset {
        store
            .update_state(|state| {
                state.equicord_dir = None;
            })
            .map_err(|err| err.to_string())?;
        return Ok("ok".into());
    }

    let Some(selected_dir) = rfd::FileDialog::new()
        .set_title("Select a custom Equicord install")
        .pick_folder()
    else {
        return Ok("cancelled".into());
    };

    let Some(root_dir) = normalize_custom_vencord_root(&selected_dir) else {
        return Ok("invalid".into());
    };

    store
        .update_state(|state| {
            state.equicord_dir = Some(root_dir.display().to_string());
        })
        .map_err(|err| err.to_string())?;

    let _ = tray::sync(&app, &store.snapshot().settings);
    Ok("ok".into())
}

#[tauri::command]
pub fn choose_user_asset(
    asset: String,
    reset: bool,
    app: AppHandle,
    store: State<'_, PersistedStore>,
) -> Result<String, String> {
    let asset_name = parse_asset_name(&asset).ok_or_else(|| format!("invalid asset: {asset}"))?;
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let asset_path = paths.user_assets_dir.join(asset_name);

    let result = if reset {
        match fs::remove_file(&asset_path) {
            Ok(()) => "ok",
            Err(err) if err.kind() == io::ErrorKind::NotFound => "ok",
            Err(err) => {
                log::warn!("Failed to remove user asset {}: {}", asset_name, err);
                "failed"
            }
        }
    } else {
        let Some(selected_file) = rfd::FileDialog::new()
            .set_title(format!("Select an image to use as {asset_name}"))
            .add_filter(
                "Images",
                &["png", "jpg", "jpeg", "webp", "gif", "avif", "svg", "ico"],
            )
            .pick_file()
        else {
            return Ok("cancelled".into());
        };

        match fs::copy(selected_file, &asset_path) {
            Ok(_) => "ok",
            Err(err) => {
                log::warn!("Failed to copy user asset {}: {}", asset_name, err);
                "failed"
            }
        }
    };

    if is_tray_asset(asset_name) {
        let _ = tray::sync(&app, &store.snapshot().settings);
    }

    Ok(result.into())
}

#[tauri::command]
pub fn get_user_asset_data(asset: String, app: AppHandle) -> Result<Option<UserAssetData>, String> {
    let paths = AppPaths::resolve(&app).map_err(|err| err.to_string())?;
    let Some(path) = resolve_user_asset_path(&paths, &asset) else {
        return Ok(None);
    };

    let bytes = fs::read(&path).map_err(|err| err.to_string())?;
    let mime_type = infer_asset_mime_type(&path).to_owned();
    Ok(Some(UserAssetData { mime_type, bytes }))
}

pub fn resolve_user_asset_path(paths: &AppPaths, asset: &str) -> Option<PathBuf> {
    let asset_name = parse_asset_name(asset)?;
    let path = paths.user_assets_dir.join(asset_name);
    path.exists().then_some(path)
}

pub fn resolve_custom_runtime_dir(store: &PersistedStore) -> Option<PathBuf> {
    let snapshot = store.snapshot();
    snapshot
        .state
        .equicord_dir
        .as_deref()
        .and_then(|value| normalize_runtime_dir(Path::new(value)))
}

pub fn open_path(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path.as_os_str())
            .spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "opening paths is unsupported on this platform",
    ))
}

fn normalize_custom_vencord_root(path: &Path) -> Option<PathBuf> {
    if normalize_runtime_dir(path).is_some() {
        return Some(
            path.file_name()
                .and_then(|value| value.to_str())
                .filter(|value| value.eq_ignore_ascii_case("equibop"))
                .and_then(|_| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| path.to_path_buf()),
        );
    }

    let nested_runtime_dir = path.join("equibop");
    if normalize_runtime_dir(&nested_runtime_dir).is_some() {
        return Some(path.to_path_buf());
    }

    None
}

fn normalize_runtime_dir(path: &Path) -> Option<PathBuf> {
    if mod_runtime::runtime_dir_has_required_assets(path) {
        return Some(path.to_path_buf());
    }

    let nested = path.join("equibop");
    if mod_runtime::runtime_dir_has_required_assets(&nested) {
        return Some(nested);
    }

    None
}

fn parse_asset_name(value: &str) -> Option<&'static str> {
    CUSTOMIZABLE_ASSETS
        .iter()
        .copied()
        .find(|asset| *asset == value)
}

fn is_tray_asset(asset: &str) -> bool {
    matches!(
        asset,
        "tray" | "trayUnread" | "traySpeaking" | "trayIdle" | "trayMuted" | "trayDeafened"
    )
}

fn infer_asset_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("avif") => "image/avif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_runtime_dir;
    use std::{
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        env::temp_dir().join(format!("equirust-{name}-{stamp}"))
    }

    fn write_runtime_files(dir: &PathBuf) {
        fs::create_dir_all(dir).expect("create runtime dir");
        fs::write(dir.join("renderer.js"), "// Equicord test\n").expect("write renderer.js");
        fs::write(dir.join("renderer.css"), "body{}\n").expect("write renderer.css");
    }

    #[test]
    fn normalize_runtime_dir_accepts_direct_runtime_folder() {
        let dir = unique_temp_dir("custom-runtime-direct");
        write_runtime_files(&dir);

        let resolved = normalize_runtime_dir(&dir);
        assert_eq!(resolved.as_deref(), Some(dir.as_path()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_runtime_dir_accepts_nested_equibop_runtime_folder() {
        let dir = unique_temp_dir("custom-runtime-nested");
        let nested = dir.join("equibop");
        write_runtime_files(&nested);

        let resolved = normalize_runtime_dir(&dir);
        assert_eq!(resolved.as_deref(), Some(nested.as_path()));

        let _ = fs::remove_dir_all(dir);
    }
}
