use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub library_root: Option<String>,
    pub library_root_opt_in_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageLocationKind {
    Local,
    Network,
    Sync,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStorageSummary {
    pub kind: StorageLocationKind,
    pub non_local_confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRootCandidate {
    pub path: String,
    pub kind: StorageLocationKind,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SetLibraryRootResult {
    Saved { library: LibrarySnapshot },
    ConfirmationRequired { candidate: LibraryRootCandidate },
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
    pub library_storage: Option<LibraryStorageSummary>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_the_library_root_command_contract() {
        let result = SetLibraryRootResult::ConfirmationRequired {
            candidate: LibraryRootCandidate {
                path: r"\\server\share\BOOTH".into(),
                kind: StorageLocationKind::Network,
                reasons: vec!["Network path".into()],
            },
        };
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({
                "status": "confirmation_required",
                "candidate": {
                    "path": r"\\server\share\BOOTH",
                    "kind": "network",
                    "reasons": ["Network path"]
                }
            })
        );
    }
}
