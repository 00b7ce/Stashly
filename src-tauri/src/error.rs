use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Application data directory is unavailable")]
    MissingAppDataDirectory,
    #[error("A library root has not been selected")]
    MissingLibraryRoot,
    #[error("The selected path is not an absolute directory: {0}")]
    InvalidLibraryRoot(PathBuf),
    #[error("The requested product is not in the local library")]
    ProductNotFound,
    #[error("The requested path is outside the configured library root")]
    PathOutsideLibrary,
    #[error("The BOOTH download URL was rejected")]
    RejectedDownloadUrl,
    #[error("The download filename is not safe")]
    UnsafeFilename,
    #[error("The download exceeds the configured size limit")]
    DownloadTooLarge,
    #[error("Downloads are still running. Wait for them to finish before deleting files")]
    DownloadsInProgress,
    #[error("Downloaded files are already being deleted")]
    LibraryCleanupInProgress,
    #[error("ZIP extraction failed: {0}")]
    Archive(String),
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("File operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Network request failed: {0}")]
    Network(String),
    #[error("URL is invalid: {0}")]
    Url(#[from] url::ParseError),
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
