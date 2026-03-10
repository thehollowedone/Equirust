use crate::{paths::AppPaths, privacy};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::{fs, path::Path};
use tauri::AppHandle;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpProxyRequest {
    pub url: String,
    pub method: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub body_base64: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpProxyResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body_base64: String,
}

#[tauri::command]
pub async fn proxy_http_request(
    request: HttpProxyRequest,
    app: AppHandle,
) -> Result<HttpProxyResponse, String> {
    tauri::async_runtime::spawn_blocking(move || proxy_http_request_blocking(request, app))
        .await
        .map_err(|err| err.to_string())?
}

fn proxy_http_request_blocking(
    request: HttpProxyRequest,
    app: AppHandle,
) -> Result<HttpProxyResponse, String> {
    let parsed_url = reqwest::Url::parse(&request.url).map_err(|err| err.to_string())?;
    let configured_cloud_origin = load_configured_cloud_origin(&app);
    let is_configured_origin =
        configured_cloud_origin.as_ref() == Some(&parsed_url.origin().ascii_serialization());

    if !parsed_url.scheme().eq_ignore_ascii_case("https")
        && !(parsed_url.scheme().eq_ignore_ascii_case("http") && is_configured_origin)
    {
        return Err(
            "Cloud proxy only allows HTTPS targets or the exact configured local backend"
                .to_owned(),
        );
    }
    // This command exists only to bridge the Discord renderer to cloud endpoints that would
    // otherwise fail under page-level CSP/CORS. The host allows:
    // - the reviewed official hosted backends
    // - the exact origin the user explicitly configured in the Cloud settings tab
    //
    // The upstream Equicloud / Vencloud clients only use this API surface:
    // - GET    /v1/oauth/settings
    // - GET    /v1/oauth/callback?... (fetching the returned callback URL for the secret)
    // - HEAD   /v1/settings
    // - GET    /v1/settings
    // - PUT    /v1/settings
    // - DELETE /v1/settings
    // - DELETE /v1 or /v1/
    if !is_allowed_cloud_proxy_target(&parsed_url, configured_cloud_origin.as_deref()) {
        return Err("Cloud proxy host is not allowed".to_owned());
    }

    log::info!(
        "Proxying cloud request method={} url={}",
        request.method.as_deref().unwrap_or("GET"),
        privacy::sanitize_url_for_log(parsed_url.as_str())
    );

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;

    let method = request
        .method
        .as_deref()
        .unwrap_or("GET")
        .parse::<reqwest::Method>()
        .map_err(|err| err.to_string())?;
    if !is_allowed_cloud_api_request(&parsed_url, &method) {
        return Err("Cloud proxy path or method is not allowed".to_owned());
    }
    let mut builder = client.request(method.clone(), parsed_url);

    if let Some(headers) = request.headers.as_ref() {
        for (name, value) in headers {
            if is_blocked_request_header(name) {
                continue;
            }
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|err| err.to_string())?;
            let value =
                reqwest::header::HeaderValue::from_str(value).map_err(|err| err.to_string())?;
            builder = builder.header(name, value);
        }
    }

    builder = builder.header(
        reqwest::header::USER_AGENT,
        crate::browser_runtime::standard_http_user_agent(),
    );

    if method != reqwest::Method::GET && method != reqwest::Method::HEAD {
        if let Some(body_base64) = request.body_base64.as_ref() {
            let body = base64::engine::general_purpose::STANDARD
                .decode(body_base64)
                .map_err(|err| err.to_string())?;
            if !body.is_empty() {
                builder = builder.body(body);
            }
        }
    }

    let response = builder.send().map_err(|err| {
        log::warn!(
            "Cloud proxy request failed url={} error={}",
            privacy::sanitize_url_for_log(&request.url),
            err
        );
        err.to_string()
    })?;
    let status = response.status();
    let status_text = status.canonical_reason().unwrap_or_default().to_owned();
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| !is_blocked_response_header(name.as_str()))
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let body = response.bytes().map_err(|err| err.to_string())?;

    Ok(HttpProxyResponse {
        status: status.as_u16(),
        status_text,
        headers,
        body_base64: base64::engine::general_purpose::STANDARD.encode(body),
    })
}

fn is_official_cloud_proxy_host(url: &reqwest::Url) -> bool {
    static ALLOWLIST: OnceLock<[&'static str; 2]> = OnceLock::new();
    let allowlist = ALLOWLIST.get_or_init(|| ["cloud.equicord.org", "api.vencord.dev"]);
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }

    allowlist.iter().any(|entry| host == *entry)
}

fn is_allowed_cloud_proxy_target(url: &reqwest::Url, configured_origin: Option<&str>) -> bool {
    if is_official_cloud_proxy_host(url) {
        return true;
    }

    let Some(configured_origin) = configured_origin else {
        return false;
    };
    url.origin().ascii_serialization() == configured_origin
}

fn is_allowed_cloud_api_request(url: &reqwest::Url, method: &reqwest::Method) -> bool {
    let path = url.path();
    match (method.as_str(), path) {
        ("GET", "/v1/oauth/settings") => true,
        ("GET", "/v1/oauth/callback") => true,
        ("HEAD", "/v1/settings") => true,
        ("GET", "/v1/settings") => true,
        ("PUT", "/v1/settings") => true,
        ("DELETE", "/v1/settings") => true,
        ("DELETE", "/v1") => true,
        ("DELETE", "/v1/") => true,
        _ => false,
    }
}

fn load_configured_cloud_origin(app: &AppHandle) -> Option<String> {
    let paths = AppPaths::resolve(app).ok()?;
    let settings = read_json(&paths.vencord_settings_file).ok()?;
    let cloud_url = settings
        .get("cloud")
        .and_then(Value::as_object)
        .and_then(|cloud| cloud.get("url"))
        .and_then(Value::as_str)?
        .trim();
    let parsed = reqwest::Url::parse(cloud_url).ok()?;
    let scheme = parsed.scheme().to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }

    Some(parsed.origin().ascii_serialization())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let contents = fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(&contents).map_err(|err| err.to_string())
}

fn is_blocked_request_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        "authorization" | "accept" | "content-type" | "if-none-match"
    )
}

fn is_blocked_response_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !matches!(lower.as_str(), "content-type" | "etag")
}
