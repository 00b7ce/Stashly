use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, webview::WebviewBuilder};
use url::Url;

use crate::{
    deeplink,
    download::{self, DownloadQueue, EnqueueError},
    error::{AppError, AppResult},
    model::{DownloadState, DownloadStatusEvent},
    security::is_allowed_browser_url,
};

pub const BROWSER_LABEL: &str = "booth-browser";
const MAIN_WINDOW_LABEL: &str = "main";
const LOCAL_ACTION_SCHEME: &str = "booth-shelf";
const OPEN_FOLDER_ACTION_HOST: &str = "open-product-folder";
const BLM_FALLBACK_WINDOW: Duration = Duration::from_secs(10);
const COMPLETED_ACTION_LIFETIME: Duration = Duration::from_secs(30);
const BOOTH_DOWNLOAD_BRIDGE: &str = include_str!("booth_download_bridge.js");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserDownloadStatus<'a> {
    request_id: &'a str,
    state: DownloadState,
}

#[derive(Clone, Default)]
pub(crate) struct CompletedDownloadActions {
    actions: Arc<Mutex<HashMap<String, CompletedDownloadAction>>>,
}

struct CompletedDownloadAction {
    item_id: i64,
    created_at: Instant,
}

impl CompletedDownloadActions {
    fn remember(&self, request_id: &str, item_id: i64, now: Instant) {
        if item_id <= 0 || uuid::Uuid::parse_str(request_id).is_err() {
            return;
        }
        let Ok(mut actions) = self.actions.lock() else {
            return;
        };
        actions
            .retain(|_, action| now.duration_since(action.created_at) <= COMPLETED_ACTION_LIFETIME);
        if actions.len() >= 64 {
            actions.clear();
        }
        actions.insert(
            request_id.to_owned(),
            CompletedDownloadAction {
                item_id,
                created_at: now,
            },
        );
    }

    fn take(&self, request_id: &str, now: Instant) -> Option<i64> {
        let mut actions = self.actions.lock().ok()?;
        let action = actions.remove(request_id)?;
        (now.duration_since(action.created_at) <= COMPLETED_ACTION_LIFETIME)
            .then_some(action.item_id)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserNavigationAction {
    Back,
    Forward,
    Reload,
}

impl BrowserBounds {
    fn validate(self) -> AppResult<Self> {
        let values = [self.x, self.y, self.width, self.height];
        if values.iter().any(|value| !value.is_finite())
            || self.x < 0.0
            || self.y < 0.0
            || self.width < 1.0
            || self.height < 1.0
            || self.width > 100_000.0
            || self.height > 100_000.0
        {
            return Err(AppError::InvalidPayload(
                "the embedded browser bounds are invalid".into(),
            ));
        }
        Ok(self)
    }

    fn position(self) -> tauri::LogicalPosition<f64> {
        tauri::LogicalPosition::new(self.x, self.y)
    }

    fn size(self) -> tauri::LogicalSize<f64> {
        tauri::LogicalSize::new(self.width, self.height)
    }
}

pub async fn show(
    app: AppHandle,
    sender: DownloadQueue,
    initial_url: String,
    bounds: BrowserBounds,
) -> AppResult<()> {
    let target = Url::parse(&initial_url)?;
    if !is_allowed_browser_url(&target) {
        return Err(AppError::RejectedDownloadUrl);
    }
    let bounds = bounds.validate()?;
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        webview
            .set_position(bounds.position())
            .and_then(|_| webview.set_size(bounds.size()))
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
        webview
            .navigate(target)
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
        webview
            .show()
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
        webview
            .set_focus()
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
        return Ok(());
    }

    let profile_dir = profile_directory(&app)?;
    let callback_app = app.clone();
    let callback_sender = sender.clone();
    let last_blm_deeplink = Arc::new(Mutex::new(None));
    let navigation_state = last_blm_deeplink.clone();

    let builder = WebviewBuilder::new(BROWSER_LABEL, WebviewUrl::External(target))
        .data_directory(profile_dir)
        .initialization_script(BOOTH_DOWNLOAD_BRIDGE)
        .on_navigation(move |url| {
            if url.scheme() == LOCAL_ACTION_SCHEME {
                if let Some(request_id) = open_folder_request_id(url) {
                    let action_app = callback_app.clone();
                    tauri::async_runtime::spawn(async move {
                        let actions = action_app.state::<crate::AppState>();
                        if let Some(item_id) = actions
                            .completed_download_actions
                            .take(&request_id, Instant::now())
                        {
                            let _ =
                                crate::commands::open_product_folder_for_app(item_id, &action_app);
                        }
                    });
                }
                false
            } else if deeplink::is_booth_library_manager_link(url) {
                if let Ok(mut last_seen) = navigation_state.lock() {
                    *last_seen = Some(Instant::now());
                }
                match deeplink::parse(url) {
                    Ok(requests) => {
                        for request in requests {
                            match callback_sender.try_send(request.clone()) {
                                Ok(()) => download::emit(
                                    &callback_app,
                                    &request,
                                    DownloadState::Queued,
                                    "Added to the download queue",
                                ),
                                Err(EnqueueError::CleanupActive) => download::emit(
                                    &callback_app,
                                    &request,
                                    DownloadState::Failed,
                                    "Downloaded files are currently being deleted",
                                ),
                                Err(EnqueueError::Unavailable) => download::emit(
                                    &callback_app,
                                    &request,
                                    DownloadState::Failed,
                                    "The download queue is full",
                                ),
                            }
                        }
                    }
                    Err(error) => {
                        download::emit_event(
                            &callback_app,
                            DownloadStatusEvent {
                                request_id: String::new(),
                                item_id: None,
                                filename: None,
                                state: DownloadState::Failed,
                                message: error.to_string(),
                            },
                        );
                    }
                }
                false
            } else if should_suppress_blm_fallback(url, &navigation_state, Instant::now()) {
                false
            } else {
                is_allowed_browser_url(url)
            }
        })
        .on_new_window(|_, _| NewWindowResponse::Deny);
    let window = app
        .get_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| AppError::InvalidPayload("the main window is unavailable".into()))?;
    let webview = window
        .add_child(builder, bounds.position(), bounds.size())
        .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
    webview
        .set_focus()
        .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
    Ok(())
}

