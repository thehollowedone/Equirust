use crate::{
    app_menu, arrpc, autostart, mod_runtime,
    paths::AppPaths,
    privacy,
    settings::{PersistedState, Settings},
    tray,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use tauri::{AppHandle, State as TauriState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreSnapshot {
    pub paths: AppPaths,
    pub settings: Settings,
    pub state: PersistedState,
}

pub struct PersistedStore {
    inner: Mutex<StoreSnapshot>,
    write_lock: Mutex<()>,
}

impl PersistedStore {
    pub fn load(app: &AppHandle) -> io::Result<Self> {
        let paths = AppPaths::resolve(app)?;
        mod_runtime::seed_from_legacy_install(&paths)?;
        let loaded_settings = load_json_file::<Settings>(&paths.settings_file)?;
        let settings = loaded_settings
            .clone()
            .with_fallbacks(&Settings::equirust_defaults());
        let loaded_state = load_json_file::<PersistedState>(&paths.state_file)?;
        let state = loaded_state.clone().normalize();

        if settings != loaded_settings {
            persist_pretty_json(&paths.settings_file, &settings)?;
        }
        if state != loaded_state {
            persist_pretty_json(&paths.state_file, &state)?;
        }

        Ok(Self {
            inner: Mutex::new(StoreSnapshot {
                paths,
                settings,
                state,
            }),
            write_lock: Mutex::new(()),
        })
    }

    pub fn snapshot(&self) -> StoreSnapshot {
        self.lock_snapshot().clone()
    }

    pub fn replace_settings(&self, settings: Settings) -> io::Result<StoreSnapshot> {
        let _write_guard = self.lock_write_guard();
        let next_snapshot = {
            let current = self.lock_snapshot();
            let mut next = current.clone();
            next.settings = settings.with_fallbacks(&Settings::equirust_defaults());
            next
        };
        persist_pretty_json(&next_snapshot.paths.settings_file, &next_snapshot.settings)?;
        *self.lock_snapshot() = next_snapshot.clone();
        Ok(next_snapshot)
    }

    pub fn replace_state(&self, state: PersistedState) -> io::Result<StoreSnapshot> {
        let _write_guard = self.lock_write_guard();
        let next_snapshot = {
            let current = self.lock_snapshot();
            let mut next = current.clone();
            next.state = state;
            next
        };
        persist_pretty_json(&next_snapshot.paths.state_file, &next_snapshot.state)?;
        *self.lock_snapshot() = next_snapshot.clone();
        Ok(next_snapshot)
    }

    pub fn update_state<F>(&self, update: F) -> io::Result<StoreSnapshot>
    where
        F: FnOnce(&mut PersistedState),
    {
        let _write_guard = self.lock_write_guard();
        let next_snapshot = {
            let current = self.lock_snapshot();
            let mut next = current.clone();
            update(&mut next.state);
            next
        };
        persist_pretty_json(&next_snapshot.paths.state_file, &next_snapshot.state)?;
        *self.lock_snapshot() = next_snapshot.clone();
        Ok(next_snapshot)
    }

    fn lock_snapshot(&self) -> MutexGuard<'_, StoreSnapshot> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Persisted store mutex was poisoned; recovering snapshot access");
                poisoned.into_inner()
            }
        }
    }

    fn lock_write_guard(&self) -> MutexGuard<'_, ()> {
        match self.write_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Persisted store write mutex was poisoned; recovering write access");
                poisoned.into_inner()
            }
        }
    }
}

#[tauri::command]
pub fn get_store_snapshot(store: TauriState<'_, PersistedStore>) -> Result<StoreSnapshot, String> {
    Ok(store.snapshot())
}

#[tauri::command]
pub fn set_settings(
    settings: Settings,
    app: AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<StoreSnapshot, String> {
    let snapshot = store
        .replace_settings(settings)
        .map_err(|err| err.to_string())?;
    autostart::sync(&app, &snapshot.settings)?;
    app_menu::sync(&app, &snapshot.settings)?;
    tray::sync(&app, &snapshot.settings)?;
    arrpc::sync(&app, &snapshot.settings)?;
    Ok(snapshot)
}

#[tauri::command]
pub fn set_state(
    state: PersistedState,
    store: TauriState<'_, PersistedStore>,
) -> Result<StoreSnapshot, String> {
    store.replace_state(state).map_err(|err| err.to_string())
}

fn load_json_file<T>(path: &Path) -> io::Result<T>
where
    T: Default + DeserializeOwned,
{
    if !path.exists() {
        return Ok(T::default());
    }

    let contents = fs::read_to_string(path)?;
    match serde_json::from_str(&contents) {
        Ok(value) => Ok(value),
        Err(err) => {
            log::warn!(
                "Failed to parse {}: {}",
                privacy::file_name_for_log(path),
                err
            );
            Ok(T::default())
        }
    }
}

fn persist_pretty_json<T>(path: &Path, value: &T) -> io::Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let temp_path = temp_json_path(path);
    fs::write(&temp_path, format!("{json}\n"))?;
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(temp_path, path)
}

fn temp_json_path(path: &Path) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if extension.is_empty() {
        path.with_extension("tmp")
    } else {
        path.with_extension(format!("{extension}.tmp"))
    }
}
