//! Two-tier freshness ladder for `temper resource show` in Local mode.
//!
//! Given a resource id and its local cached path, decide how to produce
//! the content to render:
//!
//! 1. **Debounce**: if the local file's mtime is within `DEBOUNCE_SECONDS`
//!    of now, render the local content without any API call.
//! 2. **Full-fetch**: otherwise, `GET /resources/{id}` (metadata) then
//!    `GET /resources/{id}/content` (body), rebuild the full file
//!    (frontmatter + body) from the server response, overwrite the local
//!    file, and render it. Full-fetch is the corruption-resistant path —
//!    it always reconstructs from canonical server state.
//!
//! Cloud mode never calls into this module — callers select the
//! appropriate code path before invoking.
//!
//! Offline degradation: on any network error, fall back to "render local
//! with a warn" if a local file exists, otherwise surface the error.
//!
//! Erasure: when the server answers that the resource was ERASED, the local
//! file is removed and the erasure surfaces as the error. No other answer
//! removes it — a not-found may be a soft delete, a move, or a resource the
//! caller can no longer read.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};
use temper_client::TemperClient;
use temper_core::types::ids::ResourceId;
use temper_core::types::resource_view::ResourceView;
use temper_workflow::types::ContentResponse;

use crate::actions::runtime::client_err_to_temper;
use crate::error::{Result, TemperError};
use crate::output;

/// Default debounce window: how long a freshly-projected local file is trusted
/// before a remote re-fetch is attempted. Long enough that back-to-back reads
/// in one working session skip the network, short enough that edits made
/// elsewhere surface quickly.
pub const DEFAULT_DEBOUNCE_SECONDS: u64 = 30;

pub struct ShowCacheParams<'a> {
    pub client: &'a TemperClient,
    pub resource_id: ResourceId,
    pub local_path: &'a Path,
    pub debounce: Duration,
}

