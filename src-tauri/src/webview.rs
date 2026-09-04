use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview, WebviewUrl};
use url::Url;

use crate::{
    download::{self, DownloadPermit, EnqueueError},
    error::{AppError, AppResult},
    model::{DownloadRequest, DownloadState, DownloadStatusEvent},
    security::{ensure_within_root, is_allowed_browser_url, safe_filename},
};

pub const BROWSER_LABEL: &str = "booth-browser";
const MAIN_WINDOW_LABEL: &str = "main";
const BROWSER_LOCATION_EVENT: &str = "booth-browser-location";
const LOCAL_ACTION_SCHEME: &str = "booth-shelf";
const OPEN_FOLDER_ACTION_HOST: &str = "open-product-folder";
const DOWNLOAD_INTENT_ACTION_HOST: &str = "download-intent";
const DOWNLOAD_INTENT_LIFETIME: Duration = Duration::from_secs(10);
const COMPLETED_ACTION_LIFETIME: Duration = Duration::from_secs(30);
const COMPLETED_NOTIFICATION_REPLAY_LIFETIME: Duration = Duration::from_secs(6);
const BOOTH_DOWNLOAD_BRIDGE: &str = include_str!("booth_download_bridge.js");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserDownloadStatus<'a> {
    request_id: &'a str,
    state: DownloadState,
}

#[derive(Clone, Default)]
pub(crate) struct BrowserLocations {
    locations: Arc<Mutex<BrowserLocationState>>,
}

#[derive(Default)]
struct BrowserLocationState {
    booth: Option<Url>,
    library: Option<Url>,
}

impl BrowserLocations {
    fn remember(&self, url: &Url) {
        let Ok(mut locations) = self.locations.lock() else {
            return;
        };
        if is_booth_library_page(url) {
            locations.library = Some(url.clone());
        } else if is_booth_page(url) {
            locations.booth = Some(url.clone());
        }
    }

    fn destination(&self, current: Option<&Url>, requested: &Url) -> Url {
        if is_booth_library_entry(requested) {
            if current.is_some_and(is_booth_library_page) {
                return current.cloned().unwrap_or_else(|| requested.clone());
            }
            return self
                .locations
                .lock()
                .ok()
                .and_then(|locations| locations.library.clone())
                .unwrap_or_else(|| requested.clone());
        }

        if is_booth_entry(requested) {
            if current.is_some_and(is_booth_page) {
                return current.cloned().unwrap_or_else(|| requested.clone());
            }
            return self
                .locations
                .lock()
                .ok()
                .and_then(|locations| locations.booth.clone())
                .unwrap_or_else(|| requested.clone());
        }

        requested.clone()
    }
}

#[derive(Clone, Default)]
pub(crate) struct ActiveDownloadNotifications {
    statuses: Arc<Mutex<HashMap<String, StoredDownloadNotification>>>,
}

struct StoredDownloadNotification {
    state: DownloadState,
    replay_until: Option<Instant>,
}

impl ActiveDownloadNotifications {
    pub(crate) fn update(&self, event: &DownloadStatusEvent) {
        self.update_at(event, Instant::now());
    }

    fn update_at(&self, event: &DownloadStatusEvent, now: Instant) {
        if uuid::Uuid::parse_str(&event.request_id).is_err() {
            return;
        }
        let Ok(mut statuses) = self.statuses.lock() else {
            return;
        };
        match event.state {
            DownloadState::Downloading => {
                statuses.insert(
                    event.request_id.clone(),
                    StoredDownloadNotification {
                        state: event.state,
                        replay_until: None,
                    },
                );
            }
            DownloadState::Completed => {
                statuses.insert(
                    event.request_id.clone(),
                    StoredDownloadNotification {
                        state: event.state,
                        replay_until: Some(now + COMPLETED_NOTIFICATION_REPLAY_LIFETIME),
                    },
                );
            }
            DownloadState::Failed => {
                statuses.remove(&event.request_id);
            }
        }
    }

    fn snapshot(&self) -> Vec<(String, DownloadState)> {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&self, now: Instant) -> Vec<(String, DownloadState)> {
        let Ok(mut statuses) = self.statuses.lock() else {
            return Vec::new();
        };
        statuses.retain(|_, notification| {
            notification
                .replay_until
                .is_none_or(|replay_until| replay_until > now)
        });
        let mut snapshot = statuses
            .iter()
            .map(|(request_id, notification)| (request_id.clone(), notification.state))
            .collect::<Vec<_>>();
        snapshot.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        snapshot
    }
}

