//! Differential guard for the `kb_connections` reach-grant widening (S1 chunk B2, migration
//! `20260714000020_connection_reach_grants.sql`). That migration relaxes ONE CHECK —
//! `kb_access_grants.subject_table` gains `'kb_connections'` — and touches no SQL function. This
//! test proves both halves of that claim:
//!
//!   BELT — the widening changes NOTHING for the eight existing authz readers. A `kb_connections`
//!   grant row's presence must leave every reader's output byte-identical for the seeded principals.
//!   The eight readers (their definitions live in the migrations named in each snapshot field):
//!     • resources_visible_to        (20260712000010)   — set
//!     • contexts_readable_by        (20260712000010)   — set
//!     • edges_visible_to            (20260712000010)   — set
//!     • resources_in_team_scope     (20260712000010)   — set
//!     • cogmap_visible_maps         (20260701000002)   — set
//!     • vis_team                    (20260701000003)   — set
//!     • can_modify_resource         (20260712000020)   — bool (resource subject)
//!     • profile_explicit_grant      (20260630000001)   — bool (resource / context / cogmap subject)
//!
//!   BRACES — the grant is not a no-op: it becomes LIVE through `can()` with zero function edit,
//!   because `profile_explicit_grant` is already polymorphic on its subject_table parameter. A
//!   `kb_connections` read grant to a team the profile is on flips `can(read)` false→true while
//!   `can(write)` stays false (read-only reach confers no write).
//!
//!   CHECK PROOF — the widened CHECK admits `'kb_connections'` and still rejects a bogus subject
//!   with SQLSTATE 23514.
//!
//! Belt-and-braces, mirroring `edges_visible_to_equivalence_test.rs`: runtime `sqlx::query` (no
//! macros, no `.sqlx` cache), a fresh migrated DB per test via `#[sqlx::test]`, set-equality over
//! `HashSet<Uuid>` with symmetric-difference diagnostics on failure, plus hand-computed semantic pins.
#![cfg(feature = "test-db")]

mod common;

use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

// --- fixture builders (same shapes as edges_visible_to_equivalence_test.rs) ---------------------

async fn any_event(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("bootstrap event for FK")
}

async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(format!("{slug}-{}", Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("insert team")
}

async fn add_member(pool: &PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("insert membership");
}

async fn mk_resource(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(format!("test://conn-eq/{}", Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("insert resource")
}

async fn mk_cogmap(pool: &PgPool, name: &str) -> Uuid {
    let telos = mk_resource(pool, &format!("{name}-telos")).await;
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("insert cogmap")
}

async fn mk_context(pool: &PgPool, owner_table: &str, owner_id: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) VALUES ($1, $2, $3, $3) RETURNING id",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(format!("{slug}-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert context")
}

async fn home(pool: &PgPool, resource: Uuid, anchor_table: &str, anchor_id: Uuid, owner: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(resource)
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert home");
}

async fn join_cogmap_to_team(pool: &PgPool, team: Uuid, cogmap: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(pool)
        .await
        .expect("join cogmap to team");
}

async fn team_read_grant(
    pool: &PgPool,
    subject_table: &str,
    subject_id: Uuid,
    team: Uuid,
    granted_by: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, granted_by_profile_id, can_read)
         VALUES ($1, $2, 'kb_teams', $3, $4, true)",
    )
    .bind(subject_table)
    .bind(subject_id)
    .bind(team)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("insert team read grant");
}

async fn mk_edge(pool: &PgPool, event: Uuid, endpoint: Uuid, home_ctx: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_edges
           (source_table, source_id, target_table, target_id, edge_kind,
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded)
         VALUES ('kb_resources', $1, 'kb_resources', $1, 'express', 'kb_contexts', $2, $3, $3, false)
         RETURNING id",
    )
    .bind(endpoint)
    .bind(home_ctx)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert edge")
}

// --- reader queries (one per authz reader under test) -------------------------------------------

async fn set_of(pool: &PgPool, sql: &str, profile: Uuid) -> HashSet<Uuid> {
    sqlx::query_scalar::<_, Uuid>(sql)
        .bind(profile)
        .fetch_all(pool)
        .await
        .expect("set reader query")
        .into_iter()
        .collect()
}

