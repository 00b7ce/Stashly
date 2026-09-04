use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    db::Database,
    error::{AppError, AppResult},
    security::ensure_within_root,
};

const STAGING_DIRECTORY: &str = ".booth-shelf-staging";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteLibraryResult {
    pub removed_artifacts: usize,
    pub removed_paths: usize,
}

pub fn delete_downloaded_files(
    database: &Database,
    configured_root: &Path,
) -> AppResult<DeleteLibraryResult> {
    let root = fs::canonicalize(configured_root)?;
    if !root.is_dir() {
        return Err(AppError::InvalidLibraryRoot(root));
    }

    let artifacts = database.list_artifact_locations()?;
    let mut removed_artifacts = 0;
    let mut removed_paths = 0;

    for artifact in artifacts {
        let removed_parent = remove_managed_path(&root, &artifact.local_path)?;
        database.delete_artifact(&artifact.artifact_id)?;
        removed_artifacts += 1;
        if let Some(parent) = removed_parent {
            removed_paths += 1;
            prune_empty_parents(&root, &parent);
        }
    }

    let staging = root.join(STAGING_DIRECTORY);
    if remove_managed_path(&root, &staging)?.is_some() {
        removed_paths += 1;
    }
    database.clear_library_index()?;

    Ok(DeleteLibraryResult {
        removed_artifacts,
        removed_paths,
    })
}

fn remove_managed_path(root: &Path, path: &Path) -> AppResult<Option<PathBuf>> {
    if !path.is_absolute() {
        return Err(AppError::PathOutsideLibrary);
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(AppError::PathOutsideLibrary);
    }

    let resolved = fs::canonicalize(path)?;
    ensure_within_root(root, &resolved)?;
    if resolved == root {
        return Err(AppError::PathOutsideLibrary);
    }
    let parent = resolved.parent().map(Path::to_path_buf);

    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(parent)
}

fn prune_empty_parents(root: &Path, start: &Path) {
    let mut current = PathBuf::from(start);
    while current != root && current.starts_with(root) {
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            let Some(parent) = current.parent() else {
                break;
            };
            current = parent.to_path_buf();
            continue;
        };
        if metadata.file_type().is_symlink() || fs::remove_dir(&current).is_err() {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DownloadRequest, UpsertProductInput};
    use tempfile::tempdir;

    fn add_artifact(database: &Database, path: &Path) {
        database
            .upsert_product(&UpsertProductInput {
                item_id: 123,
                name: "Example".into(),
                shop_name: "Shop".into(),
                shop_subdomain: None,
                product_url: "https://booth.pm/ja/items/123".into(),
                thumbnail_url: None,
            })
            .unwrap();
        let request = DownloadRequest {
            request_id: "request-1".into(),
            item_id: 123,
            variation_id: 456,
            downloadable_id: Some(789),
            product_name: Some("Example".into()),
            shop_name: Some("Shop".into()),
            filename: "package.zip".into(),
        };
        database.ensure_download_metadata(&request).unwrap();
        database
            .record_artifact("artifact-1", &request, path, "sha256", 42)
            .unwrap();
    }

    #[test]
    fn deletes_only_indexed_artifacts_and_keeps_unrelated_files() {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("library");
        let artifact = root.join("Shop").join("Example").join("package");
        fs::create_dir_all(&artifact).unwrap();
        fs::write(artifact.join("asset.txt"), b"downloaded").unwrap();
        fs::write(root.join("keep.txt"), b"unrelated").unwrap();
        fs::create_dir(root.join(STAGING_DIRECTORY)).unwrap();

        let database = Database::initialize(temporary.path().join("test.db")).unwrap();
        database.set_library_root_with_opt_in(&root, None).unwrap();
        add_artifact(&database, &artifact);

        let result = delete_downloaded_files(&database, &root).unwrap();

        assert_eq!(result.removed_artifacts, 1);
        assert_eq!(result.removed_paths, 2);
        assert!(!artifact.exists());
        assert!(!root.join(STAGING_DIRECTORY).exists());
        assert_eq!(
            fs::read_to_string(root.join("keep.txt")).unwrap(),
            "unrelated"
        );
        assert!(database.list_products().unwrap().is_empty());
    }

    #[test]
    fn refuses_an_indexed_path_outside_the_library_root() {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("library");
        let outside = temporary.path().join("outside.txt");
        fs::create_dir(&root).unwrap();
        fs::write(&outside, b"keep").unwrap();

        let database = Database::initialize(temporary.path().join("test.db")).unwrap();
        database.set_library_root_with_opt_in(&root, None).unwrap();
        add_artifact(&database, &outside);

        assert!(delete_downloaded_files(&database, &root).is_err());
        assert!(outside.exists());
        assert_eq!(database.list_products().unwrap().len(), 1);
    }
}