#[derive(Clone, Default)]
pub(crate) struct CompletedDownloadActions {
    actions: Arc<Mutex<HashMap<String, CompletedDownloadAction>>>,
}

struct CompletedDownloadAction {
    item_id: i64,
    created_at: Instant,
}

#[derive(Clone, Default)]
pub(crate) struct NativeDownloads {
    state: Arc<Mutex<NativeDownloadState>>,
}

#[derive(Default)]
struct NativeDownloadState {
    intent: Option<NativeDownloadIntent>,
    pending: HashMap<PathBuf, PendingNativeDownload>,
}

#[derive(Clone)]
struct NativeDownloadIntent {
    request_id: String,
    item_id: i64,
    variation_id: i64,
    downloadable_id: i64,
    created_at: Instant,
}

struct PendingNativeDownload {
    request: DownloadRequest,
    root: PathBuf,
    staging_path: PathBuf,
    source_url: Url,
    permit: DownloadPermit,
}

impl NativeDownloads {
    fn arm(&self, intent: NativeDownloadIntent, now: Instant) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.intent.as_ref().is_some_and(|current| {
            now.duration_since(current.created_at) <= DOWNLOAD_INTENT_LIFETIME
        }) {
            return false;
        }
        state.intent = Some(intent);
        true
    }

    fn take_intent(&self, now: Instant) -> Option<NativeDownloadIntent> {
        let mut state = self.state.lock().ok()?;
        let intent = state.intent.take()?;
        (now.duration_since(intent.created_at) <= DOWNLOAD_INTENT_LIFETIME).then_some(intent)
    }

    fn insert_pending(&self, path: PathBuf, pending: PendingNativeDownload) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.pending.contains_key(&path) {
            return false;
        }
        state.pending.insert(path, pending);
        true
    }

    fn take_pending(&self, path: Option<&Path>, url: &Url) -> Option<PendingNativeDownload> {
        let mut state = self.state.lock().ok()?;
        if let Some(path) = path
            && let Some(pending) = state.pending.remove(path)
        {
            return Some(pending);
        }
        let matching = state
            .pending
            .iter()
            .filter(|(_, pending)| pending.source_url == *url)
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return None;
        }
        state.pending.remove(&matching[0])
    }
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

pub async fn show(app: AppHandle, initial_url: String, bounds: BrowserBounds) -> AppResult<()> {
    let target = Url::parse(&initial_url)?;
    if !is_allowed_browser_url(&target) {
        return Err(AppError::RejectedDownloadUrl);
    }
    let bounds = bounds.validate()?;
    let browser_locations = app.state::<crate::AppState>().browser_locations.clone();
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        webview
            .set_position(bounds.position())
            .and_then(|_| webview.set_size(bounds.size()))
            .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
        let current = webview.url().ok();
        let destination = browser_locations.destination(current.as_ref(), &target);
        emit_browser_location(&app, &destination);
        if current.as_ref() != Some(&destination) {
            webview
                .navigate(destination)
                .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
        }
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
    let active_notifications = app
        .state::<crate::AppState>()
        .active_download_notifications
        .clone();
    let native_downloads = app.state::<crate::AppState>().native_downloads.clone();
    let download_callback_app = app.clone();
    let download_callback_state = native_downloads.clone();
    let page_load_locations = browser_locations.clone();
    let page_load_app = app.clone();
    let new_window_app = app.clone();
    let initial_location = target.clone();

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
                } else if url.host_str() == Some(DOWNLOAD_INTENT_ACTION_HOST) {
                    let request_id = download_intent_request_id(url);
                    let accepted = download_intent_from_url(url)
                        .is_some_and(|intent| native_downloads.arm(intent, Instant::now()));
                    if let Some(request_id) = request_id {
                        let acknowledgement_app = callback_app.clone();
                        tauri::async_runtime::spawn(async move {
                            acknowledge_download_intent(
                                &acknowledgement_app,
                                &request_id,
                                accepted,
                            );
                        });
                    }
                }
                false
            } else {
                let allowed = is_allowed_browser_url(url);
                if allowed {
                    emit_browser_location(&callback_app, url);
                }
                allowed
            }
        })
        .on_download(move |_, event| {
            handle_native_download(&download_callback_app, &download_callback_state, event)
        })
        .on_page_load(move |webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                page_load_locations.remember(payload.url());
                emit_browser_location(&page_load_app, payload.url());
                replay_download_notifications(&webview, &active_notifications);
            }
        })
        .on_new_window(move |url, _| {
            if let Some(target) = new_window_navigation_target(url) {
                let navigation_app = new_window_app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(webview) = navigation_app.get_webview(BROWSER_LABEL) {
                        let _ = webview.navigate(target);
                    }
                });
            }
            NewWindowResponse::Deny
        });
    let window = app
        .get_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| AppError::InvalidPayload("the main window is unavailable".into()))?;
    let webview = window
        .add_child(builder, bounds.position(), bounds.size())
        .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
    webview
        .set_focus()
        .map_err(|error| AppError::InvalidPayload(error.to_string()))?;
    emit_browser_location(&app, &initial_location);
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
    eval_download_notification(&webview, status);
}

