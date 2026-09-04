use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::{
    AppState,
    cleanup::DeleteLibraryResult,
    db::Database,
    error::{AppError, AppResult},
    model::{LibrarySnapshot, SetLibraryRootResult},
    security::ensure_within_root,
    storage::{
        inspect_library_root, library_storage_summary, path_for_display, same_root,
        validate_library_root,
    },
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
    library_snapshot(&state.database)
}

fn library_snapshot(database: &Database) -> AppResult<LibrarySnapshot> {
    let settings = database.settings()?;
    Ok(LibrarySnapshot {
        products: database.list_products()?,
        library_root: settings
            .library_root
            .as_deref()
            .map(PathBuf::from)
            .map(|root| path_for_display(&root)),
        library_storage: library_storage_summary(&settings),
    })
}

#[tauri::command]
pub async fn set_library_root(
    root: String,
    allow_non_local: bool,
    state: State<'_, AppState>,
) -> AppResult<SetLibraryRootResult> {
    let root = PathBuf::from(root);
    let change_guard = state.download_queue.begin_cleanup()?;
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _change_guard = change_guard;
        let inspected = inspect_library_root(&root)?;
        let settings = database.settings()?;
        let same_as_current = settings
            .library_root
            .as_deref()
            .is_some_and(|current| same_root(PathBuf::from(current).as_path(), &inspected.path));
        if !same_as_current && database.has_artifacts()? {
            return Err(AppError::LibraryRootContainsArtifacts);
        }
        if inspected.requires_confirmation() && !allow_non_local {
            return Ok(SetLibraryRootResult::ConfirmationRequired {
                candidate: inspected.candidate(),
            });
        }
        let validated = validate_library_root(&inspected.path)?;
        if validated.requires_confirmation() && !allow_non_local {
            return Ok(SetLibraryRootResult::ConfirmationRequired {
                candidate: validated.candidate(),
            });
        }
        let opt_in = validated
            .requires_confirmation()
            .then_some(validated.fingerprint.as_str());
        database.set_library_root_with_opt_in(&validated.path, opt_in)?;
        Ok(SetLibraryRootResult::Saved {
            library: library_snapshot(&database)?,
        })
    })
    .await
    .map_err(|_| {
        AppError::InvalidPayload("the storage validation worker stopped unexpectedly".into())
    })?
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
    use std::path::Path;

    use crate::storage::path_for_display;

    #[test]
    fn removes_the_windows_verbatim_prefix_for_display() {
        assert_eq!(
            path_for_display(Path::new(r"\\?\F:\BOOTH_files")),
            r"F:\BOOTH_files"
        );
        assert_eq!(
            path_for_display(Path::new(r"\\?\UNC\server\share\BOOTH")),
            r"\\server\share\BOOTH"
        );
        assert_eq!(
            path_for_display(Path::new(r"F:\BOOTH_files")),
            r"F:\BOOTH_files"
        );
    }
}
