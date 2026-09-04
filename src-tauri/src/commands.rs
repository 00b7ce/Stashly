use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::{
    AppState,
    cleanup::DeleteLibraryResult,
    error::{AppError, AppResult},
    model::LibrarySnapshot,
    security::ensure_within_root,
    webview,
};

#[tauri::command]
pub async fn delete_downloaded_files(state: State<'_, AppState>) -> AppResult<DeleteLibraryResult> {
    let root = state
        .database
        .settings()?
        .library_root
        .map(PathBuf::from)
        .ok_or(AppError::MissingLibraryRoot)?;
    let cleanup_guard = state.download_queue.begin_cleanup()?;
    let database = state.database.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let _cleanup_guard = cleanup_guard;
        crate::cleanup::delete_downloaded_files(&database, &root)
    })
    .await
    .map_err(|_| AppError::InvalidPayload("the cleanup worker stopped unexpectedly".into()))?
}

#[tauri::command]
pub fn get_library(state: State<'_, AppState>) -> AppResult<LibrarySnapshot> {
    let settings = state.database.settings()?;
    Ok(LibrarySnapshot {
        products: state.database.list_products()?,
        library_root: settings.library_root.map(|root| path_for_display(&root)),
    })
}

fn path_for_display(path: &str) -> String {
    if let Some(unc_path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc_path}")
    } else if let Some(drive_path) = path.strip_prefix(r"\\?\") {
        drive_path.to_owned()
    } else {
        path.to_owned()
    }
}

#[tauri::command]
pub fn set_library_root(root: String, state: State<'_, AppState>) -> AppResult<LibrarySnapshot> {
    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err(AppError::InvalidLibraryRoot(root));
    }
    std::fs::create_dir_all(&root)?;
    if !root.is_dir() {
        return Err(AppError::InvalidLibraryRoot(root));
    }
    let root = std::fs::canonicalize(root)?;
    state.database.set_library_root(&root)?;
    get_library(state)
}

#[tauri::command]
pub async fn open_booth_browser(
    initial_url: String,
    bounds: webview::BrowserBounds,
    app: AppHandle,
) -> AppResult<()> {
    webview::show(app, initial_url, bounds).await
}

#[tauri::command]
pub fn hide_booth_browser(app: AppHandle) -> AppResult<()> {
    webview::hide(&app)
}

#[tauri::command]
pub fn resize_booth_browser(bounds: webview::BrowserBounds, app: AppHandle) -> AppResult<()> {
    webview::resize(&app, bounds)
}

#[tauri::command]
pub fn navigate_booth_browser(
    action: webview::BrowserNavigationAction,
    app: AppHandle,
) -> AppResult<()> {
    webview::navigate(&app, action)
}

#[tauri::command]
pub fn clear_booth_browser_data(app: AppHandle) -> AppResult<()> {
    webview::clear_browsing_data(&app)
}

#[tauri::command]
pub fn open_product_folder(
    item_id: i64,
    app: AppHandle,
    _state: State<'_, AppState>,
) -> AppResult<()> {
    open_product_folder_for_app(item_id, &app)
}

pub(crate) fn open_product_folder_for_app(item_id: i64, app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let root = state
        .database
        .settings()?
        .library_root
        .map(PathBuf::from)
        .ok_or(AppError::MissingLibraryRoot)?;
    let root = std::fs::canonicalize(root)?;
    let path = state
        .database
        .product_path(item_id)?
        .ok_or(AppError::ProductNotFound)?;
    let path = std::fs::canonicalize(path)?;
    ensure_within_root(&root, &path)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| AppError::InvalidPayload(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::path_for_display;

    #[test]
    fn removes_the_windows_verbatim_prefix_for_display() {
        assert_eq!(path_for_display(r"\\?\F:\BOOTH_files"), r"F:\BOOTH_files");
        assert_eq!(
            path_for_display(r"\\?\UNC\server\share\BOOTH"),
            r"\\server\share\BOOTH"
        );
        assert_eq!(path_for_display(r"F:\BOOTH_files"), r"F:\BOOTH_files");
    }
}
