use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::redirect::{Attempt, Policy};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{fs, io::AsyncReadExt};
use url::Url;
use uuid::Uuid;

use crate::{
    db::Database,
    error::{AppError, AppResult},
    model::UpsertProductInput,
    model::{DownloadRequest, DownloadState, DownloadStatusEvent},
    security::{ensure_within_root, is_allowed_browser_url, product_directory},
};

pub const DOWNLOAD_EVENT: &str = "download-status";
const MAX_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_EXPANDED_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Default)]
struct QueueState {
    pending: usize,
    cleanup_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueError {
    CleanupActive,
    Unavailable,
}

#[derive(Clone, Default)]
pub struct DownloadQueue {
    state: Arc<Mutex<QueueState>>,
}

pub struct CleanupGuard {
    state: Arc<Mutex<QueueState>>,
}

pub(crate) struct DownloadPermit {
    state: Arc<Mutex<QueueState>>,
}

impl DownloadQueue {
    pub(crate) fn try_reserve(&self) -> Result<DownloadPermit, EnqueueError> {
        let mut state = self.state.lock().map_err(|_| EnqueueError::Unavailable)?;
        if state.cleanup_active {
            return Err(EnqueueError::CleanupActive);
        }
        if state.pending >= 2 {
            return Err(EnqueueError::Unavailable);
        }
        state.pending += 1;
        Ok(DownloadPermit {
            state: self.state.clone(),
        })
    }

    pub fn begin_cleanup(&self) -> AppResult<CleanupGuard> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::LibraryCleanupInProgress)?;
        if state.cleanup_active {
            return Err(AppError::LibraryCleanupInProgress);
        }
        if state.pending > 0 {
            return Err(AppError::DownloadsInProgress);
        }
        state.cleanup_active = true;
        Ok(CleanupGuard {
            state: self.state.clone(),
        })
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.cleanup_active = false;
        }
    }
}

impl Drop for DownloadPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.pending = state.pending.saturating_sub(1);
        }
    }
}

pub(crate) async fn finish_native_download(
    app: AppHandle,
    database: Database,
    request: DownloadRequest,
    root: PathBuf,
    staging_path: PathBuf,
    _permit: DownloadPermit,
) {
    let result = hash_native_staging_file(&root, &staging_path).await;
    let result = match result {
        Ok((hash, byte_size)) => match build_metadata_client() {
            Ok(metadata_client) => {
                finalize_download(
                    &metadata_client,
                    &database,
                    &request,
                    &root,
                    &staging_path,
                    &hash,
                    byte_size,
                    None,
                )
                .await
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };

    match result {
        Ok(_) => emit(
            &app,
            &request,
            DownloadState::Completed,
            "Download completed",
        ),
        Err(error) => {
            let _ = fs::remove_file(&staging_path).await;
            let _ = fs::remove_dir_all(staging_path.with_extension("extracting")).await;
            emit(&app, &request, DownloadState::Failed, &error.to_string());
        }
    }
}

async fn hash_native_staging_file(root: &Path, staging_path: &Path) -> AppResult<(String, u64)> {
    let metadata = fs::symlink_metadata(staging_path).await?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::PathOutsideLibrary);
    }
    if metadata.len() > MAX_DOWNLOAD_BYTES {
        return Err(AppError::DownloadTooLarge);
    }
    let resolved = fs::canonicalize(staging_path).await?;
    ensure_within_root(root, &resolved)?;

    let mut file = fs::File::open(&resolved).await?;
    let mut hasher = Sha256::new();
    let mut byte_size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        byte_size = byte_size
            .checked_add(read as u64)
            .ok_or(AppError::DownloadTooLarge)?;
        if byte_size > MAX_DOWNLOAD_BYTES {
            return Err(AppError::DownloadTooLarge);
        }
        hasher.update(&buffer[..read]);
    }
    Ok((hex::encode(hasher.finalize()), byte_size))
}

