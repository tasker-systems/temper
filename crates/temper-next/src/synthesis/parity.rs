//! Per-resource body-text parity gate (WS6 §8).
//!
//! Spec §8 (verbatim): *"recomputing the body text from synthesized blocks/chunks must reproduce the
//! same content the production read path serves today, per resource, before cutover proceeds."*
//!
//! The gate reconstructs each synthesized resource's body two ways and compares the resulting **TEXT**
//! with string equality:
//!   * **production** — over `public.kb_current_chunks` (the exact source + order production's
//!     `get_content` reads), and
//!   * **new substrate** — over `temper_next.kb_chunks WHERE is_current` joined to `kb_chunk_content`,
//!     ordered by the block `seq` then `chunk_index`.
//!
//! It NEVER compares the two `body_hash` columns: the destination `body_hash` is a structural sha256
//! merkle over chunk content-hashes (`_recompute_resource_body_hash`), while production's `body_hash`
//! is `sha256:<hex>` over the assembled markdown — different values **by construction**. Only the
//! reconstructed text is comparable.
//!
//! [`reconstruct_body`] is a verbatim port of production `get_content`'s markdown assembly
//! (`crates/temper-api/src/services/resource_service.rs:473-494`). Resources are matched
//! production↔synthesized by `origin_uri` (carried verbatim, UNIQUE in both schemas).
//!
//! All reads are schema-qualified runtime `sqlx::query` (not `query!` macros) so they work regardless
//! of the connection's `search_path` and never touch the `temper_next` offline cache. The parity reads
//! fire no SQL functions or triggers, so qualifying is sufficient.

use anyhow::Result;
use sqlx::{PgPool, Row};

/// One chunk as the body reconstruction sees it: ordering index, heading breadcrumb, heading level, and
/// prose. Mirrors the `ContentChunk` shape production's `get_content` selects from `kb_current_chunks`.
#[derive(Debug, Clone)]
pub struct ReadChunk {
    pub chunk_index: i32,
    pub header_path: String,
    pub heading_depth: i16,
    pub content: String,
}