#[derive(Debug)]
pub struct ShowCacheResult {
    pub content: String,
    pub source: FreshnessTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessTier {
    Debounced,
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
        Err(err @ TemperError::Network(_)) => {
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
        // Erased is the one answer that licenses dropping the local copy: the content is gone
        // for good, and the server said so to a caller who held it. A `NotFound` never does —
        // it may be a soft delete, a move, or a resource this caller can no longer read (spec
        // §8) — so it falls through to the arm below with the file untouched. The erasure
        // licenses removing the file only when it names the resource this read addressed: the id
        // is parsed from the server's sentence, and a 410 naming any other resource is not about
        // this file — the error still propagates, the file stays.
        Err(err @ TemperError::ResourceErased(erased)) => {
            if erased == params.resource_id {
                remove_erased_local_copy(params.local_path);
            }
            Err(err)
        }
        Err(err) => Err(err),
    }
}

/// Remove the local projection of a resource the server reported erased, and say so.
///
/// The erasure itself still propagates to the caller as the error — this reports only the
/// local half, so a removal that fails is a warning beside that error, never a replacement for
/// it. An already-absent file is nothing to remove and nothing to say.
fn remove_erased_local_copy(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => output::warning(format!(
            "resource was erased: removed the local copy at {}",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => output::warning(format!(
            "resource was erased, but its local copy at {} could not be removed: {e}",
            path.display()
        )),
    }
}

/// Tier 1 check exposed standalone so callers can debounce without
/// spinning up the async runtime. `fetch` uses this internally too.
pub fn read_if_fresh(path: &Path, debounce: Duration) -> Result<Option<String>> {
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
    let meta = params
        .client
        .resources()
        .get(*params.resource_id.as_uuid(), None)
        .await
        .map_err(client_err_to_temper)?;

    let content = params
        .client
        .resources()
        .content(*params.resource_id.as_uuid())
        .await
        .map_err(client_err_to_temper)?;

    let file_content = reconstruct_full_file_content(&meta, &content)?;

    fs::write(params.local_path, &file_content)
        .map_err(|e| TemperError::Vault(format!("cache write: {e}")))?;
    Ok(ShowCacheResult {
        content: file_content,
        source: FreshnessTier::FullFetch,
    })
}

/// Reconstruct the full vault file (frontmatter + body) from a metadata
/// response and a content response.
///
/// `content.markdown` is body-only — the server returns frontmatter as
/// structured `managed_meta` / `open_meta` fields on the same response.
/// This function rebuilds the canonical on-disk form by combining:
/// - identity fields from the `ResourceView` (id, context, created, title, slug, owner)
/// - typed managed_meta fields from `content.managed_meta` — the closed
///   Property vocabulary (stage, mode, effort, status, seq, branch, pr,
///   llm-model, llm-run, provenance)
/// - free-form open_meta fields from `content.open_meta` (tags,
///   relationships, anything user-defined)
/// - `temper-updated` set from the server's authoritative timestamp so the
///   tier-2 hash check can byte-match on the next show.
///
/// The body is normalized so it always starts with a newline — guarantees
/// a blank line between the closing `---` fence and the first body line.
pub(super) fn reconstruct_full_file_content(
    meta: &ResourceView,
    content: &ContentResponse,
) -> Result<String> {
    use temper_workflow::frontmatter::{DocType, Frontmatter};

    let body = crate::actions::ingest::normalize_body_for_vault(&content.markdown);
    let dt = DocType::from_str(&meta.doc_type_name)?;
    let mut fm = Frontmatter::new(dt, body);

    // Resource-row identity fields. The resource row is the canonical
    // source for these — managed_meta from the server might also contain
    // them, but `set_managed_meta` will overwrite with row-equal values.
    fm.set_managed_field("temper-id", serde_json::Value::String(meta.id.to_string()));
    fm.set_managed_field(
        "temper-context",
        serde_json::Value::String(meta.home_display().unwrap_or_default().to_owned()),
    );
    fm.set_managed_field(
        "temper-created",
        serde_json::Value::String(meta.created.to_rfc3339()),
    );
    // Server's authoritative `updated` so the next tier-2 hash check can
    // byte-match without forcing another tier-3 fetch (and rewrite).
    fm.set_managed_field(
        "temper-updated",
        serde_json::Value::String(meta.updated.to_rfc3339()),
    );
    fm.set_managed_field(
        "temper-title",
        serde_json::Value::String(meta.title.clone()),
    );
    if !meta.owner_handle.is_empty() {
        fm.set_managed_field(
            "temper-owner",
            serde_json::Value::String(meta.owner_handle.clone()),
        );
    }

    // Typed managed_meta knows the canonical struct-field → `temper-*` key
    // mapping (e.g. `stage` → `temper-stage`). Going through this typed API
    // (rather than copying raw JSON keys) is what keeps the rebuilt file's
    // managed-tier keys consistent with what create/update emit, so a
    // subsequent `temper resource update --stage` can find and overwrite
    // the existing `temper-stage` entry instead of leaving a duplicate.
    if let Some(managed) = &content.managed_meta {
        fm.set_managed_meta(managed);
    }

    // Open-meta fields are free-form (tags, relationships, custom keys);
    // copy each entry as-is.
    if let Some(open) = content.open_meta.as_ref().and_then(|v| v.as_object()) {
        for (k, v) in open {
            fm.set_open_field(k, v.clone());
        }
    }

    fm.serialize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
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

    #[test]
    fn read_if_fresh_treats_future_mtime_as_fresh() {
        // Laptop clock skew / filesystem weirdness can produce mtimes in the
        // future. Treat them as fresh (age = 0) rather than as an error.
        let file = NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), "future").expect("write");
        let future = FileTime::from_system_time(SystemTime::now() + Duration::from_secs(120));
        set_file_mtime(file.path(), future).expect("set mtime");

        let result = read_if_fresh(file.path(), Duration::from_secs(30))
            .expect("read_if_fresh")
            .expect("future-mtime file should be treated as fresh");

        assert_eq!(result, "future");
    }

