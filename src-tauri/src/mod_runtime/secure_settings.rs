use base64::Engine;
use serde_json::{Map, Value};
use std::{io, path::Path};

use super::{
    read_json, write_pretty_json, PROTECTED_VALUE_CIPHERTEXT_KEY, PROTECTED_VALUE_MARKER,
    PROTECTED_VALUE_MARKER_KEY,
};

pub(super) fn read_secure_vencord_settings(path: &Path) -> io::Result<Value> {
    let mut raw = read_json(path)?;
    decrypt_sensitive_values(&mut raw);

    let mut protected_copy = raw.clone();
    protect_sensitive_values(None, &mut protected_copy);
    if protected_copy != read_json(path)? {
        write_pretty_json(path, &protected_copy)?;
    }

    Ok(raw)
}

pub(super) fn write_secure_vencord_settings(path: &Path, settings: &Value) -> io::Result<()> {
    let mut protected = settings.clone();
    prune_empty_sensitive_values(&mut protected);
    protect_sensitive_values(None, &mut protected);
    write_pretty_json(path, &protected)
}

fn prune_empty_sensitive_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if is_protected_value_object(map) {
                return;
            }

            for child in map.values_mut() {
                prune_empty_sensitive_values(child);
            }

            map.retain(|key, child| {
                if is_sensitive_setting_key(key) && is_empty_secret_value(child) {
                    return false;
                }
                true
            });
        }
        Value::Array(values) => {
            for child in values.iter_mut() {
                prune_empty_sensitive_values(child);
            }
        }
        _ => {}
    }
}

fn is_empty_secret_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

fn protect_sensitive_values(current_key: Option<&str>, value: &mut Value) {
    match value {
        Value::Object(map) => {
            if is_protected_value_object(map) {
                return;
            }

            for (key, child) in map.iter_mut() {
                protect_sensitive_values(Some(key), child);
            }
        }
        Value::Array(values) => {
            for child in values.iter_mut() {
                protect_sensitive_values(current_key, child);
            }
        }
        Value::String(text) => {
            let Some(key) = current_key else {
                return;
            };
            if !is_sensitive_setting_key(key) || text.trim().is_empty() {
                return;
            }

            match protect_secret_value(text) {
                Ok(ciphertext) => {
                    *value = Value::Object(Map::from_iter([
                        (
                            PROTECTED_VALUE_MARKER_KEY.to_owned(),
                            Value::String(PROTECTED_VALUE_MARKER.to_owned()),
                        ),
                        (
                            PROTECTED_VALUE_CIPHERTEXT_KEY.to_owned(),
                            Value::String(ciphertext),
                        ),
                    ]));
                }
                Err(err) => {
                    log::warn!("Failed to protect sensitive setting key {}: {}", key, err);
                }
            }
        }
        _ => {}
    }
}

fn decrypt_sensitive_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(ciphertext) = extract_protected_ciphertext(map) {
                match unprotect_secret_value(ciphertext) {
                    Ok(plaintext) => {
                        *value = Value::String(plaintext);
                    }
                    Err(err) => {
                        log::warn!("Failed to decrypt protected Vencord setting: {}", err);
                        *value = Value::String(String::new());
                    }
                }
                return;
            }

            for child in map.values_mut() {
                decrypt_sensitive_values(child);
            }
        }
        Value::Array(values) => {
            for child in values.iter_mut() {
                decrypt_sensitive_values(child);
            }
        }
        _ => {}
    }
}

fn is_protected_value_object(map: &Map<String, Value>) -> bool {
    matches!(
        (
            map.get(PROTECTED_VALUE_MARKER_KEY).and_then(Value::as_str),
            map.get(PROTECTED_VALUE_CIPHERTEXT_KEY)
                .and_then(Value::as_str),
        ),
        (Some(PROTECTED_VALUE_MARKER), Some(_))
    )
}

fn extract_protected_ciphertext(map: &Map<String, Value>) -> Option<&str> {
    match (
        map.get(PROTECTED_VALUE_MARKER_KEY).and_then(Value::as_str),
        map.get(PROTECTED_VALUE_CIPHERTEXT_KEY)
            .and_then(Value::as_str),
    ) {
        (Some(PROTECTED_VALUE_MARKER), Some(ciphertext)) => Some(ciphertext),
        _ => None,
    }
}

