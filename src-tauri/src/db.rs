use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::{
    error::{AppError, AppResult},
    model::{AppSettings, DownloadRequest, ProductSummary, UpsertProductInput},
};

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS products (
    item_id INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    shop_name TEXT NOT NULL,
    shop_subdomain TEXT,
    product_url TEXT NOT NULL,
    thumbnail_url TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS variations (
    variation_id INTEGER PRIMARY KEY NOT NULL,
    item_id INTEGER NOT NULL REFERENCES products(item_id) ON DELETE CASCADE,
    name TEXT,
    UNIQUE(item_id, variation_id)
);

CREATE TABLE IF NOT EXISTS downloadables (
    downloadable_id INTEGER PRIMARY KEY NOT NULL,
    variation_id INTEGER NOT NULL REFERENCES variations(variation_id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    UNIQUE(variation_id, downloadable_id)
);

CREATE TABLE IF NOT EXISTS artifacts (
    artifact_id TEXT PRIMARY KEY NOT NULL,
    item_id INTEGER NOT NULL REFERENCES products(item_id) ON DELETE CASCADE,
    variation_id INTEGER NOT NULL REFERENCES variations(variation_id) ON DELETE CASCADE,
    downloadable_id INTEGER REFERENCES downloadables(downloadable_id) ON DELETE SET NULL,
    filename TEXT NOT NULL,
    local_path TEXT NOT NULL UNIQUE,
    sha256 TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    downloaded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS artifacts_item_id_idx ON artifacts(item_id);
CREATE INDEX IF NOT EXISTS artifacts_downloaded_at_idx ON artifacts(downloaded_at DESC);

CREATE TABLE IF NOT EXISTS product_metadata_fetches (
    item_id INTEGER PRIMARY KEY NOT NULL,
    last_attempt_at INTEGER NOT NULL,
    last_success_at INTEGER
);
"#;

const METADATA_LAST_REQUEST_KEY: &str = "product_metadata_last_request_at";
const METADATA_PAUSED_UNTIL_KEY: &str = "product_metadata_paused_until";

#[derive(Debug, Clone)]
pub struct ArtifactLocation {
    pub artifact_id: String,
    pub local_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub struct MetadataFetchPolicy {
    pub success_cache_seconds: i64,
    pub failed_retry_seconds: i64,
    pub minimum_interval_seconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataFetchMode {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataFetchSkipReason {
    CachedOrCoolingDown,
    MinimumInterval,
    Paused,
}

#[derive(Debug)]
pub enum MetadataFetchReservation {
    Fetch {
        stale: Option<UpsertProductInput>,
    },
    Skip {
        cached: Option<UpsertProductInput>,
        reason: MetadataFetchSkipReason,
    },
}

#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
}

impl Database {
    pub fn initialize(path: PathBuf) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let database = Self { path };
        let connection = database.open()?;
        connection.execute_batch(SCHEMA)?;
        Ok(database)
    }

    fn open(&self) -> AppResult<Connection> {
        let connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(connection)
    }

    pub fn settings(&self) -> AppResult<AppSettings> {
        let connection = self.open()?;
        let library_root = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'library_root'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let library_root_opt_in_fingerprint = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'library_root_opt_in_fingerprint'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(AppSettings {
            library_root,
            library_root_opt_in_fingerprint,
        })
    }

    pub fn set_library_root_with_opt_in(
        &self,
        root: &Path,
        opt_in_fingerprint: Option<&str>,
    ) -> AppResult<()> {
        if !root.is_absolute() {
            return Err(AppError::InvalidLibraryRoot(root.to_path_buf()));
        }
        let value = root.to_string_lossy();
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO settings(key, value) VALUES ('library_root', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value.as_ref()],
        )?;
        if let Some(fingerprint) = opt_in_fingerprint {
            transaction.execute(
                "INSERT INTO settings(key, value) VALUES ('library_root_opt_in_fingerprint', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [fingerprint],
            )?;
        } else {
            transaction.execute(
                "DELETE FROM settings WHERE key = 'library_root_opt_in_fingerprint'",
                [],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn has_artifacts(&self) -> AppResult<bool> {
        let connection = self.open()?;
        let count = connection.query_row("SELECT COUNT(*) FROM artifacts", [], |row| {
            row.get::<_, i64>(0)
        })?;
        Ok(count > 0)
    }

    pub fn has_product(&self, item_id: i64) -> AppResult<bool> {
        let connection = self.open()?;
        let count = connection.query_row(
            "SELECT COUNT(*) FROM products WHERE item_id = ?1",
            [item_id],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count > 0)
    }

    pub fn reserve_product_metadata_fetch(
        &self,
        item_id: i64,
        now: i64,
        policy: MetadataFetchPolicy,
        mode: MetadataFetchMode,
    ) -> AppResult<MetadataFetchReservation> {
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cached = cached_product_metadata(&transaction, item_id)?;
        let paused_until = setting_i64(&transaction, METADATA_PAUSED_UNTIL_KEY)?;
        if paused_until.is_some_and(|until| until > now) {
            transaction.commit()?;
            return Ok(MetadataFetchReservation::Skip {
                cached,
                reason: MetadataFetchSkipReason::Paused,
            });
        }

        if mode == MetadataFetchMode::Automatic {
            let fetch_times = transaction
                .query_row(
                    "SELECT last_attempt_at, last_success_at FROM product_metadata_fetches WHERE item_id = ?1",
                    [item_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
                )
                .optional()?;
            if let Some((last_attempt, last_success)) = fetch_times {
                let success_is_fresh = last_success.is_some_and(|last_success| {
                    elapsed_seconds(now, last_success) < policy.success_cache_seconds
                });
                let attempt_is_recent =
                    elapsed_seconds(now, last_attempt) < policy.failed_retry_seconds;
                if success_is_fresh || attempt_is_recent {
                    transaction.commit()?;
                    return Ok(MetadataFetchReservation::Skip {
                        cached,
                        reason: MetadataFetchSkipReason::CachedOrCoolingDown,
                    });
                }
            }
        }

        let last_global_request = setting_i64(&transaction, METADATA_LAST_REQUEST_KEY)?;
        if last_global_request.is_some_and(|last_request| {
            elapsed_seconds(now, last_request) < policy.minimum_interval_seconds
        }) {
            transaction.commit()?;
            return Ok(MetadataFetchReservation::Skip {
                cached,
                reason: MetadataFetchSkipReason::MinimumInterval,
            });
        }

        transaction.execute(
            "INSERT INTO product_metadata_fetches(item_id, last_attempt_at) VALUES (?1, ?2) \
             ON CONFLICT(item_id) DO UPDATE SET last_attempt_at = excluded.last_attempt_at",
            params![item_id, now],
        )?;
        set_setting_i64(&transaction, METADATA_LAST_REQUEST_KEY, now)?;
        transaction.commit()?;
        Ok(MetadataFetchReservation::Fetch { stale: cached })
    }

    pub fn record_product_metadata_fetch_success(
        &self,
        input: &UpsertProductInput,
        now: i64,
    ) -> AppResult<()> {
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        upsert_product_on(&transaction, input)?;
        transaction.execute(
            "INSERT INTO product_metadata_fetches(item_id, last_attempt_at, last_success_at) \
             VALUES (?1, ?2, ?2) \
             ON CONFLICT(item_id) DO UPDATE SET \
               last_attempt_at = excluded.last_attempt_at, \
               last_success_at = excluded.last_success_at",
            params![input.item_id, now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn pause_product_metadata_fetches_until(&self, until: i64) -> AppResult<()> {
        let connection = self.open()?;
        set_setting_i64(&connection, METADATA_PAUSED_UNTIL_KEY, until)
    }

    pub fn ensure_download_metadata(&self, request: &DownloadRequest) -> AppResult<()> {
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        let fallback_name = format!("BOOTH item {}", request.item_id);
        transaction.execute(
            "INSERT OR IGNORE INTO products(item_id, name, shop_name, product_url) VALUES (?1, ?2, ?3, ?4)",
            params![
                request.item_id,
                request
                    .product_name
                    .as_deref()
                    .unwrap_or(&fallback_name),
                request.shop_name.as_deref().unwrap_or("BOOTH"),
                format!("https://booth.pm/ja/items/{}", request.item_id),
            ],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO variations(variation_id, item_id) VALUES (?1, ?2)",
            params![request.variation_id, request.item_id],
        )?;
        if let Some(downloadable_id) = request.downloadable_id {
            transaction.execute(
                "INSERT INTO downloadables(downloadable_id, variation_id, filename) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(downloadable_id) DO UPDATE SET filename = excluded.filename",
                params![downloadable_id, request.variation_id, request.filename],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_artifact(
        &self,
        artifact_id: &str,
        request: &DownloadRequest,
        path: &Path,
        sha256: &str,
        byte_size: u64,
    ) -> AppResult<()> {
        let connection = self.open()?;
        connection.execute(
            "INSERT INTO artifacts(artifact_id, item_id, variation_id, downloadable_id, filename, local_path, sha256, byte_size) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(local_path) DO UPDATE SET \
               item_id = excluded.item_id, variation_id = excluded.variation_id, \
               downloadable_id = excluded.downloadable_id, filename = excluded.filename, \
               sha256 = excluded.sha256, byte_size = excluded.byte_size, \
               downloaded_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![
                artifact_id,
                request.item_id,
                request.variation_id,
                request.downloadable_id,
                request.filename,
                path.to_string_lossy(),
                sha256,
                i64::try_from(byte_size).unwrap_or(i64::MAX)
            ],
        )?;
        Ok(())
    }

    pub fn product_path(&self, item_id: i64) -> AppResult<Option<PathBuf>> {
        let connection = self.open()?;
        let path = connection
            .query_row(
                "SELECT local_path FROM artifacts WHERE item_id = ?1 ORDER BY downloaded_at DESC LIMIT 1",
                [item_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(path
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(Path::to_path_buf)))
    }

    pub fn artifact_paths_for_download(
        &self,
        request: &DownloadRequest,
    ) -> AppResult<Vec<PathBuf>> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT local_path FROM artifacts \
             WHERE item_id = ?1 AND variation_id = ?2 AND filename = ?3 COLLATE NOCASE \
             ORDER BY downloaded_at DESC",
        )?;
        let rows = statement.query_map(
            params![request.item_id, request.variation_id, request.filename],
            |row| row.get::<_, String>(0).map(PathBuf::from),
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn list_artifact_locations(&self) -> AppResult<Vec<ArtifactLocation>> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT artifact_id, local_path FROM artifacts ORDER BY length(local_path) DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ArtifactLocation {
                artifact_id: row.get(0)?,
                local_path: PathBuf::from(row.get::<_, String>(1)?),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn delete_artifact(&self, artifact_id: &str) -> AppResult<()> {
        let connection = self.open()?;
        connection.execute(
            "DELETE FROM artifacts WHERE artifact_id = ?1",
            [artifact_id],
        )?;
        Ok(())
    }

    pub fn clear_library_index(&self) -> AppResult<()> {
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM product_metadata_fetches", [])?;
        transaction.execute("DELETE FROM products", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn list_products(&self) -> AppResult<Vec<ProductSummary>> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT p.item_id, p.name, p.shop_name, p.product_url, p.thumbnail_url, \
                    (SELECT a.local_path FROM artifacts a WHERE a.item_id = p.item_id ORDER BY a.downloaded_at DESC LIMIT 1), \
                    (SELECT COUNT(*) FROM artifacts a WHERE a.item_id = p.item_id), \
                    (SELECT MAX(a.downloaded_at) FROM artifacts a WHERE a.item_id = p.item_id) \
             FROM products p ORDER BY p.updated_at DESC, p.name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            let artifact_path: Option<String> = row.get(5)?;
            let local_path = artifact_path.as_deref().and_then(|path| {
                PathBuf::from(path)
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
            });
            Ok(ProductSummary {
                item_id: row.get(0)?,
                name: row.get(1)?,
                shop_name: row.get(2)?,
                product_url: row.get(3)?,
                thumbnail_url: row.get(4)?,
                local_path,
                latest_artifact_path: artifact_path,
                artifact_count: row.get(6)?,
                last_downloaded_at: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }
}

fn upsert_product_on(connection: &Connection, input: &UpsertProductInput) -> AppResult<()> {
    connection.execute(
        "INSERT INTO products(item_id, name, shop_name, shop_subdomain, product_url, thumbnail_url) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(item_id) DO UPDATE SET \
           name = excluded.name, shop_name = excluded.shop_name, \
           shop_subdomain = excluded.shop_subdomain, product_url = excluded.product_url, \
           thumbnail_url = excluded.thumbnail_url, \
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![
            input.item_id,
            input.name,
            input.shop_name,
            input.shop_subdomain,
            input.product_url,
            input.thumbnail_url
        ],
    )?;
    Ok(())
}

fn cached_product_metadata(
    connection: &Connection,
    item_id: i64,
) -> AppResult<Option<UpsertProductInput>> {
    connection
        .query_row(
            "SELECT p.item_id, p.name, p.shop_name, p.shop_subdomain, p.product_url, p.thumbnail_url \
             FROM products p \
             JOIN product_metadata_fetches f ON f.item_id = p.item_id \
             WHERE p.item_id = ?1 AND f.last_success_at IS NOT NULL",
            [item_id],
            |row| {
                Ok(UpsertProductInput {
                    item_id: row.get(0)?,
                    name: row.get(1)?,
                    shop_name: row.get(2)?,
                    shop_subdomain: row.get(3)?,
                    product_url: row.get(4)?,
                    thumbnail_url: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
}

fn setting_i64(connection: &Connection, key: &str) -> AppResult<Option<i64>> {
    let value = connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()?;
    Ok(value.and_then(|value| value.parse().ok()))
}

fn set_setting_i64(connection: &Connection, key: &str, value: i64) -> AppResult<()> {
    connection.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

fn elapsed_seconds(now: i64, then: i64) -> i64 {
    now.saturating_sub(then).max(0)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    const METADATA_POLICY: MetadataFetchPolicy = MetadataFetchPolicy {
        success_cache_seconds: 3_600,
        failed_retry_seconds: 600,
        minimum_interval_seconds: 10,
    };

    fn product_metadata(item_id: i64) -> UpsertProductInput {
        UpsertProductInput {
            item_id,
            name: "Example".into(),
            shop_name: "Shop".into(),
            shop_subdomain: Some("shop".into()),
            product_url: format!("https://booth.pm/ja/items/{item_id}"),
            thumbnail_url: Some("https://booth.pximg.net/example.png".into()),
        }
    }

    #[test]
    fn initializes_and_persists_settings() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        let root = directory.path().join("library");

        database
            .set_library_root_with_opt_in(&root, None)
            .expect("set root");

        assert_eq!(
            database.settings().expect("settings").library_root,
            Some(root.to_string_lossy().into_owned())
        );
        assert_eq!(
            database
                .settings()
                .expect("settings")
                .library_root_opt_in_fingerprint,
            None
        );
    }

    #[test]
    fn atomically_replaces_or_clears_the_root_opt_in() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        let first_root = directory.path().join("first");
        let second_root = directory.path().join("second");

        database
            .set_library_root_with_opt_in(&first_root, Some("first-fingerprint"))
            .expect("set root with opt-in");
        let settings = database.settings().expect("settings");
        assert_eq!(
            settings.library_root_opt_in_fingerprint.as_deref(),
            Some("first-fingerprint")
        );

        database
            .set_library_root_with_opt_in(&second_root, None)
            .expect("replace with local root");
        let settings = database.settings().expect("settings");
        assert_eq!(
            settings.library_root,
            Some(second_root.to_string_lossy().into_owned())
        );
        assert_eq!(settings.library_root_opt_in_fingerprint, None);
    }

    #[test]
    fn reports_whether_artifacts_are_registered() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        assert!(!database.has_artifacts().expect("no artifacts"));

        let request = DownloadRequest {
            request_id: "request-1".into(),
            item_id: 123,
            variation_id: 456,
            downloadable_id: None,
            product_name: Some("Example".into()),
            shop_name: Some("Shop".into()),
            filename: "package.zip".into(),
        };
        database
            .ensure_download_metadata(&request)
            .expect("metadata");
        database
            .record_artifact(
                "artifact-1",
                &request,
                directory.path().join("package").as_path(),
                "hash",
                1,
            )
            .expect("artifact");
        assert!(database.has_artifacts().expect("has artifacts"));
    }

    #[test]
    fn upserts_and_lists_products() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        database
            .record_product_metadata_fetch_success(&product_metadata(123), 1_000)
            .expect("upsert");

        let products = database.list_products().expect("products");
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].item_id, 123);
    }

    #[test]
    fn metadata_fetch_reservations_are_cached_and_rate_limited() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");

        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    123,
                    1_000,
                    METADATA_POLICY,
                    MetadataFetchMode::Automatic,
                )
                .expect("first reservation"),
            MetadataFetchReservation::Fetch { stale: None }
        ));
        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    456,
                    1_005,
                    METADATA_POLICY,
                    MetadataFetchMode::Automatic,
                )
                .expect("global rate limit"),
            MetadataFetchReservation::Skip {
                cached: None,
                reason: MetadataFetchSkipReason::MinimumInterval,
            }
        ));

        database
            .record_product_metadata_fetch_success(&product_metadata(123), 1_000)
            .expect("record success");
        match database
            .reserve_product_metadata_fetch(
                123,
                2_000,
                METADATA_POLICY,
                MetadataFetchMode::Automatic,
            )
            .expect("cached reservation")
        {
            MetadataFetchReservation::Skip {
                cached: Some(cached),
                reason: MetadataFetchSkipReason::CachedOrCoolingDown,
            } => assert_eq!(cached.name, "Example"),
            reservation => panic!("unexpected reservation: {reservation:?}"),
        }
        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    123,
                    5_000,
                    METADATA_POLICY,
                    MetadataFetchMode::Automatic,
                )
                .expect("stale reservation"),
            MetadataFetchReservation::Fetch { stale: Some(_) }
        ));
    }

    #[test]
    fn manual_metadata_fetch_bypasses_item_cache_but_keeps_global_interval() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    123,
                    1_000,
                    METADATA_POLICY,
                    MetadataFetchMode::Automatic,
                )
                .expect("initial reservation"),
            MetadataFetchReservation::Fetch { stale: None }
        ));
        database
            .record_product_metadata_fetch_success(&product_metadata(123), 1_000)
            .expect("record success");

        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    123,
                    1_005,
                    METADATA_POLICY,
                    MetadataFetchMode::Manual,
                )
                .expect("manual interval limit"),
            MetadataFetchReservation::Skip {
                cached: Some(_),
                reason: MetadataFetchSkipReason::MinimumInterval,
            }
        ));
        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    123,
                    1_010,
                    METADATA_POLICY,
                    MetadataFetchMode::Manual,
                )
                .expect("manual refresh"),
            MetadataFetchReservation::Fetch { stale: Some(_) }
        ));
    }

    #[test]
    fn metadata_fetch_pause_survives_new_reservations() {
        let directory = tempdir().expect("tempdir");
        let database_path = directory.path().join("test.db");
        let database = Database::initialize(database_path.clone()).expect("database");
        database
            .pause_product_metadata_fetches_until(10_000)
            .expect("pause metadata fetches");
        drop(database);
        let database = Database::initialize(database_path).expect("reopened database");

        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    123,
                    5_000,
                    METADATA_POLICY,
                    MetadataFetchMode::Automatic,
                )
                .expect("paused reservation"),
            MetadataFetchReservation::Skip {
                cached: None,
                reason: MetadataFetchSkipReason::Paused,
            }
        ));
        assert!(matches!(
            database
                .reserve_product_metadata_fetch(
                    123,
                    10_000,
                    METADATA_POLICY,
                    MetadataFetchMode::Automatic,
                )
                .expect("pause expired"),
            MetadataFetchReservation::Fetch { stale: None }
        ));
    }

    #[test]
    fn finds_only_matching_download_artifacts() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        let request = DownloadRequest {
            request_id: "request-1".into(),
            item_id: 123,
            variation_id: 456,
            downloadable_id: None,
            product_name: Some("Example".into()),
            shop_name: Some("Shop".into()),
            filename: "package.zip".into(),
        };
        database
            .ensure_download_metadata(&request)
            .expect("metadata");
        let artifact = directory.path().join("package");
        database
            .record_artifact("artifact-1", &request, &artifact, "hash", 1)
            .expect("artifact");

        assert_eq!(
            database
                .artifact_paths_for_download(&request)
                .expect("matching artifact"),
            vec![artifact]
        );

        let mut other_file = request;
        other_file.filename = "another.zip".into();
        assert!(
            database
                .artifact_paths_for_download(&other_file)
                .expect("other file")
                .is_empty()
        );
    }

    #[test]
    fn refreshes_a_stale_artifact_record_at_the_same_path() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        let request = DownloadRequest {
            request_id: "request-1".into(),
            item_id: 123,
            variation_id: 456,
            downloadable_id: None,
            product_name: Some("Example".into()),
            shop_name: Some("Shop".into()),
            filename: "package.zip".into(),
        };
        database
            .ensure_download_metadata(&request)
            .expect("metadata");
        let artifact = directory.path().join("package");
        database
            .record_artifact("artifact-1", &request, &artifact, "old-hash", 1)
            .expect("initial artifact");
        database
            .record_artifact("artifact-2", &request, &artifact, "new-hash", 2)
            .expect("refreshed artifact");

        let products = database.list_products().expect("products");
        assert_eq!(products[0].artifact_count, 1);
        assert_eq!(
            database
                .artifact_paths_for_download(&request)
                .expect("artifact paths"),
            vec![artifact]
        );
    }
}
