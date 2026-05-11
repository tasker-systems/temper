//! Pure cmd → vault-flow translators (no I/O).

use temper_core::error::TemperError;
#[cfg(feature = "embed")]
use temper_core::hash::compute_body_hash;

/// Pre-computed body trio: SHA-256 content hash + packed chunks.
/// Mirrors the trio rule from `resource_service::update`: when a body
/// update is present, all three of (content, content_hash, chunks_packed)
/// must be supplied together.
///
/// Callers land in Task 4 (`cmd_to_update_request`) and Tasks 7-8 (create/update).
/// Remove the `dead_code` suppressions when those tasks land.
#[expect(
    dead_code,
    reason = "callers land in Tasks 4/7/8 (Phase 4a); scaffolded now \
              so the type is in place for Task 4's cmd_to_update_request"
)]
#[derive(Debug, Clone)]
pub(crate) struct BodyTrio {
    pub content_hash: String,
    pub chunks_packed: String,
}

/// Compute (content_hash, chunks_packed) for a body update.
///
/// **Duplicated from `temper-api/src/backend/translators.rs::prepare_body_trio`.**
/// Lift to `temper-core::operations::body` deferred to a focused cleanup
/// (vault task `lift-prepare-body-trio-to-temper-core-shared-helper`) because
/// it requires adding `temper-ingest` as an optional dep of `temper-core`,
/// which is a structural feature-graph change outside Phase 4a's scope.
///
/// In `temper-cli`, the relevant feature gate is `embed` (mirrors
/// `ingest-pipeline` in `temper-api`): the `embed` feature wires
/// `temper-ingest/embed-download` which provides `pipeline::prepare_markdown`.
#[cfg(feature = "embed")]
#[expect(
    dead_code,
    reason = "callers land in Tasks 7-8 (Phase 4a create/update body path); \
              remove suppression when Task 7 lands"
)]
pub(crate) fn prepare_body_trio(body: &str) -> Result<BodyTrio, TemperError> {
    let content_hash = compute_body_hash(body);
    let packed_chunks = temper_ingest::pipeline::prepare_markdown(body)
        .map_err(|e| TemperError::Api(format!("embed: {e}")))?;
    let chunks_packed = temper_core::types::ingest::pack_chunks(&packed_chunks)
        .map_err(|e| TemperError::Api(format!("pack: {e}")))?;
    Ok(BodyTrio {
        content_hash,
        chunks_packed,
    })
}

#[cfg(not(feature = "embed"))]
pub(crate) fn prepare_body_trio(_body: &str) -> Result<BodyTrio, TemperError> {
    Err(TemperError::BadRequest(
        "chunks_packed required when embed pipeline is not available".to_owned(),
    ))
}

// Only one test exists here and it's gated on not(embed), so the whole
// test module is guarded to avoid an unused-import warning under --all-features.
#[cfg(all(test, not(feature = "embed")))]
mod tests {
    use super::*;

    #[test]
    fn prepare_body_trio_no_embed_returns_bad_request() {
        let err = prepare_body_trio("body").expect_err("no-embed path");
        assert!(matches!(
            err,
            temper_core::error::TemperError::BadRequest(_)
        ));
    }
}
