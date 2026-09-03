mod cleanup;
mod commands;
mod db;
mod deeplink;
mod download;
mod error;
mod model;
mod security;
mod webview;

use tauri::Manager;

use crate::{db::Database, download::DownloadQueue, error::AppError};

pub struct AppState {
    database: Database,
    download_queue: DownloadQueue,
    completed_download_actions: webview::CompletedDownloadActions,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|_| AppError::MissingAppDataDirectory)?;
            let database = Database::initialize(data_dir.join("booth-shelf.db"))?;
            let (download_queue, download_receiver) = download::channel();
            app.manage(AppState {
                database: database.clone(),
                download_queue: download_queue.clone(),
                completed_download_actions: webview::CompletedDownloadActions::default(),
            });
            tauri::async_runtime::spawn(download::run(
                app.handle().clone(),
                database,
                download_queue,
                download_receiver,
            ));
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
        .expect("error while running Booth Shelf");
}