fn replay_download_notifications<R: Runtime>(
    webview: &Webview<R>,
    notifications: &ActiveDownloadNotifications,
) {
    for (request_id, state) in notifications.snapshot() {
        eval_download_notification(
            webview,
            BrowserDownloadStatus {
                request_id: &request_id,
                state,
            },
        );
    }
}

fn eval_download_notification<R: Runtime>(webview: &Webview<R>, status: BrowserDownloadStatus<'_>) {
    let Ok(payload) = serde_json::to_string(&status) else {
        return;
    };
    let _ = webview.eval(format!("window.__boothShelfNotify?.({payload});"));
}

fn download_intent_request_id(url: &Url) -> Option<String> {
    url.query_pairs()
        .find_map(|(key, value)| (key == "request_id").then(|| value.into_owned()))
        .and_then(|value| uuid::Uuid::parse_str(&value).ok())
        .map(|value| value.to_string())
}

fn download_intent_from_url(url: &Url) -> Option<NativeDownloadIntent> {
    if url.as_str().len() > 512
        || url.scheme() != LOCAL_ACTION_SCHEME
        || url.host_str() != Some(DOWNLOAD_INTENT_ACTION_HOST)
        || !matches!(url.path(), "" | "/")
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }

    let pairs = url.query_pairs().collect::<Vec<_>>();
    if pairs.len() != 4 {
        return None;
    }
    let value = |expected: &str| {
        let matches = pairs
            .iter()
            .filter(|(key, _)| key == expected)
            .map(|(_, value)| value.as_ref())
            .collect::<Vec<_>>();
        (matches.len() == 1).then_some(matches[0])
    };
    let parse_id = |name| value(name)?.parse::<i64>().ok().filter(|id| *id > 0);
    let request_id = uuid::Uuid::parse_str(value("request_id")?)
        .ok()?
        .to_string();
    Some(NativeDownloadIntent {
        request_id,
        item_id: parse_id("item_id")?,
        variation_id: parse_id("variation_id")?,
        downloadable_id: parse_id("downloadable_id")?,
        created_at: Instant::now(),
    })
}

fn acknowledge_download_intent(app: &AppHandle, request_id: &str, accepted: bool) {
    let Some(webview) = app.get_webview(BROWSER_LABEL) else {
        return;
    };
    let Ok(request_id) = serde_json::to_string(request_id) else {
        return;
    };
    let callback = if accepted {
        "__boothShelfAcceptDownloadIntent"
    } else {
        "__boothShelfRejectDownloadIntent"
    };
    let _ = webview.eval(format!("window.{callback}?.({request_id});"));
}

fn handle_native_download(
    app: &AppHandle,
    native_downloads: &NativeDownloads,
    event: DownloadEvent<'_>,
) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            let Some(intent) = native_downloads.take_intent(Instant::now()) else {
                return false;
            };
            let request = match prepare_native_download(app, &intent, &url, destination) {
                Ok(Some(pending)) => pending,
                Ok(None) => return false,
                Err(error) => {
                    emit_intent_failure(app, &intent, error.to_string());
                    return false;
                }
            };
            let path = destination.clone();
            if !native_downloads.insert_pending(path, request) {
                emit_intent_failure(app, &intent, "the download could not be reserved".into());
                return false;
            }
            if let Ok(state) = native_downloads.state.lock()
                && let Some(pending) = state.pending.get(destination.as_path())
            {
                download::emit(
                    app,
                    &pending.request,
                    DownloadState::Downloading,
                    "Downloading",
                );
            }
            true
        }
        DownloadEvent::Finished { url, path, success } => {
            let Some(pending) = native_downloads.take_pending(path.as_deref(), &url) else {
                return false;
            };
            if !success {
                let _ = std::fs::remove_file(&pending.staging_path);
                download::emit(
                    app,
                    &pending.request,
                    DownloadState::Failed,
                    "The BOOTH download did not complete",
                );
                return false;
            }
            let task_app = app.clone();
            tauri::async_runtime::spawn(download::finish_native_download(
                task_app,
                app.state::<crate::AppState>().database.clone(),
                pending.request,
                pending.root,
                pending.staging_path,
                pending.permit,
            ));
            true
        }
        _ => false,
    }
}

