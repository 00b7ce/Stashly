use std::collections::HashMap;

use percent_encoding::percent_decode_str;
use url::Url;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    model::DownloadRequest,
    security::{is_allowed_download_url, safe_filename},
};

const MAX_DEEPLINK_LENGTH: usize = 32 * 1024;
const MAX_DOWNLOADS_PER_LINK: usize = 32;

pub fn is_booth_library_manager_link(url: &Url) -> bool {
    url.scheme().eq_ignore_ascii_case("booth-library-manager")
}

pub fn parse(url: &Url) -> AppResult<Vec<DownloadRequest>> {
    if !is_booth_library_manager_link(url)
        || url.as_str().len() > MAX_DEEPLINK_LENGTH
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::UnsupportedDeepLink);
    }

    let mut scalar = HashMap::<String, String>::new();
    let mut download_urls = Vec::new();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "dlurls[]" => download_urls.push(value.into_owned()),
            "item_id"
            | "variation_id"
            | "order_id"
            | "dlurl"
            | "client"
            | "downloadable_filename" => {
                if scalar
                    .insert(key.into_owned(), value.into_owned())
                    .is_some()
                {
                    return Err(AppError::InvalidPayload(
                        "a scalar query field was repeated".into(),
                    ));
                }
            }
            _ => {
                return Err(AppError::InvalidPayload(
                    "the deeplink contains an unknown query field".into(),
                ));
            }
        }
    }

    let item_id = parse_id(scalar.get("item_id"), "item_id")?;
    let variation_id = parse_id(scalar.get("variation_id"), "variation_id")?;
    if let Some(order_id) = scalar.get("order_id") {
        parse_ascii_id(order_id, "order_id")?;
    }

    match (scalar.remove("dlurl"), download_urls.is_empty()) {
        (Some(single), true) => download_urls.push(single),
        (None, false) => {}
        _ => {
            return Err(AppError::InvalidPayload(
                "expected either dlurl or one or more dlurls[] fields".into(),
            ));
        }
    }
    if download_urls.len() > MAX_DOWNLOADS_PER_LINK {
        return Err(AppError::InvalidPayload(
            "too many downloads in one deeplink".into(),
        ));
    }

    let explicit_filename = scalar.get("downloadable_filename");
    download_urls
        .into_iter()
        .map(|raw_url| {
            let signed_url = Url::parse(&raw_url)?;
            if !is_allowed_download_url(&signed_url)
                || !signed_url.username().is_empty()
                || signed_url.password().is_some()
                || signed_url.port_or_known_default() != Some(443)
            {
                return Err(AppError::RejectedDownloadUrl);
            }
            let filename = if let Some(filename) = explicit_filename {
                safe_filename(filename)?
            } else {
                filename_from_url(&signed_url)?
            };
            Ok(DownloadRequest {
                request_id: Uuid::new_v4().to_string(),
                item_id,
                variation_id,
                downloadable_id: None,
                product_name: None,
                shop_name: None,
                filename,
                signed_url,
            })
        })
        .collect()
}

fn parse_id(value: Option<&String>, field: &str) -> AppResult<i64> {
    let value = value.ok_or_else(|| AppError::InvalidPayload(format!("missing {field}")))?;
    parse_ascii_id(value, field)
}

fn parse_ascii_id(value: &str, field: &str) -> AppResult<i64> {
    if value.is_empty() || value.len() > 19 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AppError::InvalidPayload(format!("invalid {field}")));
    }
    value
        .parse::<i64>()
        .map_err(|_| AppError::InvalidPayload(format!("invalid {field}")))
}

fn filename_from_url(url: &Url) -> AppResult<String> {
    let encoded = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .ok_or(AppError::UnsafeFilename)?;
    let decoded = percent_decode_str(encoded)
        .decode_utf8()
        .map_err(|_| AppError::UnsafeFilename)?;
    safe_filename(&decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_url() -> &'static str {
        "https%3A%2F%2Fs6.booth.pm%2Fdownloads%2Fsample%2520file.zip%3FX-Amz-Signature%3Dredacted"
    }

    #[test]
    fn parses_single_download_without_retaining_order_id() {
        let url = Url::parse(&format!(
            "booth-library-manager://download?item_id=123&variation_id=456&order_id=789&dlurl={}",
            signed_url()
        ))
        .unwrap();
        let parsed = parse(&url).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].item_id, 123);
        assert_eq!(parsed[0].filename, "sample file.zip");
    }

    #[test]
    fn rejects_unknown_fields_and_unapproved_hosts() {
        let unknown = Url::parse(&format!(
            "booth-library-manager://download?item_id=1&variation_id=2&extra=3&dlurl={}",
            signed_url()
        ))
        .unwrap();
        assert!(parse(&unknown).is_err());

        let other_host = Url::parse(
            "booth-library-manager://download?item_id=1&variation_id=2&dlurl=https%3A%2F%2Fexample.com%2Fa.zip",
        )
        .unwrap();
        assert!(parse(&other_host).is_err());
    }

    #[test]
    fn rejects_single_and_array_forms_together() {
        let url = Url::parse(&format!(
            "booth-library-manager://download?item_id=1&variation_id=2&dlurl={0}&dlurls%5B%5D={0}",
            signed_url()
        ))
        .unwrap();
        assert!(parse(&url).is_err());
    }
}
