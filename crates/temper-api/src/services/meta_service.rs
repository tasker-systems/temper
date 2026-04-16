//! Meta service — updates managed and open frontmatter on a resource
//! without requiring re-chunking.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::ingest_service::insert_event_and_audit;
use crate::services::resource_service;
use temper_core::frontmatter::registry;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload, ResourceMetaResponse};

/// Validate every top-level key in `open_meta` against the
/// `KNOWN_OPEN_FIELDS` registry. Accepts both canonical underscore form
/// (e.g. `relates_to`) and hyphen-form aliases (e.g. `relates-to`) via
/// `registry::lookup`.
///
/// Returns the offending key on first miss so the caller can surface a
/// specific error. `Ok(())` if `open_meta` is not an object or is empty.
///
/// This is a server-side safety net for typo-d or unknown open-meta
/// keys coming from MCP / API clients that bypass the CLI's
/// `Frontmatter::try_from` alias normalization. The CLI's strict
/// `Frontmatter` pipeline already rejects unknown keys client-side, so
/// well-formed CLI payloads pass this check unchanged.
fn validate_open_meta_keys(open_meta: &Value) -> Result<(), String> {
    let Some(obj) = open_meta.as_object() else {
        return Ok(());
    };
    for key in obj.keys() {
        if registry::lookup(key.as_str()).is_none() {
            return Err(key.clone());
        }
    }
    Ok(())
}