    // --- the remote tier's error ladder: erased drops the file, nothing else does ---

    /// A client whose every request is answered by `status` + `body`, through the in-process
    /// door — the server's half is a fixed answer, so the test is about what `fetch` does with it.
    fn client_answering(status: axum::http::StatusCode, body: String) -> TemperClient {
        let app = axum::Router::new().fallback(axum::routing::any(move || {
            let body = body.clone();
            std::future::ready((status, body))
        }));
        TemperClient::in_process_with_token(
            app,
            temper_workflow::operations::Surface::CliCloud,
            "tok".to_owned(),
            std::sync::Arc::new(temper_client::auth::MemoryTokenStore::empty()),
        )
        .expect("in-process client builds")
    }

    fn local_copy() -> NamedTempFile {
        let file = NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), "---\ntemper-id: x\n---\n\nlocal body\n").expect("write");
        file
    }

    /// The server says erased: the local copy is removed AND the erasure reaches the caller as
    /// the error — never a silent success, never the stale copy rendered. FAILS IF the erased
    /// arm in `fetch` is removed (the file survives).
    #[tokio::test]
    async fn an_erased_answer_removes_the_local_copy_and_reports_the_erasure() {
        let id = ResourceId::from(uuid::Uuid::now_v7());
        let body = serde_json::json!({"error": {
            "code": temper_core::error::RESOURCE_ERASED_CODE,
            "message": TemperError::ResourceErased(id).to_string(),
        }})
        .to_string();
        let client = client_answering(axum::http::StatusCode::GONE, body);
        let file = local_copy();

        let result = fetch(ShowCacheParams {
            client: &client,
            resource_id: id,
            local_path: file.path(),
            debounce: Duration::ZERO,
        })
        .await;

        assert!(
            matches!(result, Err(TemperError::ResourceErased(got)) if got == id),
            "got {result:?}"
        );
        assert!(
            !file.path().exists(),
            "the erased resource's local copy survived"
        );
    }

    /// An erasure naming a DIFFERENT resource than the one addressed is not about this file: the
    /// error still reaches the caller, and the local copy stays. FAILS IF the removal keys on the
    /// erased variant alone, without comparing its id to `params.resource_id` (the file is removed).
    #[tokio::test]
    async fn an_erased_answer_naming_another_resource_keeps_the_local_copy() {
        let addressed = ResourceId::from(uuid::Uuid::now_v7());
        let other = ResourceId::from(uuid::Uuid::now_v7());
        let body = serde_json::json!({"error": {
            "code": temper_core::error::RESOURCE_ERASED_CODE,
            "message": TemperError::ResourceErased(other).to_string(),
        }})
        .to_string();
        let client = client_answering(axum::http::StatusCode::GONE, body);
        let file = local_copy();

        let result = fetch(ShowCacheParams {
            client: &client,
            resource_id: addressed,
            local_path: file.path(),
            debounce: Duration::ZERO,
        })
        .await;

        assert!(
            matches!(result, Err(TemperError::ResourceErased(got)) if got == other),
            "the erasure still propagates: got {result:?}"
        );
        assert!(
            file.path().exists(),
            "an erasure naming another resource removed this one's local copy"
        );
    }

    /// A 404 is not an erasure — it may be a soft delete, a move, or an unreadable resource
    /// (spec §8) — so the file stays. FAILS IF the removal keys on "any 4xx" rather than the
    /// erased variant.
    #[tokio::test]
    async fn a_not_found_answer_keeps_the_local_copy() {
        let body = r#"{"error":{"code":"NOT_FOUND","message":"resource not found"}}"#.to_owned();
        let client = client_answering(axum::http::StatusCode::NOT_FOUND, body);
        let file = local_copy();

        let result = fetch(ShowCacheParams {
            client: &client,
            resource_id: ResourceId::from(uuid::Uuid::now_v7()),
            local_path: file.path(),
            debounce: Duration::ZERO,
        })
        .await;

        assert!(result.is_err(), "a 404 must not render as success");
        assert!(file.path().exists(), "a not-found removed the local copy");
    }

    /// A server that cannot be reached keeps today's offline fallback: the local copy renders,
    /// and is still there. FAILS IF the network arm is folded into the erased one.
    #[tokio::test]
    async fn a_network_error_keeps_and_renders_the_local_copy() {
        // A loopback port nothing listens on: bind, read the port, release it.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind loopback")
            .local_addr()
            .expect("local addr")
            .port();
        let client = TemperClient::with_token(
            &format!("http://127.0.0.1:{port}"),
            None,
            temper_workflow::operations::Surface::CliCloud,
            "tok".to_owned(),
            std::sync::Arc::new(temper_client::auth::MemoryTokenStore::empty()),
        )
        .expect("loopback http validates");
        let file = local_copy();

        let result = fetch(ShowCacheParams {
            client: &client,
            resource_id: ResourceId::from(uuid::Uuid::now_v7()),
            local_path: file.path(),
            debounce: Duration::ZERO,
        })
        .await
        .expect("an unreachable server falls back to the local copy");

        assert_eq!(result.source, FreshnessTier::OfflineFallback);
        assert!(result.content.contains("local body"));
        assert!(
            file.path().exists(),
            "a network error removed the local copy"
        );
    }

    fn test_resource_row() -> ResourceView {
        use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
        ResourceView {
            id: ResourceId(uuid::Uuid::nil()),
            r#ref: String::new(),
            kb_context_id: Some(ContextId(uuid::Uuid::nil())),
            origin_uri: "test://origin".to_string(),
            title: "Test Title".to_string(),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: Some("temper".to_string()),
            doc_type_name: "task".to_string(),
            owner_handle: "@me".to_string(),
            context_slug: Some("temper".to_string()),
            context_owner_ref: Some("@me".to_string()),
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            body_hash: None,
            ingest_state: Some(temper_core::types::resource::IngestState::Complete),
            ingest_ended: None,
            body_storage: Some(temper_core::types::resource::BodyStorage::Derived),
            managed_meta: temper_core::types::managed_meta::ManagedMeta::default(),
            open_meta: None,
            content: None,
            embedding_status: None,
        }
        .with_derived_refs()
    }

    /// Tier-3's regression test: the reconstructed file must include both
    /// the `---` frontmatter fences AND the body. The original bug wrote
    /// `content.markdown` only, dropping the entire frontmatter block.
    #[test]
    fn reconstruct_emits_frontmatter_fences_and_body() {
        use temper_core::types::ids::ResourceId;
        let meta = test_resource_row();
        let content = ContentResponse {
            resource_id: ResourceId(uuid::Uuid::nil()),
            markdown: "# Test Title\n\n## Section\n\nbody text\n".to_string(),
            managed_meta: None,
            open_meta: None,
        };

        let out = reconstruct_full_file_content(&meta, &content).expect("reconstruct");

        assert!(
            out.starts_with("---\n"),
            "must start with frontmatter fence; got:\n{out}"
        );
        assert!(
            out.contains("\n---\n"),
            "must close the frontmatter fence; got:\n{out}"
        );
        assert!(
            out.contains("temper-id:"),
            "must include temper-id; got:\n{out}"
        );
        assert!(
            out.contains("temper-context: temper"),
            "must include context; got:\n{out}"
        );
        assert!(
            out.contains("temper-type: task"),
            "must include doc type; got:\n{out}"
        );
        assert!(out.contains("# Test Title"), "must include H1; got:\n{out}");
        assert!(out.contains("## Section"), "must include H2; got:\n{out}");
    }

    /// Body normalization: when `content.markdown` does not start with a
    /// newline, the writer must inject one so that the H1 (or first line
    /// of body) is separated from the closing `---` by a blank line.
    #[test]
    fn reconstruct_separates_frontmatter_close_from_body_with_blank_line() {
        use temper_core::types::ids::ResourceId;
        let meta = test_resource_row();
        let content = ContentResponse {
            resource_id: ResourceId(uuid::Uuid::nil()),
            markdown: "# Title\n\n## Section\n".to_string(),
            managed_meta: None,
            open_meta: None,
        };

        let out = reconstruct_full_file_content(&meta, &content).expect("reconstruct");

        assert!(
            out.contains("---\n\n# Title"),
            "frontmatter close must be followed by blank line then H1; got:\n{out}"
        );
    }

    /// Managed-meta fields from the server response must round-trip into
    /// the rebuilt frontmatter (stage, mode, effort, etc.) — these are
    /// what `temper resource update --stage` operates on next.
    #[test]
    fn reconstruct_preserves_typed_managed_meta_fields() {
        use temper_core::types::ids::ResourceId;
        use temper_workflow::types::ManagedMeta;
        let meta = test_resource_row();
        let managed = ManagedMeta {
            stage: Some("in-progress".to_string()),
            mode: Some("build".to_string()),
            effort: Some("medium".to_string()),
            seq: Some(42),
            ..Default::default()
        };
        let content = ContentResponse {
            resource_id: ResourceId(uuid::Uuid::nil()),
            markdown: "# Body".to_string(),
            managed_meta: Some(managed),
            open_meta: None,
        };

        let out = reconstruct_full_file_content(&meta, &content).expect("reconstruct");

        assert!(
            out.contains("temper-stage: in-progress"),
            "stage missing:\n{out}"
        );
        assert!(out.contains("temper-mode: build"), "mode missing:\n{out}");
        assert!(
            out.contains("temper-effort: medium"),
            "effort missing:\n{out}"
        );
        assert!(out.contains("temper-seq: 42"), "seq missing:\n{out}");
    }

    /// Open-meta fields (tags, relationships, custom keys) must also
    /// round-trip — the open tier is the user's editable space.
    #[test]
    fn reconstruct_preserves_open_meta_fields() {
        use temper_core::types::ids::ResourceId;
        let meta = test_resource_row();
        let open = serde_json::json!({
            "tags": ["bug", "regression"],
            "relates_to": ["task-abc"],
        });
        let content = ContentResponse {
            resource_id: ResourceId(uuid::Uuid::nil()),
            markdown: "body".to_string(),
            managed_meta: None,
            open_meta: Some(open),
        };

        let out = reconstruct_full_file_content(&meta, &content).expect("reconstruct");

        assert!(out.contains("tags:"), "tags key missing:\n{out}");
        assert!(out.contains("- bug"), "tags entry missing:\n{out}");
        assert!(
            out.contains("relates_to:"),
            "relates_to key missing:\n{out}"
        );
        assert!(
            out.contains("- task-abc"),
            "relates_to entry missing:\n{out}"
        );
    }

    /// `temper-updated` must come from the server's authoritative
    /// timestamp so the next tier-2 hash check byte-matches without
    /// forcing another tier-3 fetch (and thus another file rewrite).
    #[test]
    fn reconstruct_sets_temper_updated_from_server_timestamp() {
        use chrono::TimeZone;
        use temper_core::types::ids::ResourceId;
        let mut meta = test_resource_row();
        let server_updated = chrono::Utc.with_ymd_and_hms(2026, 5, 3, 14, 30, 0).unwrap();
        meta.updated = server_updated;
        let content = ContentResponse {
            resource_id: ResourceId(uuid::Uuid::nil()),
            markdown: "body".to_string(),
            managed_meta: None,
            open_meta: None,
        };

        let out = reconstruct_full_file_content(&meta, &content).expect("reconstruct");

        // RFC3339 of the server timestamp; chrono emits "+00:00" suffix for Utc.
        // YAML emits this scalar unquoted because it isn't a YAML timestamp tag
        // value (the offset format isn't `Z`-suffixed), so match the bare form.
        assert!(
            out.contains("temper-updated: 2026-05-03T14:30:00+00:00"),
            "expected server-stamped temper-updated; got:\n{out}"
        );
    }
}
