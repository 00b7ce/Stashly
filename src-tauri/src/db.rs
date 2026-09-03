use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

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
"#;

#[derive(Debug, Clone)]
pub struct ArtifactLocation {
    pub artifact_id: String,
    pub local_path: PathBuf,
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
        Ok(AppSettings { library_root })
    }

    pub fn set_library_root(&self, root: &Path) -> AppResult<()> {
        if !root.is_absolute() {
            return Err(AppError::InvalidLibraryRoot(root.to_path_buf()));
        }
        let value = root.to_string_lossy();
        let connection = self.open()?;
        connection.execute(
            "INSERT INTO settings(key, value) VALUES ('library_root', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value.as_ref()],
        )?;
        Ok(())
    }

    pub fn upsert_product(&self, input: &UpsertProductInput) -> AppResult<()> {
        let connection = self.open()?;
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
        let connection = self.open()?;
        connection.execute("DELETE FROM products", [])?;
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn initializes_and_persists_settings() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        let root = directory.path().join("library");

        database.set_library_root(&root).expect("set root");

        assert_eq!(
            database.settings().expect("settings").library_root,
            Some(root.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn upserts_and_lists_products() {
        let directory = tempdir().expect("tempdir");
        let database = Database::initialize(directory.path().join("test.db")).expect("database");
        database
            .upsert_product(&UpsertProductInput {
                item_id: 123,
                name: "Example".into(),
                shop_name: "Shop".into(),
                shop_subdomain: Some("shop".into()),
                product_url: "https://booth.pm/ja/items/123".into(),
                thumbnail_url: None,
            })
            .expect("upsert");

        let products = database.list_products().expect("products");
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].item_id, 123);
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
            signed_url: url::Url::parse("https://s6.booth.pm/package.zip").unwrap(),
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
            signed_url: url::Url::parse("https://s6.booth.pm/package.zip").unwrap(),
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