pub fn hide(app: &AppHandle) -> AppResult<()> {
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        webview
            .hide()
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
    }
    if let Some(main_webview) = app.get_webview(MAIN_WINDOW_LABEL) {
        main_webview
            .set_focus()
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
    }
    Ok(())
}

pub fn resize(app: &AppHandle, bounds: BrowserBounds) -> AppResult<()> {
    let bounds = bounds.validate()?;
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        webview
            .set_position(bounds.position())
            .and_then(|_| webview.set_size(bounds.size()))
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
    }
    Ok(())
}

pub fn navigate(app: &AppHandle, action: BrowserNavigationAction) -> AppResult<()> {
    let webview = app
        .get_webview(BROWSER_LABEL)
        .ok_or_else(|| AppError::InvalidPayload("the BOOTH browser is unavailable".into()))?;
    let script = match action {
        BrowserNavigationAction::Back => "window.history.back()",
        BrowserNavigationAction::Forward => "window.history.forward()",
        BrowserNavigationAction::Reload => "window.location.reload()",
    };
    webview
        .eval(script)
        .and_then(|_| webview.set_focus())
        .map_err(|error| AppError::InvalidPayload(error.to_string()))
}

pub fn clear_browsing_data(app: &AppHandle) -> AppResult<()> {
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        return webview
            .clear_all_browsing_data()
            .map_err(|error| AppError::InvalidPayload(error.to_string()));
    }

    let profile_dir = profile_directory(app)?;
    remove_profile_directory(&profile_dir)
}

fn remove_profile_directory(profile_dir: &Path) -> AppResult<()> {
    match std::fs::remove_dir_all(profile_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn notify_download_status(app: &AppHandle, event: &DownloadStatusEvent) {
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return;
    };
    let status = BrowserDownloadStatus {
        request_id: &event.request_id,
        state: event.state,
    };
    if matches!(event.state, DownloadState::Completed)
        && let Some(item_id) = event.item_id
    {
        app.state::<crate::AppState>()
            .completed_download_actions
            .remember(&event.request_id, item_id, Instant::now());
    }
    let Ok(payload) = serde_json::to_string(&status) else {
        return;
    };
    let _ = webview.eval(format!("window.__boothShelfNotify?.({payload});"));
}

fn open_folder_request_id(url: &Url) -> Option<String> {
    if url.scheme() != LOCAL_ACTION_SCHEME
        || url.host_str() != Some(OPEN_FOLDER_ACTION_HOST)
        || !matches!(url.path(), "" | "/")
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }

    let mut pairs = url.query_pairs();
    let (key, value) = pairs.next()?;
    if key != "request_id" || pairs.next().is_some() {
        return None;
    }
    uuid::Uuid::parse_str(&value).ok().map(|id| id.to_string())
}

