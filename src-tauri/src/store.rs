use crate::{
    app_menu, arrpc, autostart,
    paths::AppPaths,
    privacy,
    settings::{PersistedState, Settings},
    tray, vencord,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{fs, io, path::Path, sync::Mutex};
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
}

impl PersistedStore {
    pub fn load(app: &AppHandle) -> io::Result<Self> {
        let paths = AppPaths::resolve(app)?;
        vencord::seed_from_legacy_install(&paths)?;
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
        })
    }

    pub fn snapshot(&self) -> StoreSnapshot {
        self.inner
            .lock()
            .expect("persisted store mutex poisoned")
            .clone()
    }

    pub fn replace_settings(&self, settings: Settings) -> io::Result<StoreSnapshot> {
        let mut guard = self.inner.lock().expect("persisted store mutex poisoned");
        guard.settings = settings.with_fallbacks(&Settings::equirust_defaults());
        persist_pretty_json(&guard.paths.settings_file, &guard.settings)?;
        Ok(guard.clone())
    }

    pub fn replace_state(&self, state: PersistedState) -> io::Result<StoreSnapshot> {
        let mut guard = self.inner.lock().expect("persisted store mutex poisoned");
        guard.state = state;
        persist_pretty_json(&guard.paths.state_file, &guard.state)?;
        Ok(guard.clone())
    }

    pub fn update_state<F>(&self, update: F) -> io::Result<StoreSnapshot>
    where
        F: FnOnce(&mut PersistedState),
    {
        let mut guard = self.inner.lock().expect("persisted store mutex poisoned");
        update(&mut guard.state);
        persist_pretty_json(&guard.paths.state_file, &guard.state)?;
        Ok(guard.clone())
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
    fs::write(path, format!("{json}\n"))
}
