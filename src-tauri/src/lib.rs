mod app_menu;
mod arrpc;
mod autostart;
mod browser_runtime;
mod capturer;
mod csp;
mod desktop;
mod desktop_host;
mod desktop_stream;
mod discord;
mod doctor;
mod file_manager;
mod http_proxy;
mod ipc_bridge;
mod mod_runtime;
mod notifications;
mod paths;
mod privacy;
mod processes;
mod protocol;
mod settings;
mod spellcheck;
mod store;
mod tray;
mod updater;
mod utilities;
mod virtmic;
mod voice;
mod win32_window_snapshot;
mod window;
use std::{
    backtrace::Backtrace,
    fs::{self, OpenOptions},
    io::Write,
    panic,
    sync::Once,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

static PANIC_HOOK_INSTALLED: Once = Once::new();

fn build_log_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let builder = tauri_plugin_log::Builder::default()
        .clear_targets()
        .target(Target::new(TargetKind::LogDir { file_name: None }))
        .timezone_strategy(TimezoneStrategy::UseLocal);

    if cfg!(debug_assertions) || browser_runtime::profiling_diagnostics_enabled() {
        builder
            .level(log::LevelFilter::Info)
            .rotation_strategy(RotationStrategy::KeepSome(3))
            .max_file_size(1_048_576)
            .build()
    } else {
        builder
            .level(log::LevelFilter::Error)
            .rotation_strategy(RotationStrategy::KeepOne)
            .max_file_size(65_536)
            .build()
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn append_crash_log_line<R: tauri::Runtime>(app: &tauri::AppHandle<R>, line: &str) {
    let Ok(log_dir) = app.path().app_log_dir() else {
        return;
    };
    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let crash_file = log_dir.join("Equirust-crash.log");
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(crash_file)
    else {
        return;
    };
    let _ = writeln!(file, "[{}] {}", now_millis(), line);
}

fn install_panic_hook<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let app_handle = app.clone();
    PANIC_HOOK_INSTALLED.call_once(move || {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let payload = if let Some(payload) = info.payload().downcast_ref::<&str>() {
                payload.to_string()
            } else if let Some(payload) = info.payload().downcast_ref::<String>() {
                payload.clone()
            } else {
                "unknown panic payload".to_owned()
            };
            let location = info
                .location()
                .map(|location| {
                    format!(
                        "{}:{}:{}",
                        location.file(),
                        location.line(),
                        location.column()
                    )
                })
                .unwrap_or_else(|| "<unknown>".to_owned());
            let thread_name = std::thread::current()
                .name()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| "unnamed-thread".to_owned());
            let backtrace = Backtrace::force_capture().to_string();
            let message = format!(
                "panic thread={} location={} payload={} backtrace={}",
                thread_name, location, payload, backtrace
            );
            let sanitized = privacy::sanitize_text_for_log(&message);
            log::error!("{}", sanitized);
            append_crash_log_line(&app_handle, &sanitized);
            default_hook(info);
        }));
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .register_uri_scheme_protocol("vencord", mod_runtime::handle_protocol)
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            match voice::handle_second_instance(app, &args) {
                Ok(true) => {}
                Ok(false) => {
                    let _ = tray::restore_main_window(app);
                }
                Err(err) => {
                    log::warn!("Failed to dispatch second-instance voice toggle: {err}");
                }
            }
        }))
        .plugin(build_log_plugin())
        .on_menu_event(|app, event| {
            app_menu::handle_menu_event(app, event);
        })
        .setup(|app| {
            install_panic_hook(app.handle());
            app.manage(arrpc::RuntimeState::default());
            app.manage(notifications::RuntimeState::default());
            app.manage(desktop_stream::stream_session::RuntimeState::default());
            app.manage(processes::RuntimeState::default());
            app.manage(window::WebviewRuntimeState::default());
            app.manage(tray::RuntimeState::default());
            app.manage(updater::RuntimeState::default());
            app.manage(ipc_bridge::RuntimeState::default());
            let store = store::PersistedStore::load(app.handle())
                .map_err(|err| -> Box<dyn std::error::Error> { Box::new(err) })?;
            app.manage(store);
            window::create_main_window(app.handle()).map_err(
                |err| -> Box<dyn std::error::Error> { Box::new(std::io::Error::other(err)) },
            )?;
            if let Err(err) = protocol::init(app.handle()) {
                log::warn!("Failed to initialize deep-link protocol handling: {err}");
            }
            let snapshot = app.state::<store::PersistedStore>().snapshot();
            if let Err(err) = autostart::sync(app.handle(), &snapshot.settings) {
                log::warn!("Failed to sync autostart settings: {err}");
            }
            if let Err(err) = app_menu::sync(app.handle(), &snapshot.settings) {
                log::warn!("Failed to sync app menu settings: {err}");
            }
            if let Err(err) = tray::sync(app.handle(), &snapshot.settings) {
                log::warn!("Failed to sync tray settings: {err}");
            }
            if let Err(err) = arrpc::sync(app.handle(), &snapshot.settings) {
                log::warn!("Failed to sync arRPC runtime: {err}");
            }
            log::info!(
                "Starting {} {}",
                app.package_info().name,
                app.package_info().version
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            arrpc::get_arrpc_current_activity,
            arrpc::get_arrpc_status,
            arrpc::restart_arrpc,
            autostart::get_auto_start_status,
            autostart::set_auto_start_enabled,
            capturer::get_capturer_large_thumbnail,
            capturer::get_capturer_sources,
            capturer::get_capturer_thumbnail,
            csp::csp_is_domain_allowed,
            csp::csp_remove_override,
            csp::csp_request_add_override,
            desktop_host::app_relaunch,
            desktop_host::log_client_runtime,
            desktop_host::log_client_runtime_batch,
            desktop_host::window_close,
            desktop_host::window_focus,
            desktop_host::window_is_maximized,
            desktop_host::window_minimize,
            desktop_host::window_start_dragging,
            desktop_host::window_set_title,
            desktop_host::window_start_resize_dragging,
            desktop_host::window_toggle_maximize,
            doctor::run_doctor,
            discord::get_discord_target,
            discord::launch_discord,
            file_manager::choose_user_asset,
            file_manager::get_user_asset_data,
            file_manager::get_file_manager_state,
            file_manager::open_user_assets_folder,
            file_manager::select_vencord_dir,
            file_manager::show_custom_vencord_dir,
            http_proxy::proxy_http_request,
            ipc_bridge::respond_renderer_command,
            notifications::flash_frame,
            notifications::set_badge_count,
            desktop_stream::stream_session::get_desktop_stream_session_state,
            desktop_stream::stream_session::get_desktop_stream_encoder_preview,
            desktop_stream::stream_session::start_desktop_stream_session,
            desktop_stream::stream_session::stop_desktop_stream_session,
            mod_runtime::get_vencord_renderer_css,
            mod_runtime::get_vencord_file_state,
            mod_runtime::get_vencord_quick_css,
            mod_runtime::get_vencord_theme_entries,
            mod_runtime::get_vencord_theme_data,
            mod_runtime::get_vencord_settings_dir,
            mod_runtime::get_vencord_themes_dir,
            mod_runtime::get_vencord_themes_list,
            mod_runtime::delete_vencord_theme,
            mod_runtime::open_external_link,
            mod_runtime::open_vencord_quick_css,
            mod_runtime::open_vencord_settings_folder,
            mod_runtime::open_vencord_themes_folder,
            mod_runtime::set_vencord_quick_css,
            mod_runtime::set_vencord_theme_data,
            mod_runtime::set_vencord_settings,
            mod_runtime::upload_vencord_theme,
            spellcheck::check_spelling,
            store::get_store_snapshot,
            store::set_settings,
            store::set_state,
            tray::set_tray_voice_call_state,
            tray::set_tray_voice_state,
            updater::get_host_update_download_state,
            updater::get_host_update_status,
            updater::get_runtime_update_status,
            updater::ignore_host_update,
            updater::ignore_runtime_update,
            updater::install_host_update,
            updater::install_runtime_update,
            updater::open_host_update,
            updater::open_runtime_update,
            updater::snooze_host_update,
            updater::snooze_runtime_update,
            utilities::copy_image_to_clipboard,
            utilities::get_system_theme_values,
            utilities::open_debug_page,
            virtmic::virtmic_list,
            virtmic::virtmic_start,
            virtmic::virtmic_start_system,
            virtmic::virtmic_stop
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| {
            let message =
                privacy::sanitize_text_for_log(&format!("error while running Equirust: {err}"));
            log::error!("{}", message);
            eprintln!("{}", message);
        });
}
