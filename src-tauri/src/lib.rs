mod cleanup;
mod commands;
mod db;
mod download;
mod error;
mod metadata;
mod model;
mod security;
mod storage;
mod webview;

use tauri::Manager;

use crate::{db::Database, download::DownloadQueue, error::AppError};

pub struct AppState {
    database: Database,
    download_queue: DownloadQueue,
    browser_locations: webview::BrowserLocations,
    active_download_notifications: webview::ActiveDownloadNotifications,
    completed_download_actions: webview::CompletedDownloadActions,
    native_downloads: webview::NativeDownloads,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(debug_assertions)]
            let data_dir = std::env::var_os("STASHLY_DEV_DATA_DIR")
                .map(std::path::PathBuf::from)
                .map(Ok)
                .unwrap_or_else(|| {
                    app.path()
                        .app_local_data_dir()
                        .map_err(|_| AppError::MissingAppDataDirectory)
                })?;
            #[cfg(not(debug_assertions))]
            let data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|_| AppError::MissingAppDataDirectory)?;
            let database = Database::initialize(data_dir.join("stashly.db"))?;
            let download_queue = DownloadQueue::default();
            app.manage(AppState {
                database: database.clone(),
                download_queue: download_queue.clone(),
                browser_locations: webview::BrowserLocations::default(),
                active_download_notifications: webview::ActiveDownloadNotifications::default(),
                completed_download_actions: webview::CompletedDownloadActions::default(),
                native_downloads: webview::NativeDownloads::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_library,
            commands::set_library_root,
            commands::open_booth_browser,
            commands::hide_booth_browser,
            commands::resize_booth_browser,
            commands::navigate_booth_browser,
            commands::clear_booth_browser_data,
            commands::open_product_folder,
            commands::delete_downloaded_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Stashly");
}