fn profile_directory(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|_| AppError::MissingAppDataDirectory)?
        .join("booth-webview"))
}

fn should_suppress_blm_fallback(
    url: &Url,
    last_blm_deeplink: &Mutex<Option<Instant>>,
    now: Instant,
) -> bool {
    if !is_blm_announcement(url) {
        return false;
    }
    last_blm_deeplink
        .lock()
        .ok()
        .and_then(|last_seen| *last_seen)
        .is_some_and(|last_seen| now.duration_since(last_seen) <= BLM_FALLBACK_WINDOW)
}

fn is_blm_announcement(url: &Url) -> bool {
    if url.scheme() != "https" || url.host_str() != Some("booth.pm") {
        return false;
    }
    matches!(
        url.path().trim_end_matches('/'),
        "/announcements/893" | "/ja/announcements/893"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN_CAPABILITY: &str = include_str!("../capabilities/main.json");

    #[test]
    fn suppresses_blm_announcement_only_just_after_a_deeplink() {
        let captured_at = Instant::now();
        let state = Mutex::new(Some(captured_at));
        let announcement = Url::parse("https://booth.pm/announcements/893").unwrap();

        assert!(should_suppress_blm_fallback(
            &announcement,
            &state,
            captured_at + Duration::from_secs(2)
        ));
        assert!(!should_suppress_blm_fallback(
            &announcement,
            &state,
            captured_at + Duration::from_secs(11)
        ));
    }

    #[test]
    fn allows_other_announcements_and_lookalike_hosts() {
        let captured_at = Instant::now();
        let state = Mutex::new(Some(captured_at));
        let other = Url::parse("https://booth.pm/announcements/970").unwrap();
        let lookalike = Url::parse("https://booth.pm.example.test/announcements/893").unwrap();

        assert!(!should_suppress_blm_fallback(&other, &state, captured_at));
        assert!(!should_suppress_blm_fallback(
            &lookalike,
            &state,
            captured_at
        ));
    }

    #[test]
    fn download_bridge_is_origin_scoped_and_does_not_expose_tauri_ipc() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("https://accounts.booth.pm"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("booth-library-manager://"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("data-booth-shelf-theme"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("__TAURI__"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("invoke("));
    }

    #[test]
    fn download_bridge_dismisses_menus_with_additional_actions() {
        assert!(
            BOOTH_DOWNLOAD_BRIDGE
                .contains("normalize(overlay.textContent).includes(normalize(BLM_LABEL))")
        );
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("labelIs(overlay, BLM_LABEL)"));
    }

    #[test]
    fn download_bridge_hides_only_library_chrome_before_the_tabs() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("LIBRARY_TAB_LABELS"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("hidePrecedingSiblings"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("tabs.closest(\"main\")"));
    }

    #[test]
    fn download_bridge_hides_the_library_footer() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("'footer, [role=\"contentinfo\"]'"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("hideLibraryFooter();"));
    }

    #[test]
    fn download_bridge_exposes_a_one_way_notification_target() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("window.__boothShelfNotify ="));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("status.requestId"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("status.state === \"completed\""));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("booth-shelf-notification-expiry"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("status.message"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("status.filename"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("status.itemId"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("open-product-folder?request_id="));
    }

    #[test]
    fn folder_notification_action_accepts_only_one_uuid_request_id() {
        let request_id = "7cf15df3-9714-4ff5-bf56-ab976127be9d";
        let valid = Url::parse(&format!(
            "booth-shelf://open-product-folder?request_id={request_id}"
        ))
        .unwrap();
        assert_eq!(open_folder_request_id(&valid).as_deref(), Some(request_id));

        for invalid in [
            "booth-shelf://open-product-folder?request_id=not-a-uuid",
            "booth-shelf://open-product-folder?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d&extra=true",
            "booth-shelf://open-product-folder/path?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d",
            "booth-shelf://other?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d",
            "https://accounts.booth.pm/library?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d",
        ] {
            assert_eq!(open_folder_request_id(&Url::parse(invalid).unwrap()), None);
        }
    }

    #[test]
    fn completed_action_is_one_time_and_expires() {
        let actions = CompletedDownloadActions::default();
        let now = Instant::now();
        let request_id = "7cf15df3-9714-4ff5-bf56-ab976127be9d";
        actions.remember(request_id, 3_848_152, now);

        assert_eq!(actions.take(request_id, now), Some(3_848_152));
        assert_eq!(actions.take(request_id, now), None);

        actions.remember(request_id, 3_848_152, now);
        assert_eq!(
            actions.take(
                request_id,
                now + COMPLETED_ACTION_LIFETIME + Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn browser_notification_payload_excludes_local_and_remote_metadata() {
        let event = DownloadStatusEvent {
            request_id: "opaque-request-id".into(),
            item_id: Some(3_848_152),
            filename: Some("private-file.zip".into()),
            state: DownloadState::Completed,
            message: r"Saved to C:\Users\person\Downloads\private-file.zip".into(),
        };
        let payload = serde_json::to_value(BrowserDownloadStatus {
            request_id: &event.request_id,
            state: event.state,
        })
        .unwrap();

        assert_eq!(
            payload,
            serde_json::json!({
                "requestId": "opaque-request-id",
                "state": "completed",
            })
        );
    }

    #[test]
    fn main_capability_is_scoped_to_the_local_webview_only() {
        let capability: serde_json::Value = serde_json::from_str(MAIN_CAPABILITY).unwrap();
        assert_eq!(capability["webviews"], serde_json::json!(["main"]));
        assert!(capability.get("windows").is_none());
        assert!(capability.get("remote").is_none());
        assert!(
            capability["permissions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("allow-navigate-booth-browser"))
        );
        assert!(
            capability["permissions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("allow-clear-booth-browser-data"))
        );
        assert!(
            !capability["permissions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(
                    "core:webview:allow-clear-all-browsing-data"
                ))
        );
        assert!(
            !capability["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|permission| {
                    permission
                        .as_str()
                        .is_some_and(|permission| permission.contains("booth-browser-theme"))
                })
        );
        assert!(
            !capability["permissions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("dialog:allow-ask"))
        );
    }

    #[test]
    fn browser_navigation_accepts_only_known_actions() {
        assert_eq!(
            serde_json::from_str::<BrowserNavigationAction>(r#""back""#).unwrap(),
            BrowserNavigationAction::Back
        );
        assert_eq!(
            serde_json::from_str::<BrowserNavigationAction>(r#""forward""#).unwrap(),
            BrowserNavigationAction::Forward
        );
        assert_eq!(
            serde_json::from_str::<BrowserNavigationAction>(r#""reload""#).unwrap(),
            BrowserNavigationAction::Reload
        );
        assert!(serde_json::from_str::<BrowserNavigationAction>(r#""open-devtools""#).is_err());
    }

    #[test]
    fn browser_bounds_reject_non_finite_and_non_positive_sizes() {
        assert!(
            BrowserBounds {
                x: 236.0,
                y: 0.0,
                width: 1000.0,
                height: 800.0,
            }
            .validate()
            .is_ok()
        );
        assert!(
            BrowserBounds {
                x: 0.0,
                y: 0.0,
                width: f64::NAN,
                height: 800.0,
            }
            .validate()
            .is_err()
        );
        assert!(
            BrowserBounds {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 800.0,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn profile_cleanup_removes_only_the_given_directory_and_accepts_missing_data() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("booth-webview");
        let sibling = temp.path().join("booth-shelf.db");
        std::fs::create_dir_all(profile.join("Network")).unwrap();
        std::fs::write(profile.join("Network").join("Cookies"), b"private").unwrap();
        std::fs::write(&sibling, b"library").unwrap();

        remove_profile_directory(&profile).unwrap();
        assert!(!profile.exists());
        assert!(sibling.exists());
        remove_profile_directory(&profile).unwrap();
    }
}
