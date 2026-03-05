use crate::{discord, privacy};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
pub fn proxy_http_request(request: HttpProxyRequest) -> Result<HttpProxyResponse, String> {
    log::info!(
        "Proxying cloud request method={} url={}",
        request.method.as_deref().unwrap_or("GET"),
        privacy::sanitize_url_for_log(&request.url)
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
    let mut builder = client.request(method.clone(), &request.url);

    if let Some(headers) = request.headers.as_ref() {
        for (name, value) in headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|err| err.to_string())?;
            let value =
                reqwest::header::HeaderValue::from_str(value).map_err(|err| err.to_string())?;
            builder = builder.header(name, value);
        }
    }

    builder = builder.header(
        reqwest::header::USER_AGENT,
        discord::standard_http_user_agent(),
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
