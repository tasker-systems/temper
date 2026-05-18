use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::LedgerError;
use crate::projection::project_concept;
use crate::types::concept::Concept;

pub async fn rebuild_concept(pool: &PgPool, concept_id: Uuid) -> Result<Concept, LedgerError> {
    let concept = sqlx::query_as!(
        Concept,
        r#"
        SELECT
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        FROM event_substrate.concepts
        WHERE id = $1
        "#,
        concept_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::ConceptNotFound(concept_id))?;

    let chain = collect_chain(pool, concept.created_by_event_id).await?;

    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM event_substrate.concepts WHERE id = $1",
        concept_id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut latest: Option<Concept> = None;
    for event_id in chain {
        latest = Some(project_concept(pool, event_id).await?);
    }

    latest.ok_or(LedgerError::ConceptNotFound(concept_id))
}

/// Walk forward from a genesis event by collecting all events whose
/// `Supersedes` reference points (transitively) back to it, ordered by
/// `recorded_at`.
async fn collect_chain(pool: &PgPool, root_event_id: Uuid) -> Result<Vec<Uuid>, LedgerError> {
    // Recursive CTE traversing forward through Supersedes references.
    let rows = sqlx::query!(
        r#"
        WITH RECURSIVE chain AS (
            SELECT id, recorded_at, "references"
              FROM event_substrate.events
             WHERE id = $1
            UNION ALL
            SELECT e.id, e.recorded_at, e."references"
              FROM event_substrate.events e
              JOIN chain c
                ON e."references" @> jsonb_build_array(
                     jsonb_build_object('kind', 'Supersedes', 'event_id', c.id)
                   )
        )
        SELECT id AS "id!: Uuid", recorded_at AS "recorded_at!: chrono::DateTime<chrono::Utc>"
        FROM chain
        ORDER BY recorded_at ASC, id ASC
        "#,
        root_event_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| r.id).collect())
}
