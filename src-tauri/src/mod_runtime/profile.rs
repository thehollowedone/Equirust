use crate::{autostart, paths::AppPaths, settings::Settings};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, env, io};
use tauri::AppHandle;

use super::{
    collect_arg_values, compat_disabled_plugins, has_truthy_flag, join_names, read_text,
    secure_settings::read_secure_vencord_settings, EQUICORD_REPO, MINIMAL_MOD_PLUGIN_ALLOWLIST,
};

#[derive(Debug, Clone, Default)]
pub struct ModRuntimeProfile {
    pub minimal: bool,
    pub disabled_plugins: BTreeSet<String>,
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
        (
            "profilingDiagnostics".into(),
            Value::Bool(crate::browser_runtime::profiling_diagnostics_enabled()),
        ),
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
                (
                    "browserRuntime".into(),
                    Value::String(crate::browser_runtime::active_browser_runtime_name().into()),
                ),
                ("tauri".into(), Value::String(tauri::VERSION.to_string())),
                (
                    "platform".into(),
                    Value::String(env::consts::OS.to_string()),
                ),
                ("arch".into(), Value::String(env::consts::ARCH.to_string())),
                ("vencordRepo".into(), Value::String(EQUICORD_REPO.into())),
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
