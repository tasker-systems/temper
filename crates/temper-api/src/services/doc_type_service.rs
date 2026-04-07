//! Doc type service — query system-level document types.

use sqlx::PgPool;

use crate::error::ApiResult;

/// A document type row from kb_doc_types.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct DocTypeRow {
    pub id: uuid::Uuid,
    pub name: String,
}

/// List all system-level document types.
pub async fn list_all(pool: &PgPool) -> ApiResult<Vec<DocTypeRow>> {
    let rows = sqlx::query_as!(
        DocTypeRow,
        r#"SELECT id, name FROM kb_doc_types ORDER BY name"#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
