use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::ingest_service::{insert_event_and_audit, replace_chunks};
use temper_core::hash::{compute_managed_hash, compute_open_hash};
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::ingest::unpack_chunks;
use temper_core::types::managed_meta::ManagedMeta;

pub use temper_core::types::resource::{
    ContentChunk, ContentResponse, ResourceCreateRequest, ResourceFacets, ResourceListParams,
    ResourceListResponse, ResourceRow, ResourceSortField, ResourceUpdateRequest, SortOrder,
};

/// Query parameters for resolving a resource by its URI components.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct ResolveByUriParams {
    pub owner: String,
    pub context: String,
    pub doc_type: String,
    pub ident: String,
}

// ---------------------------------------------------------------------------
// FilterBuilder — collects dynamic WHERE conditions
// ---------------------------------------------------------------------------

struct FilterBuilder {
    /// SQL fragments like "vb.kb_context_id = $2"
    conditions: Vec<String>,
    /// Bind values, stored as strings (converted from Uuid/String as needed)
    binds: Vec<BindValue>,
    /// Next bind parameter index (starts at 2 because $1 is always profile_id)
    next_param: usize,
}

#[derive(Debug)]
enum BindValue {
    Uuid(Uuid),
    Text(String),
}

impl FilterBuilder {
    fn new() -> Self {
        Self {
            conditions: Vec::new(),
            binds: Vec::new(),
            // $1 is profile_id, so we start at $2
            next_param: 2,
        }
    }

    fn push_uuid(&mut self, column: &str, value: Uuid) {
        self.conditions
            .push(format!("{column} = ${}", self.next_param));
        self.binds.push(BindValue::Uuid(value));
        self.next_param += 1;
    }

    fn push_text(&mut self, column: &str, value: &str) {
        self.conditions
            .push(format!("{column} = ${}", self.next_param));
        self.binds.push(BindValue::Text(value.to_string()));
        self.next_param += 1;
    }

    fn push_fts(&mut self, query: &str) {
        self.conditions.push(format!(
            "EXISTS (SELECT 1 FROM kb_resource_search_index fts WHERE fts.resource_id = vb.id AND fts.search_vector @@ plainto_tsquery('english', ${}))",
            self.next_param
        ));
        self.binds.push(BindValue::Text(query.to_string()));
        self.next_param += 1;
    }

    /// Push owner filter. "@me" matches profile-owned, "+slug" matches team-owned.
    fn push_owner(&mut self, owner: &str, profile_id: Uuid) {
        if owner == "@me" {
            self.conditions.push(format!(
                "(vb.kb_owner_table = 'kb_profiles' AND vb.kb_owner_id = ${})",
                self.next_param
            ));
            self.binds.push(BindValue::Uuid(profile_id));
            self.next_param += 1;
        } else if let Some(slug) = owner.strip_prefix('+') {
            self.conditions.push(format!(
                "(vb.kb_owner_table = 'kb_teams' AND vb.team_slug = ${})",
                self.next_param
            ));
            self.binds.push(BindValue::Text(slug.to_string()));
            self.next_param += 1;
        } else {
            // Unrecognized owner format — match nothing.
            self.conditions.push("false".to_string());
        }
    }

    fn where_clause(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conditions.join(" AND "))
        }
    }

    /// Bind all accumulated values onto a query in order.
    fn bind_all<'q, T: Send + Unpin>(
        &'q self,
        mut query: sqlx::query::QueryAs<'q, sqlx::Postgres, T, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::QueryAs<'q, sqlx::Postgres, T, sqlx::postgres::PgArguments> {
        for bind in &self.binds {
            match bind {
                BindValue::Uuid(u) => query = query.bind(u),
                BindValue::Text(t) => query = query.bind(t.as_str()),
            }
        }
        query
    }

    fn bind_all_scalar<'q, T>(
        &'q self,
        mut query: sqlx::query::QueryScalar<'q, sqlx::Postgres, T, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::QueryScalar<'q, sqlx::Postgres, T, sqlx::postgres::PgArguments> {
        for bind in &self.binds {
            match bind {
                BindValue::Uuid(u) => query = query.bind(u),
                BindValue::Text(t) => query = query.bind(t.as_str()),
            }
        }
        query
    }
}

