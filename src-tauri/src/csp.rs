use crate::{privacy, settings::CspOverride, store::PersistedStore};
use std::{borrow::Cow, collections::HashMap};
use tauri::{
    http::{header::HeaderValue, Request, Response},
    utils::config::{Csp, CspDirectiveSources},
    AppHandle, Emitter, Manager, State as TauriState,
};
use url::Url;

const DEFAULT_VENCORD_CSP_ORIGIN: &str = "https://api.vencord.dev";
const DEFAULT_VENCORD_CSP_DIRECTIVES: &[&str] = &["connect-src"];

#[tauri::command]
pub fn csp_is_domain_allowed(
    url: String,
    directives: Vec<String>,
    store: TauriState<'_, PersistedStore>,
) -> Result<bool, String> {
    let origin = normalize_origin(&url)?;
    let normalized_directives = normalize_directives(directives);
    Ok(
        all_active_overrides(&store.snapshot().settings.csp_overrides)
            .into_iter()
            .any(|override_entry| {
                override_entry.origin.eq_ignore_ascii_case(&origin)
                    && directives_match(&override_entry.directives, &normalized_directives)
            }),
    )
}

#[tauri::command]
pub fn csp_request_add_override(
    url: String,
    directives: Vec<String>,
    reason: Option<String>,
    store: TauriState<'_, PersistedStore>,
    app: AppHandle,
) -> Result<String, String> {
    let origin = normalize_origin(&url)?;
    let normalized_directives = normalize_directives(directives);
    let snapshot = store.snapshot();

    if all_active_overrides(&snapshot.settings.csp_overrides)
        .into_iter()
        .any(|override_entry| {
            override_entry.origin.eq_ignore_ascii_case(&origin)
                && directives_match(&override_entry.directives, &normalized_directives)
        })
    {
        return Ok("already-allowed".into());
    }

    let label = reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("A desktop integration");
    let directives_label = normalized_directives.join(", ");
    let approved = rfd::MessageDialog::new()
        .set_title("Allow Domain For Equirust")
        .set_description(format!(
            "{label} wants to allow {origin} for {directives_label} on Discord pages.\n\nThis changes the page Content Security Policy and takes effect after restart."
        ))
        .set_buttons(rfd::MessageButtons::OkCancel)
        .set_level(rfd::MessageLevel::Info)
        .show();

    if !matches!(approved, rfd::MessageDialogResult::Ok) {
        return Ok("cancelled".into());
    }

    let mut settings = snapshot.settings.clone();
    let mut overrides = settings.csp_overrides.take().unwrap_or_default();
    if let Some(existing) = overrides
        .iter_mut()
        .find(|entry| entry.origin.eq_ignore_ascii_case(&origin))
    {
        for directive in &normalized_directives {
            if !existing
                .directives
                .iter()
                .any(|current| current.eq_ignore_ascii_case(directive))
            {
                existing.directives.push(directive.clone());
            }
        }
        existing.directives.sort();
        existing.directives.dedup();
    } else {
        overrides.push(CspOverride {
            origin: origin.clone(),
            directives: normalized_directives.clone(),
        });
    }
    overrides.sort_by(|left, right| left.origin.cmp(&right.origin));
    settings.csp_overrides = Some(overrides);
    store
        .replace_settings(settings)
        .map_err(|err| err.to_string())?;

    let _ = app.emit("equirust:csp-overrides-updated", ());
    log::info!(
        "Added CSP override origin={} directives={}",
        privacy::sanitize_url_for_log(&origin),
        normalized_directives.join(",")
    );
    Ok("ok".into())
}

