//! Three-tier freshness ladder for `temper resource show` in Local mode.
//!
//! Given a resource id and its local cached path, decide how to produce
//! the content to render:
//!
//! 1. **Debounce**: if the local file's mtime is within `DEBOUNCE_SECONDS`
//!    of now, render the local content without any API call.
//! 2. **Hash-verify**: otherwise, `GET /resources/{id}` (metadata only, no
//!    body). If the server's `updated` timestamp matches the local
//!    frontmatter's `temper-updated`, touch the local mtime to now and
//!    render the local content.
//! 3. **Full-fetch**: if metadata diverges or no local file exists, call
//!    `GET /resources/{id}/content`, overwrite the local file, render
//!    the server response.
//!
//! Cloud mode never calls into this module — callers match on
//! `VaultState` before invoking.
//!
//! Offline degradation: on any network error inside tier 2 or 3, fall
//! back to "render local with a warn" if a local file exists, otherwise
//! surface the error.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use filetime::{set_file_mtime, FileTime};
use temper_client::TemperClient;
use temper_core::types::ids::ResourceId;

use crate::error::{Result, TemperError};
use crate::output;

/// Default debounce window.
pub const DEFAULT_DEBOUNCE_SECONDS: u64 = 30;

pub struct ShowCacheParams<'a> {
    pub client: &'a TemperClient,
    pub resource_id: ResourceId,
    pub local_path: &'a Path,
    pub debounce: Duration,
}

pub struct ShowCacheResult {
    pub content: String,
    pub source: FreshnessTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessTier {
    Debounced,
    HashMatch,
    FullFetch,
    OfflineFallback,
}

pub async fn fetch(params: ShowCacheParams<'_>) -> Result<ShowCacheResult> {
    if let Some(fresh) = read_if_fresh(params.local_path, params.debounce)? {
        return Ok(ShowCacheResult {
            content: fresh,
            source: FreshnessTier::Debounced,
        });
    }
    match attempt_remote(&params).await {
        Ok(result) => Ok(result),
        Err(err) if is_network_error(&err) => {
            if let Ok(body) = fs::read_to_string(params.local_path) {
                output::hint(format!(
                    "offline: rendering cached copy of {} (reason: {err})",
                    params.local_path.display()
                ));
                Ok(ShowCacheResult {
                    content: body,
                    source: FreshnessTier::OfflineFallback,
                })
            } else {
                Err(err)
            }
        }
        Err(err) => Err(err),
    }
}

fn read_if_fresh(path: &Path, debounce: Duration) -> Result<Option<String>> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    let mtime = meta
        .modified()
        .map_err(|e| TemperError::Vault(format!("mtime read: {e}")))?;
    let age = SystemTime::now()
        .duration_since(mtime)
        .unwrap_or(Duration::ZERO);
    if age < debounce {
        let body = fs::read_to_string(path).map_err(|e| TemperError::Vault(e.to_string()))?;
        Ok(Some(body))
    } else {
        Ok(None)
    }
}

async fn attempt_remote(params: &ShowCacheParams<'_>) -> Result<ShowCacheResult> {
    let meta_check = params
        .client
        .resources()
        .get(*params.resource_id.as_uuid())
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    if let Ok(local_body) = fs::read_to_string(params.local_path) {
        if let Some(local_updated) = parse_frontmatter_updated(&local_body) {
            if local_updated == meta_check.updated {
                let now = FileTime::from_system_time(SystemTime::now());
                set_file_mtime(params.local_path, now)
                    .map_err(|e| TemperError::Vault(format!("touch mtime: {e}")))?;
                return Ok(ShowCacheResult {
                    content: local_body,
                    source: FreshnessTier::HashMatch,
                });
            }
        }
    }

    let content = params
        .client
        .resources()
        .content(*params.resource_id.as_uuid())
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;
    fs::write(params.local_path, &content.markdown)
        .map_err(|e| TemperError::Vault(format!("cache write: {e}")))?;
    Ok(ShowCacheResult {
        content: content.markdown,
        source: FreshnessTier::FullFetch,
    })
}

fn parse_frontmatter_updated(body: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let fm = temper_core::frontmatter::Frontmatter::try_from(body).ok()?;
    let updated = fm.value().get("temper-updated")?;
    let s = updated.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn is_network_error(err: &TemperError) -> bool {
    matches!(err, TemperError::Api(msg) if msg.contains("connect") || msg.contains("dns") || msg.contains("timeout"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    #[test]
    fn read_if_fresh_returns_content_when_mtime_within_window() {
        let file = NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), "hello").expect("write");
        let result = read_if_fresh(file.path(), Duration::from_secs(30))
            .expect("read_if_fresh")
            .expect("fresh within window");
        assert_eq!(result, "hello");
    }

    #[test]
    fn read_if_fresh_returns_none_when_stale() {
        let file = NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), "stale").expect("write");
        let past = FileTime::from_system_time(SystemTime::now() - Duration::from_secs(60));
        set_file_mtime(file.path(), past).expect("set mtime");
        let result = read_if_fresh(file.path(), Duration::from_secs(30)).expect("read_if_fresh");
        assert!(result.is_none(), "stale file should not be read");
    }

    #[test]
    fn read_if_fresh_returns_none_when_file_missing() {
        let path = std::path::PathBuf::from("/tmp/definitely-not-a-file-xyz-123.md");
        let result = read_if_fresh(&path, Duration::from_secs(30)).expect("read_if_fresh");
        assert!(result.is_none());
    }
}
