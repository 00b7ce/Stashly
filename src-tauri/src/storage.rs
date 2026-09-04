use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    model::{AppSettings, LibraryRootCandidate, LibraryStorageSummary, StorageLocationKind},
};

#[derive(Debug, Clone)]
pub struct ValidatedLibraryRoot {
    pub path: PathBuf,
    pub kind: StorageLocationKind,
    pub reasons: Vec<String>,
    pub fingerprint: String,
}

impl ValidatedLibraryRoot {
    pub fn candidate(&self) -> LibraryRootCandidate {
        LibraryRootCandidate {
            path: path_for_display(&self.path),
            kind: self.kind,
            reasons: self.reasons.clone(),
        }
    }

    pub fn requires_confirmation(&self) -> bool {
        self.kind != StorageLocationKind::Local
    }
}

pub fn validate_library_root(path: &Path) -> AppResult<ValidatedLibraryRoot> {
    let validated = inspect_library_root(path)?;
    probe_writable(&validated.path)?;
    Ok(validated)
}

pub fn inspect_library_root(path: &Path) -> AppResult<ValidatedLibraryRoot> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(AppError::InvalidLibraryRoot(path.to_path_buf()));
    }
    let canonical =
        fs::canonicalize(path).map_err(|_| AppError::LibraryRootUnavailable(path.to_path_buf()))?;
    if !canonical.is_dir() {
        return Err(AppError::InvalidLibraryRoot(canonical));
    }
    let (kind, reasons) = classify_library_root(&canonical);
    let fingerprint = storage_fingerprint(&canonical, kind);
    Ok(ValidatedLibraryRoot {
        path: canonical,
        kind,
        reasons,
        fingerprint,
    })
}

pub fn validate_configured_library_root(settings: &AppSettings) -> AppResult<PathBuf> {
    let configured = settings
        .library_root
        .as_deref()
        .map(Path::new)
        .ok_or(AppError::MissingLibraryRoot)?;
    let validated = inspect_library_root(configured)?;
    if !storage_consent_matches(
        validated.kind,
        &validated.fingerprint,
        settings.library_root_opt_in_fingerprint.as_deref(),
    ) {
        return Err(AppError::NonLocalLibraryRootNotConfirmed);
    }
    probe_writable(&validated.path)?;
    Ok(validated.path)
}

pub fn library_storage_summary(settings: &AppSettings) -> Option<LibraryStorageSummary> {
    let root = settings.library_root.as_deref().map(Path::new)?;
    let (kind, _) = classify_library_root(root);
    let fingerprint = storage_fingerprint(root, kind);
    Some(LibraryStorageSummary {
        kind,
        non_local_confirmed: kind != StorageLocationKind::Local
            && settings.library_root_opt_in_fingerprint.as_deref() == Some(fingerprint.as_str()),
    })
}

pub fn same_root(left: &Path, right: &Path) -> bool {
    normalized_path_identity(left) == normalized_path_identity(right)
}

pub fn path_for_display(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc_path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc_path}")
    } else if let Some(drive_path) = path.strip_prefix(r"\\?\") {
        drive_path.to_owned()
    } else {
        path.into_owned()
    }
}

fn probe_writable(root: &Path) -> AppResult<()> {
    let probe_id = Uuid::new_v4();
    let source = root.join(format!(".stashly-storage-probe-{probe_id}.tmp"));
    let renamed = root.join(format!(".stashly-storage-probe-{probe_id}.ready"));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&source)?;
        file.write_all(b"Stashly storage probe")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&source, &renamed)?;
        fs::remove_file(&renamed)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&source);
        let _ = fs::remove_file(&renamed);
        return Err(AppError::LibraryRootUnavailable(root.to_path_buf()));
    }
    Ok(())
}

