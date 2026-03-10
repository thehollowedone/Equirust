use crate::paths::AppPaths;
use std::fs;
use tauri::{
    http::{Response, StatusCode},
    Runtime,
};

use super::{
    assets::resolve_runtime_dir, content_type_for, protocol_error, themes::safe_theme_path,
};

pub fn handle_protocol<R: Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let paths = match AppPaths::resolve(ctx.app_handle()) {
        Ok(paths) => paths,
        Err(err) => return protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    let raw_path = request.uri().path().trim_start_matches('/');
    match raw_path {
        path if path.starts_with("themes/") => {
            let file_name = &path["themes/".len()..];
            match safe_theme_path(&paths.vencord_themes_dir, file_name) {
                Some(theme_path) => match fs::read(theme_path) {
                    Ok(contents) => Response::builder()
                        .status(StatusCode::OK)
                        .header(tauri::http::header::CONTENT_TYPE, "text/css; charset=utf-8")
                        .body(contents)
                        .unwrap_or_else(|err| {
                            protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                        }),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        protocol_error(StatusCode::NOT_FOUND, "theme not found")
                    }
                    Err(err) => protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
                },
                None => protocol_error(StatusCode::BAD_REQUEST, "invalid theme path"),
            }
        }
        "renderer.css.map" | "renderer.js.map" => {
            let runtime = match resolve_runtime_dir(Some(ctx.app_handle())) {
                Ok(path) => path,
                Err(err) => {
                    return protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
                }
            };

            match fs::read(runtime.join(raw_path)) {
                Ok(contents) => Response::builder()
                    .status(StatusCode::OK)
                    .header(
                        tauri::http::header::CONTENT_TYPE,
                        content_type_for(raw_path),
                    )
                    .body(contents)
                    .unwrap_or_else(|err| {
                        protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                    }),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    protocol_error(StatusCode::NOT_FOUND, "asset not found")
                }
                Err(err) => protocol_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        _ => protocol_error(StatusCode::NOT_FOUND, "unsupported vencord asset"),
    }
}
