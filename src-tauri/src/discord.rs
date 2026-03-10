use crate::{
    browser_runtime, privacy,
    settings::{DiscordBranch, Settings},
    store::PersistedStore,
};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    webview::NewWindowResponse, window::Color, AppHandle, Manager, Runtime, State, Url, WebviewUrl,
    WebviewWindowBuilder,
};

pub const DISCORD_HOSTNAMES: [&str; 3] = ["discord.com", "canary.discord.com", "ptb.discord.com"];
static EXTERNAL_LINK_WINDOW_COUNTER: AtomicU64 = AtomicU64::new(1);
const DEFAULT_WEBVIEW_BACKGROUND: Color = Color(11, 16, 32, 255);

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

pub fn discord_target_url(settings: &Settings, route: Option<String>) -> Result<Url, String> {
    build_discord_url(settings, route)
}

pub fn is_allowed_navigation(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return true;
    }

    let host = url.host_str().unwrap_or_default();
    DISCORD_HOSTNAMES.contains(&host)
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
    let browser_options =
        browser_runtime::external_window_options(&snapshot.settings, &snapshot.state);
    let popup_app = app.clone();

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url.clone()))
        .title(&title)
        .inner_size(1100.0, 760.0)
        .resizable(true)
        .focused(true)
        .background_color(DEFAULT_WEBVIEW_BACKGROUND)
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

    if let Some(user_agent) = browser_options.user_agent.as_deref() {
        builder = builder.user_agent(user_agent);
    }

    if let Some(additional_browser_args) = browser_options.additional_args.as_deref() {
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
