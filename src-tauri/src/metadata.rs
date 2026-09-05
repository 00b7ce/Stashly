use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::{
    StatusCode,
    header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, RETRY_AFTER},
    redirect::{Attempt, Policy},
};
use scraper::{Html, Selector};
use url::Url;

use crate::{
    db::{Database, MetadataFetchPolicy, MetadataFetchReservation},
    error::{AppError, AppResult},
    model::UpsertProductInput,
};

const SUCCESS_CACHE_SECONDS: i64 = 30 * 24 * 60 * 60;
const FAILED_RETRY_SECONDS: i64 = 24 * 60 * 60;
const MINIMUM_REQUEST_INTERVAL_SECONDS: i64 = 10;
const DEFAULT_RATE_LIMIT_PAUSE_SECONDS: i64 = 24 * 60 * 60;
const MAX_HTML_BYTES: usize = 1024 * 1024;
const METADATA_USER_AGENT: &str = concat!(
    "Stashly/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/00b7ce/Stashly)"
);

const FETCH_POLICY: MetadataFetchPolicy = MetadataFetchPolicy {
    success_cache_seconds: SUCCESS_CACHE_SECONDS,
    failed_retry_seconds: FAILED_RETRY_SECONDS,
    minimum_interval_seconds: MINIMUM_REQUEST_INTERVAL_SECONDS,
};

enum FetchOutcome {
    Success(UpsertProductInput),
    RateLimited { pause_until: i64 },
    Unavailable,
}

pub async fn product_metadata_for_download(
    database: &Database,
    item_id: i64,
) -> Option<UpsertProductInput> {
    let now = Utc::now().timestamp();
    let reservation = database
        .reserve_product_metadata_fetch(item_id, now, FETCH_POLICY)
        .ok()?;
    let stale = match reservation {
        MetadataFetchReservation::Skip { cached } => return cached,
        MetadataFetchReservation::Fetch { stale } => stale,
    };

    match fetch_product_metadata(item_id, now).await {
        FetchOutcome::Success(product) => {
            let _ = database.record_product_metadata_fetch_success(&product, now);
            Some(product)
        }
        FetchOutcome::RateLimited { pause_until } => {
            let _ = database.pause_product_metadata_fetches_until(pause_until);
            stale
        }
        FetchOutcome::Unavailable => stale,
    }
}

async fn fetch_product_metadata(item_id: i64, now: i64) -> FetchOutcome {
    let product_url = format!("https://booth.pm/ja/items/{item_id}");
    let Ok(client) = build_metadata_client() else {
        return FetchOutcome::Unavailable;
    };
    let Ok(response) = client
        .get(&product_url)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .header(ACCEPT_LANGUAGE, "ja,en;q=0.5")
        .send()
        .await
    else {
        return FetchOutcome::Unavailable;
    };

    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        let pause_seconds = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_retry_after_seconds(value, now))
            .unwrap_or(DEFAULT_RATE_LIMIT_PAUSE_SECONDS)
            .max(DEFAULT_RATE_LIMIT_PAUSE_SECONDS);
        return FetchOutcome::RateLimited {
            pause_until: now.saturating_add(pause_seconds),
        };
    }
    if !response.status().is_success() || !is_public_product_url(response.url()) {
        return FetchOutcome::Unavailable;
    }
    let is_html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"));
    if !is_html {
        return FetchOutcome::Unavailable;
    }
    let Some(text) = read_html_head(response).await else {
        return FetchOutcome::Unavailable;
    };
    parse_product_metadata(&text, item_id, product_url)
        .map(FetchOutcome::Success)
        .unwrap_or(FetchOutcome::Unavailable)
}

