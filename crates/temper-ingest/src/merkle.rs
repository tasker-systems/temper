//! Block/resource merkle hashing for streaming (segmented) ingest.
//!
//! Mirrors two SQL-computed hashes so the CLI's locally-computed segment hashes match what
//! the server reports back, enabling an apples-to-apples resume diff
//! (`temper-cli`'s `actions::ingest_manifest::resume_gap`):
//!
//! - [`block_merkle`] mirrors `_project_blocks`'s `kb_block_revisions.block_body_hash`
//!   computation (`migrations/20260704000003_block_provenance_write_path.sql`) — the value
//!   the wire type `temper_core::types::ingest::SegmentInfo.content_hash` reports for a
//!   landed segment.
//! - [`resource_body_hash`] mirrors `_recompute_resource_body_hash`'s `kb_resources.body_hash`
//!   computation (`migrations/20260624000002_canonical_functions.sql`) — the value a
//!   segmented ingest's `FinalizePayload.expected_body_hash` must match.
//!
//! Both are duplicated here rather than reusing `temper_substrate::content`'s
//! `body_hash_from_chunk_hashes`/`body_hash_from_block_chunk_hashes` twins, because
//! `temper-cli` deliberately never depends on `temper-substrate` (the persistence-layer
//! crate) — see the crate architecture north star (`cli` → `client`/`workflow`/`ingest`
//! only). Ground truth for both formulas below is the live SQL, not
//! `temper_core::types::ingest::SegmentInfo`'s doc comment, which describes the per-block
//! hash as `body_hash_from_chunk_hashes` (a *double* sha256): the actual
//! `_project_blocks`/`block_append` SQL computes a *single* sha256 over the concatenated
//! chunk hashes for `block_body_hash` — verified by reading the migration directly rather
//! than trusting the prior doc-comment's phrasing.

use crate::chunk::sha256_hex;

/// The per-block merkle the server stores as `kb_block_revisions.block_body_hash` and
/// reports back as `SegmentInfo.content_hash` — sha256 hex over the concatenation of the
/// block's chunk `content_hash`es, in `chunk_index` order. `chunk_content_hashes` MUST
/// already be in that order.
pub fn block_merkle(chunk_content_hashes: &[String]) -> String {
    sha256_hex(&chunk_content_hashes.concat())
}

/// The resource-level `kb_resources.body_hash` the server validates at finalize — sha256 hex
/// over the concatenation of each block's [`block_merkle`], in `seq` order. `block_merkles`
/// MUST already be in that order.
pub fn resource_body_hash(block_merkles: &[String]) -> String {
    sha256_hex(&block_merkles.concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn manual_sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn block_merkle_is_single_sha256_over_concatenated_chunk_hashes() {
        let hashes = vec!["aaa".to_string(), "bbb".to_string()];
        assert_eq!(block_merkle(&hashes), manual_sha256_hex(b"aaabbb"));
    }

    #[test]
    fn resource_body_hash_is_single_sha256_over_concatenated_block_merkles() {
        let merkles = vec!["h1".to_string(), "h2".to_string(), "h3".to_string()];
        assert_eq!(resource_body_hash(&merkles), manual_sha256_hex(b"h1h2h3"));
    }

    #[test]
    fn empty_inputs_hash_the_empty_string() {
        let expected = manual_sha256_hex(b"");
        assert_eq!(block_merkle(&[]), expected);
        assert_eq!(resource_body_hash(&[]), expected);
    }

    #[test]
    fn single_block_resource_hash_double_hashes_the_chunk_concat() {
        // For exactly one block, resource_body_hash([block_merkle(chunks)]) applies sha256
        // TWICE over the chunk-hash concatenation — the "two-level merkle" the SQL comments
        // describe for the create (one-shot) path, and the same value
        // `temper_substrate::content::body_hash_from_chunk_hashes` computes for its
        // single-block case.
        let chunk_hashes = vec!["abc".to_string(), "def".to_string()];
        let block = block_merkle(&chunk_hashes);
        let resource = resource_body_hash(std::slice::from_ref(&block));

        let inner = manual_sha256_hex(b"abcdef");
        let outer = manual_sha256_hex(inner.as_bytes());
        assert_eq!(resource, outer);
    }

    #[test]
    fn block_merkle_is_order_sensitive() {
        let a = block_merkle(&["x".to_string(), "y".to_string()]);
        let b = block_merkle(&["y".to_string(), "x".to_string()]);
        assert_ne!(
            a, b,
            "chunk_index order must matter — a re-order is a different merkle"
        );
    }
}
