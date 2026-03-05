use crate::{
    privacy,
    settings::{DiscordBranch, PersistedState, Settings},
    store::PersistedStore,
};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    utils::config::BackgroundThrottlingPolicy, webview::NewWindowResponse, AppHandle, Manager,
    Runtime, State, Url, WebviewUrl, WebviewWindowBuilder,
};

pub const DISCORD_HOSTNAMES: [&str; 3] = ["discord.com", "canary.discord.com", "ptb.discord.com"];
static EXTERNAL_LINK_WINDOW_COUNTER: AtomicU64 = AtomicU64::new(1);

#[tauri::command]
pub fn get_discord_target(store: State<'_, PersistedStore>) -> Result<String, String> {
    let snapshot = store.snapshot();
    discord_target_url(&snapshot.settings, route_from_process_args()).map(|url| url.to_string())
}

#[tauri::command]
pub fn launch_discord(app: AppHandle, _store: State<'_, PersistedStore>) -> Result<String, String> {
    let url = navigate_main_window_to_route(&app, route_from_process_args())?;
    Ok(url.to_string())
}

pub fn navigate_main_window_to_route(
    app: &AppHandle,
    route: Option<String>,
) -> Result<Url, String> {
    let snapshot = app.state::<PersistedStore>().snapshot();
    let url = discord_target_url(&snapshot.settings, route)?;
    log::info!(
        "Navigating main window to {}",
        privacy::sanitize_url_for_log(url.as_str())
    );

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is not available".to_owned())?;

    window
        .navigate(url.clone())
        .map_err(|err| err.to_string())?;
    Ok(url)
}

pub fn browser_user_agent(_settings: &Settings) -> Option<String> {
    None
}

pub fn standard_http_user_agent() -> String {
    expected_edge_user_agent().unwrap_or_else(|_| {
        format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36 Edg/145.0.0.0",
            edge_platform_token()
        )
    })
}

pub fn discord_target_url(settings: &Settings, route: Option<String>) -> Result<Url, String> {
    build_discord_url(settings, route)
}

pub fn browser_additional_args(settings: &Settings, state: &PersistedState) -> Option<String> {
    let mut args = Vec::new();
    let mut enable_features = Vec::new();
    let mut disable_features = vec![
        "WinRetrieveSuggestionsOnlyOnDemand".to_owned(),
        "HardwareMediaKeyHandling".to_owned(),
        "MediaSessionService".to_owned(),
    ];

    if settings.hardware_acceleration == Some(false) {
        args.extend([
            "--disable-gpu".to_owned(),
            "--disable-gpu-compositing".to_owned(),
            "--disable-gpu-rasterization".to_owned(),
            "--disable-zero-copy".to_owned(),
        ]);
    } else if settings.hardware_video_acceleration != Some(false) {
        enable_features.extend([
            "AcceleratedVideoEncoder".to_owned(),
            "AcceleratedVideoDecoder".to_owned(),
        ]);

        #[cfg(target_os = "linux")]
        enable_features.extend([
            "AcceleratedVideoDecodeLinuxGL".to_owned(),
            "AcceleratedVideoDecodeLinuxZeroCopyGL".to_owned(),
        ]);
    }

    if settings.disable_smooth_scroll == Some(true) {
        args.push("--disable-smooth-scrolling".to_owned());
    }

    if settings.middle_click_autoscroll == Some(true) {
        args.push("--enable-blink-features=MiddleClickAutoscroll".to_owned());
    }

    args.push("--autoplay-policy=no-user-gesture-required".to_owned());
    args.push("--no-first-run".to_owned());

    #[cfg(target_os = "windows")]
    {
        disable_features.push("CalculateNativeWinOcclusion".to_owned());
        args.extend([
            "--edge-webview-foreground-boost-opt-in".to_owned(),
            "--msWebView2NativeEventDispatch".to_owned(),
            "--msWebView2CodeCache".to_owned(),
            "--UseNativeThreadPool".to_owned(),
            "--UseBackgroundNativeThreadPool".to_owned(),
        ]);
    }

    if !enable_features.is_empty() {
        args.push(format!("--enable-features={}", enable_features.join(",")));
    }

    if !disable_features.is_empty() {
        args.push(format!("--disable-features={}", disable_features.join(",")));
    }

    if let Some(extra) = state
        .launch_arguments
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(extra.to_owned());
    }

    if args.is_empty() {
        None
    } else {
        Some(args.join(" "))
    }
}

pub fn is_allowed_navigation(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return true;
    }

    let host = url.host_str().unwrap_or_default();
    host == "localhost"
        || host == "127.0.0.1"
        || host == "tauri.localhost"
        || DISCORD_HOSTNAMES.contains(&host)
}

