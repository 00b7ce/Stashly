use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex as AsyncMutex;

use crate::{AppState, download::ExclusiveOperationError};

pub const UPDATE_INSTALL_EVENT: &str = "update-install-progress";
const MAX_RELEASE_NOTES_CHARS: usize = 12_000;

#[derive(Default)]
pub struct UpdateState {
    operation: AsyncMutex<()>,
    pending: Mutex<Option<Update>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    current_version: String,
    version: String,
    notes: Option<String>,
}

impl From<&Update> for AvailableUpdate {
    fn from(update: &Update) -> Self {
        Self {
            current_version: update.current_version.clone(),
            version: update.version.clone(),
            notes: release_notes(update.body.as_deref()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum UpdateInstallEvent {
    Downloading {
        #[serde(rename = "downloadedBytes")]
        downloaded_bytes: u64,
        #[serde(rename = "contentLength")]
        content_length: Option<u64>,
    },
    Downloaded,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum UpdateCommandError {
    #[error("更新情報を確認できませんでした。ネットワーク接続を確認して、もう一度お試しください。")]
    CheckFailed,
    #[error("更新処理を開始できませんでした。少し待ってから、もう一度お試しください。")]
    OperationInProgress,
    #[error(
        "BOOTH商品のダウンロード中は更新できません。ダウンロード完了後に、もう一度お試しください。"
    )]
    DownloadsInProgress,
    #[error("利用できる更新情報がありません。もう一度更新を確認してください。")]
    NoPendingUpdate,
    #[error("更新をダウンロードまたは適用できませんでした。もう一度お試しください。")]
    InstallFailed,
}

impl Serialize for UpdateCommandError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<AvailableUpdate>, UpdateCommandError> {
    let _operation = state.updates.operation.lock().await;
    let updater = app.updater().map_err(|_| UpdateCommandError::CheckFailed)?;
    let update = updater
        .check()
        .await
        .map_err(|_| UpdateCommandError::CheckFailed)?;
    let available = update.as_ref().map(AvailableUpdate::from);
    replace_pending(&state.updates, update)?;
    Ok(available)
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), UpdateCommandError> {
    let _operation = state
        .updates
        .operation
        .try_lock()
        .map_err(|_| UpdateCommandError::OperationInProgress)?;
    let _download_exclusion = state
        .download_queue
        .begin_exclusive_operation()
        .map_err(map_exclusive_operation_error)?;
    let update = pending_update(&state.updates)?.ok_or(UpdateCommandError::NoPendingUpdate)?;

    let downloaded = Arc::new(AtomicU64::new(0));
    let progress_app = app.clone();
    let progress_downloaded = downloaded.clone();
    let finished_app = app.clone();
    update
        .download_and_install(
            move |chunk_length, content_length| {
                let downloaded_bytes = progress_downloaded
                    .fetch_add(chunk_length as u64, Ordering::Relaxed)
                    .saturating_add(chunk_length as u64);
                let _ = progress_app.emit_to(
                    "main",
                    UPDATE_INSTALL_EVENT,
                    UpdateInstallEvent::Downloading {
                        downloaded_bytes,
                        content_length,
                    },
                );
            },
            move || {
                let _ = finished_app.emit_to(
                    "main",
                    UPDATE_INSTALL_EVENT,
                    UpdateInstallEvent::Downloaded,
                );
            },
        )
        .await
        .map_err(|_| UpdateCommandError::InstallFailed)?;

    replace_pending(&state.updates, None)?;
    Ok(())
}

fn pending_update(state: &UpdateState) -> Result<Option<Update>, UpdateCommandError> {
    state
        .pending
        .lock()
        .map(|update| update.clone())
        .map_err(|_| UpdateCommandError::OperationInProgress)
}

fn replace_pending(state: &UpdateState, update: Option<Update>) -> Result<(), UpdateCommandError> {
    *state
        .pending
        .lock()
        .map_err(|_| UpdateCommandError::OperationInProgress)? = update;
    Ok(())
}

fn map_exclusive_operation_error(error: ExclusiveOperationError) -> UpdateCommandError {
    match error {
        ExclusiveOperationError::DownloadsInProgress => UpdateCommandError::DownloadsInProgress,
        ExclusiveOperationError::OperationInProgress => UpdateCommandError::OperationInProgress,
    }
}

fn release_notes(body: Option<&str>) -> Option<String> {
    let body = body?.trim();
    if body.is_empty() {
        return None;
    }

    let mut characters = body.chars();
    let mut notes: String = characters.by_ref().take(MAX_RELEASE_NOTES_CHARS).collect();
    if characters.next().is_some() {
        notes.push_str("\n…");
    }
    Some(notes)
}

#[cfg(test)]
mod tests {
    use super::{MAX_RELEASE_NOTES_CHARS, release_notes};

    const TAURI_CONFIG: &str = include_str!("../tauri.conf.json");
    const RELEASE_WORKFLOW: &str = include_str!("../../.github/workflows/release.yml");

    #[test]
    fn release_notes_are_trimmed_and_bounded_as_plain_text() {
        assert_eq!(release_notes(Some("  Changes  ")), Some("Changes".into()));
        assert_eq!(release_notes(Some(" \n ")), None);

        let oversized = "あ".repeat(MAX_RELEASE_NOTES_CHARS + 1);
        let notes = release_notes(Some(&oversized)).unwrap();
        assert_eq!(notes.chars().count(), MAX_RELEASE_NOTES_CHARS + 2);
        assert!(notes.ends_with("\n…"));
    }

    #[test]
    fn updater_configuration_uses_the_fixed_signed_nsis_channel() {
        let config: serde_json::Value = serde_json::from_str(TAURI_CONFIG).unwrap();
        let updater = &config["plugins"]["updater"];

        assert_eq!(config["bundle"]["createUpdaterArtifacts"], true);
        assert_eq!(updater["windows"]["installMode"], "passive");
        assert_eq!(
            updater["endpoints"],
            serde_json::json!([
                "https://github.com/00b7ce/Stashly/releases/latest/download/latest.json"
            ])
        );
        assert!(
            updater["pubkey"]
                .as_str()
                .is_some_and(|key| !key.is_empty())
        );
        assert!(updater.get("dangerousInsecureTransportProtocol").is_none());
        assert!(updater.get("dangerousAcceptInvalidCerts").is_none());
    }

    #[test]
    fn release_workflow_requires_signing_and_prefers_nsis_updates() {
        assert!(
            RELEASE_WORKFLOW
                .contains("TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}")
        );
        assert!(RELEASE_WORKFLOW.contains("updaterJsonPreferNsis: true"));
        assert!(RELEASE_WORKFLOW.contains("releaseDraft: true"));
        assert!(RELEASE_WORKFLOW.contains("prerelease: false"));
    }
}