/// Verbatim port of production `get_content`'s markdown assembly
/// (`crates/temper-api/src/services/resource_service.rs:473-494`):
///
/// per chunk (ordered by `chunk_index`): `heading_depth == 0` ⇒ content as-is; else the innermost
/// breadcrumb segment becomes a markdown heading (`{hashes} {title}\n\n{content}`, depth capped at 6,
/// empty breadcrumb ⇒ `"Untitled"`). Pieces join with `"\n\n"`.
pub fn reconstruct_body(chunks: &[ReadChunk]) -> String {
    chunks
        .iter()
        .map(|c| {
            if c.heading_depth == 0 {
                // Preamble or unheaded content — emit body only.
                c.content.clone()
            } else {
                // Extract the innermost heading title from the breadcrumb.
                // rsplit always yields at least one element on non-empty input.
                let title = if c.header_path.is_empty() {
                    "Untitled"
                } else {
                    c.header_path.rsplit(" > ").next().unwrap_or(&c.header_path)
                };
                let depth = (c.heading_depth as usize).min(6);
                let hashes = "#".repeat(depth);
                format!("{hashes} {title}\n\n{}", c.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One resource whose production and new-substrate reconstructed bodies differ. Carries both bodies so
/// a caller can diagnose the divergence (the gate refuses cutover, so a non-empty list is fatal).
#[derive(Debug, Clone)]
pub struct BodyMismatch {
    pub origin_uri: String,
    pub production_body: String,
    pub new_body: String,
}

/// Outcome of the §8 parity sweep. `checked` is the number of synthesized resources compared (so a
/// clean report can be asserted non-vacuous); `mismatches` is empty iff every body matched.
#[derive(Debug, Clone, Default)]
pub struct ParityReport {
    pub checked: usize,
    pub mismatches: Vec<BodyMismatch>,
}

impl ParityReport {
    /// True iff every reconstructed body matched production (no resource diverged).
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// The origin_uris that diverged — the per-resource list §8 says blocks cutover.
    pub fn mismatched_uris(&self) -> Vec<&str> {
        self.mismatches
            .iter()
            .map(|m| m.origin_uri.as_str())
            .collect()
    }
}

/// Compare, per synthesized resource, the body reconstructed from `temper_next` against the body the
/// production read path serves today (`public.kb_current_chunks`). Resources are matched by `origin_uri`
/// (carried verbatim, UNIQUE in both schemas). Returns the per-resource mismatch list (§8).
pub async fn body_parity_report(pool: &PgPool) -> Result<ParityReport> {
    // Every synthesized resource is in scope (synthesis covers active state only, §0). Ordered for a
    // stable report.
    let uris: Vec<String> =
        sqlx::query("SELECT origin_uri FROM temper_next.kb_resources ORDER BY origin_uri")
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| r.get::<String, _>("origin_uri"))
            .collect();

    let mut report = ParityReport {
        checked: uris.len(),
        mismatches: Vec::new(),
    };

    for uri in &uris {
        let production_body = reconstruct_body(&production_chunks(pool, uri).await?);
        let new_body = reconstruct_body(&new_substrate_chunks(pool, uri).await?);
        if production_body != new_body {
            report.mismatches.push(BodyMismatch {
                origin_uri: uri.clone(),
                production_body,
                new_body,
            });
        }
    }

    Ok(report)
}

/// The production read source for a resource's body: `public.kb_current_chunks` (the same view + order
/// `get_content` uses), matched to the resource by `origin_uri`.
async fn production_chunks(pool: &PgPool, origin_uri: &str) -> Result<Vec<ReadChunk>> {
    let rows = sqlx::query(
        "SELECT cc.chunk_index, cc.header_path, cc.heading_depth, cc.content \
         FROM public.kb_current_chunks cc \
         JOIN public.kb_resources r ON r.id = cc.resource_id \
         WHERE r.origin_uri = $1 \
         ORDER BY cc.chunk_index",
    )
    .bind(origin_uri)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(read_chunk).collect())
}

/// The new-substrate read source: `temper_next.kb_chunks WHERE is_current` joined to `kb_chunk_content`,
/// ordered by the block `seq` then `chunk_index` (a single block per resource at migration, so this is
/// effectively `chunk_index`). `header_path`/`heading_depth` are nullable in the destination but carry
/// production's NOT-NULL `''`/`0` defaults verbatim; coalescing keeps reconstruction well-defined.
///
/// Also the read-surface chunk reader for [`crate::readback::body`] (WS6 §9): the §8 cutover gate and
/// the §9 body read share this one reader so they exercise the same chunk source + order (SG-3).
pub async fn new_substrate_chunks(pool: &PgPool, origin_uri: &str) -> Result<Vec<ReadChunk>> {
    let rows = sqlx::query(
        "SELECT c.chunk_index, COALESCE(c.header_path, '') AS header_path, \
                COALESCE(c.heading_depth, 0::smallint) AS heading_depth, cc.content \
         FROM temper_next.kb_chunks c \
         JOIN temper_next.kb_content_blocks b ON b.id = c.block_id \
         JOIN temper_next.kb_chunk_content cc ON cc.chunk_id = c.id \
         JOIN temper_next.kb_resources r ON r.id = c.resource_id \
         WHERE r.origin_uri = $1 AND c.is_current \
         ORDER BY b.seq, c.chunk_index",
    )
    .bind(origin_uri)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(read_chunk).collect())
}

fn read_chunk(row: &sqlx::postgres::PgRow) -> ReadChunk {
    ReadChunk {
        chunk_index: row.get("chunk_index"),
        header_path: row.get("header_path"),
        heading_depth: row.get("heading_depth"),
        content: row.get("content"),
    }
}

#[cfg(test)]
mod tests {
    use super::{reconstruct_body, ReadChunk};

    fn chunk(idx: i32, header_path: &str, depth: i16, content: &str) -> ReadChunk {
        ReadChunk {
            chunk_index: idx,
            header_path: header_path.to_owned(),
            heading_depth: depth,
            content: content.to_owned(),
        }
    }

    #[test]
    fn unheaded_chunk_emits_content_only() {
        assert_eq!(
            reconstruct_body(&[chunk(0, "", 0, "Just prose.")]),
            "Just prose."
        );
    }

    #[test]
    fn headed_chunk_uses_innermost_breadcrumb_segment() {
        assert_eq!(
            reconstruct_body(&[chunk(0, "Intro > Goals", 2, "Body.")]),
            "## Goals\n\nBody."
        );
    }

    #[test]
    fn mixed_chunks_join_with_blank_line() {
        // Mirrors the fixture's R2 (task-doc): a preamble chunk + a depth-2 headed chunk.
        assert_eq!(
            reconstruct_body(&[
                chunk(0, "", 0, "Task intro paragraph."),
                chunk(1, "Intro > Goals", 2, "Task goals section body."),
            ]),
            "Task intro paragraph.\n\n## Goals\n\nTask goals section body."
        );
    }

    #[test]
    fn empty_breadcrumb_with_depth_falls_back_to_untitled_and_caps_at_six() {
        assert_eq!(
            reconstruct_body(&[chunk(0, "", 9, "x")]),
            "###### Untitled\n\nx"
        );
    }
}
