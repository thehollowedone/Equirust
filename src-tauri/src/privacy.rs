use std::{env, path::Path};
use url::{form_urlencoded::Serializer, Url};

const REDACTED_VALUE: &str = "<redacted>";

pub fn sanitize_text_for_log(input: &str) -> String {
    if input.trim().is_empty() {
        return input.to_owned();
    }

    let mut sanitized = String::new();

    for (index, token) in input.split_whitespace().enumerate() {
        if index > 0 {
            sanitized.push(' ');
        }
        sanitized.push_str(&sanitize_token(token));
    }

    if sanitized.is_empty() {
        sanitize_path_string(input)
    } else {
        sanitized
    }
}

pub fn sanitize_url_for_log(input: &str) -> String {
    let Ok(mut url) = Url::parse(input) else {
        return sanitize_path_string(input);
    };

    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);

    if let Some(segments) = url.path_segments() {
        let sanitized_segments = segments.map(sanitize_path_segment).collect::<Vec<_>>();
        if let Ok(mut path_segments) = url.path_segments_mut() {
            path_segments.clear();
            for segment in &sanitized_segments {
                path_segments.push(segment);
            }
        }
    }

    let sanitized_query = url.query_pairs().fold(None, |state, (key, _value)| {
        let mut serializer = state.unwrap_or_else(|| Serializer::new(String::new()));
        serializer.append_pair(&key, REDACTED_VALUE);
        Some(serializer)
    });
    let sanitized_query = sanitized_query.map(|mut query| query.finish());
    url.set_query(sanitized_query.as_deref());

    url.to_string()
}

pub fn file_name_for_log(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "<unknown>".into())
}

fn sanitize_token(token: &str) -> String {
    let (prefix, core, suffix) = split_token(token);
    if core.is_empty() {
        return token.to_owned();
    }

    if let Some(redacted) = redact_sensitive_assignment(core) {
        return format!("{prefix}{redacted}{suffix}");
    }

    if looks_like_bearer(core) {
        return format!("{prefix}Bearer {REDACTED_VALUE}{suffix}");
    }

    let sanitized = if looks_like_url(core) {
        sanitize_url_for_log(core)
    } else {
        sanitize_path_string(core)
    };

    format!("{prefix}{sanitized}{suffix}")
}

fn redact_sensitive_assignment(value: &str) -> Option<String> {
    for separator in ['=', ':'] {
        let Some(index) = value.find(separator) else {
            continue;
        };

        let key = value[..index].trim();
        let secret = value[index + 1..].trim();
        if key.is_empty() || secret.is_empty() {
            continue;
        }

        if is_sensitive_log_key(key) {
            return Some(format!("{key}{separator}{REDACTED_VALUE}"));
        }
    }

    None
}

fn is_sensitive_log_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    matches!(
        normalized.as_str(),
        "token"
            | "secret"
            | "password"
            | "passphrase"
            | "apikey"
            | "api_key"
            | "clientsecret"
            | "client_secret"
            | "authorization"
            | "auth"
            | "oauth"
            | "bearer"
    ) || [
        "token",
        "secret",
        "password",
        "passphrase",
        "apikey",
        "api_key",
        "api-key",
        "clientsecret",
        "client_secret",
        "authorization",
        "oauth",
    ]
    .iter()
    .any(|needle| {
        normalized.ends_with(needle)
            || normalized.contains(&format!("_{needle}"))
            || normalized.contains(&format!("-{needle}"))
    })
}

fn looks_like_bearer(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.starts_with("bearer ")
}

fn split_token(token: &str) -> (&str, &str, &str) {
    let prefix_len = token
        .char_indices()
        .find_map(|(index, ch)| (!is_wrapper_char(ch)).then_some(index))
        .unwrap_or(token.len());

    let suffix_start = token
        .char_indices()
        .rev()
        .find_map(|(index, ch)| (!is_wrapper_char(ch)).then_some(index + ch.len_utf8()))
        .unwrap_or(prefix_len);

    let prefix = &token[..prefix_len];
    let core = &token[prefix_len..suffix_start];
    let suffix = &token[suffix_start..];
    (prefix, core, suffix)
}

fn is_wrapper_char(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | ',' | ';'
    )
}

fn looks_like_url(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("discord://")
        || value.starts_with("file://")
}

fn sanitize_path_segment(segment: &str) -> String {
    if segment.eq_ignore_ascii_case("@me") {
        return segment.to_owned();
    }

    if segment.len() >= 5 && segment.chars().all(|ch| ch.is_ascii_digit()) {
        return "<id>".into();
    }

    if segment.len() >= 16
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return "<opaque>".into();
    }

    segment.to_owned()
}

fn sanitize_path_string(input: &str) -> String {
    let mut sanitized = input.to_owned();

    for (env_key, placeholder) in [
        ("LOCALAPPDATA", "%LOCALAPPDATA%"),
        ("APPDATA", "%APPDATA%"),
        ("USERPROFILE", "%USERPROFILE%"),
        ("HOME", "%HOME%"),
        ("TEMP", "%TEMP%"),
        ("TMP", "%TEMP%"),
    ] {
        if let Some(value) = env::var_os(env_key) {
            sanitized = replace_case_insensitive(&sanitized, &value.to_string_lossy(), placeholder);
        }
    }

    if let Ok(username) = env::var("USERNAME") {
        sanitized = replace_case_insensitive(&sanitized, &username, "<user>");
    }

    sanitized
}

fn replace_case_insensitive(input: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return input.to_owned();
    }

    let mut result = String::new();
    let haystack = input.to_lowercase();
    let needle = from.to_lowercase();
    let mut search_start = 0;

    while let Some(relative_index) = haystack[search_start..].find(&needle) {
        let absolute_index = search_start + relative_index;
        result.push_str(&input[search_start..absolute_index]);
        result.push_str(to);
        search_start = absolute_index + from.len();
    }

    result.push_str(&input[search_start..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_query_values_from_urls() {
        let sanitized =
            sanitize_url_for_log("https://api.vencord.dev/v1/oauth/callback?code=abc&state=xyz");
        assert!(sanitized.contains("code=%3Credacted%3E"));
        assert!(sanitized.contains("state=%3Credacted%3E"));
        assert!(!sanitized.contains("abc"));
        assert!(!sanitized.contains("xyz"));
    }

    #[test]
    fn redacts_explicit_path_prefix() {
        let sanitized = replace_case_insensitive(
            r#"C:\Users\Equirust\Documents\Equirust\target\debug\equirust.exe"#,
            r#"C:\Users\Equirust"#,
            "%USERPROFILE%",
        );
        assert_eq!(
            sanitized,
            r#"%USERPROFILE%\Documents\Equirust\target\debug\equirust.exe"#
        );
    }

    #[test]
    fn redacts_sensitive_assignments() {
        let sanitized = sanitize_text_for_log("apiKey=abc123 token:xyz Authorization:Bearer qwerty");
        assert!(sanitized.contains("apiKey=<redacted>"));
        assert!(sanitized.contains("token:<redacted>"));
        assert!(sanitized.contains("Authorization:<redacted>"));
    }
}
