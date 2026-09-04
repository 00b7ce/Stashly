use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub library_root: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductSummary {
    pub item_id: i64,
    pub name: String,
    pub shop_name: String,
    pub product_url: String,
    pub thumbnail_url: Option<String>,
    pub local_path: Option<String>,
    pub latest_artifact_path: Option<String>,
    pub artifact_count: i64,
    pub last_downloaded_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub products: Vec<ProductSummary>,
    pub library_root: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatusEvent {
    pub request_id: String,
    pub item_id: Option<i64>,
    pub filename: Option<String>,
    pub state: DownloadState,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Downloading,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub request_id: String,
    pub item_id: i64,
    pub variation_id: i64,
    pub downloadable_id: Option<i64>,
    pub product_name: Option<String>,
    pub shop_name: Option<String>,
    pub filename: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertProductInput {
    pub item_id: i64,
    pub name: String,
    pub shop_name: String,
    pub shop_subdomain: Option<String>,
    pub product_url: String,
    pub thumbnail_url: Option<String>,
}