async fn resources_visible_to(pool: &PgPool, profile: Uuid) -> HashSet<Uuid> {
    set_of(
        pool,
        "SELECT resource_id FROM resources_visible_to($1)",
        profile,
    )
    .await
}

async fn contexts_readable_by(pool: &PgPool, profile: Uuid) -> HashSet<Uuid> {
    set_of(
        pool,
        "SELECT context_id FROM contexts_readable_by($1)",
        profile,
    )
    .await
}

async fn edges_visible_to(pool: &PgPool, profile: Uuid) -> HashSet<Uuid> {
    set_of(pool, "SELECT edge_id FROM edges_visible_to($1)", profile).await
}

async fn cogmap_visible_maps(pool: &PgPool, profile: Uuid) -> HashSet<Uuid> {
    set_of(
        pool,
        "SELECT cogmap_visible_maps AS id FROM cogmap_visible_maps($1)",
        profile,
    )
    .await
}

async fn resources_in_team_scope(pool: &PgPool, profile: Uuid, team: Uuid) -> HashSet<Uuid> {
    sqlx::query_scalar::<_, Uuid>("SELECT resource_id FROM resources_in_team_scope($1, $2)")
        .bind(profile)
        .bind(team)
        .fetch_all(pool)
        .await
        .expect("resources_in_team_scope query")
        .into_iter()
        .collect()
}

async fn vis_team(pool: &PgPool, team: Uuid) -> HashSet<Uuid> {
    sqlx::query_scalar::<_, Uuid>("SELECT resource_id FROM vis_team($1)")
        .bind(team)
        .fetch_all(pool)
        .await
        .expect("vis_team query")
        .into_iter()
        .collect()
}

async fn can_modify_resource(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("can_modify_resource query")
}

async fn profile_explicit_grant(
    pool: &PgPool,
    profile: Uuid,
    action: &str,
    subject_table: &str,
    subject: Uuid,
) -> bool {
    sqlx::query_scalar("SELECT profile_explicit_grant($1, $2, $3, $4)")
        .bind(profile)
        .bind(action)
        .bind(subject_table)
        .bind(subject)
        .fetch_one(pool)
        .await
        .expect("profile_explicit_grant query")
}

async fn can(
    pool: &PgPool,
    principal_table: &str,
    principal: Uuid,
    action: &str,
    subject_table: &str,
    subject: Uuid,
) -> bool {
    sqlx::query_scalar("SELECT can($1, $2, $3, $4, $5)")
        .bind(principal_table)
        .bind(principal)
        .bind(action)
        .bind(subject_table)
        .bind(subject)
        .fetch_one(pool)
        .await
        .expect("can query")
}

/// The whole authz-reader surface for the seeded principals, captured at one instant.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    visible: HashSet<Uuid>,
    contexts: HashSet<Uuid>,
    edges: HashSet<Uuid>,
    team_scope: HashSet<Uuid>,
    cogmaps: HashSet<Uuid>,
    vis_team: HashSet<Uuid>,
    can_modify: bool,
    pg_resource: bool,
    pg_context: bool,
    pg_cogmap: bool,
}

/// Subjects the scalar readers are probed against — held stable across before/after snapshots so the
/// two snapshots compare like-for-like.
struct Probes {
    r_owned: Uuid,
    r_grant: Uuid,
    c_grant: Uuid,
    m_grant: Uuid,
}

async fn snapshot(pool: &PgPool, viewer: Uuid, team: Uuid, p: &Probes) -> Snapshot {
    Snapshot {
        visible: resources_visible_to(pool, viewer).await,
        contexts: contexts_readable_by(pool, viewer).await,
        edges: edges_visible_to(pool, viewer).await,
        team_scope: resources_in_team_scope(pool, viewer, team).await,
        cogmaps: cogmap_visible_maps(pool, viewer).await,
        vis_team: vis_team(pool, team).await,
        can_modify: can_modify_resource(pool, viewer, p.r_owned).await,
        pg_resource: profile_explicit_grant(pool, viewer, "read", "kb_resources", p.r_grant).await,
        pg_context: profile_explicit_grant(pool, viewer, "read", "kb_contexts", p.c_grant).await,
        pg_cogmap: profile_explicit_grant(pool, viewer, "read", "kb_cogmaps", p.m_grant).await,
    }
}

