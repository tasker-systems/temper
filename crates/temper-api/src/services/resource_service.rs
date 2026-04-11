use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::ingest_service::insert_event_and_audit;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

pub use temper_core::types::resource::{
    ContentChunk, ResourceCreateRequest, ResourceFacets, ResourceListParams, ResourceListResponse,
    ResourceRow, ResourceSortField, ResourceUpdateRequest, SortOrder,
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
fn select_columns() -> String {
    format!(
        r#"vb.id, vb.kb_context_id, vb.kb_doc_type_id, vb.origin_uri, vb.title,
       vb.slug, vb.originator_profile_id, vb.owner_profile_id, vb.is_active,
       vb.created, vb.updated, vb.context_name, vb.doc_type_name,
       {OWNER_HANDLE_EXPR},
       vb.stage, vb.seq, vb.mode, vb.effort"#
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

/// Reconstitute resource content from `kb_current_chunks`, returning markdown.
pub async fn get_content(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<String> {
    // Visibility check first.
    get_visible(pool, profile_id, resource_id).await?;

    let chunks = sqlx::query_as!(
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
    .fetch_all(pool)
    .await?;

    let markdown = chunks
        .into_iter()
        .map(|c| {
            if c.heading_depth == 0 || c.header_path.is_empty() {
                c.content
            } else {
                // Extract the innermost heading title from the breadcrumb
                let title = c.header_path.rsplit(" > ").next().unwrap_or(&c.header_path);
                let hashes = "#".repeat(c.heading_depth as usize);
                format!("{hashes} {title}\n\n{}", c.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(markdown)
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

/// Create a new resource. The caller is set as both originator and owner.
pub async fn create(
    pool: &PgPool,
    profile_id: Uuid,
    req: ResourceCreateRequest,
) -> ApiResult<ResourceRow> {
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())
        "#,
        id,
        req.kb_context_id,
        req.kb_doc_type_id,
        req.origin_uri,
        req.title,
        req.slug,
        profile_id,
    )
    .execute(pool)
    .await?;

    get_visible(pool, profile_id, id).await
}

/// Update mutable fields on a resource. Requires `can_modify_resource()` to return true.
pub async fn update(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
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

    let current = get_visible(pool, profile_id, resource_id).await?;

    let new_title = req.title.as_deref().unwrap_or(&current.title);
    let new_slug = req.slug.as_deref().or(current.slug.as_deref());

    sqlx::query!(
        r#"
        UPDATE kb_resources
           SET title    = $1,
               slug     = $2,
               updated  = now()
         WHERE id = $3
           AND is_active = true
        "#,
        new_title,
        new_slug,
        resource_id,
    )
    .execute(pool)
    .await?;

    get_visible(pool, profile_id, resource_id).await
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