/// Build filters from request params, returning the FilterBuilder.
fn build_filters(params: &ResourceListParams, profile_id: Uuid) -> FilterBuilder {
    let mut fb = FilterBuilder::new();

    if let Some(id) = params.kb_context_id {
        fb.push_uuid("vb.kb_context_id", id);
    }
    if let Some(id) = params.kb_doc_type_id {
        fb.push_uuid("vb.kb_doc_type_id", id);
    }
    if let Some(ref name) = params.context_name {
        fb.push_text("vb.context_name", name);
    }
    if let Some(ref name) = params.doc_type_name {
        fb.push_text("vb.doc_type_name", name);
    }
    if let Some(ref owner) = params.owner {
        fb.push_owner(owner, profile_id);
    }
    if let Some(ref q) = params.q {
        if !q.trim().is_empty() {
            fb.push_fts(q);
        }
    }

    fb
}

/// Map sort field + direction to an ORDER BY clause.
fn order_clause(sort: Option<ResourceSortField>, order: Option<SortOrder>) -> String {
    let column = match sort.unwrap_or_default() {
        ResourceSortField::Updated => "vb.updated",
        ResourceSortField::Created => "vb.created",
        ResourceSortField::Title => "vb.title",
        ResourceSortField::Stage => "vb.stage",
        ResourceSortField::Seq => "vb.seq",
        ResourceSortField::ContextName => "vb.context_name",
        ResourceSortField::DocTypeName => "vb.doc_type_name",
    };
    let direction = match order.unwrap_or_default() {
        SortOrder::Desc => "DESC",
        SortOrder::Asc => "ASC",
    };
    let nulls = match sort.unwrap_or_default() {
        ResourceSortField::Stage | ResourceSortField::Seq => " NULLS LAST",
        _ => "",
    };
    format!(" ORDER BY {column} {direction}{nulls}")
}

/// Owner handle SQL expression (CASE ... END AS owner_handle).
const OWNER_HANDLE_EXPR: &str = r#"CASE
  WHEN vb.kb_owner_table = 'kb_profiles' AND vb.kb_owner_id = $1 THEN '@me'
  WHEN vb.kb_owner_table = 'kb_teams' THEN '+' || vb.team_slug
  ELSE '@unknown'
END AS owner_handle"#;

/// The SELECT columns for ResourceRow from the vault_resources_browse view.
///
/// `body_hash`, `managed_hash`, and `open_hash` are fetched via correlated
/// scalar subqueries because `vault_resources_browse` joins
/// `kb_resource_manifests` internally but does not expose these columns.
/// Each subquery is a LEFT JOIN equivalent — `NULL` when no manifest row
/// exists.
fn select_columns() -> String {
    format!(
        r#"vb.id, vb.kb_context_id, vb.kb_doc_type_id, vb.origin_uri, vb.title,
       vb.slug, vb.originator_profile_id, vb.owner_profile_id, vb.is_active,
       vb.created, vb.updated, vb.context_name, vb.doc_type_name,
       {OWNER_HANDLE_EXPR},
       vb.stage, vb.seq, vb.mode, vb.effort,
       (SELECT m.body_hash    FROM kb_resource_manifests m WHERE m.resource_id = vb.id) AS body_hash,
       (SELECT m.managed_hash FROM kb_resource_manifests m WHERE m.resource_id = vb.id) AS managed_hash,
       (SELECT m.open_hash    FROM kb_resource_manifests m WHERE m.resource_id = vb.id) AS open_hash"#
    )
}

// ---------------------------------------------------------------------------
// Facet row for internal deserialization
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct FacetRow {
    doc_type_name: String,
    count: i64,
}