fn build_metadata_client() -> AppResult<reqwest::Client> {
    let redirect_policy = Policy::custom(|attempt: Attempt<'_>| {
        if attempt.previous().len() >= 5 || !is_allowed_browser_url(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .redirect(redirect_policy)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("StashlyForBOOTH/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| AppError::Network("could not initialize the metadata client".into()))
}

#[allow(clippy::too_many_arguments)]
async fn finalize_download(
    metadata_client: &reqwest::Client,
    database: &Database,
    request: &DownloadRequest,
    root: &Path,
    staging_path: &Path,
    hash: &str,
    byte_size: u64,
    response_filename: Option<String>,
) -> AppResult<PathBuf> {
    let mut enriched_request = request.clone();
    if let Some(filename) = response_filename {
        enriched_request.filename = filename;
    }
    if let Some(product) = fetch_product_metadata(metadata_client, request.item_id).await {
        enriched_request.product_name = Some(product.name.clone());
        enriched_request.shop_name = Some(product.shop_name.clone());
        database.upsert_product(&product)?;
    }
    database.ensure_download_metadata(&enriched_request)?;
    let fallback_name = format!("BOOTH item {}", request.item_id);
    let product_name = enriched_request
        .product_name
        .as_deref()
        .unwrap_or(&fallback_name);
    let shop_name = enriched_request.shop_name.as_deref().unwrap_or("BOOTH");
    let product_dir = product_directory(
        root,
        shop_name,
        product_name,
        request.item_id,
        request.variation_id,
    );
    ensure_within_root(root, &product_dir)?;
    fs::create_dir_all(&product_dir).await?;
    let product_dir = fs::canonicalize(product_dir).await?;
    ensure_within_root(root, &product_dir)?;

    if is_zip_filename(&enriched_request.filename) {
        return finalize_zip(
            database,
            &enriched_request,
            root,
            &product_dir,
            staging_path,
            hash,
            byte_size,
        )
        .await;
    }

    let destination = product_dir.join(&enriched_request.filename);
    ensure_within_root(root, &destination)?;
    fs::rename(staging_path, &destination).await?;
    database.record_artifact(
        &Uuid::new_v4().to_string(),
        &enriched_request,
        &destination,
        hash,
        byte_size,
    )?;
    Ok(destination)
}

#[allow(clippy::too_many_arguments)]
async fn finalize_zip(
    database: &Database,
    request: &DownloadRequest,
    root: &Path,
    product_dir: &Path,
    staging_path: &Path,
    hash: &str,
    byte_size: u64,
) -> AppResult<PathBuf> {
    let archive_stem = Path::new(&request.filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| crate::security::sanitize_path_component(value, "archive"))
        .unwrap_or_else(|| "archive".into());
    let extraction_staging = staging_path.with_extension("extracting");
    ensure_within_root(root, &extraction_staging)?;

    let archive = staging_path.to_path_buf();
    let extraction = extraction_staging.clone();
    tauri::async_runtime::spawn_blocking(move || extract_zip_safely(&archive, &extraction))
        .await
        .map_err(|_| AppError::Archive("the extraction worker stopped unexpectedly".into()))??;

    let destination = product_dir.join(&archive_stem);
    ensure_within_root(root, &destination)?;
    let archive = staging_path.to_path_buf();
    let extraction = extraction_staging.clone();
    let published = destination.clone();
    let publish_name = archive_stem.clone();
    tauri::async_runtime::spawn_blocking(move || {
        publish_extracted_directory(&archive, &extraction, &published, &publish_name)
    })
    .await
    .map_err(|_| AppError::Archive("the publish worker stopped unexpectedly".into()))??;

    database.record_artifact(
        &Uuid::new_v4().to_string(),
        request,
        &destination,
        hash,
        byte_size,
    )?;
    Ok(destination)
}

fn publish_extracted_directory(
    archive_path: &Path,
    extraction_staging: &Path,
    destination: &Path,
    archive_stem: &str,
) -> AppResult<()> {
    let publish_source = redundant_single_root(extraction_staging, archive_stem)
        .unwrap_or_else(|| extraction_staging.to_path_buf());
    std::fs::remove_file(archive_path)?;
    if let Err(error) = std::fs::rename(&publish_source, destination) {
        let _ = std::fs::remove_dir_all(extraction_staging);
        return Err(AppError::Io(error));
    }
    if publish_source != extraction_staging {
        let _ = std::fs::remove_dir_all(extraction_staging);
    }
    Ok(())
}

fn redundant_single_root(extraction_root: &Path, archive_stem: &str) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(extraction_root).ok()?;
    let only_entry = entries.next()?.ok()?;
    if entries.next().is_some() || !only_entry.file_type().ok()?.is_dir() {
        return None;
    }
    let directory_name = only_entry.file_name();
    let directory_name = directory_name.to_str()?;
    if directory_name.eq_ignore_ascii_case(archive_stem) {
        Some(only_entry.path())
    } else {
        None
    }
}

fn is_zip_filename(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn extract_zip_safely(archive_path: &Path, output_root: &Path) -> AppResult<usize> {
    if output_root.exists() {
        return Err(AppError::Archive(
            "the temporary extraction directory already exists".into(),
        ));
    }
    std::fs::create_dir(output_root)?;
    let result = extract_zip_entries(archive_path, output_root);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(output_root);
    }
    result
}

fn extract_zip_entries(archive_path: &Path, output_root: &Path) -> AppResult<usize> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| AppError::Archive("the file is not a supported ZIP archive".into()))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(AppError::Archive(format!(
            "the archive contains more than {MAX_ARCHIVE_ENTRIES} entries"
        )));
    }
    if archive
        .decompressed_size()
        .is_some_and(|size| size > u128::from(MAX_EXPANDED_BYTES))
    {
        return Err(AppError::Archive(
            "the expanded archive exceeds the size limit".into(),
        ));
    }

    let mut total_written = 0_u64;
    let mut extracted_files = 0_usize;
    let mut buffer = [0_u8; 64 * 1024];
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| AppError::Archive("an archive entry could not be read".into()))?;
        if entry.encrypted() {
            return Err(AppError::Archive(
                "password-protected ZIP archives are not supported".into(),
            ));
        }
        if entry.is_symlink() {
            return Err(AppError::Archive(
                "symbolic links are not allowed in ZIP archives".into(),
            ));
        }
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            AppError::Archive("an archive entry points outside the destination".into())
        })?;
        let relative = sanitize_archive_path(&enclosed)?;
        let destination = output_root.join(relative);
        ensure_within_root(output_root, &destination)?;

        if entry.is_dir() {
            std::fs::create_dir_all(&destination)?;
            continue;
        }
        if !entry.is_file() {
            return Err(AppError::Archive(
                "the archive contains an unsupported entry type".into(),
            ));
        }
        if entry.size() > MAX_DOWNLOAD_BYTES {
            return Err(AppError::Archive(
                "an extracted file exceeds the size limit".into(),
            ));
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)?;
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            total_written = total_written
                .checked_add(read as u64)
                .ok_or_else(|| AppError::Archive("expanded size overflow".into()))?;
            if total_written > MAX_EXPANDED_BYTES {
                return Err(AppError::Archive(
                    "the expanded archive exceeds the size limit".into(),
                ));
            }
            output.write_all(&buffer[..read])?;
        }
        output.flush()?;
        output.sync_all()?;
        extracted_files += 1;
    }
    if extracted_files == 0 {
        return Err(AppError::Archive(
            "the archive did not contain files".into(),
        ));
    }
    Ok(extracted_files)
}

