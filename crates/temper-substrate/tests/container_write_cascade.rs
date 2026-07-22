#![cfg(feature = "artifact-tests")]
//! Container-write → node-write cascade (decision + spec:
//! `docs/superpowers/specs/2026-07-06-container-write-cascade-and-authz-hardening-design.md`).
//!
//! Pins the predicate-level decision directly: whoever may author a container (cogmap or context)
//! may modify any node homed in it (unix directory-write ⇒ file-rwx), regardless of the node's own
//! owner/originator. Two SQL surfaces:
//!
//!   1. `context_authorable_by_profile` — NEW: personal-owner OR reachable-member-of-owning-team OR
//!      explicit write grant. The team-owner arm is deliberate (owning ≠ the Q-A joined-for-read case).
//!   2. `can_modify_resource` — gains a container-cascade arm branching on `kb_resource_homes.anchor_table`.
//!
//! Minimal-anchor pattern (mirrors `access_grants_seam`): each `#[sqlx::test]` gets a fresh migrated
//! `public`-schema DB; runtime `sqlx::query` (no macros, no per-crate `.sqlx` cache); no ONNX.

use uuid::Uuid;

// ── minimal anchors ─────────────────────────────────────────────────────────────────

/// A profile that reaches nothing by default (`system_access='none'` skips the root-join trigger).
async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_resource(pool: &sqlx::PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id")
        .bind(title)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A cognitive map (ownerless). Needs a telos resource to satisfy the FK.
async fn insert_cogmap(pool: &sqlx::PgPool, name: &str) -> Uuid {
    let telos = insert_resource(pool, &format!("{name}-telos")).await;
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A context owned by a profile or a team (`owner_table` ∈ {kb_profiles, kb_teams}).
async fn insert_context(
    pool: &sqlx::PgPool,
    owner_table: &str,
    owner_id: Uuid,
    slug: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ($1, $2, $3, $3) RETURNING id",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Home a resource in a container with an explicit originator/owner.
async fn home_resource(
    pool: &sqlx::PgPool,
    resource: Uuid,
    anchor_table: &str,
    anchor_id: Uuid,
    originator: Uuid,
    owner: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(resource)
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(originator)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (gen_random_uuid(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .unwrap();
}

/// A `can_write` grant on any subject, anchored to a profile or team.
async fn grant_write(
    pool: &sqlx::PgPool,
    subject_table: &str,
    subject: Uuid,
    principal_table: &str,
    principal: Uuid,
    granted_by: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ($1, $2, $3, $4, true, true, $5)",
    )
    .bind(subject_table)
    .bind(subject)
    .bind(principal_table)
    .bind(principal)
    .bind(granted_by)
    .execute(pool)
    .await
    .unwrap();
}

// ── predicate probes ─────────────────────────────────────────────────────────────────

async fn can_modify(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn context_authorable(pool: &sqlx::PgPool, profile: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(profile)
        .bind(context)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ── tests ────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn context_authorable_arms(pool: sqlx::PgPool) {
    let owner = insert_profile(&pool, "ctx-owner").await;
    let stranger = insert_profile(&pool, "ctx-stranger").await;
    let grantee = insert_profile(&pool, "ctx-grantee").await;

    // Personal-owned context: the owner authors it; a stranger does not.
    let personal = insert_context(&pool, "kb_profiles", owner, "personal").await;
    assert!(
        context_authorable(&pool, owner, personal).await,
        "personal-owner authors their own context"
    );
    assert!(
        !context_authorable(&pool, stranger, personal).await,
        "a stranger cannot author someone else's personal context"
    );

    // Explicit write grant lifts the grantee (owner floor is not the only path).
    grant_write(
        &pool,
        "kb_contexts",
        personal,
        "kb_profiles",
        grantee,
        owner,
    )
    .await;
    assert!(
        context_authorable(&pool, grantee, personal).await,
        "an explicit write grant confers authoring"
    );

    // Team-owned context: a reachable member of the OWNING team authors it (deliberate — owning is
    // stronger than the Q-A joined-for-read case); a non-member does not.
    let team = insert_team(&pool, "owning-team").await;
    let member = insert_profile(&pool, "ctx-member").await;
    let outsider = insert_profile(&pool, "ctx-outsider").await;
    add_member(&pool, team, member).await;
    let team_ctx = insert_context(&pool, "kb_teams", team, "team-ctx").await;
    assert!(
        context_authorable(&pool, member, team_ctx).await,
        "a member of the owning team authors a team-owned context"
    );
    assert!(
        !context_authorable(&pool, outsider, team_ctx).await,
        "a non-member cannot author a team-owned context"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_container_write_cascades_to_node_modify(pool: sqlx::PgPool) {
    let steward = insert_profile(&pool, "steward").await;
    let coauthor = insert_profile(&pool, "coauthor").await;
    let reader = insert_profile(&pool, "reader").await;

    // A node the steward originated + owns, homed in the map.
    let cogmap = insert_cogmap(&pool, "shared-map").await;
    let node = insert_resource(&pool, "steward-node").await;
    home_resource(&pool, node, "kb_cogmaps", cogmap, steward, steward).await;

    // Baseline: the originator modifies their own node.
    assert!(
        can_modify(&pool, steward, node).await,
        "originator modifies their own node (arm a)"
    );

    // BEFORE any grant, the co-author cannot modify a node they did not originate.
    assert!(
        !can_modify(&pool, coauthor, node).await,
        "no container write yet ⇒ no cascade"
    );

    // Grant the co-author cogmap write. The cascade now lets them modify the steward's node —
    // THE NEW CAPABILITY (was denied pre-cascade).
    grant_write(
        &pool,
        "kb_cogmaps",
        cogmap,
        "kb_profiles",
        coauthor,
        steward,
    )
    .await;
    assert!(
        can_modify(&pool, coauthor, node).await,
        "cogmap write cascades to modify a node the grantee did not originate"
    );

    // A reader with no write grant still cannot modify — the cascade keys on WRITE, not read.
    assert!(
        !can_modify(&pool, reader, node).await,
        "a mere reader gets no cascade"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn context_container_write_cascades_to_node_modify(pool: sqlx::PgPool) {
    let owner = insert_profile(&pool, "ctx-owner").await;
    let author_b = insert_profile(&pool, "author-b").await;
    let stranger = insert_profile(&pool, "ctx-stranger").await;

    // Owner A's personal context; a DIFFERENT principal B authored a resource into it (home owner = B).
    let ctx = insert_context(&pool, "kb_profiles", owner, "shared-ctx").await;
    let node = insert_resource(&pool, "b-node").await;
    home_resource(&pool, node, "kb_contexts", ctx, author_b, author_b).await;

    // B modifies their own node (arm a).
    assert!(
        can_modify(&pool, author_b, node).await,
        "originator B modifies own node"
    );

    // A owns the container ⇒ A modifies B's node via the cascade (was denied pre-cascade).
    assert!(
        can_modify(&pool, owner, node).await,
        "context owner modifies a node another principal authored in their context"
    );

    // A stranger with no relationship to the container gets nothing.
    assert!(
        !can_modify(&pool, stranger, node).await,
        "a stranger cannot modify a context-homed node"
    );
}
