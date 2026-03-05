use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub portable: bool,
    pub data_dir: PathBuf,
    pub settings_file: PathBuf,
    pub state_file: PathBuf,
    pub vencord_settings_dir: PathBuf,
    pub vencord_settings_file: PathBuf,
    pub vencord_quickcss_file: PathBuf,
    pub vencord_themes_dir: PathBuf,
    pub user_assets_dir: PathBuf,
    pub app_cache_dir: PathBuf,
    pub app_log_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> io::Result<Self> {
        let env_data_dir = std::env::var_os("EQUICORD_USER_DATA_DIR").map(PathBuf::from);
        let portable_dir = portable_data_dir()?;

        let (data_dir, portable) = match (env_data_dir, portable_dir) {
            (Some(path), _) => (path, false),
            (None, Some(path)) => (path, true),
            (None, None) => (
                app.path()
                    .app_data_dir()
                    .map_err(|err| io::Error::other(err.to_string()))?,
                false,
            ),
        };

        let app_cache_dir = app
            .path()
            .app_cache_dir()
            .map_err(|err| io::Error::other(err.to_string()))?;
        let app_log_dir = app
            .path()
            .app_log_dir()
            .map_err(|err| io::Error::other(err.to_string()))?;

        ensure_dir(&data_dir)?;
        ensure_dir(&app_cache_dir)?;
        ensure_dir(&app_log_dir)?;

        let vencord_settings_dir = data_dir.join("settings");
        let vencord_themes_dir = data_dir.join("themes");
        let user_assets_dir = data_dir.join("userAssets");

        ensure_dir(&vencord_settings_dir)?;
        ensure_dir(&vencord_themes_dir)?;
        ensure_dir(&user_assets_dir)?;

        Ok(Self {
            portable,
            settings_file: data_dir.join("settings.json"),
            state_file: data_dir.join("state.json"),
            vencord_settings_file: vencord_settings_dir.join("settings.json"),
            vencord_quickcss_file: vencord_settings_dir.join("quickCss.css"),
            data_dir,
            vencord_settings_dir,
            vencord_themes_dir,
            user_assets_dir,
            app_cache_dir,
            app_log_dir,
        })
    }
}

fn portable_data_dir() -> io::Result<Option<PathBuf>> {
    if !cfg!(target_os = "windows") || cfg!(debug_assertions) {
        return Ok(None);
    }

    let current_exe = std::env::current_exe()?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| io::Error::other("current executable has no parent directory"))?;
    let file_name = current_exe
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    let uninstall_equibop = install_dir.join("Uninstall Equibop.exe");
    let uninstall_equirust = install_dir.join("Uninstall Equirust.exe");
    let is_portable = !file_name.eq_ignore_ascii_case("electron.exe")
        && !uninstall_equibop.exists()
        && !uninstall_equirust.exists();

    if !is_portable || !is_directory_writable(install_dir) {
        return Ok(None);
    }

    Ok(Some(install_dir.join("Data")))
}

fn ensure_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

fn is_directory_writable(path: &Path) -> bool {
    let probe_name = format!(
        ".equirust-write-probe-{}-{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let probe_path = path.join(probe_name);

    let probe_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path);

    match probe_file {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe_path);
            true
        }
        Err(_) => false,
    }
}
