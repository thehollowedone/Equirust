use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscordBranch {
    Stable,
    Canary,
    Ptb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransparencyOption {
    None,
    Mica,
    Tabbed,
    Acrylic,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    pub device_select: Option<bool>,
    pub granular_select: Option<bool>,
    pub ignore_virtual: Option<bool>,
    pub ignore_devices: Option<bool>,
    pub ignore_input_media: Option<bool>,
    pub only_speakers: Option<bool>,
    pub only_default_speakers: Option<bool>,
}

impl AudioSettings {
    pub fn with_fallbacks(mut self, defaults: &Self) -> Self {
        macro_rules! apply_option_default {
            ($field:ident) => {
                if self.$field.is_none() {
                    self.$field = defaults.$field;
                }
            };
        }

        apply_option_default!(device_select);
        apply_option_default!(granular_select);
        apply_option_default!(ignore_virtual);
        apply_option_default!(ignore_devices);
        apply_option_default!(ignore_input_media);
        apply_option_default!(only_speakers);
        apply_option_default!(only_default_speakers);

        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CspOverride {
    pub origin: String,
    pub directives: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub discord_branch: Option<DiscordBranch>,
    pub transparency_option: Option<TransparencyOption>,
    pub tray: Option<bool>,
    pub minimize_to_tray: Option<bool>,
    pub auto_start_minimized: Option<bool>,
    pub middle_click_autoscroll: Option<bool>,
    pub mouse_side_buttons_navigation: Option<bool>,
    pub open_links_with_electron: Option<bool>,
    pub static_title: Option<bool>,
    pub enable_menu: Option<bool>,
    pub disable_smooth_scroll: Option<bool>,
    pub hardware_acceleration: Option<bool>,
    pub hardware_video_acceleration: Option<bool>,
    #[serde(alias = "arRPC")]
    pub ar_rpc: Option<bool>,
    #[serde(alias = "arRPCDisabled")]
    pub ar_rpc_disabled: Option<bool>,
    #[serde(alias = "arRPCProcessScanning")]
    pub ar_rpc_process_scanning: Option<bool>,
    #[serde(alias = "arRPCBridge")]
    pub ar_rpc_bridge: Option<bool>,
    #[serde(alias = "arRPCBridgePort")]
    pub ar_rpc_bridge_port: Option<u16>,
    #[serde(alias = "arRPCBridgeHost")]
    pub ar_rpc_bridge_host: Option<String>,
    #[serde(alias = "arRPCWebSocketHost")]
    pub ar_rpc_web_socket_host: Option<String>,
    #[serde(alias = "arRPCWebSocketAutoReconnect")]
    pub ar_rpc_web_socket_auto_reconnect: Option<bool>,
    #[serde(alias = "arRPCWebSocketReconnectInterval")]
    pub ar_rpc_web_socket_reconnect_interval: Option<u64>,
    #[serde(alias = "arRPCWebSocketCustomHost")]
    pub ar_rpc_web_socket_custom_host: Option<String>,
    #[serde(alias = "arRPCWebSocketCustomPort")]
    pub ar_rpc_web_socket_custom_port: Option<u16>,
    pub app_badge: Option<bool>,
    pub badge_only_for_mentions: Option<bool>,
    pub enable_taskbar_flashing: Option<bool>,
    pub disable_min_size: Option<bool>,
    pub click_tray_to_show_hide: Option<bool>,
    pub custom_title_bar: Option<bool>,
    pub enable_splash_screen: Option<bool>,
    pub splash_theming: Option<bool>,
    pub splash_color: Option<String>,
    pub splash_background: Option<String>,
    pub splash_progress: Option<bool>,
    pub splash_pixelated: Option<bool>,
    pub csp_overrides: Option<Vec<CspOverride>>,
    pub spell_check_languages: Option<Vec<String>>,
    pub spell_check_dictionary: Option<Vec<String>>,
    pub audio: Option<AudioSettings>,
    #[cfg(debug_assertions)]
    #[serde(alias = "runtimeDiagnostics", alias = "arRPCDebug")]
    pub debug_standard_diagnostics: Option<bool>,
    #[cfg(debug_assertions)]
    pub debug_media_diagnostics: Option<bool>,
}

impl Settings {
    pub fn equirust_defaults() -> Self {
        Self {
            discord_branch: Some(DiscordBranch::Stable),
            transparency_option: Some(TransparencyOption::None),
            tray: Some(true),
            minimize_to_tray: Some(true),
            auto_start_minimized: Some(false),
            middle_click_autoscroll: Some(false),
            mouse_side_buttons_navigation: Some(true),
            open_links_with_electron: Some(false),
            static_title: Some(false),
            enable_menu: Some(false),
            disable_smooth_scroll: Some(false),
            hardware_acceleration: Some(false),
            hardware_video_acceleration: Some(false),
            ar_rpc: Some(true),
            ar_rpc_disabled: Some(false),
            ar_rpc_process_scanning: Some(true),
            ar_rpc_bridge: None,
            ar_rpc_bridge_port: None,
            ar_rpc_bridge_host: None,
            ar_rpc_web_socket_host: None,
            ar_rpc_web_socket_auto_reconnect: Some(true),
            ar_rpc_web_socket_reconnect_interval: None,
            ar_rpc_web_socket_custom_host: None,
            ar_rpc_web_socket_custom_port: None,
            app_badge: Some(true),
            badge_only_for_mentions: Some(true),
            enable_taskbar_flashing: Some(false),
            disable_min_size: Some(false),
            click_tray_to_show_hide: Some(false),
            custom_title_bar: Some(cfg!(target_os = "windows")),
            enable_splash_screen: Some(false),
            splash_theming: Some(false),
            splash_color: None,
            splash_background: None,
            splash_progress: Some(false),
            splash_pixelated: Some(false),
            csp_overrides: None,
            spell_check_languages: None,
            spell_check_dictionary: None,
            audio: Some(AudioSettings::default()),
            #[cfg(debug_assertions)]
            debug_standard_diagnostics: Some(true),
            #[cfg(debug_assertions)]
            debug_media_diagnostics: Some(true),
        }
    }

    pub fn with_fallbacks(mut self, defaults: &Self) -> Self {
        macro_rules! apply_option_default {
            ($field:ident) => {
                if self.$field.is_none() {
                    self.$field = defaults.$field.clone();
                }
            };
        }

        apply_option_default!(discord_branch);
        apply_option_default!(transparency_option);
        apply_option_default!(tray);
        apply_option_default!(minimize_to_tray);
        apply_option_default!(auto_start_minimized);
        apply_option_default!(middle_click_autoscroll);
        apply_option_default!(mouse_side_buttons_navigation);
        apply_option_default!(open_links_with_electron);
        apply_option_default!(static_title);
        apply_option_default!(enable_menu);
        apply_option_default!(disable_smooth_scroll);
        apply_option_default!(hardware_acceleration);
        apply_option_default!(hardware_video_acceleration);
        apply_option_default!(ar_rpc);
        apply_option_default!(ar_rpc_disabled);
        apply_option_default!(ar_rpc_process_scanning);
        apply_option_default!(ar_rpc_bridge);
        apply_option_default!(ar_rpc_bridge_port);
        apply_option_default!(ar_rpc_bridge_host);
        apply_option_default!(ar_rpc_web_socket_host);
        apply_option_default!(ar_rpc_web_socket_auto_reconnect);
        apply_option_default!(ar_rpc_web_socket_reconnect_interval);
        apply_option_default!(ar_rpc_web_socket_custom_host);
        apply_option_default!(ar_rpc_web_socket_custom_port);
        apply_option_default!(app_badge);
        apply_option_default!(badge_only_for_mentions);
        apply_option_default!(enable_taskbar_flashing);
        apply_option_default!(disable_min_size);
        apply_option_default!(click_tray_to_show_hide);
        apply_option_default!(custom_title_bar);
        apply_option_default!(enable_splash_screen);
        apply_option_default!(splash_theming);
        apply_option_default!(splash_color);
        apply_option_default!(splash_background);
        apply_option_default!(splash_progress);
        apply_option_default!(splash_pixelated);
        apply_option_default!(csp_overrides);
        apply_option_default!(spell_check_languages);
        apply_option_default!(spell_check_dictionary);
        #[cfg(debug_assertions)]
        apply_option_default!(debug_standard_diagnostics);
        #[cfg(debug_assertions)]
        apply_option_default!(debug_media_diagnostics);

        self.audio = Some(
            self.audio
                .take()
                .unwrap_or_default()
                .with_fallbacks(&defaults.audio.clone().unwrap_or_default()),
        );

        self
    }

    pub fn debug_standard_diagnostics_enabled(&self) -> bool {
        if crate::browser_runtime::profiling_diagnostics_enabled() {
            return true;
        }

        #[cfg(debug_assertions)]
        {
            self.debug_standard_diagnostics.unwrap_or(true)
        }

        #[cfg(not(debug_assertions))]
        {
            false
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterState {
    pub ignored_version: Option<String>,
    pub snooze_until: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedState {
    pub maximized: Option<bool>,
    pub minimized: Option<bool>,
    pub window_bounds: Option<WindowBounds>,
    pub equicord_dir: Option<String>,
    pub launch_arguments: Option<String>,
    pub host_updater: Option<UpdaterState>,
    pub runtime_updater: Option<UpdaterState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updater: Option<UpdaterState>,
}

impl PersistedState {
    pub fn normalize(mut self) -> Self {
        if self.runtime_updater.is_none() {
            self.runtime_updater = self.updater.clone();
        }

        self.updater = None;
        self
    }
}