/// List resources visible to the given profile.
///
/// Uses the `vault_resources_browse` view with dynamic filters.
/// Returns `ResourceListResponse` with rows, total count, and facets.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<ResourceListResponse> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0).max(0);

    let fb = build_filters(&params, profile_id);
    let where_clause = fb.where_clause();
    let order = order_clause(params.sort, params.order);

    // Rows query
    let rows_sql = format!(
        "SELECT {cols}\n  FROM vault_resources_browse vb\n  JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id\n {where_clause}{order}\n LIMIT ${lim} OFFSET ${off}",
        cols = select_columns(),
        lim = fb.next_param,
        off = fb.next_param + 1,
    );

    // Count query
    let count_sql = format!(
        "SELECT COUNT(*)::bigint\n  FROM vault_resources_browse vb\n  JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id\n {where_clause}"
    );

    // Facets query
    let facets_sql = format!(
        "SELECT vb.doc_type_name, COUNT(*)::bigint AS count\n  FROM vault_resources_browse vb\n  JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id\n {where_clause}\n GROUP BY vb.doc_type_name"
    );

    // Build all three queries and execute in parallel
    let rows_query = fb
        .bind_all(sqlx::query_as::<_, ResourceRow>(&rows_sql).bind(profile_id))
        .bind(limit)
        .bind(offset);

    let count_query = fb.bind_all_scalar(sqlx::query_scalar::<_, i64>(&count_sql).bind(profile_id));

    let facets_query_bound =
        fb.bind_all(sqlx::query_as::<_, FacetRow>(&facets_sql).bind(profile_id));

    let (rows, count_opt, facet_rows) = tokio::try_join!(
        rows_query.fetch_all(pool),
        count_query.fetch_one(pool),
        facets_query_bound.fetch_all(pool),
    )?;

    let total = count_opt;
    let facets = ResourceFacets {
        doc_type: facet_rows
            .into_iter()
            .map(|r| (r.doc_type_name, r.count))
            .collect::<HashMap<String, i64>>(),
    };

    Ok(ResourceListResponse {
        rows,
        total,
        facets,
    })
}