fn classify_library_root(path: &Path) -> (StorageLocationKind, Vec<String>) {
    if is_unc_path(path) {
        return (
            StorageLocationKind::Network,
            vec!["The selected folder is on a UNC network path.".into()],
        );
    }

    let drive_type = platform::drive_type(path);
    if drive_type == DriveType::Network {
        return (
            StorageLocationKind::Network,
            vec!["The selected folder is on a mapped network drive.".into()],
        );
    }

    // Cloud Files covers modern OneDrive and providers using the Windows sync-root API.
    // Environment/configuration roots supplement it, but folder-watcher products such as
    // Syncthing cannot be detected reliably by any general Windows API.
    if platform::is_cloud_sync_root(path) || is_under_known_sync_root(path) {
        return (
            StorageLocationKind::Sync,
            vec!["The selected folder is managed by a file synchronization provider.".into()],
        );
    }

    match drive_type {
        DriveType::Fixed => (StorageLocationKind::Local, Vec::new()),
        DriveType::Network => unreachable!(),
        DriveType::Other | DriveType::Unknown => (
            StorageLocationKind::Unknown,
            vec!["The storage type could not be verified as a fixed local drive.".into()],
        ),
    }
}

fn is_unc_path(path: &Path) -> bool {
    let value = path.to_string_lossy();
    value.starts_with(r"\\?\UNC\") || (value.starts_with(r"\\") && !value.starts_with(r"\\?\"))
}

fn is_under_known_sync_root(path: &Path) -> bool {
    let mut roots = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    roots.extend(dropbox_roots());
    roots
        .iter()
        .any(|root| path_is_same_or_child(path, root.as_path()))
}

fn dropbox_roots() -> Vec<PathBuf> {
    ["APPDATA", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .map(|base| base.join("Dropbox").join("info.json"))
        .filter_map(|config| fs::read_to_string(config).ok())
        .filter_map(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .flat_map(|document| {
            ["personal", "business"]
                .into_iter()
                .filter_map(move |account| {
                    document
                        .get(account)
                        .and_then(|value| value.get("path"))
                        .and_then(serde_json::Value::as_str)
                        .map(PathBuf::from)
                })
        })
        .collect()
}

fn path_is_same_or_child(path: &Path, root: &Path) -> bool {
    let path = normalized_path_identity(path);
    let root = normalized_path_identity(root);
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with(['\\', '/']))
}

fn normalized_path_identity(path: &Path) -> String {
    path_for_display(path)
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn storage_fingerprint(path: &Path, kind: StorageLocationKind) -> String {
    let mut hasher = Sha256::new();
    hasher.update(match kind {
        StorageLocationKind::Local => b"local".as_slice(),
        StorageLocationKind::Network => b"network".as_slice(),
        StorageLocationKind::Sync => b"sync".as_slice(),
        StorageLocationKind::Unknown => b"unknown".as_slice(),
    });
    hasher.update([0]);
    hasher.update(normalized_path_identity(path).as_bytes());
    hex::encode(hasher.finalize())
}

fn storage_consent_matches(
    kind: StorageLocationKind,
    expected_fingerprint: &str,
    stored_fingerprint: Option<&str>,
) -> bool {
    kind == StorageLocationKind::Local || stored_fingerprint == Some(expected_fingerprint)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriveType {
    Fixed,
    Network,
    Other,
    Unknown,
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};

    use windows_sys::Win32::{
        Storage::{
            CloudFilters::{
                CF_SYNC_ROOT_BASIC_INFO, CF_SYNC_ROOT_INFO_BASIC, CfGetSyncRootInfoByPath,
            },
            FileSystem::GetDriveTypeW,
        },
        System::WindowsProgramming::{
            DRIVE_CDROM, DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOTE,
            DRIVE_REMOVABLE, DRIVE_UNKNOWN,
        },
    };

    use super::DriveType;

    pub(super) fn drive_type(path: &Path) -> DriveType {
        let display = super::path_for_display(path);
        let bytes = display.as_bytes();
        if bytes.len() < 3 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
            return DriveType::Unknown;
        }
        let root = format!("{}:\\", bytes[0] as char);
        let wide = wide_null(OsStr::new(&root));
        match unsafe { GetDriveTypeW(wide.as_ptr()) } {
            DRIVE_FIXED => DriveType::Fixed,
            DRIVE_REMOTE => DriveType::Network,
            DRIVE_REMOVABLE | DRIVE_CDROM | DRIVE_RAMDISK => DriveType::Other,
            DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => DriveType::Unknown,
            _ => DriveType::Unknown,
        }
    }

    pub(super) fn is_cloud_sync_root(path: &Path) -> bool {
        let wide = wide_null(path.as_os_str());
        let mut info = CF_SYNC_ROOT_BASIC_INFO::default();
        let mut returned = 0_u32;
        let result = unsafe {
            CfGetSyncRootInfoByPath(
                wide.as_ptr(),
                CF_SYNC_ROOT_INFO_BASIC,
                (&mut info as *mut CF_SYNC_ROOT_BASIC_INFO).cast(),
                std::mem::size_of::<CF_SYNC_ROOT_BASIC_INFO>() as u32,
                &mut returned,
            )
        };
        result >= 0
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use std::path::Path;

    use super::DriveType;

    pub(super) fn drive_type(_path: &Path) -> DriveType {
        DriveType::Fixed
    }

    pub(super) fn is_cloud_sync_root(_path: &Path) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn identifies_normal_and_verbatim_unc_paths() {
        assert!(is_unc_path(Path::new(r"\\server\share\library")));
        assert!(is_unc_path(Path::new(r"\\?\UNC\server\share\library")));
        assert!(!is_unc_path(Path::new(r"C:\library")));
    }

    #[test]
    fn fingerprints_are_bound_to_path_and_kind() {
        let path = Path::new(r"C:\library");
        assert_eq!(
            storage_fingerprint(path, StorageLocationKind::Network),
            storage_fingerprint(Path::new(r"c:/LIBRARY/"), StorageLocationKind::Network)
        );
        assert_ne!(
            storage_fingerprint(path, StorageLocationKind::Network),
            storage_fingerprint(path, StorageLocationKind::Sync)
        );
        assert_ne!(
            storage_fingerprint(path, StorageLocationKind::Network),
            storage_fingerprint(Path::new(r"C:\other"), StorageLocationKind::Network)
        );
    }

    #[test]
    fn validates_a_writable_directory_and_removes_the_probe() {
        let directory = tempdir().expect("tempdir");
        let validated = validate_library_root(directory.path()).expect("validated root");
        assert_eq!(validated.path, fs::canonicalize(directory.path()).unwrap());
        assert_eq!(validated.kind, StorageLocationKind::Local);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn non_local_consent_is_fail_closed_and_bound_to_one_fingerprint() {
        assert!(storage_consent_matches(
            StorageLocationKind::Local,
            "local-fingerprint",
            None
        ));
        assert!(!storage_consent_matches(
            StorageLocationKind::Network,
            "expected",
            None
        ));
        assert!(!storage_consent_matches(
            StorageLocationKind::Sync,
            "expected",
            Some("another-root")
        ));
        assert!(storage_consent_matches(
            StorageLocationKind::Unknown,
            "expected",
            Some("expected")
        ));
    }

    #[test]
    fn legacy_non_local_settings_are_reported_as_unconfirmed() {
        let root = Path::new(r"\\server\share\library");
        let mut settings = AppSettings {
            library_root: Some(root.to_string_lossy().into_owned()),
            library_root_opt_in_fingerprint: None,
        };
        let summary = library_storage_summary(&settings).expect("storage summary");
        assert_eq!(summary.kind, StorageLocationKind::Network);
        assert!(!summary.non_local_confirmed);

        settings.library_root_opt_in_fingerprint =
            Some(storage_fingerprint(root, StorageLocationKind::Network));
        assert!(
            library_storage_summary(&settings)
                .expect("confirmed storage summary")
                .non_local_confirmed
        );
    }

    #[test]
    fn refuses_a_file_as_a_library_root() {
        let directory = tempdir().expect("tempdir");
        let file = directory.path().join("not-a-directory");
        fs::write(&file, b"file").unwrap();
        assert!(matches!(
            validate_library_root(&file),
            Err(AppError::InvalidLibraryRoot(_))
        ));
    }

    #[test]
    fn display_paths_hide_windows_verbatim_prefixes() {
        assert_eq!(
            path_for_display(Path::new(r"\\?\UNC\server\share\BOOTH")),
            r"\\server\share\BOOTH"
        );
        assert_eq!(path_for_display(Path::new(r"\\?\F:\BOOTH")), r"F:\BOOTH");
    }
}