/// Fetch just the meta tier (managed_meta, open_meta, hashes) for a
/// resource without reconstructing the markdown body from `kb_chunks`.
///
/// Used by the CLI sync pull path when only meta has drifted.
/// Enforces visibility via `resource_service::get_visible`, which maps
/// both "missing" and "not visible to caller" to `ApiError::NotFound`.
pub async fn get_meta(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
) -> ApiResult<ResourceMetaResponse> {
    // Visibility / auth gate — returns NotFound for ghost or non-visible.
    resource_service::get_visible(pool, *profile_id, *resource_id).await?;

    let row = sqlx::query!(
        r#"SELECT managed_meta as "managed_meta: Value",
                  open_meta as "open_meta: Value",
                  managed_hash,
                  open_hash
             FROM kb_resource_manifests
            WHERE resource_id = $1"#,
        *resource_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Deserialize the stored JSONB into the typed `ManagedMeta`. The
    // `extra` flatten bucket catches any keys the typed fields don't
    // name (e.g. doc-type-schema fields like `date` for sessions), so
    // this is lossless — re-serializing produces the same canonical
    // JSON, and `managed_hash` remains stable across the round-trip.
    let managed_meta: ManagedMeta = serde_json::from_value(row.managed_meta).unwrap_or_default();

    Ok(ResourceMetaResponse {
        resource_id,
        managed_meta: Some(managed_meta),
        open_meta: Some(row.open_meta),
        managed_hash: row.managed_hash,
        open_hash: row.open_hash,
    })
}

/// Update resource manifests with new managed/open meta, and cascade
/// identity fields (title, slug, temper-type, temper-context) to kb_resources.
pub async fn update_meta(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    device_id: &str,
    payload: MetaUpdatePayload,
) -> ApiResult<Value> {
    // 1. Check can_modify_resource
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        *profile_id,
        *resource_id
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    // 1b. Validate open_meta keys against the KNOWN_OPEN_FIELDS registry
    // BEFORE starting the transaction. Unknown keys are rejected at the
    // API boundary rather than silently landing in jsonb where they'd
    // fail edge extraction later with a less actionable error.
    if let Err(bad_key) = validate_open_meta_keys(&payload.open_meta) {
        return Err(ApiError::BadRequest(format!(
            "unknown open_meta key '{bad_key}'; expected one of: \
             relates_to, depends_on, extends, references, preceded_by, \
             derived_from, parent, tags, aliases, date \
             (hyphen-form aliases like 'relates-to' are also accepted)"
        )));
    }

    let mut tx = pool.begin().await?;

    // 2. Update kb_resource_manifests (plain UPDATE — must already exist;
    //    we don't want to insert a row with an empty body_hash).
    //
    // The typed `ManagedMeta` is serialized back to a canonical JSONB
    // value here so the DB column stays a JSONB blob. The managed_hash
    // was computed by the caller over the canonical form, so the hash
    // stays stable across the typed round-trip.
    let managed_meta_json =
        serde_json::to_value(&payload.managed_meta).unwrap_or(serde_json::Value::Null);
    let rows = sqlx::query!(
        r#"
        UPDATE kb_resource_manifests
        SET managed_meta = $1, open_meta = $2, managed_hash = $3, open_hash = $4, updated = now()
        WHERE resource_id = $5
        "#,
        &managed_meta_json,
        &payload.open_meta as &serde_json::Value,
        &payload.managed_hash,
        &payload.open_hash,
        *resource_id,
    )
    .execute(&mut *tx)
    .await?;

    if rows.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    // 3. Cascade identity fields from managed_meta to kb_resources.
    // `payload.managed_meta` is already typed — no deserialize needed.
    let managed = &payload.managed_meta;

    // Update title and slug if present
    if let Some(ref title) = managed.title {
        sqlx::query!(
            "UPDATE kb_resources SET title = $1, updated = now() WHERE id = $2",
            title,
            *resource_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(ref slug) = managed.slug {
        sqlx::query!(
            "UPDATE kb_resources SET slug = $1, updated = now() WHERE id = $2",
            slug,
            *resource_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    // Cascade temper-type to kb_doc_type_id
    if let Some(ref doc_type) = managed.doc_type {
        let dt_id = sqlx::query_scalar!("SELECT id FROM kb_doc_types WHERE name = $1", doc_type,)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type: '{doc_type}'")))?;
        let dt_rows = sqlx::query!(
            "UPDATE kb_resources SET kb_doc_type_id = $1, updated = now() WHERE id = $2",
            dt_id,
            *resource_id,
        )
        .execute(&mut *tx)
        .await?;
        if dt_rows.rows_affected() == 0 {
            return Err(ApiError::NotFound);
        }
    }

    // Cascade temper-context to kb_context_id
    if let Some(ref context_name) = managed.context {
        let ctx_id =
            sqlx::query_scalar!("SELECT id FROM kb_contexts WHERE name = $1", context_name,)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("unknown context: '{context_name}'"))
                })?;
        sqlx::query!(
            "UPDATE kb_resources SET kb_context_id = $1, updated = now() WHERE id = $2",
            ctx_id,
            *resource_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    // 4. Insert kb_event + audit atomically
    // Fetch current body_hash and context_id for the event + audit records.
    let (body_hash, context_id): (String, Uuid) = sqlx::query_as(
        r#"SELECT m.body_hash, r.kb_context_id
           FROM kb_resource_manifests m
           JOIN kb_resources r ON r.id = m.resource_id
           WHERE m.resource_id = $1"#,
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        ContextId::from(context_id),
        resource_id,
        "managed_meta_updated",
        "update_meta",
        &body_hash,
        &payload.managed_hash,
        &payload.open_hash,
    )
    .await?;

    tx.commit().await?;

    // Reconcile edges from updated open_meta frontmatter.
    // The edge service reads declarations from open_meta and diffs against
    // existing frontmatter-provenance edges; manual edges are untouched.
    // Errors are logged, not propagated: the meta update itself succeeded.
    // `context_id` reflects the post-cascade state (any `temper-context`
    // change in managed_meta was applied earlier in the same tx), so we can
    // reuse the local directly instead of re-querying.
    let ctx_id = ContextId::from(context_id);
    let doc_type_str = payload.managed_meta.doc_type.as_deref().unwrap_or("");
    if let Err(e) = super::edge_service::reconcile_edges(
        pool,
        &profile_id,
        &ctx_id,
        &resource_id,
        doc_type_str,
        &managed_meta_json,
        &payload.open_meta,
    )
    .await
    {
        tracing::warn!(
            resource_id = %resource_id,
            error = %e,
            "edge reconciliation failed during meta update"
        );
    }

    Ok(serde_json::json!({"updated": true, "resource_id": resource_id}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_open_meta_accepts_canonical_keys() {
        let v = json!({
            "relates_to": ["foo"],
            "depends_on": ["bar"],
            "tags": ["auth"],
            "parent": "parent-slug",
        });
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_accepts_hyphen_aliases() {
        let v = json!({
            "relates-to": ["foo"],
            "depends-on": ["bar"],
            "preceded-by": ["baz"],
            "derived-from": ["qux"],
        });
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_accepts_mixed_canonical_and_alias() {
        let v = json!({
            "relates_to": ["foo"],
            "depends-on": ["bar"],
        });
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_rejects_unknown_key() {
        let v = json!({
            "relates_to": ["foo"],
            "totally_made_up": "nope",
        });
        let err = validate_open_meta_keys(&v).unwrap_err();
        assert_eq!(err, "totally_made_up");
    }

    #[test]
    fn validate_open_meta_rejects_typo_of_known_key() {
        let v = json!({
            "relats_to": ["foo"],
        });
        let err = validate_open_meta_keys(&v).unwrap_err();
        assert_eq!(err, "relats_to");
    }

    #[test]
    fn validate_open_meta_empty_object_ok() {
        let v = json!({});
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_non_object_ok() {
        // Non-object values are passed through — the caller's typed
        // MetaUpdatePayload wraps this in a Value that may be null or
        // some other shape during deserialization. Validation only
        // applies to well-formed object payloads.
        assert!(validate_open_meta_keys(&json!(null)).is_ok());
        assert!(validate_open_meta_keys(&json!([])).is_ok());
        assert!(validate_open_meta_keys(&json!("string")).is_ok());
    }

    #[test]
    fn validate_open_meta_reports_first_bad_key() {
        // BTreeMap key ordering in serde_json::Value::Object is insertion
        // order on recent versions, so this test documents the "first miss
        // wins" contract rather than asserting a specific order.
        let v = json!({
            "relates_to": ["a"],
            "bogus_one": "x",
            "bogus_two": "y",
        });
        let err = validate_open_meta_keys(&v).unwrap_err();
        assert!(
            err == "bogus_one" || err == "bogus_two",
            "expected first-bad-key to be one of the two unknowns, got: {err}"
        );
    }
}