fn sanitize_archive_path(path: &Path) -> AppResult<PathBuf> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        let std::path::Component::Normal(value) = component else {
            return Err(AppError::Archive(
                "the archive contains an unsafe path".into(),
            ));
        };
        let value = value
            .to_str()
            .ok_or_else(|| AppError::Archive("an archive filename is not valid Unicode".into()))?;
        safe.push(crate::security::sanitize_path_component(value, "unnamed"));
    }
    if safe.as_os_str().is_empty() {
        return Err(AppError::Archive(
            "the archive contains an empty path".into(),
        ));
    }
    Ok(safe)
}

async fn fetch_product_metadata(
    client: &reqwest::Client,
    item_id: i64,
) -> Option<UpsertProductInput> {
    let product_url = format!("https://booth.pm/ja/items/{item_id}");
    let response = client.get(&product_url).send().await.ok()?;
    if !response.status().is_success() || !is_allowed_browser_url(response.url()) {
        return None;
    }
    let body = response.bytes().await.ok()?;
    if body.len() > 4 * 1024 * 1024 {
        return None;
    }
    let text = std::str::from_utf8(&body).ok()?;
    parse_product_metadata(text, item_id, product_url)
}

fn parse_product_metadata(
    text: &str,
    item_id: i64,
    product_url: String,
) -> Option<UpsertProductInput> {
    let document = Html::parse_document(text);
    let title_selector = Selector::parse("meta[property='og:title']").ok()?;
    let image_selector = Selector::parse("meta[property='og:image']").ok()?;
    let raw_title = document
        .select(&title_selector)
        .next()?
        .value()
        .attr("content")?
        .trim();
    let without_booth = raw_title
        .strip_suffix(" - BOOTH")
        .or_else(|| raw_title.strip_suffix(" | BOOTH"))
        .unwrap_or(raw_title);
    let (name, shop_name) = without_booth
        .rsplit_once(" - ")
        .map(|(name, shop)| (name.trim(), shop.trim()))
        .unwrap_or((without_booth, "BOOTH"));
    if name.is_empty() {
        return None;
    }
    let thumbnail_url = document
        .select(&image_selector)
        .next()
        .and_then(|element| element.value().attr("content"))
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str().is_some_and(|host| {
                    host == "booth.pximg.net" || host.ends_with(".booth.pximg.net")
                })
        })
        .map(|url| url.to_string());
    Some(UpsertProductInput {
        item_id,
        name: name.to_owned(),
        shop_name: shop_name.to_owned(),
        shop_subdomain: None,
        product_url,
        thumbnail_url,
    })
}

