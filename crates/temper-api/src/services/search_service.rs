use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub use temper_core::types::api::{SearchParams, SearchResultRow};

const MAX_LIMIT: i64 = 50;
const DEFAULT_LIMIT: i64 = 10;
const EMBEDDING_DIM: usize = 768;

/// Validate search params. Returns the sanitized limit.
pub fn validate_params(params: &SearchParams) -> ApiResult<i64> {
    if params.embedding.len() != EMBEDDING_DIM {
        return Err(ApiError::BadRequest(format!(
            "embedding must be {EMBEDDING_DIM} dimensions, got {}",
            params.embedding.len()
        )));
    }
    Ok(params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT))
}

/// Format an embedding vector as a pgvector literal string: `[0.1,0.2,...]`
pub fn format_embedding(embedding: &[f32]) -> String {
    format!(
        "[{}]",
        embedding
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Build the WHERE clause fragments and corresponding bind values for optional filters.
///
/// Returns (where_clause, context_bind, doc_type_bind) where where_clause is
/// appended to the base query. The bind values are Options that the caller
/// binds in order — only non-None values produce parameter placeholders.
pub fn build_filter_clause(
    context: Option<Uuid>,
    doc_type: Option<&str>,
    next_param: &mut i32,
) -> (String, Option<Uuid>, Option<String>) {
    let mut clause = String::new();
    let mut ctx_bind = None;
    let mut dt_bind = None;

    if let Some(ctx) = context {
        clause.push_str(&format!(" AND r.kb_context_id = ${next_param}"));
        *next_param += 1;
        ctx_bind = Some(ctx);
    }
    if let Some(dt) = doc_type {
        clause.push_str(&format!(" AND dt.name = ${next_param}"));
        *next_param += 1;
        dt_bind = Some(dt.to_string());
    }

    (clause, ctx_bind, dt_bind)
}

/// Execute the vector similarity search query.
pub async fn search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<SearchResultRow>> {
    let limit = validate_params(&params)?;
    let embedding_str = format_embedding(&params.embedding);

    // Parameter slots: $1 = embedding, $2 = profile_id, then optional filters, then limit.
    let mut next_param: i32 = 3;
    let (filter_clause, ctx_bind, dt_bind) =
        build_filter_clause(params.context, params.doc_type.as_deref(), &mut next_param);
    let limit_param = next_param;

    let sql = format!(
        "SELECT r.id AS resource_id, r.title, \
         kb_resource_uri(r.id) AS kb_uri, r.origin_uri, \
         ctx.name AS context, dt.name AS doc_type, \
         c.content AS snippet, c.header_path, \
         (1 - (c.embedding <=> $1::vector))::real AS score \
         FROM kb_current_chunks c \
         JOIN kb_resources r ON c.resource_id = r.id \
         LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id \
         JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id \
         WHERE r.id IN (SELECT resource_id FROM resources_visible_to($2)) \
         {filter_clause} \
         ORDER BY c.embedding <=> $1::vector LIMIT ${limit_param}"
    );

    // Build the query with dynamic binds.
    let mut query = sqlx::query_as::<_, SearchResultRow>(&sql)
        .bind(&embedding_str)
        .bind(profile_id);

    if let Some(ctx) = ctx_bind {
        query = query.bind(ctx);
    }
    if let Some(dt) = dt_bind {
        query = query.bind(dt);
    }

    let rows = query.bind(limit).fetch_all(pool).await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_params_wrong_dimension() {
        let params = SearchParams {
            embedding: vec![0.0; 100],
            context: None,
            doc_type: None,
            limit: None,
        };
        let result = validate_params(&params);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_params_correct_dimension() {
        let params = SearchParams {
            embedding: vec![0.0; 768],
            context: None,
            doc_type: None,
            limit: None,
        };
        let result = validate_params(&params);
        assert_eq!(result.unwrap(), 10); // default limit
    }

    #[test]
    fn test_validate_params_clamps_limit() {
        let params = SearchParams {
            embedding: vec![0.0; 768],
            context: None,
            doc_type: None,
            limit: Some(200),
        };
        assert_eq!(validate_params(&params).unwrap(), 50);
    }

    #[test]
    fn test_format_embedding() {
        let embedding = vec![0.1, 0.2, 0.3];
        let result = format_embedding(&embedding);
        assert_eq!(result, "[0.1,0.2,0.3]");
    }

    #[test]
    fn test_format_embedding_empty() {
        let result = format_embedding(&[]);
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_build_filter_clause_no_filters() {
        let mut next = 3;
        let (clause, ctx, dt) = build_filter_clause(None, None, &mut next);
        assert_eq!(clause, "");
        assert!(ctx.is_none());
        assert!(dt.is_none());
        assert_eq!(next, 3);
    }

    #[test]
    fn test_build_filter_clause_context_only() {
        let mut next = 3;
        let id = Uuid::nil();
        let (clause, ctx, dt) = build_filter_clause(Some(id), None, &mut next);
        assert_eq!(clause, " AND r.kb_context_id = $3");
        assert_eq!(ctx, Some(id));
        assert!(dt.is_none());
        assert_eq!(next, 4);
    }

    #[test]
    fn test_build_filter_clause_both_filters() {
        let mut next = 3;
        let id = Uuid::nil();
        let (clause, ctx, dt) = build_filter_clause(Some(id), Some("task"), &mut next);
        assert_eq!(clause, " AND r.kb_context_id = $3 AND dt.name = $4");
        assert_eq!(ctx, Some(id));
        assert_eq!(dt.as_deref(), Some("task"));
        assert_eq!(next, 5);
    }

    #[test]
    fn test_build_filter_clause_doc_type_only() {
        let mut next = 3;
        let (clause, ctx, dt) = build_filter_clause(None, Some("session"), &mut next);
        assert_eq!(clause, " AND dt.name = $3");
        assert!(ctx.is_none());
        assert_eq!(dt.as_deref(), Some("session"));
        assert_eq!(next, 4);
    }
}
