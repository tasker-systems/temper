//! Sync service — computes diffs and finalizes sync rounds.
//!
//! Port of packages/temper-cloud/src/sync.ts. Uses the same SQL functions
//! (sync_diff_for_device) but with batch updates for completeSyncRound
//! (fixes code review audit item 5e).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

use temper_core::types::sync::{
    SyncCompleteRequest, SyncCompleteResponse, SyncConflictItem, SyncManifestItem,
    SyncManifestResponse, SyncPullItem, SyncPushItem, SyncRemovedItem, SyncStatusRequest,
    SyncStatusResponse,
};

/// Raw row from sync_diff_for_device().
#[derive(Debug, sqlx::FromRow)]
struct DiffRow {
    resource_id: Option<Uuid>,
    kb_uri: String,
    body_hash: String,
    #[expect(
        dead_code,
        reason = "returned by SQL; will be used for three-tier sync"
    )]
    managed_hash: String,
    #[expect(
        dead_code,
        reason = "returned by SQL; will be used for three-tier sync"
    )]
    open_hash: String,
    #[expect(dead_code, reason = "returned by SQL but not used in categorization")]
    updated: Option<DateTime<Utc>>,
    diff_type: String,
}

/// Categorize raw diff rows into typed response buckets.
/// Port of TypeScript `categorizeDiffRows()` — pure function.
fn categorize_diff_rows(rows: Vec<DiffRow>) -> SyncStatusResponse {
    let mut to_push = Vec::new();
    let mut to_pull = Vec::new();
    let mut conflicts = Vec::new();
    let mut removed = Vec::new();

    for row in rows {
        match row.diff_type.as_str() {
            "to_push" | "to_push_body" | "to_push_meta" => to_push.push(SyncPushItem {
                uri: row.kb_uri,
                resource_id: row.resource_id,
            }),
            "to_pull" | "to_pull_body" | "to_pull_meta" => to_pull.push(SyncPullItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("to_pull must have resource_id"),
                content_hash: row.body_hash,
            }),
            "conflict" => conflicts.push(SyncConflictItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("conflict must have resource_id"),
                server_hash: row.body_hash,
            }),
            "removed" => removed.push(SyncRemovedItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("removed must have resource_id"),
            }),
            _ => {} // ignore unknown diff types
        }
    }

    SyncStatusResponse {
        to_push,
        to_pull,
        conflicts,
        removed,
    }
}

/// Compute sync diff by calling sync_diff_for_device() and categorizing results.
pub async fn compute_sync_diff(
    pool: &PgPool,
    profile_id: Uuid,
    request: SyncStatusRequest,
) -> ApiResult<SyncStatusResponse> {
    let mut context_names: Vec<String> = Vec::new();
    let mut manifest_entries = Vec::new();

    for ctx in &request.contexts {
        context_names.push(ctx.name.clone());
        for entry in &ctx.entries {
            manifest_entries.push(serde_json::json!({
                "uri": entry.uri,
                "local_hash": entry.local_hash,
                "remote_hash": entry.remote_hash,
                "managed_hash": entry.managed_hash,
                "remote_managed_hash": entry.remote_managed_hash,
                "open_hash": entry.open_hash,
                "remote_open_hash": entry.remote_open_hash,
            }));
        }
    }

    let manifest_jsonb = serde_json::Value::Array(manifest_entries);

    let rows = sqlx::query_as::<_, DiffRow>(
        r#"
        SELECT resource_id, kb_uri, body_hash, managed_hash, open_hash, updated, diff_type
          FROM sync_diff_for_device($1, $2::text[], $3::jsonb)
        "#,
    )
    .bind(profile_id)
    .bind(&context_names)
    .bind(&manifest_jsonb)
    .fetch_all(pool)
    .await?;

    Ok(categorize_diff_rows(rows))
}

