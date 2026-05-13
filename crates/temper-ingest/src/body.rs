//! Shared body-trio computation for create/update resource flows.
//!
//! Both `temper-api` (server-side, `DbBackend::update_resource`) and
//! `temper-cli` (client-side, `VaultBackend::update_resource`) need to compute
//! `(content_hash, chunks_packed)` for a body when the caller didn't pre-compute
//! it. Lives here because the success path needs `temper_ingest::pipeline::prepare_markdown`,
//! which is not reachable from `temper-core` (would create a circular dep —
//! `temper-ingest` already depends on `temper-core`).
//!
//! Callers keep their own no-pipeline fallback (CLI's `embed` feature off, API's
//! `ingest-pipeline` feature off) because the higher-level feature gate is
//! caller-driven, not pipeline-driven.
//!
//! Gated alongside `pipeline` on `embed` / `embed-download` at the lib.rs level,
//! which is what makes `prepare_markdown` available.

use temper_core::error::TemperError;
use temper_core::hash::compute_body_hash;
use temper_core::types::ingest::pack_chunks;

/// Compute `(content_hash, chunks_packed)` for a body update.
///
/// Pipeline: SHA-256 of the body bytes, then `prepare_markdown` → `pack_chunks`.
/// Errors are surfaced as `TemperError::Api` for caller-side parity with the
/// previous per-crate copies.
pub fn compute_body_trio(body: &str) -> Result<(String, String), TemperError> {
    let hash = compute_body_hash(body);
    let packed_chunks = crate::pipeline::prepare_markdown(body)
        .map_err(|e| TemperError::Api(format!("embed: {e}")))?;
    let packed = pack_chunks(&packed_chunks).map_err(|e| TemperError::Api(format!("pack: {e}")))?;
    Ok((hash, packed))
}