/// Insert a `kb_connections` reach grant to `team`. `subject_id` is a fresh random uuid — there is NO
/// FK on the (subject_table, subject_id) pair (the polymorphic-anchor idiom; integrity is the CHECK +
/// the granting path), so this SQL-level test needs no real `kb_connections` row.
async fn insert_connection_grant(
    pool: &PgPool,
    subject: Uuid,
    team: Uuid,
    granted_by: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, granted_by_profile_id, can_read)
         VALUES ('kb_connections', $1, 'kb_teams', $2, $3, true)",
    )
    .bind(subject)
    .bind(team)
    .bind(granted_by)
    .execute(pool)
    .await
    .map(|_| ())
}

// ------------------------------------------------------------------------------------------------

/// BELT + BRACES: a connection grant changes nothing for the eight readers, yet is honored by can().
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn connection_grant_is_inert_for_the_eight_readers_yet_live_via_can(pool: PgPool) {
    // create_test_profile mints the profile + its personal team + a profile-owned `temper` context,
    // so several readers are already non-empty before we add anything.
    let viewer = common::fixtures::create_test_profile(
        &pool,
        &format!("viewer-{}@conn.test", Uuid::new_v4()),
    )
    .await;
    let event = any_event(&pool).await;

    // A team viewer is a member of — this is the connection grant's principal (reach flows
    // viewer → team membership → profile_explicit_grant's reachable_teams arm).
    let team = mk_team(&pool, "conn-team").await;
    add_member(&pool, team, viewer).await;

    // Enough seeded state that each reader returns something meaningful.
    // resources_visible_to / can_modify_resource: a resource viewer owns, homed in a readable context.
    let c_owned = mk_context(&pool, "kb_profiles", viewer, "conn-owned-ctx").await;
    let r_owned = mk_resource(&pool, "conn-r-owned").await;
    home(&pool, r_owned, "kb_contexts", c_owned, viewer).await;

    // contexts_readable_by / resources_in_team_scope: a team-owned context + a resource homed in it.
    let c_team = mk_context(&pool, "kb_teams", team, "conn-team-ctx").await;
    let r_team = mk_resource(&pool, "conn-r-team").await;
    home(&pool, r_team, "kb_contexts", c_team, viewer).await;

    // cogmap_visible_maps: a map joined to the team.
    let m_joined = mk_cogmap(&pool, "conn-m-joined").await;
    join_cogmap_to_team(&pool, team, m_joined).await;

    // edges_visible_to: an edge homed in a readable context with a visible endpoint.
    let _edge = mk_edge(&pool, event, r_owned, c_owned).await;

    // profile_explicit_grant / vis_team: existing (pre-connection) team read-grants on ONE subject of
    // each already-admitted kind. These make the scalar oracles non-trivially true and give vis_team a
    // row — all part of the BEFORE baseline.
    let r_grant = mk_resource(&pool, "conn-r-grant").await;
    let c_grant = mk_context(&pool, "kb_profiles", viewer, "conn-c-grant").await;
    let m_grant = mk_cogmap(&pool, "conn-m-grant").await;
    team_read_grant(&pool, "kb_resources", r_grant, team, viewer).await;
    team_read_grant(&pool, "kb_contexts", c_grant, team, viewer).await;
    team_read_grant(&pool, "kb_cogmaps", m_grant, team, viewer).await;

    let probes = Probes {
        r_owned,
        r_grant,
        c_grant,
        m_grant,
    };

    // Sanity: the baseline is non-empty / non-trivial, so the belt is not vacuously satisfied.
    let before = snapshot(&pool, viewer, team, &probes).await;
    assert!(
        before.visible.contains(&r_owned),
        "baseline: viewer sees owned resource"
    );
    assert!(
        before.contexts.contains(&c_team),
        "baseline: viewer reads team-owned context"
    );
    assert!(
        !before.edges.is_empty(),
        "baseline: viewer sees the seeded edge"
    );
    assert!(
        before.team_scope.contains(&r_team),
        "baseline: team-scope holds the team-ctx resource"
    );
    assert!(
        before.cogmaps.contains(&m_joined),
        "baseline: viewer sees the joined map"
    );
    assert!(
        before.vis_team.contains(&r_grant),
        "baseline: vis_team holds the team-granted resource"
    );
    assert!(
        before.can_modify,
        "baseline: viewer can modify owned resource"
    );
    assert!(
        before.pg_resource && before.pg_context && before.pg_cogmap,
        "baseline: grants resolve"
    );

    // BRACES precondition: no connection grant yet ⇒ can() denies the connection subject.
    let conn_subject = Uuid::new_v4();
    assert!(
        !can(
            &pool,
            "kb_profiles",
            viewer,
            "read",
            "kb_connections",
            conn_subject
        )
        .await,
        "before the grant, can(read, kb_connections) must be false"
    );

    // Land the connection reach grant.
    insert_connection_grant(&pool, conn_subject, team, viewer)
        .await
        .expect("kb_connections grant must insert after the widened CHECK");

    // BELT: every reader is byte-identical for the seeded principals.
    let after = snapshot(&pool, viewer, team, &probes).await;
    let diff = |a: &HashSet<Uuid>, b: &HashSet<Uuid>| {
        (
            a.difference(b).copied().collect::<Vec<_>>(),
            b.difference(a).copied().collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        after.visible,
        before.visible,
        "resources_visible_to drifted: {:?}",
        diff(&after.visible, &before.visible)
    );
    assert_eq!(
        after.contexts,
        before.contexts,
        "contexts_readable_by drifted: {:?}",
        diff(&after.contexts, &before.contexts)
    );
    assert_eq!(
        after.edges,
        before.edges,
        "edges_visible_to drifted: {:?}",
        diff(&after.edges, &before.edges)
    );
    assert_eq!(
        after.team_scope,
        before.team_scope,
        "resources_in_team_scope drifted: {:?}",
        diff(&after.team_scope, &before.team_scope)
    );
    assert_eq!(
        after.cogmaps,
        before.cogmaps,
        "cogmap_visible_maps drifted: {:?}",
        diff(&after.cogmaps, &before.cogmaps)
    );
    assert_eq!(
        after.vis_team,
        before.vis_team,
        "vis_team drifted: {:?}",
        diff(&after.vis_team, &before.vis_team)
    );
    assert_eq!(
        after.can_modify, before.can_modify,
        "can_modify_resource drifted"
    );
    assert_eq!(
        after.pg_resource, before.pg_resource,
        "profile_explicit_grant(kb_resources) drifted"
    );
    assert_eq!(
        after.pg_context, before.pg_context,
        "profile_explicit_grant(kb_contexts) drifted"
    );
    assert_eq!(
        after.pg_cogmap, before.pg_cogmap,
        "profile_explicit_grant(kb_cogmaps) drifted"
    );
    // The whole struct, for good measure (one diff surface if any field slipped past above).
    assert_eq!(
        after, before,
        "connection grant must leave the whole reader surface unchanged"
    );

    // BRACES: the grant IS live through can() — polymorphic profile_explicit_grant, zero function edit.
    assert!(
        can(
            &pool,
            "kb_profiles",
            viewer,
            "read",
            "kb_connections",
            conn_subject
        )
        .await,
        "after the grant, can(read, kb_connections) must be true (reach via team membership)"
    );
    // A can_read-only reach confers no write.
    assert!(
        !can(
            &pool,
            "kb_profiles",
            viewer,
            "write",
            "kb_connections",
            conn_subject
        )
        .await,
        "a read-only connection grant must not confer write"
    );
}

/// CHECK PROOF: the widened constraint admits 'kb_connections' and still rejects a bogus subject_table.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn subject_table_check_admits_connections_rejects_bogus(pool: PgPool) {
    let granter = common::fixtures::create_test_profile(
        &pool,
        &format!("granter-{}@conn.test", Uuid::new_v4()),
    )
    .await;
    let team = mk_team(&pool, "conn-check-team").await;

    // (a) A kb_connections grant is accepted (the point of the migration).
    insert_connection_grant(&pool, Uuid::new_v4(), team, granter)
        .await
        .expect("kb_connections must be admitted by the widened CHECK");

    // (b) A bogus subject_table still raises check_violation (23514).
    let res = sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, granted_by_profile_id, can_read)
         VALUES ('kb_bogus', $1, 'kb_teams', $2, $3, true)",
    )
    .bind(Uuid::new_v4())
    .bind(team)
    .bind(granter)
    .execute(&pool)
    .await;

    let err = res.expect_err("a bogus subject_table must be rejected");
    let is_check_violation = matches!(
        &err,
        sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")
    );
    assert!(
        is_check_violation,
        "expected check_violation (23514), got {err:?}"
    );
}
