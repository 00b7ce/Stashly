fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
        "get_library",
        "set_library_root",
        "open_booth_browser",
        "hide_booth_browser",
        "resize_booth_browser",
        "navigate_booth_browser",
        "open_product_folder",
        "delete_downloaded_files",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to run tauri build script");
}
