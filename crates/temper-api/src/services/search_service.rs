//! Search service — routes queries to the `unified_search()` SQL function,
//! combining full-text (tsvector) and vector (pgvector) search.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};

const MAX_LIMIT: i64 = 50;
const DEFAULT_LIMIT: i64 = 10;
const EMBEDDING_DIM: usize = 768;

/// Validate search params. Returns the sanitized limit.
pub fn validate_params(params: &SearchParams) -> ApiResult<i64> {
    let has_query = params.query.as_ref().is_some_and(|q| !q.trim().is_empty());
    let has_embedding = params.embedding.is_some();
    let has_seeds = params.seed_ids.as_ref().is_some_and(|s| !s.is_empty());

    if !has_query && !has_embedding && !has_seeds {
        return Err(ApiError::BadRequest(
            "at least one of 'query', 'embedding', or 'seed_ids' must be provided".into(),
        ));
    }

    if let Some(ref emb) = params.embedding {
        if emb.len() != EMBEDDING_DIM {
            return Err(ApiError::BadRequest(format!(
                "embedding must be {EMBEDDING_DIM} dimensions, got {}",
                emb.len()
            )));
        }
    }

    Ok(params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT))
}

/// Compute FTS/vector weights based on which inputs are provided.
pub fn compute_weights(query: &Option<String>, embedding: &Option<Vec<f32>>) -> (f64, f64) {
    let has_query = query.as_ref().is_some_and(|q| !q.trim().is_empty());
    match (has_query, embedding.is_some()) {
        (true, true) => (0.5, 0.5),
        (true, false) => (1.0, 0.0),
        (false, true) => (0.0, 1.0),
        (false, false) => (0.0, 0.0),
    }
}

const DEFAULT_GRAPH_DEPTH: i32 = 2;
const MAX_GRAPH_DEPTH: i32 = 10;

/// Execute search — routes to graph-enhanced or unified based on `graph_expand`.
pub async fn search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    if params.graph_expand {
        graph_search_impl(pool, profile_id, params).await
    } else {
        unified_search(pool, profile_id, params).await
    }
}

/// Execute the unified search (FTS + optional vector) without graph expansion.
async fn unified_search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    let limit = validate_params(&params)?;
    let offset = params.offset.unwrap_or(0);
    let (fts_weight, vec_weight) = compute_weights(&params.query, &params.embedding);

    let embedding_str = params
        .embedding
        .as_ref()
        .map(|e| temper_core::types::ingest::format_embedding(e));

    // NOTE: Uses runtime query_as — pgvector ::vector cast not supported by sqlx macro
    let rows = sqlx::query_as::<_, UnifiedSearchResultRow>(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM unified_search($1, $2, $3::vector, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(profile_id)
    .bind(params.query.as_deref().unwrap_or(""))
    .bind(embedding_str.as_deref())
    .bind(&params.search_config)
    .bind(params.context_name.as_deref())
    .bind(params.doc_type.as_deref())
    .bind(fts_weight)
    .bind(vec_weight)
    .bind(limit as i32)
    .bind(offset as i32)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Execute graph-enhanced search (unified_search + graph_traverse).
async fn graph_search_impl(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    let limit = validate_params(&params)?;
    let offset = params.offset.unwrap_or(0);
    let (fts_weight, vec_weight) = compute_weights(&params.query, &params.embedding);

    let embedding_str = params
        .embedding
        .as_ref()
        .map(|e| temper_core::types::ingest::format_embedding(e));

    let seed_ids: Vec<Uuid> = params.seed_ids.unwrap_or_default();
    let edge_types: Vec<String> = params.edge_types.unwrap_or_default();
    let graph_depth = params
        .graph_depth
        .unwrap_or(DEFAULT_GRAPH_DEPTH)
        .min(MAX_GRAPH_DEPTH);

    let rows = sqlx::query_as::<_, UnifiedSearchResultRow>(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM graph_search($1, $2, $3::vector, $4, $5, $6, $7, $8, $9, $10, $11, 0.3, $12, $13)
        "#,
    )
    .bind(profile_id)
    .bind(params.query.as_deref().unwrap_or(""))
    .bind(embedding_str.as_deref())
    .bind(&params.search_config)
    .bind(params.context_name.as_deref())
    .bind(params.doc_type.as_deref())
    .bind(fts_weight)
    .bind(vec_weight)
    .bind(&seed_ids)
    .bind(&edge_types)
    .bind(graph_depth)
    .bind(limit as i32)
    .bind(offset as i32)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_neither_query_nor_embedding() {
        let params = SearchParams {
            query: None,
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn validate_accepts_query_only() {
        let params = SearchParams {
            query: Some("test".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert_eq!(validate_params(&params).unwrap(), DEFAULT_LIMIT);
    }

    #[test]
    fn validate_accepts_embedding_only() {
        let params = SearchParams {
            query: None,
            embedding: Some(vec![0.0; 768]),
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert!(validate_params(&params).is_ok());
    }

    #[test]
    fn validate_rejects_wrong_dimension() {
        let params = SearchParams {
            query: None,
            embedding: Some(vec![0.0; 100]),
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn validate_clamps_limit() {
        let params = SearchParams {
            query: Some("test".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: Some(200),
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert_eq!(validate_params(&params).unwrap(), MAX_LIMIT);
    }

    #[test]
    fn validate_accepts_seed_ids_only() {
        let params = SearchParams {
            query: None,
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: Some(vec![uuid::Uuid::nil()]),
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert!(validate_params(&params).is_ok());
    }

    #[test]
    fn validate_rejects_empty_query_with_no_embedding() {
        let params = SearchParams {
            query: Some("".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn compute_weights_query_only() {
        let (fts, vec) = compute_weights(&Some("test".into()), &None);
        assert_eq!(fts, 1.0);
        assert_eq!(vec, 0.0);
    }

    #[test]
    fn compute_weights_embedding_only() {
        let (fts, vec) = compute_weights(&None, &Some(vec![0.0; 768]));
        assert_eq!(fts, 0.0);
        assert_eq!(vec, 1.0);
    }

    #[test]
    fn compute_weights_both() {
        let (fts, vec) = compute_weights(&Some("q".into()), &Some(vec![0.0; 768]));
        assert_eq!(fts, 0.5);
        assert_eq!(vec, 0.5);
    }
}