fn prepare_native_download(
    app: &AppHandle,
    intent: &NativeDownloadIntent,
    url: &Url,
    destination: &mut PathBuf,
) -> AppResult<Option<PendingNativeDownload>> {
    if !native_download_url_matches(url, intent) {
        return Err(AppError::RejectedDownloadUrl);
    }
    let filename = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(AppError::UnsafeFilename)
        .and_then(safe_filename)?;
    let request = DownloadRequest {
        request_id: intent.request_id.clone(),
        item_id: intent.item_id,
        variation_id: intent.variation_id,
        downloadable_id: Some(intent.downloadable_id),
        product_name: None,
        shop_name: None,
        filename,
    };
    let state = app.state::<crate::AppState>();
    let permit = match state.download_queue.try_reserve() {
        Ok(permit) => permit,
        Err(EnqueueError::CleanupActive) => {
            return Err(AppError::LibraryCleanupInProgress);
        }
        Err(EnqueueError::Unavailable) => {
            return Err(AppError::DownloadsInProgress);
        }
    };
    let root = state.database.settings()?;
    let root = crate::storage::validate_configured_library_root(&root)?;

    for existing in state.database.artifact_paths_for_download(&request)? {
        if let Ok(existing) = std::fs::canonicalize(existing)
            && ensure_within_root(&root, &existing).is_ok()
        {
            download::emit(
                app,
                &request,
                DownloadState::Completed,
                "Already downloaded",
            );
            return Ok(None);
        }
    }

    let staging_dir = root.join(".booth-shelf-staging");
    ensure_within_root(&root, &staging_dir)?;
    std::fs::create_dir_all(&staging_dir)?;
    let staging_dir = std::fs::canonicalize(staging_dir)?;
    ensure_within_root(&root, &staging_dir)?;
    let staging_path = staging_dir.join(format!("{}.part", intent.request_id));
    ensure_within_root(&root, &staging_path)?;
    if staging_path.exists() {
        return Err(AppError::InvalidPayload(
            "the download staging path is already in use".into(),
        ));
    }
    *destination = staging_path;
    Ok(Some(PendingNativeDownload {
        request,
        root,
        staging_path: destination.clone(),
        source_url: url.clone(),
        permit,
    }))
}

fn native_download_url_matches(url: &Url, intent: &NativeDownloadIntent) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    if url.host_str() == Some("s6.booth.pm") {
        return true;
    }
    if url.host_str() != Some("booth.pm") {
        return false;
    }
    let expected_path = format!("/downloadables/{}", intent.downloadable_id);
    if url.path().trim_end_matches('/') != expected_path {
        return false;
    }
    let variations = url
        .query_pairs()
        .filter(|(key, _)| key == "variation_id")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    match variations.as_slice() {
        [] => intent.variation_id == intent.downloadable_id,
        [variation_id] => variation_id == &intent.variation_id.to_string(),
        _ => false,
    }
}

