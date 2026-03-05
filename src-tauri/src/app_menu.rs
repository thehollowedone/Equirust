use crate::{settings::Settings, tray};
use tauri::{
    menu::{AboutMetadata, MenuBuilder, MenuEvent, SubmenuBuilder},
    AppHandle, Manager,
};

const MENU_RESTART: &str = "app-menu-restart";
const MENU_QUIT: &str = "app-menu-quit";

pub fn sync(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };

    if should_attach_window_menu(settings) {
        let menu = build_menu(app)?;
        let _ = window.set_menu(menu).map_err(|err| err.to_string())?;
        window.show_menu().map_err(|err| err.to_string())?;
    } else if window.menu().is_some() {
        let _ = window.remove_menu().map_err(|err| err.to_string())?;
    }

    Ok(())
}

pub fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        MENU_RESTART => {
            tray::mark_quitting(app);
            app.restart();
        }
        MENU_QUIT => {
            tray::mark_quitting(app);
            app.exit(0);
        }
        _ => {}
    }
}

fn should_attach_window_menu(settings: &Settings) -> bool {
    settings.enable_menu == Some(true) && settings.custom_title_bar == Some(false)
}

fn build_menu(app: &AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, String> {
    let about_metadata = AboutMetadata {
        name: Some(app.package_info().name.clone()),
        version: Some(app.package_info().version.to_string()),
        authors: Some(vec![app.package_info().authors.to_string()]),
        comments: Some("Rust-first desktop host for Equibop".to_owned()),
        ..Default::default()
    };

    let app_submenu = SubmenuBuilder::new(app, "&Equirust")
        .about(Some(about_metadata))
        .separator()
        .text(MENU_RESTART, "Restart")
        .text(MENU_QUIT, "Quit")
        .build()
        .map_err(|err| err.to_string())?;

    let edit_submenu = SubmenuBuilder::new(app, "&Edit")
        .copy()
        .cut()
        .paste()
        .select_all()
        .build()
        .map_err(|err| err.to_string())?;

    let window_submenu = SubmenuBuilder::new(app, "&Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()
        .map_err(|err| err.to_string())?;

    MenuBuilder::new(app)
        .items(&[&app_submenu, &edit_submenu, &window_submenu])
        .build()
        .map_err(|err| err.to_string())
}
