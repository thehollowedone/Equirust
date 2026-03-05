use crate::store::PersistedStore;
use serde::Serialize;
use std::collections::BTreeSet;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpellcheckResult {
    pub misspelled: bool,
    pub suggestions: Vec<String>,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[tauri::command]
pub fn check_spelling(
    word: String,
    languages: Option<Vec<String>>,
    store: State<'_, PersistedStore>,
) -> Result<SpellcheckResult, String> {
    let snapshot = store.snapshot();
    let normalized_word = normalize_word(&word);
    if normalized_word.is_empty() {
        return Ok(empty_result("none", None));
    }

    let learned = snapshot
        .settings
        .spell_check_dictionary
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.trim().to_lowercase())
        .collect::<BTreeSet<_>>();
    if learned.contains(&normalized_word.to_lowercase()) {
        return Ok(empty_result("dictionary", None));
    }

    let resolved_languages = resolve_languages(
        languages,
        snapshot
            .settings
            .spell_check_languages
            .clone()
            .unwrap_or_default(),
    );

    #[cfg(target_os = "windows")]
    {
        if let Some(result) = native_windows_spellcheck(&normalized_word, &resolved_languages) {
            return Ok(result);
        }
    }

    Ok(heuristic_spellcheck(&normalized_word, &resolved_languages))
}

fn empty_result(backend: &str, language: Option<String>) -> SpellcheckResult {
    SpellcheckResult {
        misspelled: false,
        suggestions: Vec::new(),
        backend: backend.to_owned(),
        language,
    }
}

fn resolve_languages(explicit: Option<Vec<String>>, fallback: Vec<String>) -> Vec<String> {
    let candidates = if let Some(explicit) = explicit {
        explicit
    } else if !fallback.is_empty() {
        fallback
    } else {
        default_languages()
    };

    let mut unique = Vec::new();
    for value in candidates {
        let normalized = value.trim();
        if normalized.is_empty() || unique.iter().any(|entry| entry == normalized) {
            continue;
        }
        unique.push(normalized.to_owned());
        if unique.len() >= 5 {
            break;
        }
    }

    if unique.is_empty() {
        vec!["en-US".to_owned()]
    } else {
        unique
    }
}

fn default_languages() -> Vec<String> {
    let mut languages = Vec::new();

    if let Ok(lang) = std::env::var("LANG") {
        let value = lang
            .split('.')
            .next()
            .unwrap_or_default()
            .replace('_', "-")
            .trim()
            .to_owned();
        if !value.is_empty() {
            languages.push(value);
        }
    }

    if languages.is_empty() {
        languages.push("en-US".to_owned());
    }

    languages
}

fn normalize_word(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '\'' && ch != '-' && ch != '_')
        .replace(char::is_whitespace, " ")
}

#[cfg(target_os = "windows")]
fn native_windows_spellcheck(word: &str, languages: &[String]) -> Option<SpellcheckResult> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            Globalization::{ISpellCheckerFactory, SpellCheckerFactory},
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
        },
    };

    struct ComGuard(bool);

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _guard = if initialized.is_ok() {
        ComGuard(true)
    } else if initialized == RPC_E_CHANGED_MODE {
        ComGuard(false)
    } else {
        return None;
    };

    let factory: ISpellCheckerFactory =
        unsafe { CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) }.ok()?;

    for language in languages {
        let wide = widestring::U16CString::from_str(language.as_str()).ok()?;
        let Ok(is_supported) = (unsafe { factory.IsSupported(PCWSTR(wide.as_ptr())) }) else {
            continue;
        };
        if !is_supported.as_bool() {
            continue;
        }

        let Ok(checker) = (unsafe { factory.CreateSpellChecker(PCWSTR(wide.as_ptr())) }) else {
            continue;
        };

        let word_wide = widestring::U16CString::from_str(word).ok()?;
        let Ok(view) = (unsafe { checker.Suggest(PCWSTR(word_wide.as_ptr())) }) else {
            continue;
        };

        let suggestions = collect_enum_strings(&view)
            .into_iter()
            .filter(|entry| !entry.is_empty() && entry != word)
            .collect::<Vec<_>>();

        return Some(SpellcheckResult {
            misspelled: !suggestions.is_empty(),
            suggestions,
            backend: "windows-spellchecker".to_owned(),
            language: Some(language.clone()),
        });
    }

    None
}

#[cfg(target_os = "windows")]
fn collect_enum_strings(view: &windows::Win32::System::Com::IEnumString) -> Vec<String> {
    use windows::{core::PWSTR, Win32::System::Com::CoTaskMemFree};

    let mut values = Vec::new();

    loop {
        let mut fetched = 0u32;
        let mut buffer = [PWSTR::null(); 1];
        let status = unsafe { view.Next(&mut buffer, Some(&mut fetched)) };
        if status.is_err() || fetched == 0 {
            break;
        }

        let value = unsafe { buffer[0].to_string() }.unwrap_or_default();
        unsafe {
            CoTaskMemFree(Some(buffer[0].0 as _));
        }

        if !value.is_empty() {
            values.push(value);
        }
    }

    values
}

fn heuristic_spellcheck(word: &str, languages: &[String]) -> SpellcheckResult {
    let lower = word.to_lowercase();
    let mut suggestions = Vec::new();

    if lower.len() >= 3 {
        let mut chars = lower.chars();
        if let Some(first) = chars.next() {
            let title_case = format!("{}{}", first.to_uppercase(), chars.collect::<String>());
            if title_case != word {
                suggestions.push(title_case);
            }
        }

        let de_doubled = collapse_repeated_chars(&lower);
        if de_doubled != word && !suggestions.contains(&de_doubled) {
            suggestions.push(de_doubled);
        }
    }

    SpellcheckResult {
        misspelled: !suggestions.is_empty(),
        suggestions,
        backend: "heuristic".to_owned(),
        language: languages.first().cloned(),
    }
}

fn collapse_repeated_chars(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous = None;
    let mut repeat_count = 0usize;

    for ch in value.chars() {
        if Some(ch) == previous {
            repeat_count += 1;
            if repeat_count >= 2 {
                continue;
            }
        } else {
            repeat_count = 0;
            previous = Some(ch);
        }

        output.push(ch);
    }

    output
}