pub fn handle_main_window_navigation<R: Runtime>(app: &AppHandle<R>, url: &Url) -> bool {
    if url.scheme() == "about" {
        return true;
    }

    if matches!(url.scheme(), "http" | "https") && is_allowed_navigation(url) {
        return true;
    }

    route_external_url(app, url);
    false
}

pub fn handle_main_window_new_window<R: Runtime>(
    app: &AppHandle<R>,
    url: &Url,
) -> NewWindowResponse<R> {
    if url.scheme() == "about" {
        return NewWindowResponse::Allow;
    }

    if matches!(url.scheme(), "http" | "https") && is_allowed_navigation(url) {
        return NewWindowResponse::Allow;
    }

    route_external_url(app, url);
    NewWindowResponse::Deny
}

pub fn route_external_url<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    if should_open_links_in_app(app, url) {
        if let Err(err) = open_external_link_window(app, url.clone()) {
            log::warn!(
                "Failed to open external link in-app ({}): {}",
                privacy::sanitize_url_for_log(url.as_str()),
                err
            );
            let _ = webbrowser::open(url.as_str());
        }
        return;
    }

    let _ = webbrowser::open(url.as_str());
}

fn build_discord_url(settings: &Settings, route: Option<String>) -> Result<Url, String> {
    let subdomain = match settings.discord_branch {
        Some(DiscordBranch::Canary) => "canary.",
        Some(DiscordBranch::Ptb) => "ptb.",
        _ => "",
    };

    let path = route.unwrap_or_else(|| "app".to_owned());
    Url::parse(&format!("https://{subdomain}discord.com/{path}")).map_err(|err| err.to_string())
}

pub fn route_from_process_args() -> Option<String> {
    std::env::args().find_map(|arg| {
        if !arg.starts_with("discord://") {
            return None;
        }

        let url = Url::parse(&arg).ok()?;
        route_from_url(&url).ok()
    })
}

pub fn route_from_url(url: &Url) -> Result<String, String> {
    let path = url.path().trim_matches('/');
    if !path.is_empty() {
        return Ok(path.to_owned());
    }

    let host = url.host_str().unwrap_or_default().trim_matches('/');
    if !host.is_empty() && host != "-" {
        return Ok(host.to_owned());
    }

    Ok("app".to_owned())
}

fn expected_edge_user_agent() -> Result<String, String> {
    let version = tauri::webview_version().map_err(|err| err.to_string())?;
    let platform = edge_platform_token();

    Ok(format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version} Safari/537.36 Edg/{version}"
    ))
}

fn edge_platform_token() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Windows NT 10.0; Win64; x64"
    }

    #[cfg(target_os = "macos")]
    {
        "Macintosh; Intel Mac OS X 10_15_7"
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "X11; Linux x86_64"
    }
}

fn should_open_links_in_app<R: Runtime>(app: &AppHandle<R>, url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && app
            .state::<PersistedStore>()
            .snapshot()
            .settings
            .open_links_with_electron
            == Some(true)
}

fn open_external_link_window<R: Runtime>(app: &AppHandle<R>, url: Url) -> Result<(), String> {
    let snapshot = app.state::<PersistedStore>().snapshot();
    let label = format!(
        "external-link-{}",
        EXTERNAL_LINK_WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let title = external_window_title(&url);
    let user_agent = browser_user_agent(&snapshot.settings);
    let additional_browser_args = browser_additional_args(&snapshot.settings, &snapshot.state);
    let popup_app = app.clone();

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url.clone()))
        .title(&title)
        .inner_size(1100.0, 760.0)
        .resizable(true)
        .focused(true)
        .background_throttling(BackgroundThrottlingPolicy::Disabled)
        .on_navigation(|next_url| {
            if matches!(next_url.scheme(), "http" | "https" | "about") {
                true
            } else {
                let _ = webbrowser::open(next_url.as_str());
                false
            }
        })
        .on_new_window(move |next_url, _features| {
            if matches!(next_url.scheme(), "http" | "https") {
                if let Err(err) = open_external_link_window(&popup_app, next_url.clone()) {
                    log::warn!(
                        "Failed to open popup external link in-app ({}): {}",
                        privacy::sanitize_url_for_log(next_url.as_str()),
                        err
                    );
                    let _ = webbrowser::open(next_url.as_str());
                }
            } else {
                let _ = webbrowser::open(next_url.as_str());
            }

            NewWindowResponse::Deny
        });

    if let Some(user_agent) = user_agent.as_deref() {
        builder = builder.user_agent(user_agent);
    }

    if let Some(additional_browser_args) = additional_browser_args.as_deref() {
        builder = builder.additional_browser_args(additional_browser_args);
    }

    builder.build().map_err(|err| err.to_string())?;
    log::info!(
        "Opened external link in-app: {}",
        privacy::sanitize_url_for_log(url.as_str())
    );
    Ok(())
}

fn external_window_title(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().trim();
    if host.is_empty() {
        "Equirust Link".into()
    } else {
        host.to_owned()
    }
}