/// Get a single resource by ID, scoped to profile visibility.
pub async fn get_visible(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<ResourceRow> {
    let sql = format!(
        "SELECT {cols}\n  FROM vault_resources_browse vb\n  JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id\n WHERE vb.id = $2",
        cols = select_columns(),
    );

    let row = sqlx::query_as::<_, ResourceRow>(&sql)
        .bind(profile_id)
        .bind(resource_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(row)
}

/// Get a single resource by slug within a context, scoped to profile visibility.
pub async fn get_by_slug(
    pool: &PgPool,
    profile_id: Uuid,
    slug: &str,
    context_id: Uuid,
) -> ApiResult<ResourceRow> {
    let sql = format!(
        "SELECT {cols}\n  FROM vault_resources_browse vb\n  JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id\n WHERE vb.slug = $2\n   AND vb.kb_context_id = $3",
        cols = select_columns(),
    );

    let row = sqlx::query_as::<_, ResourceRow>(&sql)
        .bind(profile_id)
        .bind(slug)
        .bind(context_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(row)
}

/// Resolve a resource by its URI components (owner, context, doc_type, ident).
///
/// `ident` can be a UUID (matched against `id`) or a slug.
pub async fn resolve_by_uri(
    pool: &PgPool,
    profile_id: Uuid,
    params: &ResolveByUriParams,
) -> ApiResult<ResourceRow> {
    // Determine if ident is a UUID or slug
    let by_id = Uuid::parse_str(&params.ident).ok();

    let mut fb = FilterBuilder::new();

    // Owner filter
    fb.push_owner(&params.owner, profile_id);

    // Context name
    fb.push_text("vb.context_name", &params.context);

    // Doc type name
    fb.push_text("vb.doc_type_name", &params.doc_type);

    // Ident — UUID or slug
    if let Some(id) = by_id {
        fb.push_uuid("vb.id", id);
    } else {
        fb.push_text("vb.slug", &params.ident);
    }

    let where_clause = fb.where_clause();
    let sql = format!(
        "SELECT {cols}\n  FROM vault_resources_browse vb\n  JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id\n {where_clause}",
        cols = select_columns(),
    );

    let row = fb
        .bind_all(sqlx::query_as::<_, ResourceRow>(&sql).bind(profile_id))
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(row)
}

/// Fetch a resource's full content response: reconstituted markdown body
/// plus managed_meta and open_meta from the manifest. Runs the visibility
/// check up front and owns every subsequent query, so there is no way for
/// a caller to assemble a partial response that skips authorization.
///
/// Replaces the previous `get_content` + `get_managed_meta` + `get_open_meta`
/// split. Those helpers ran unauthenticated reads that relied on the
/// handler to have already called `get_visible` — a convention-based
/// safety model that is one careless new handler away from a data leak.
pub async fn get_content(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<ContentResponse> {
    // Visibility / auth gate. Returns NotFound for missing or not-visible.
    get_visible(pool, profile_id, resource_id).await?;

    // Chunks and manifest meta come from different tables and are both
    // cheap; fetch them concurrently on the same pool.
    let chunks_fut = sqlx::query_as!(
        ContentChunk,
        r#"
        SELECT chunk_index as "chunk_index!: i32",
               header_path as "header_path!: String",
               heading_depth as "heading_depth!: i16",
               content as "content!: String"
          FROM kb_current_chunks
         WHERE resource_id = $1
         ORDER BY chunk_index
        "#,
        resource_id,
    )
    .fetch_all(pool);

    let meta_fut = sqlx::query!(
        r#"SELECT managed_meta as "managed_meta: serde_json::Value",
                  open_meta    as "open_meta: serde_json::Value"
             FROM kb_resource_manifests
            WHERE resource_id = $1"#,
        resource_id,
    )
    .fetch_optional(pool);

    let (chunks, meta_row) = tokio::try_join!(chunks_fut, meta_fut)?;

    let markdown = chunks
        .into_iter()
        .map(|c| {
            if c.heading_depth == 0 {
                // Preamble or unheaded content — emit body only.
                c.content
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
        .join("\n\n");

    // Deserialize the JSONB managed_meta into the typed `ManagedMeta`.
    // The `extra` flatten bucket on `ManagedMeta` captures any fields
    // the typed struct doesn't name (e.g. doc-type-schema fields like
    // `date` for sessions), so this round-trip is lossless.
    let (managed_meta, open_meta) = match meta_row {
        Some(row) => {
            let typed: temper_core::types::managed_meta::ManagedMeta =
                serde_json::from_value(row.managed_meta).unwrap_or_default();
            (Some(typed), Some(row.open_meta))
        }
        None => (None, None),
    };

    Ok(ContentResponse {
        resource_id: ResourceId::from(resource_id),
        markdown,
        managed_meta,
        open_meta,
    })
}

/// Check whether the profile can modify a resource. Returns Forbidden if not.
pub async fn check_can_modify(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<()> {
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        profile_id,
        resource_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    Ok(())
}

/// Update mutable fields on a resource. Requires `can_modify_resource()` to return true.
///
/// Performs a partial merge for `managed_meta` and `open_meta`:
/// - Typed `managed_meta` fields: `Some` incoming value overwrites stored; `None` preserves.
/// - `managed_meta.extra` bucket: incoming keys are merged in (incoming wins per-key).
/// - `open_meta` (JSON object): incoming keys are merged in (incoming wins per-key).
///
/// When `content`, `content_hash`, and `chunks_packed` are all `Some` (body
/// trio), chunk-store is updated via `replace_chunks` inside the same
/// transaction. If `content_hash` matches the stored `body_hash`, the chunk
/// work is skipped (short-circuit dedupe). The handler validates the trio is
/// all-or-nothing before this function is called.
///
/// `managed_hash` and `open_hash` are recomputed whenever their respective
/// metadata changes.
pub async fn update(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    device_id: &str,
    req: ResourceUpdateRequest,
) -> ApiResult<ResourceRow> {
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        profile_id,
        resource_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // 1. Update title/slug on kb_resources. We need current.title/slug for
    //    fallback values and current.doc_type_name to compute the managed_hash
    //    later, so read once via get_visible. Safety: the UPDATE below uses
    //    `WHERE is_active = true` and we check `rows_affected()` to detect a
    //    resource that became inactive between this read and the write.
    let current = get_visible(pool, profile_id, resource_id).await?;
    let new_title = req.title.as_deref().unwrap_or(&current.title);
    let new_slug = req.slug.as_deref().or(current.slug.as_deref());
    let update_result = sqlx::query!(
        r#"
        UPDATE kb_resources
           SET title   = $1,
               slug    = $2,
               updated = now()
         WHERE id = $3
           AND is_active = true
        "#,
        new_title,
        new_slug,
        resource_id,
    )
    .execute(&mut *tx)
    .await?;

    if update_result.rows_affected() == 0 {
        // Resource was deleted (is_active = false) between the get_visible
        // read and this UPDATE. Surface as NotFound rather than silently
        // committing a manifest write for a deleted resource.
        return Err(ApiError::NotFound);
    }

    // 2. Merge managed_meta + open_meta into kb_resource_manifests.
    //    `resource_service::create` does not create a manifest row (only
    //    `ingest_service::create_resource_with_manifest` does), so resources
    //    born via POST /api/resources have no manifest until their first
    //    PATCH or ingest. The ON CONFLICT upsert below is load-bearing for
    //    that create-then-patch flow; do not simplify to a plain UPDATE.
    // Enter the manifest-rewrite block whenever ANY field that affects the
    // canonical managed_meta JSONB is present. A title/slug-only PATCH still
    // needs to refresh the JSONB so its temper-title / temper-slug keys (and
    // the managed_hash) stay in lockstep with the kb_resources columns.
    //
    // Capture before the if-block moves req.managed_meta / req.open_meta.
    // post_merge_managed/open are hoisted so reconcile_edges (after tx.commit())
    // can read the post-merge JSONB values without re-querying.
    let meta_touched = req.managed_meta.is_some() || req.open_meta.is_some();
    let mut post_merge_managed: Option<serde_json::Value> = None;
    let mut post_merge_open: Option<serde_json::Value> = None;

    if req.managed_meta.is_some()
        || req.open_meta.is_some()
        || req.title.is_some()
        || req.slug.is_some()
    {
        let stored = sqlx::query!(
            r#"SELECT managed_meta as "managed_meta: serde_json::Value",
                      open_meta    as "open_meta: serde_json::Value"
                 FROM kb_resource_manifests
                WHERE resource_id = $1"#,
            resource_id,
        )
        .fetch_optional(&mut *tx)
        .await?;

        let (stored_managed_json, stored_open_json) = match stored {
            Some(row) => (row.managed_meta, row.open_meta),
            None => (
                serde_json::Value::Object(Default::default()),
                serde_json::Value::Object(Default::default()),
            ),
        };

        // Surface JSONB→ManagedMeta failures as data-integrity errors rather
        // than silently overwriting the stored value with an empty default.
        // ManagedMeta has a flatten extras bucket so the only way this fails
        // is if the column holds a non-object JSON value, which would be
        // structural corruption worth knowing about.
        let mut merged_managed: ManagedMeta = serde_json::from_value(stored_managed_json)
            .map_err(|e| ApiError::Internal(format!("malformed managed_meta JSONB: {e}")))?;
        if let Some(incoming) = req.managed_meta {
            apply_managed_meta_partial(&mut merged_managed, incoming);
        }

        let mut merged_open = stored_open_json;
        if let Some(incoming_open) = req.open_meta {
            apply_open_meta_partial(&mut merged_open, incoming_open);
        }

        let mut managed_value = serde_json::to_value(&merged_managed)?;
        // Inject canonical identity keys from the resolved top-level title /
        // slug before hashing. `new_slug` is `Option<&str>` and flows through
        // unchanged: when the resource has no slug (column NULL on a resource
        // born via POST /api/resources without one), the helper drops the
        // `temper-slug` key so column-NULL and JSONB-key-absent agree.
        temper_core::operations::ensure_managed_identity_keys(
            &mut managed_value,
            new_title,
            new_slug,
        );
        // Apply doc-type defaults to fill in any required fields that aren't
        // already present. Mirrors ingest_service::update:674 — the canonical
        // site for defaulting on meta-bearing updates. Without this, meta
        // updates routed through DbBackend → resource_service::update would
        // silently regress required-field defaulting.
        // This affects PATCH /api/resources, PUT /api/ingest/{id}, and
        // PUT /api/resources/{id}/meta, making defaulting consistent across
        // all meta-touching update paths.
        temper_core::operations::apply_defaults_value(&current.doc_type_name, &mut managed_value);
        let managed_hash = compute_managed_hash(&current.doc_type_name, &managed_value);
        let open_hash = compute_open_hash(&merged_open);

        sqlx::query!(
            r#"INSERT INTO kb_resource_manifests
                   (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
               VALUES ($5, '', $1, $3, $2, $4, now())
               ON CONFLICT (resource_id) DO UPDATE
                   SET managed_meta = $1, managed_hash = $2,
                       open_meta    = $3, open_hash    = $4,
                       updated      = now()"#,
            managed_value,
            managed_hash,
            merged_open,
            open_hash,
            resource_id,
        )
        .execute(&mut *tx)
        .await?;

        // Capture post-merge values for edge reconciliation after tx.commit().
        post_merge_managed = Some(managed_value);
        post_merge_open = Some(merged_open);
    }

    // 3. Body trio path: persist + dedupe chunks if all three fields present.
    //    The handler guarantees all-or-nothing — if any one is Some, all are Some.
    if let (Some(incoming_hash), Some(chunks_packed_str)) = (req.content_hash, req.chunks_packed) {
        // Read the stored body_hash to decide whether chunk work is needed.
        // Returns None when no manifest row exists (fresh resource).
        let stored_body_hash: String = sqlx::query_scalar!(
            "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
            resource_id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or_default();

        if incoming_hash != stored_body_hash {
            // Hash changed — decode chunks and rewire via the shared primitive.
            let chunks = unpack_chunks(&chunks_packed_str)
                .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

            // Fetch context_id and current hashes for the event + audit record.
            let context_id: Uuid = sqlx::query_scalar!(
                "SELECT kb_context_id FROM kb_resources WHERE id = $1",
                resource_id,
            )
            .fetch_one(&mut *tx)
            .await?;

            // Fetch current manifest hashes for the audit trail; fall back to
            // empty strings when no manifest row exists (body-trio-only PATCH on
            // a resource that never had a manifest).
            let (managed_hash, open_hash): (String, String) = sqlx::query_as(
                "SELECT COALESCE(managed_hash, ''), COALESCE(open_hash, '') \
                 FROM kb_resource_manifests WHERE resource_id = $1",
            )
            .bind(resource_id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_default();

            let (_event_id, audit_id) = insert_event_and_audit(
                &mut tx,
                ProfileId::from(profile_id),
                device_id,
                ContextId::from(context_id),
                ResourceId::from(resource_id),
                "body_updated",
                "update_body",
                &incoming_hash,
                &managed_hash,
                &open_hash,
            )
            .await?;

            // Replace chunks: version-bump old, batch-insert new, rebuild search.
            replace_chunks(
                &mut tx,
                ResourceId::from(resource_id),
                audit_id,
                &incoming_hash,
                &chunks,
            )
            .await?;

            // Update body_hash in the manifest (upsert: body-trio-only PATCH may
            // arrive before any managed_meta write on a fresh resource).
            sqlx::query!(
                r#"INSERT INTO kb_resource_manifests
                       (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
                   VALUES ($1, $2, '{}', '{}', '', '', now())
                   ON CONFLICT (resource_id) DO UPDATE
                       SET body_hash = $2, updated = now()"#,
                resource_id,
                incoming_hash,
            )
            .execute(&mut *tx)
            .await?;
        }
        // else: hash matches stored → short-circuit, no chunk work.
    }

    tx.commit().await?;

    // Reconcile frontmatter-provenance edges when managed/open meta were touched.
    // Mirrors the call in meta_service::update_meta (line 275) and
    // ingest_service::ingest (line 744). Errors are warn-and-continue: the
    // update itself succeeded and the edge table is an eventually-consistent
    // derived view of the frontmatter declarations.
    if meta_touched {
        let context_id = current.kb_context_id;
        let res_id = ResourceId::from(resource_id);
        let prof_id = ProfileId::from(profile_id);
        let managed =
            post_merge_managed.expect("populated by manifest-rewrite block when meta_touched");
        let open = post_merge_open.expect("populated by manifest-rewrite block when meta_touched");
        if let Err(e) = super::edge_service::reconcile_edges(
            pool,
            &prof_id,
            &context_id,
            &res_id,
            &current.doc_type_name,
            &managed,
            &open,
        )
        .await
        {
            tracing::warn!(
                resource_id = %resource_id,
                error = %e,
                "edge reconciliation failed during resource update"
            );
        }
    }

    get_visible(pool, profile_id, resource_id).await
}

/// Overlay `Some` fields from `incoming` onto `target`. `None` fields preserve target.
/// The `extra` bucket merges by key — incoming keys win.
fn apply_managed_meta_partial(target: &mut ManagedMeta, incoming: ManagedMeta) {
    if incoming.doc_type.is_some() {
        target.doc_type = incoming.doc_type;
    }
    if incoming.context.is_some() {
        target.context = incoming.context;
    }
    if incoming.updated.is_some() {
        target.updated = incoming.updated;
    }
    if incoming.source.is_some() {
        target.source = incoming.source;
    }
    if incoming.stage.is_some() {
        target.stage = incoming.stage;
    }
    if incoming.mode.is_some() {
        target.mode = incoming.mode;
    }
    if incoming.effort.is_some() {
        target.effort = incoming.effort;
    }
    if incoming.goal.is_some() {
        target.goal = incoming.goal;
    }
    if incoming.seq.is_some() {
        target.seq = incoming.seq;
    }
    if incoming.branch.is_some() {
        target.branch = incoming.branch;
    }
    if incoming.pr.is_some() {
        target.pr = incoming.pr;
    }
    if incoming.status.is_some() {
        target.status = incoming.status;
    }
    if incoming.provenance.is_some() {
        target.provenance = incoming.provenance;
    }
    if incoming.llm_model.is_some() {
        target.llm_model = incoming.llm_model;
    }
    if incoming.llm_run.is_some() {
        target.llm_run = incoming.llm_run;
    }
    if incoming.title.is_some() {
        target.title = incoming.title;
    }
    if incoming.slug.is_some() {
        target.slug = incoming.slug;
    }
    for (k, v) in incoming.extra {
        target.extra.insert(k, v);
    }
}

/// Merge incoming JSON object keys into `target`. Object types only.
///
/// For each key in `incoming`, it overwrites the corresponding key in
/// `target`. Keys absent from `incoming` are untouched. If either side
/// is not a JSON object, `incoming` replaces `target` entirely.
fn apply_open_meta_partial(target: &mut serde_json::Value, incoming: serde_json::Value) {
    if let (Some(target_obj), Some(incoming_obj)) = (target.as_object_mut(), incoming.as_object()) {
        for (k, v) in incoming_obj {
            target_obj.insert(k.clone(), v.clone());
        }
    } else {
        // Either side is not an object — incoming replaces target (best-effort).
        *target = incoming;
    }
}

/// Soft-delete a resource. Requires `can_modify_resource()` to return true.
pub async fn delete(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    device_id: &str,
) -> ApiResult<()> {
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        *profile_id,
        *resource_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // Fetch current hashes for the audit snapshot before soft-delete
    let hashes = sqlx::query!(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
        *resource_id,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let (body_hash, managed_hash, open_hash) = hashes
        .map(|h| (h.body_hash, h.managed_hash, h.open_hash))
        .unwrap_or_default();

    // Fetch context_id for the event
    let context_id = sqlx::query_scalar!(
        "SELECT kb_context_id FROM kb_resources WHERE id = $1",
        *resource_id,
    )
    .fetch_one(&mut *tx)
    .await?;

    // Soft-delete the resource
    sqlx::query!(
        r#"
        UPDATE kb_resources
           SET is_active = false,
               updated   = now()
         WHERE id = $1
           AND is_active = true
        "#,
        *resource_id,
    )
    .execute(&mut *tx)
    .await?;

    // Record event and audit atomically
    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        ContextId::from(context_id),
        resource_id,
        "resource_deleted",
        "delete",
        &body_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}
