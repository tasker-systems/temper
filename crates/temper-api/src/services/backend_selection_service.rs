//! Reads the singleton `kb_backend_selection` flag. The only SQL touching
//! the gate table (service layer owns SQL).

use sqlx::PgPool;
use temper_core::error::TemperError;

use crate::backend::selection::BackendSelection;

/// Read the current backend selection. The table is seeded with exactly one
/// row by migration `20260614000001`, so a missing row is a hard error.
pub async fn read(pool: &PgPool) -> Result<BackendSelection, TemperError> {
    let value = sqlx::query_scalar!("SELECT backend FROM kb_backend_selection WHERE id = true")
        .fetch_optional(pool)
        .await
        .map_err(|e| TemperError::Api(format!("read backend selection: {e}")))?
        .ok_or_else(|| {
            TemperError::Config("kb_backend_selection row missing (migration not run?)".into())
        })?;

    BackendSelection::from_db(&value)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "../../migrations")]
    async fn read_defaults_to_legacy(pool: PgPool) {
        let sel = read(&pool).await.expect("read flag");
        assert_eq!(sel, BackendSelection::Legacy);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn read_reflects_next(pool: PgPool) {
        sqlx::query!("UPDATE kb_backend_selection SET backend = 'next' WHERE id = true")
            .execute(&pool)
            .await
            .unwrap();
        let sel = read(&pool).await.expect("read flag");
        assert_eq!(sel, BackendSelection::Next);
    }
}
