//! Typed reads of the production source (`public.*`) that synthesis regenerates from.
//!
//! These are **runtime** `sqlx::query` calls (not `query!` macros) on purpose: temper-next's macro
//! cache resolves against the `temper_next` search_path (`crates/temper-next/.sqlx`), so a compile-time
//! macro over `public.*` would conflict with that namespace. Runtime queries with explicit `public.`
//! qualification sidestep it entirely and never touch the offline cache.

use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// One active production resource joined to its manifest meta and doc-type name. This is the unit the
/// per-resource synthesis sequence (§0) iterates over.
#[derive(Debug, Clone)]
pub struct SourceResource {
    pub id: Uuid,
    pub title: String,
    pub origin_uri: String,
    pub slug: Option<String>,
    pub kb_context_id: Uuid,
    pub doc_type: String,
    pub originator_profile_id: Uuid,
    pub owner_profile_id: Uuid,
    pub managed_meta: serde_json::Value,
    pub open_meta: serde_json::Value,
}

/// All **active** resources (`is_active`), each joined to its manifest (`managed_meta`/`open_meta`)
/// and doc-type name. Soft-deleted resources are excluded (§0: synthesis covers active state only).
pub async fn active_resources(pool: &PgPool) -> Result<Vec<SourceResource>> {
    let rows = sqlx::query(
        "SELECT r.id, r.title, r.origin_uri, r.slug, r.kb_context_id, dt.name AS doc_type, \
                r.originator_profile_id, r.owner_profile_id, m.managed_meta, m.open_meta \
         FROM public.kb_resources r \
         JOIN public.kb_resource_manifests m ON m.resource_id = r.id \
         JOIN public.kb_doc_types dt ON dt.id = r.kb_doc_type_id \
         WHERE r.is_active \
         ORDER BY r.created",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| SourceResource {
            id: row.get("id"),
            title: row.get("title"),
            origin_uri: row.get("origin_uri"),
            slug: row.get("slug"),
            kb_context_id: row.get("kb_context_id"),
            doc_type: row.get("doc_type"),
            originator_profile_id: row.get("originator_profile_id"),
            owner_profile_id: row.get("owner_profile_id"),
            managed_meta: row.get("managed_meta"),
            open_meta: row.get("open_meta"),
        })
        .collect())
}

/// One production profile (the human/principal behind owned + originated resources). `slug` is
/// `NOT NULL UNIQUE` in production (migration `20260407000002`), but read as `Option` so a defensive
/// caller can fall back to a sluggified `display_name` if it is ever absent.
#[derive(Debug, Clone)]
pub struct SourceProfile {
    pub id: Uuid,
    pub display_name: String,
    pub slug: Option<String>,
}

/// The production profiles for the given ids (the distinct originator ∪ owner set across active
/// resources). Bootstrap turns each into a `temper_next.kb_profiles` row (§1/§2).
pub async fn profiles(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<SourceProfile>> {
    let rows = sqlx::query(
        "SELECT id, display_name, slug FROM public.kb_profiles WHERE id = ANY($1) ORDER BY created",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| SourceProfile {
            id: row.get("id"),
            display_name: row.get("display_name"),
            slug: row.get("slug"),
        })
        .collect())
}

/// One current chunk of a resource, carrying the heading metadata and embedding verbatim (§8: chunks,
/// sha256 content hashes, and bge-768 embeddings carry as-is).
#[derive(Debug, Clone)]
pub struct SourceChunk {
    pub chunk_index: i32,
    pub content_hash: String,
    pub content: String,
    pub header_path: String,
    pub heading_depth: i16,
    pub embedding: Vec<f32>,
}

/// Current chunks of `resource` in `chunk_index` order, via the `public.kb_current_chunks` view (the
/// same view production's `get_content` reads). The pgvector `embedding` is read as text and parsed.
pub async fn chunks_for(pool: &PgPool, resource: Uuid) -> Result<Vec<SourceChunk>> {
    let rows = sqlx::query(
        "SELECT chunk_index, content_hash, content, header_path, heading_depth, \
                embedding::text AS embedding \
         FROM public.kb_current_chunks \
         WHERE resource_id = $1 \
         ORDER BY chunk_index",
    )
    .bind(resource)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let emb_text: String = row.get("embedding");
            Ok(SourceChunk {
                chunk_index: row.get("chunk_index"),
                content_hash: row.get("content_hash"),
                content: row.get("content"),
                header_path: row.get("header_path"),
                heading_depth: row.get("heading_depth"),
                embedding: parse_pgvector(&emb_text)?,
            })
        })
        .collect()
}

/// One production edge whose endpoints are both active resources. `edge_kind`/`polarity` read as text.
#[derive(Debug, Clone)]
pub struct SourceEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
    pub edge_kind: String,
    pub polarity: String,
    pub label: String,
    pub weight: f64,
    pub is_folded: bool,
}

/// All edges from `public.kb_resource_edges` whose source and target are both active resources.
/// Kind, polarity, label, weight, and the folded flag carry verbatim (§4).
pub async fn edges(pool: &PgPool) -> Result<Vec<SourceEdge>> {
    let rows = sqlx::query(
        "SELECT e.id, e.source_resource_id, e.target_resource_id, e.edge_kind::text AS edge_kind, \
                e.polarity::text AS polarity, e.label, e.weight, e.is_folded \
         FROM public.kb_resource_edges e \
         JOIN public.kb_resources s ON s.id = e.source_resource_id AND s.is_active \
         JOIN public.kb_resources t ON t.id = e.target_resource_id AND t.is_active \
         ORDER BY e.created",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| SourceEdge {
            id: row.get("id"),
            source: row.get("source_resource_id"),
            target: row.get("target_resource_id"),
            edge_kind: row.get("edge_kind"),
            polarity: row.get("polarity"),
            label: row.get("label"),
            weight: row.get("weight"),
            is_folded: row.get("is_folded"),
        })
        .collect())
}

/// One production context (id + owner + name). Contexts migrate with their owner carried verbatim
/// into the owner-scoped destination `kb_contexts` (§2 amendment 2026-06-13); `slug = slugify(name)`.
#[derive(Debug, Clone)]
pub struct SourceContext {
    pub id: Uuid,
    pub owner_table: String,
    pub owner_id: Uuid,
    pub name: String,
}

/// All production contexts.
pub async fn contexts(pool: &PgPool) -> Result<Vec<SourceContext>> {
    let rows = sqlx::query(
        "SELECT id, kb_owner_table, kb_owner_id, name FROM public.kb_contexts ORDER BY created",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| SourceContext {
            id: row.get("id"),
            owner_table: row.get("kb_owner_table"),
            owner_id: row.get("kb_owner_id"),
            name: row.get("name"),
        })
        .collect())
}

/// Parse pgvector's text rendering (`[a,b,c]`) into a `Vec<f32>`.
fn parse_pgvector(text: &str) -> Result<Vec<f32>> {
    let inner = text
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| anyhow::anyhow!("malformed pgvector text: {text:?}"))?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|n| {
            n.trim()
                .parse::<f32>()
                .map_err(|e| anyhow::anyhow!("bad vector component {n:?}: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_pgvector;

    #[test]
    fn parses_pgvector_text() {
        assert_eq!(parse_pgvector("[1,2,3]").unwrap(), vec![1.0, 2.0, 3.0]);
        assert_eq!(
            parse_pgvector("[0.5, -0.25, 0]").unwrap(),
            vec![0.5, -0.25, 0.0]
        );
        assert_eq!(parse_pgvector("[]").unwrap(), Vec::<f32>::new());
        assert!(parse_pgvector("1,2,3").is_err());
    }
}