/// Finalize a sync round: batch-update body hashes and upsert device state.
///
/// Uses a single UPDATE with unnest() instead of per-row loop
/// (fixes code review audit item 5e).
pub async fn complete_sync_round(
    pool: &PgPool,
    profile_id: Uuid,
    request: SyncCompleteRequest,
) -> ApiResult<SyncCompleteResponse> {
    let mut tx = pool.begin().await?;

    let updated_count = if !request.merged_resources.is_empty() {
        let ids: Vec<Uuid> = request
            .merged_resources
            .iter()
            .map(|m| m.resource_id)
            .collect();
        let hashes: Vec<String> = request
            .merged_resources
            .iter()
            .map(|m| m.content_hash.clone())
            .collect();

        let result = sqlx::query(
            r#"
            UPDATE kb_resource_manifests m
            SET body_hash = u.body_hash, updated = now()
            FROM unnest($1::uuid[], $2::text[]) AS u(resource_id, body_hash)
            WHERE m.resource_id = u.resource_id
            "#,
        )
        .bind(&ids)
        .bind(&hashes)
        .execute(&mut *tx)
        .await?;

        result.rows_affected() as u32
    } else {
        0
    };

    // Upsert device sync state
    sqlx::query(
        r#"
        INSERT INTO kb_device_sync_state (id, profile_id, device_id, last_sync_at)
        VALUES (gen_random_uuid(), $1, $2, now())
        ON CONFLICT (profile_id, device_id)
        DO UPDATE SET last_sync_at = now()
        "#,
    )
    .bind(profile_id)
    .bind(&request.device_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(SyncCompleteResponse {
        last_sync_at: Utc::now(),
        updated_count,
    })
}

/// Raw row from the manifest query.
#[derive(Debug, sqlx::FromRow)]
struct ManifestRow {
    resource_id: Uuid,
    context_name: String,
    doc_type_name: String,
    slug: String,
    body_hash: String,
    managed_hash: String,
    open_hash: String,
}

/// Fetch all active resources for a profile — metadata only, no content.
/// Used by `GET /api/sync/manifest` for manifest recovery (refresh/reset).
pub async fn fetch_manifest(pool: &PgPool, profile_id: Uuid) -> ApiResult<SyncManifestResponse> {
    let rows = sqlx::query_as::<_, ManifestRow>(
        r#"
        SELECT r.id AS resource_id,
               c.name AS context_name,
               d.name AS doc_type_name,
               COALESCE(r.slug, '') AS slug,
               COALESCE(m.body_hash, '') AS body_hash,
               COALESCE(m.managed_hash, '') AS managed_hash,
               COALESCE(m.open_hash, '') AS open_hash
          FROM kb_resources r
          JOIN kb_contexts c ON c.id = r.kb_context_id
          JOIN kb_doc_types d ON d.id = r.kb_doc_type_id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
         WHERE r.owner_profile_id = $1
           AND r.is_active = true
         ORDER BY c.name, d.name, r.slug
        "#,
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;

    let items = rows
        .into_iter()
        .map(|row| {
            let uri = format!(
                "kb://{}/{}/{}",
                row.context_name, row.doc_type_name, row.resource_id
            );
            SyncManifestItem {
                resource_id: row.resource_id,
                context: row.context_name,
                doc_type: row.doc_type_name,
                slug: row.slug,
                content_hash: row.body_hash,
                managed_hash: row.managed_hash,
                open_hash: row.open_hash,
                uri,
            }
        })
        .collect();

    Ok(SyncManifestResponse { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_diff_rows_sorts_correctly() {
        let rows = vec![
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/a".to_owned(),
                body_hash: "h1".to_owned(),
                managed_hash: String::new(),
                open_hash: String::new(),
                updated: None,
                diff_type: "to_push_body".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/b".to_owned(),
                body_hash: "h2".to_owned(),
                managed_hash: String::new(),
                open_hash: String::new(),
                updated: None,
                diff_type: "to_pull".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/c".to_owned(),
                body_hash: "h3".to_owned(),
                managed_hash: String::new(),
                open_hash: String::new(),
                updated: None,
                diff_type: "conflict".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/d".to_owned(),
                body_hash: "h4".to_owned(),
                managed_hash: String::new(),
                open_hash: String::new(),
                updated: None,
                diff_type: "removed".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/e".to_owned(),
                body_hash: "h5".to_owned(),
                managed_hash: String::new(),
                open_hash: String::new(),
                updated: None,
                diff_type: "to_push_meta".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/f".to_owned(),
                body_hash: "h6".to_owned(),
                managed_hash: String::new(),
                open_hash: String::new(),
                updated: None,
                diff_type: "to_pull_body".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/g".to_owned(),
                body_hash: "h7".to_owned(),
                managed_hash: String::new(),
                open_hash: String::new(),
                updated: None,
                diff_type: "to_pull_meta".to_owned(),
            },
        ];

        let result = categorize_diff_rows(rows);
        assert_eq!(result.to_push.len(), 2);
        assert_eq!(result.to_pull.len(), 3); // to_pull + to_pull_body + to_pull_meta
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.to_push[0].uri, "kb://ctx/task/a");
        assert_eq!(result.to_push[1].uri, "kb://ctx/task/e");
        assert_eq!(result.to_pull[0].uri, "kb://ctx/task/b");
        assert_eq!(result.to_pull[1].uri, "kb://ctx/task/f");
        assert_eq!(result.to_pull[2].uri, "kb://ctx/task/g");
    }
}