fn is_sensitive_setting_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty() {
        return false;
    }

    if matches!(
        key.as_str(),
        "keyboard" | "hotkey" | "keybind" | "keybinds" | "shortcut" | "shortcuts"
    ) {
        return false;
    }

    if matches!(
        key.as_str(),
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
            | "webhook"
            | "key"
    ) {
        return true;
    }

    [
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
        "webhook",
    ]
    .iter()
    .any(|needle| {
        key.ends_with(needle)
            || key.contains(&format!("_{needle}"))
            || key.contains(&format!("-{needle}"))
    })
}

#[cfg(target_os = "windows")]
fn protect_secret_value(value: &str) -> io::Result<String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut input = value.as_bytes().to_vec();
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_mut_ptr(),
    };
    let mut output_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &input_blob,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output_blob,
        )
        .map_err(|err| io::Error::other(format!("CryptProtectData failed: {err}")))?;

        let bytes = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let _ = LocalFree(Some(HLOCAL(output_blob.pbData.cast())));
        Ok(encoded)
    }
}

#[cfg(not(target_os = "windows"))]
fn protect_secret_value(_value: &str) -> io::Result<String> {
    Err(io::Error::other(
        "secret encryption is not available on this platform",
    ))
}

#[cfg(target_os = "windows")]
fn unprotect_secret_value(value: &str) -> io::Result<String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut ciphertext = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(io::Error::other)?;
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_mut_ptr(),
    };
    let mut output_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(
            &input_blob,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output_blob,
        )
        .map_err(|err| io::Error::other(format!("CryptUnprotectData failed: {err}")))?;

        let bytes = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
        let plaintext = String::from_utf8(bytes.to_vec()).map_err(io::Error::other)?;
        let _ = LocalFree(Some(HLOCAL(output_blob.pbData.cast())));
        Ok(plaintext)
    }
}

#[cfg(not(target_os = "windows"))]
fn unprotect_secret_value(_value: &str) -> io::Result<String> {
    Err(io::Error::other(
        "secret decryption is not available on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        extract_protected_ciphertext, is_protected_value_object, is_sensitive_setting_key,
        prune_empty_sensitive_values,
    };
    use crate::mod_runtime::{
        PROTECTED_VALUE_CIPHERTEXT_KEY, PROTECTED_VALUE_MARKER, PROTECTED_VALUE_MARKER_KEY,
    };
    use serde_json::{json, Map, Value};

    #[test]
    fn detects_sensitive_setting_keys() {
        assert!(is_sensitive_setting_key("token"));
        assert!(is_sensitive_setting_key("discordAuthToken"));
        assert!(is_sensitive_setting_key("api_key"));
        assert!(is_sensitive_setting_key("clientSecret"));
        assert!(!is_sensitive_setting_key("keyboard"));
        assert!(!is_sensitive_setting_key("keybind"));
    }

    #[test]
    fn detects_protected_marker_object() {
        let map = Map::from_iter([
            (
                PROTECTED_VALUE_MARKER_KEY.to_owned(),
                Value::String(PROTECTED_VALUE_MARKER.to_owned()),
            ),
            (
                PROTECTED_VALUE_CIPHERTEXT_KEY.to_owned(),
                Value::String("abc".to_owned()),
            ),
        ]);

        assert!(is_protected_value_object(&map));
        assert_eq!(extract_protected_ciphertext(&map), Some("abc"));
    }

    #[test]
    fn removes_empty_sensitive_values_before_write() {
        let mut value = json!({
            "plugins": {
                "Example": {
                    "token": "",
                    "apiKey": "abc123",
                    "password": null,
                    "safeField": ""
                }
            }
        });

        prune_empty_sensitive_values(&mut value);
        let settings = &value["plugins"]["Example"];
        assert!(settings.get("token").is_none());
        assert!(settings.get("password").is_none());
        assert_eq!(
            settings.get("apiKey").and_then(Value::as_str),
            Some("abc123")
        );
        assert_eq!(settings.get("safeField").and_then(Value::as_str), Some(""));
    }
}