#[tauri::command]
pub fn csp_remove_override(
    url: String,
    store: TauriState<'_, PersistedStore>,
    app: AppHandle,
) -> Result<bool, String> {
    let origin = normalize_origin(&url)?;
    let snapshot = store.snapshot();
    let mut settings = snapshot.settings.clone();
    let mut overrides = settings.csp_overrides.take().unwrap_or_default();
    let previous_len = overrides.len();
    overrides.retain(|entry| !entry.origin.eq_ignore_ascii_case(&origin));
    let removed = overrides.len() != previous_len;
    settings.csp_overrides = (!overrides.is_empty()).then_some(overrides);
    store
        .replace_settings(settings)
        .map_err(|err| err.to_string())?;

    if removed {
        let _ = app.emit("equirust:csp-overrides-updated", ());
        log::info!(
            "Removed CSP override origin={}",
            privacy::sanitize_url_for_log(&origin)
        );
    }

    Ok(removed)
}

pub fn apply_response_overrides(
    app: &AppHandle,
    request: &Request<Vec<u8>>,
    response: &mut Response<Cow<'static, [u8]>>,
) {
    if !is_discord_document_request(request) {
        return;
    }

    let Some(header) = response.headers_mut().get_mut("Content-Security-Policy") else {
        return;
    };

    let overrides = app
        .try_state::<PersistedStore>()
        .map(|store| all_active_overrides(&store.snapshot().settings.csp_overrides))
        .unwrap_or_default();
    if overrides.is_empty() {
        return;
    }

    let Ok(current) = header.to_str() else {
        return;
    };
    let mut csp_map: HashMap<String, CspDirectiveSources> = Csp::Policy(current.to_owned()).into();
    let mut changed = false;

    for override_entry in overrides {
        for directive in override_entry.directives {
            let sources = csp_map.entry(directive).or_default();
            if !sources.contains(&override_entry.origin) {
                sources.push(override_entry.origin.clone());
                changed = true;
            }
        }
    }

    if !changed {
        return;
    }

    let next_policy = Csp::from(csp_map).to_string();
    if let Ok(value) = HeaderValue::from_str(&next_policy) {
        *header = value;
    }
}

fn normalize_origin(input: &str) -> Result<String, String> {
    let parsed = Url::parse(input)
        .or_else(|_| Url::parse(&format!("https://{input}")))
        .map_err(|err| err.to_string())?;
    let scheme = parsed.scheme();
    let host = parsed
        .host_str()
        .ok_or_else(|| "CSP override URL is missing a host".to_owned())?;
    let port = parsed.port();

    let origin = match port {
        Some(port) if !((scheme == "https" && port == 443) || (scheme == "http" && port == 80)) => {
            format!("{scheme}://{host}:{port}")
        }
        _ => format!("{scheme}://{host}"),
    };

    Ok(origin)
}

fn normalize_directives(mut directives: Vec<String>) -> Vec<String> {
    directives.retain(|directive| !directive.trim().is_empty());
    directives = directives
        .into_iter()
        .map(|directive| directive.trim().to_ascii_lowercase())
        .collect();
    directives.sort();
    directives.dedup();
    if directives.is_empty() {
        directives.push("connect-src".into());
    }
    directives
}

fn directives_match(allowed: &[String], requested: &[String]) -> bool {
    requested.iter().all(|directive| {
        allowed
            .iter()
            .any(|allowed_directive| allowed_directive.eq_ignore_ascii_case(directive))
    })
}

fn all_active_overrides(user_overrides: &Option<Vec<CspOverride>>) -> Vec<CspOverride> {
    let mut overrides = default_overrides();
    overrides.extend(user_overrides.clone().unwrap_or_default());
    overrides
}

fn default_overrides() -> Vec<CspOverride> {
    vec![CspOverride {
        origin: DEFAULT_VENCORD_CSP_ORIGIN.to_owned(),
        directives: DEFAULT_VENCORD_CSP_DIRECTIVES
            .iter()
            .map(|directive| (*directive).to_owned())
            .collect(),
    }]
}

fn is_discord_document_request(request: &Request<Vec<u8>>) -> bool {
    let uri = request.uri();
    let Some(host) = uri.host() else {
        return false;
    };
    let path = uri.path();
    host.ends_with("discord.com")
        && (path == "/" || path.starts_with("/app") || path.starts_with("/channels"))
}