fn emit_intent_failure(app: &AppHandle, intent: &NativeDownloadIntent, message: String) {
    download::emit_event(
        app,
        DownloadStatusEvent {
            request_id: intent.request_id.clone(),
            item_id: Some(intent.item_id),
            filename: None,
            state: DownloadState::Failed,
            message,
        },
    );
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

fn browser_url_for_display(url: &Url) -> Option<String> {
    if !is_allowed_browser_url(url) {
        return None;
    }

    let library_page = is_booth_library_page(url)
        .then(|| {
            url.query_pairs().find_map(|(key, value)| {
                (key == "page"
                    && !value.is_empty()
                    && value.len() <= 6
                    && value.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| value.into_owned())
            })
        })
        .flatten();

    let mut display = url.clone();
    display.set_fragment(None);
    display.set_query(None);
    display.set_username("").ok()?;
    display.set_password(None).ok()?;
    if let Some(page) = library_page {
        display.query_pairs_mut().append_pair("page", &page);
    }
    Some(display.into())
}

fn emit_browser_location(app: &AppHandle, url: &Url) {
    let Some(display_url) = browser_url_for_display(url) else {
        return;
    };
    let _ = app.emit_to(MAIN_WINDOW_LABEL, BROWSER_LOCATION_EVENT, display_url);
}

fn new_window_navigation_target(url: Url) -> Option<Url> {
    is_allowed_browser_url(&url).then_some(url)
}

fn is_booth_entry(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("booth.pm")
        && url.path().trim_end_matches('/') == "/ja"
        && url.query().is_none()
        && url.fragment().is_none()
}

fn is_booth_library_entry(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("accounts.booth.pm")
        && url.path().trim_end_matches('/') == "/library"
        && url.query().is_none()
        && url.fragment().is_none()
}

fn is_booth_page(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("booth.pm")
}

fn is_booth_library_page(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("accounts.booth.pm")
        && (url.path().trim_end_matches('/') == "/library" || url.path().starts_with("/library/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN_CAPABILITY: &str = include_str!("../capabilities/main.json");

    #[test]
    fn download_intent_accepts_only_exact_positive_fields() {
        let request_id = "7cf15df3-9714-4ff5-bf56-ab976127be9d";
        let valid = Url::parse(&format!(
            "booth-shelf://download-intent?request_id={request_id}&item_id=123&variation_id=456&downloadable_id=789"
        ))
        .unwrap();
        let intent = download_intent_from_url(&valid).unwrap();
        assert_eq!(intent.request_id, request_id);
        assert_eq!(intent.item_id, 123);
        assert_eq!(intent.variation_id, 456);
        assert_eq!(intent.downloadable_id, 789);

        for invalid in [
            "booth-shelf://download-intent?request_id=bad&item_id=1&variation_id=2&downloadable_id=3",
            "booth-shelf://download-intent?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d&item_id=0&variation_id=2&downloadable_id=3",
            "booth-shelf://download-intent?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d&item_id=1&variation_id=2&downloadable_id=3&extra=4",
            "booth-shelf://download-intent/path?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d&item_id=1&variation_id=2&downloadable_id=3",
            "https://accounts.booth.pm/library?request_id=7cf15df3-9714-4ff5-bf56-ab976127be9d&item_id=1&variation_id=2&downloadable_id=3",
        ] {
            assert!(download_intent_from_url(&Url::parse(invalid).unwrap()).is_none());
        }
    }

    #[test]
    fn download_intent_is_one_shot_and_expires() {
        let downloads = NativeDownloads::default();
        let now = Instant::now();
        let intent = NativeDownloadIntent {
            request_id: "7cf15df3-9714-4ff5-bf56-ab976127be9d".into(),
            item_id: 123,
            variation_id: 456,
            downloadable_id: 789,
            created_at: now,
        };

        assert!(downloads.arm(intent.clone(), now));
        assert_eq!(downloads.take_intent(now).unwrap().item_id, 123);
        assert!(downloads.take_intent(now).is_none());

        assert!(downloads.arm(intent, now));
        assert!(
            downloads
                .take_intent(now + DOWNLOAD_INTENT_LIFETIME + Duration::from_secs(1))
                .is_none()
        );
    }

    #[test]
    fn native_download_url_is_bound_to_the_armed_download() {
        let intent = NativeDownloadIntent {
            request_id: "7cf15df3-9714-4ff5-bf56-ab976127be9d".into(),
            item_id: 123,
            variation_id: 456,
            downloadable_id: 789,
            created_at: Instant::now(),
        };

        assert!(native_download_url_matches(
            &Url::parse("https://booth.pm/downloadables/789?variation_id=456").unwrap(),
            &intent
        ));
        assert!(native_download_url_matches(
            &Url::parse("https://s6.booth.pm/signed-response").unwrap(),
            &intent
        ));

        let queryless_intent = NativeDownloadIntent {
            variation_id: 789,
            ..intent.clone()
        };
        assert!(native_download_url_matches(
            &Url::parse("https://booth.pm/downloadables/789").unwrap(),
            &queryless_intent
        ));
        for rejected in [
            "https://booth.pm/downloadables/789",
            "https://booth.pm/downloadables/790?variation_id=456",
            "https://booth.pm/downloadables/789?variation_id=457",
            "https://s6.booth.pm.example.test/signed-response",
            "http://s6.booth.pm/signed-response",
        ] {
            assert!(!native_download_url_matches(
                &Url::parse(rejected).unwrap(),
                &intent
            ));
        }
    }

    #[test]
    fn browser_locations_restore_booth_and_library_destinations_independently() {
        let locations = BrowserLocations::default();
        let library = Url::parse("https://accounts.booth.pm/library").unwrap();
        let page_two =
            Url::parse("https://accounts.booth.pm/library/free_downloads?page=2").unwrap();
        let product = Url::parse("https://booth.pm/ja/items/123").unwrap();
        let booth_top = Url::parse("https://booth.pm/ja").unwrap();

        locations.remember(&page_two);
        locations.remember(&product);

        assert_eq!(locations.destination(Some(&product), &library), page_two);
        assert_eq!(locations.destination(Some(&page_two), &booth_top), product);
    }

    #[test]
    fn browser_locations_keep_the_current_destination_and_honor_explicit_urls() {
        let locations = BrowserLocations::default();
        let library = Url::parse("https://accounts.booth.pm/library").unwrap();
        let page_two = Url::parse("https://accounts.booth.pm/library?page=2").unwrap();
        let product = Url::parse("https://booth.pm/ja/items/123").unwrap();
        let other_product = Url::parse("https://booth.pm/ja/items/456").unwrap();
        let booth_top = Url::parse("https://booth.pm/ja").unwrap();

        locations.remember(&page_two);
        locations.remember(&product);

        assert_eq!(locations.destination(Some(&page_two), &library), page_two);
        assert_eq!(locations.destination(Some(&product), &booth_top), product);
        assert_eq!(
            locations.destination(Some(&page_two), &other_product),
            other_product
        );
    }

    #[test]
    fn browser_locations_ignore_authentication_and_unrelated_pages() {
        let locations = BrowserLocations::default();
        let library = Url::parse("https://accounts.booth.pm/library").unwrap();
        let booth_top = Url::parse("https://booth.pm/ja").unwrap();
        locations.remember(&Url::parse("https://accounts.pixiv.net/login").unwrap());
        locations.remember(&Url::parse("https://accounts.booth.pm/settings").unwrap());

        assert_eq!(locations.destination(None, &library), library);
        assert_eq!(locations.destination(None, &booth_top), booth_top);
    }

    #[test]
    fn browser_location_display_omits_sensitive_query_and_fragment() {
        let login =
            Url::parse("https://accounts.pixiv.net/login?code=secret&state=opaque#callback")
                .unwrap();

        assert_eq!(
            browser_url_for_display(&login).as_deref(),
            Some("https://accounts.pixiv.net/login")
        );
    }

    #[test]
    fn browser_location_display_keeps_only_a_numeric_library_page() {
        let library =
            Url::parse("https://accounts.booth.pm/library?page=12&token=secret#downloads").unwrap();

        assert_eq!(
            browser_url_for_display(&library).as_deref(),
            Some("https://accounts.booth.pm/library?page=12")
        );
    }

    #[test]
    fn new_window_navigation_reuses_the_browser_only_for_allowed_urls() {
        for allowed in [
            "https://accounts.booth.pm/dashboard",
            "https://accounts.booth.pm/library/free_downloads?page=1",
            "https://sample-shop.booth.pm/items/123",
            "https://accounts.pixiv.net/login",
        ] {
            let url = Url::parse(allowed).unwrap();
            assert_eq!(new_window_navigation_target(url.clone()), Some(url));
        }

        for rejected in [
            "http://accounts.booth.pm/dashboard",
            "https://booth.pm.example.test/items/123",
            "https://example.test/",
            "about:blank",
        ] {
            assert!(new_window_navigation_target(Url::parse(rejected).unwrap()).is_none());
        }
    }

    #[test]
    fn active_download_notifications_retain_completed_state_for_page_replay() {
        let notifications = ActiveDownloadNotifications::default();
        let now = Instant::now();
        let request_id = "7cf15df3-9714-4ff5-bf56-ab976127be9d";
        let mut event = DownloadStatusEvent {
            request_id: request_id.into(),
            item_id: Some(3_848_152),
            filename: Some("private-file.zip".into()),
            state: DownloadState::Downloading,
            message: "Queued".into(),
        };

        notifications.update_at(&event, now);
        assert_eq!(
            notifications.snapshot_at(now),
            vec![(request_id.into(), DownloadState::Downloading)]
        );

        event.state = DownloadState::Downloading;
        notifications.update_at(&event, now);
        assert_eq!(
            notifications.snapshot_at(now),
            vec![(request_id.into(), DownloadState::Downloading)]
        );

        event.state = DownloadState::Completed;
        notifications.update_at(&event, now);
        assert_eq!(
            notifications.snapshot_at(now),
            vec![(request_id.into(), DownloadState::Completed)]
        );
        assert!(
            notifications
                .snapshot_at(now + COMPLETED_NOTIFICATION_REPLAY_LIFETIME)
                .is_empty()
        );

        event.request_id = "027584e5-c497-44f2-830b-4172bed74a23".into();
        event.state = DownloadState::Downloading;
        notifications.update_at(&event, now);
        event.state = DownloadState::Failed;
        notifications.update_at(&event, now);
        assert!(notifications.snapshot_at(now).is_empty());
    }

    #[test]
    fn active_download_notifications_ignore_non_uuid_request_ids() {
        let notifications = ActiveDownloadNotifications::default();
        notifications.update(&DownloadStatusEvent {
            request_id: "not-a-uuid".into(),
            item_id: None,
            filename: None,
            state: DownloadState::Downloading,
            message: "Downloading".into(),
        });

        assert!(notifications.snapshot().is_empty());
    }

    #[test]
    fn download_bridge_is_origin_scoped_and_does_not_expose_tauri_ipc() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("https://accounts.booth.pm"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("booth-library-manager://"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("booth-shelf://download-intent"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("data-booth-shelf-theme"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("__TAURI__"));
        assert!(!BOOTH_DOWNLOAD_BRIDGE.contains("invoke("));
    }

    #[test]
    fn download_bridge_uses_the_official_download_url_after_acknowledgement() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("downloadables"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("variation_id"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("__boothShelfAcceptDownloadIntent"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("window.location.assign(pending.href)"));
        let notify = BOOTH_DOWNLOAD_BRIDGE
            .find("window.__boothShelfNotify({ requestId: key, state: \"downloading\" });")
            .expect("the bridge should show the in-progress notification");
        let navigate = BOOTH_DOWNLOAD_BRIDGE
            .find("window.location.assign(pending.href)")
            .expect("the bridge should follow the official download URL");
        assert!(notify < navigate);
    }

    #[test]
    fn download_bridge_hides_only_library_chrome_before_the_tabs() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("LIBRARY_TAB_LABELS"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("window.location.pathname === \"/library\""));
        assert!(
            BOOTH_DOWNLOAD_BRIDGE.contains("window.location.pathname.startsWith(\"/library/\")")
        );
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("hidePrecedingSiblings"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("tabs.closest(\"main\")"));
    }

    #[test]
    fn download_bridge_hides_the_library_footer() {
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("'footer, [role=\"contentinfo\"]'"));
        assert!(BOOTH_DOWNLOAD_BRIDGE.contains("footer.classList.add(HIDDEN_CHROME_CLASS)"));
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
        let permissions = capability["permissions"].as_array().unwrap();
        assert_eq!(capability["webviews"], serde_json::json!(["main"]));
        assert!(capability.get("windows").is_none());
        assert!(capability.get("remote").is_none());
        assert!(permissions.contains(&serde_json::json!("allow-navigate-booth-browser")));
        assert!(permissions.contains(&serde_json::json!("allow-clear-booth-browser-data")));
        assert!(!permissions.contains(&serde_json::json!(
            "core:webview:allow-clear-all-browsing-data"
        )));
        assert!(!permissions.iter().any(|permission| {
            permission
                .as_str()
                .is_some_and(|permission| permission.contains("booth-browser-theme"))
        }));
        assert!(!permissions.contains(&serde_json::json!("dialog:allow-ask")));
        assert!(!permissions.contains(&serde_json::json!("opener:default")));
        let opener = permissions
            .iter()
            .find(|permission| permission["identifier"] == "opener:allow-open-url")
            .unwrap();
        assert_eq!(
            opener["allow"],
            serde_json::json!([
                { "url": "https://booth.pm/terms" },
                { "url": "https://booth.pm/privacy" }
            ])
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
