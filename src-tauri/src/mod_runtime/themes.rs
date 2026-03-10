use serde::Serialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VencordTheme {
    pub file_name: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub version: Option<String>,
    pub license: Option<String>,
    pub source: Option<String>,
    pub website: Option<String>,
    pub invite: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VencordThemeEntry {
    pub file_name: String,
    pub content: String,
}

pub(super) fn read_theme_list(dir: &Path) -> io::Result<Vec<VencordTheme>> {
    let mut themes = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("css") {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        let contents = fs::read_to_string(&path)?;
        themes.push(parse_theme_metadata(&file_name, &contents));
    }

    themes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(themes)
}

pub(super) fn read_theme_entries(dir: &Path) -> io::Result<Vec<VencordThemeEntry>> {
    let mut themes = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("css") {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        let content = fs::read_to_string(path)?;
        themes.push(VencordThemeEntry { file_name, content });
    }

    themes.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    Ok(themes)
}

pub(super) fn file_revision(path: &Path) -> io::Result<i64> {
    match fs::metadata(path) {
        Ok(metadata) => modified_millis(&metadata),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(-1),
        Err(err) => Err(err),
    }
}

pub(super) fn compute_theme_revision(dir: &Path) -> io::Result<(i64, usize)> {
    let mut latest_revision = -1_i64;
    let mut count = 0_usize;

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("css") {
            continue;
        }

        count += 1;
        let revision = modified_millis(&entry.metadata()?)?;
        latest_revision = latest_revision.max(revision);
    }

    Ok((latest_revision, count))
}

pub(super) fn safe_theme_path(dir: &Path, file_name: &str) -> Option<PathBuf> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || !file_name.ends_with(".css")
    {
        return None;
    }

    Some(dir.join(file_name))
}

fn modified_millis(metadata: &fs::Metadata) -> io::Result<i64> {
    let modified = metadata.modified()?;
    let duration = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(duration.as_millis() as i64)
}

fn parse_theme_metadata(file_name: &str, contents: &str) -> VencordTheme {
    let mut theme = VencordTheme {
        file_name: file_name.to_string(),
        name: file_name.trim_end_matches(".css").to_string(),
        author: "Unknown Author".into(),
        description: "A Discord theme.".into(),
        version: None,
        license: None,
        source: None,
        website: None,
        invite: None,
    };

    let Some(start) = contents.find("/**") else {
        return theme;
    };
    let Some(end) = contents[start + 3..].find("*/") else {
        return theme;
    };
    let block = &contents[start + 3..start + 3 + end];

    let mut current_key = String::new();
    let mut current_value = String::new();
    let flush = |key: &str, value: &str, theme: &mut VencordTheme| {
        let value = value.trim();
        if value.is_empty() {
            return;
        }

        match key {
            "name" => theme.name = value.to_string(),
            "author" => theme.author = value.to_string(),
            "description" => theme.description = value.to_string(),
            "version" => theme.version = Some(value.to_string()),
            "license" => theme.license = Some(value.to_string()),
            "source" => theme.source = Some(value.to_string()),
            "website" => theme.website = Some(value.to_string()),
            "invite" => theme.invite = Some(value.to_string()),
            _ => {}
        }
    };

    for raw_line in block.lines() {
        let line = raw_line.trim().trim_start_matches('*').trim();
        if let Some(stripped) = line.strip_prefix('@') {
            flush(&current_key, &current_value, &mut theme);
            if let Some((key, value)) = stripped.split_once(' ') {
                current_key = key.to_string();
                current_value = value.trim().to_string();
            } else {
                current_key = stripped.to_string();
                current_value.clear();
            }
        } else if !line.is_empty() {
            if !current_value.is_empty() {
                current_value.push('\n');
            }
            current_value.push_str(line);
        }
    }

    flush(&current_key, &current_value, &mut theme);
    theme
}
