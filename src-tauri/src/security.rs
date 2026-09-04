use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, AppResult};

const RESERVED_WINDOWS_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub fn sanitize_path_component(value: &str, fallback: &str) -> String {
    let mut result = value
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'..='\u{1f}' => '_',
            _ => character,
        })
        .collect::<String>();
    result = result.trim().trim_end_matches(['.', ' ']).to_owned();
    if result.is_empty() {
        result = fallback.to_owned();
    }
    let stem = result.split('.').next().unwrap_or_default();
    if RESERVED_WINDOWS_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        result.insert(0, '_');
    }
    result.chars().take(120).collect()
}

pub fn safe_filename(value: &str) -> AppResult<String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || value.contains(':')
    {
        return Err(AppError::UnsafeFilename);
    }
    let safe = sanitize_path_component(value, "download");
    if safe == "." || safe == ".." {
        return Err(AppError::UnsafeFilename);
    }
    Ok(safe)
}

pub fn ensure_within_root(root: &Path, candidate: &Path) -> AppResult<()> {
    if candidate.starts_with(root) {
        Ok(())
    } else {
        Err(AppError::PathOutsideLibrary)
    }
}

pub fn product_directory(
    root: &Path,
    shop_name: &str,
    product_name: &str,
    item_id: i64,
    variation_id: i64,
) -> PathBuf {
    root.join(sanitize_path_component(shop_name, "ショップ未取得"))
        .join(format!(
            "{} [booth-{item_id}]",
            sanitize_path_component(product_name, "名称未取得の商品")
        ))
        .join(format!("variation-{variation_id}"))
}

pub fn is_allowed_browser_url(url: &url::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    host == "booth.pm"
        || host.ends_with(".booth.pm")
        || host == "pixiv.net"
        || host.ends_with(".pixiv.net")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_windows_names_and_reserved_characters() {
        assert_eq!(sanitize_path_component("CON.txt", "fallback"), "_CON.txt");
        assert_eq!(sanitize_path_component("a:b/c", "fallback"), "a_b_c");
    }

    #[test]
    fn rejects_filename_traversal() {
        assert!(safe_filename("../secret.txt").is_err());
        assert!(safe_filename("C:\\secret.txt").is_err());
        assert!(safe_filename("ok.zip").is_ok());
    }

    #[test]
    fn browser_allowlist_rejects_lookalike_hosts() {
        assert!(is_allowed_browser_url(
            &url::Url::parse("https://accounts.booth.pm/library").unwrap()
        ));
        assert!(!is_allowed_browser_url(
            &url::Url::parse("https://booth.pm.example.test/").unwrap()
        ));
    }
}