fn build_metadata_client() -> AppResult<reqwest::Client> {
    let redirect_policy = Policy::custom(|attempt: Attempt<'_>| {
        if attempt.previous().len() >= 3 || !is_public_product_url(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .redirect(redirect_policy)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(METADATA_USER_AGENT)
        .build()
        .map_err(|_| AppError::Network("could not initialize the metadata client".into()))
}

async fn read_html_head(response: reqwest::Response) -> Option<String> {
    let mut body = Vec::with_capacity(32 * 1024);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if body.len().saturating_add(chunk.len()) > MAX_HTML_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
        if contains_ascii_case_insensitive(&body, b"</head>") {
            break;
        }
    }
    String::from_utf8(body).ok()
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn is_public_product_url(url: &Url) -> bool {
    if url.scheme() != "https"
        || url.host_str() != Some("booth.pm")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some(segments) = url.path_segments() else {
        return false;
    };
    let segments = segments.collect::<Vec<_>>();
    let item_segment = match segments.as_slice() {
        ["items", item] => *item,
        [locale, "items", item] if matches!(*locale, "ja" | "en" | "ko" | "zh-cn" | "zh-tw") => {
            *item
        }
        _ => return false,
    };
    !item_segment.is_empty() && item_segment.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_retry_after_seconds(value: &str, now: i64) -> Option<i64> {
    if let Ok(seconds) = value.trim().parse::<i64>() {
        return (seconds >= 0).then_some(seconds);
    }
    let retry_at = DateTime::parse_from_rfc2822(value.trim()).ok()?.timestamp();
    Some(retry_at.saturating_sub(now).max(0))
}

fn parse_product_metadata(
    text: &str,
    item_id: i64,
    product_url: String,
) -> Option<UpsertProductInput> {
    let document = Html::parse_document(text);
    let title_selector = Selector::parse("meta[property='og:title']").ok()?;
    let image_selector = Selector::parse("meta[property='og:image']").ok()?;
    let raw_title = document
        .select(&title_selector)
        .next()?
        .value()
        .attr("content")?
        .trim();
    let without_booth = raw_title
        .strip_suffix(" - BOOTH")
        .or_else(|| raw_title.strip_suffix(" | BOOTH"))
        .unwrap_or(raw_title);
    let (name, shop_name) = without_booth
        .rsplit_once(" - ")
        .map(|(name, shop)| (name.trim(), shop.trim()))
        .unwrap_or((without_booth, "BOOTH"));
    if name.is_empty() {
        return None;
    }
    let thumbnail_url = document
        .select(&image_selector)
        .next()
        .and_then(|element| element.value().attr("content"))
        .map(str::trim)
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str().is_some_and(|host| {
                    host == "booth.pximg.net" || host.ends_with(".booth.pximg.net")
                })
        })
        .map(|url| url.to_string());
    Some(UpsertProductInput {
        item_id,
        name: name.to_owned(),
        shop_name: shop_name.to_owned(),
        shop_subdomain: None,
        product_url,
        thumbnail_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_public_product_open_graph_metadata() {
        let html = r#"<html><head>
          <meta property="og:title" content="Star Accessory - Moon Shop - BOOTH">
          <meta property="og:image" content="https://booth.pximg.net/example.png">
        </head></html>"#;
        let product =
            parse_product_metadata(html, 123, "https://booth.pm/ja/items/123".into()).unwrap();
        assert_eq!(product.name, "Star Accessory");
        assert_eq!(product.shop_name, "Moon Shop");
        assert_eq!(
            product.thumbnail_url.as_deref(),
            Some("https://booth.pximg.net/example.png")
        );
    }

    #[test]
    fn accepts_only_public_booth_product_urls() {
        assert!(is_public_product_url(
            &Url::parse("https://booth.pm/ja/items/123").unwrap()
        ));
        assert!(is_public_product_url(
            &Url::parse("https://booth.pm/items/123").unwrap()
        ));
        assert!(!is_public_product_url(
            &Url::parse("https://accounts.booth.pm/ja/items/123").unwrap()
        ));
        assert!(!is_public_product_url(
            &Url::parse("https://booth.pm/ja/items/not-a-number").unwrap()
        ));
        assert!(!is_public_product_url(
            &Url::parse("https://booth.pm/ja/library").unwrap()
        ));
        assert!(!is_public_product_url(
            &Url::parse("https://booth.pm/arbitrary/items/123").unwrap()
        ));
        assert!(!is_public_product_url(
            &Url::parse("https://booth.pm/ja/items/123?tracking=value").unwrap()
        ));
    }

    #[test]
    fn rejects_untrusted_open_graph_images() {
        let html = r#"<html><head>
          <meta property="og:title" content="Example - Shop - BOOTH">
          <meta property="og:image" content="https://example.com/tracker.png">
        </head></html>"#;
        let product =
            parse_product_metadata(html, 123, "https://booth.pm/ja/items/123".into()).unwrap();
        assert_eq!(product.thumbnail_url, None);
    }

    #[test]
    fn parses_retry_after_seconds_and_http_dates() {
        assert_eq!(parse_retry_after_seconds("120", 1_000), Some(120));
        let retry_at = DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:28:00 GMT")
            .unwrap()
            .timestamp();
        assert_eq!(
            parse_retry_after_seconds("Wed, 21 Oct 2015 07:28:00 GMT", retry_at - 60),
            Some(60)
        );
        assert_eq!(parse_retry_after_seconds("invalid", 1_000), None);
    }
}
