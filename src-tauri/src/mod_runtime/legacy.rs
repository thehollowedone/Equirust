use crate::{paths::AppPaths, privacy};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

pub fn seed_from_legacy_install(paths: &AppPaths) -> io::Result<()> {
    let legacy_dirs = legacy_equibop_dirs();

    copy_first_missing(
        &paths.settings_file,
        legacy_dirs.iter().map(|dir| dir.join("settings.json")),
    )?;
    copy_first_missing(
        &paths.state_file,
        legacy_dirs.iter().map(|dir| dir.join("state.json")),
    )?;
    copy_first_missing(
        &paths.vencord_settings_file,
        legacy_dirs
            .iter()
            .map(|dir| dir.join("settings").join("settings.json"))
            .chain(
                vencord_legacy_dir()
                    .into_iter()
                    .map(|dir| dir.join("settings").join("settings.json")),
            ),
    )?;
    copy_first_missing(
        &paths.vencord_quickcss_file,
        legacy_dirs
            .iter()
            .map(|dir| dir.join("settings").join("quickCss.css"))
            .chain(
                vencord_legacy_dir()
                    .into_iter()
                    .map(|dir| dir.join("settings").join("quickCss.css")),
            ),
    )?;

    if !has_css_files(&paths.vencord_themes_dir)? {
        for source_dir in legacy_dirs.iter().map(|dir| dir.join("themes")).chain(
            vencord_legacy_dir()
                .into_iter()
                .map(|dir| dir.join("themes")),
        ) {
            copy_theme_dir_if_present(&source_dir, &paths.vencord_themes_dir)?;
            if has_css_files(&paths.vencord_themes_dir)? {
                break;
            }
        }
    }

    Ok(())
}

fn legacy_equibop_dirs() -> Vec<PathBuf> {
    let Some(app_data) = env::var_os("APPDATA").map(PathBuf::from) else {
        return Vec::new();
    };

    vec![app_data.join("Equibop"), app_data.join("equibop")]
}

fn vencord_legacy_dir() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|dir| dir.join("Vencord"))
}

fn copy_first_missing<I>(target: &Path, sources: I) -> io::Result<()>
where
    I: IntoIterator<Item = PathBuf>,
{
    if target.exists() && fs::metadata(target)?.len() > 0 {
        return Ok(());
    }

    for source in sources {
        if !source.exists() {
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(&source, target)?;
        log::info!(
            "Imported {} from {}",
            privacy::file_name_for_log(target),
            privacy::file_name_for_log(&source)
        );
        break;
    }

    Ok(())
}

fn copy_theme_dir_if_present(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    if !source_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("css") {
            continue;
        }

        let Some(file_name) = path.file_name() else {
            continue;
        };

        let target_path = target_dir.join(file_name);
        if target_path.exists() {
            continue;
        }

        fs::copy(&path, &target_path)?;
        log::info!(
            "Imported theme {}",
            privacy::file_name_for_log(&target_path)
        );
    }

    Ok(())
}

fn has_css_files(dir: &Path) -> io::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("css") {
            return Ok(true);
        }
    }

    Ok(false)
}
