//! Doc type service — query system-level document types.

use sqlx::PgPool;

use crate::error::ApiResult;

/// A document type row from kb_doc_types.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct DocTypeRow {
    pub id: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
}

/// Get a doc type name by its UUID.
pub async fn get_name_by_id(pool: &PgPool, id: uuid::Uuid) -> ApiResult<String> {
    let name = sqlx::query_scalar!("SELECT name FROM kb_doc_types WHERE id = $1", id,)
        .fetch_optional(pool)
        .await?
        .ok_or(crate::error::ApiError::NotFound)?;

    Ok(name)
}

/// List all system-level document types.
pub async fn list_all(pool: &PgPool) -> ApiResult<Vec<DocTypeRow>> {
    let rows = sqlx::query_as!(
        DocTypeRow,
        r#"SELECT id, name, description FROM kb_doc_types ORDER BY name"#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