pub fn emit(app: &AppHandle, request: &DownloadRequest, state: DownloadState, message: &str) {
    emit_event(
        app,
        DownloadStatusEvent {
            request_id: request.request_id.clone(),
            item_id: Some(request.item_id),
            filename: Some(request.filename.clone()),
            state,
            message: message.to_owned(),
        },
    );
}

pub fn emit_event(app: &AppHandle, event: DownloadStatusEvent) {
    app.state::<crate::AppState>()
        .active_download_notifications
        .update(&event);
    let _ = app.emit_to("main", DOWNLOAD_EVENT, event.clone());
    let notification_app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::webview::notify_download_status(&notification_app, &event);
    });
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::{
        DownloadQueue, EnqueueError, extract_zip_safely, hash_native_staging_file, is_zip_filename,
        parse_product_metadata, publish_extracted_directory, redundant_single_root,
        sanitize_archive_path,
    };
    use crate::error::AppError;

    #[test]
    fn reads_public_product_open_graph_metadata() {
        let html = r#"<html><head>
          <meta property="og:title" content="Star Accessory - Moon Shop - BOOTH">
          <meta property="og:image" content="https://booth.pximg.net/example.png">
        </head></html>"#;
        let product =
            parse_product_metadata(html, 123, "https://booth.pm/ja/items/123".into()).unwrap();
        assert_eq!(product.name, "Star Accessory");
        assert_eq!(product.shop_name, "Moon Shop");
        assert_eq!(
            product.thumbnail_url.as_deref(),
            Some("https://booth.pximg.net/example.png")
        );
    }

    #[test]
    fn safely_extracts_nested_zip_contents() {
        let temporary = tempfile::tempdir().unwrap();
        let archive_path = temporary.path().join("package.zip");
        let output_path = temporary.path().join("extracted");
        let archive_file = std::fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(archive_file);
        writer
            .start_file(
                "Avatar/readme.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"hello").unwrap();
        writer.finish().unwrap();

        assert_eq!(extract_zip_safely(&archive_path, &output_path).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(output_path.join("Avatar/readme.txt")).unwrap(),
            "hello"
        );
        assert!(archive_path.exists());
    }

    #[test]
    fn rejects_archive_traversal_paths() {
        assert!(sanitize_archive_path(std::path::Path::new("../outside.txt")).is_err());
        assert!(sanitize_archive_path(std::path::Path::new("safe/file.txt")).is_ok());
    }

    #[test]
    fn detects_zip_extension_case_insensitively() {
        assert!(is_zip_filename("package.ZIP"));
        assert!(!is_zip_filename("package.unitypackage"));
    }

    #[test]
    fn cleanup_and_download_admission_are_mutually_exclusive() {
        let queue = DownloadQueue::default();

        let cleanup = queue.begin_cleanup().unwrap();
        assert!(matches!(
            queue.try_reserve(),
            Err(EnqueueError::CleanupActive)
        ));
        drop(cleanup);

        let permit = queue.try_reserve().unwrap();
        assert!(matches!(
            queue.begin_cleanup(),
            Err(AppError::DownloadsInProgress)
        ));
        drop(permit);
        assert!(queue.begin_cleanup().is_ok());
    }

    #[test]
    fn hashes_a_completed_native_download() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("library");
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let staged = root.join("download.part");
        std::fs::write(&staged, b"hello").unwrap();

        let (hash, size) =
            tauri::async_runtime::block_on(hash_native_staging_file(&root, &staged)).unwrap();

        assert_eq!(size, 5);
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn publishes_extracted_directory_without_the_original_zip() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("package.zip.part");
        let extraction = temporary.path().join("package.extracting");
        let destination = temporary.path().join("package");
        std::fs::write(&archive, b"temporary archive").unwrap();
        std::fs::create_dir(&extraction).unwrap();
        std::fs::write(extraction.join("asset.txt"), b"asset").unwrap();

        publish_extracted_directory(&archive, &extraction, &destination, "package").unwrap();

        assert!(!archive.exists());
        assert!(!extraction.exists());
        assert_eq!(
            std::fs::read_to_string(destination.join("asset.txt")).unwrap(),
            "asset"
        );
    }

    #[test]
    fn flattens_a_redundant_single_root_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("Cybanin3000.zip.part");
        let extraction = temporary.path().join("package.extracting");
        let nested = extraction.join("Cybanin3000");
        let destination = temporary.path().join("Cybanin3000");
        std::fs::write(&archive, b"temporary archive").unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("asset.txt"), b"asset").unwrap();

        assert_eq!(
            redundant_single_root(&extraction, "Cybanin3000"),
            Some(nested)
        );
        publish_extracted_directory(&archive, &extraction, &destination, "Cybanin3000").unwrap();

        assert!(!archive.exists());
        assert!(!extraction.exists());
        assert!(destination.join("asset.txt").is_file());
        assert!(!destination.join("Cybanin3000").exists());
    }
}
