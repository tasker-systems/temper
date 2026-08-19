//! Shared seeding helpers for the subscription/intake/delivery test suites.
//!
//! Extracted from `intake_service`'s test module when `delivery_service` (S2 chunk C) needed the
//! same world: an admin, a team, a team-owned context, a connection with a reach grant, and a
//! subscription against it. Two copies of a six-step seed drift the moment the authz gate gains a
//! leg — which it did once already between chunks A and B — so this is one definition, used by
//! both.
//!
//! Test-only and DB-gated: compiled solely under `cfg(feature = "test-db")`, so a no-DB
//! `cargo make test` neither builds nor links it. The gate is on the feature alone rather than
//! `all(test, ...)` because S3's transport suite lives in temper-api and needs the same world —
//! and a second copy of a six-step seed drifts the moment the authz gate gains a leg, which is
//! the exact reason this module was extracted in the first place.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::connection::ProvisionConnectionRequest;
use temper_core::types::ids::ProfileId;
use temper_core::types::subscription::{CreateSubscriptionRequest, SubscriptionSelector};

pub const GITHUB_REPO: &str = "acme/temper";

/// Seed a system admin and return its profile id. Mirrors subscription_service::seed_admin.
pub async fn seed_admin(pool: &PgPool) -> ProfileId {
    let id = Uuid::now_v7();
    let handle = format!("admin-{id}");
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
        id,
        &handle,
        &handle,
    )
    .execute(pool)
    .await
    .expect("seed admin profile");

    let mut conn = pool.acquire().await.expect("acquire");
    crate::services::profile_service::provision_profile_entities(&mut conn, id, &handle)
        .await
        .expect("provision caller emitters");
    drop(conn);

    let team: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("gating team");

    sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
        .execute(pool)
        .await
        .expect("configure gating team");

    sqlx::query!(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         VALUES ($1, $2, 'owner'::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        team,
        id,
    )
    .execute(pool)
    .await
    .expect("join gating team as owner");

    crate::test_support::approved_admin(pool, id).await;
    ProfileId::from(id)
}

pub async fn seed_team(pool: &PgPool, owner: ProfileId) -> Uuid {
    let team_id = Uuid::now_v7();
    let slug = format!("team-{team_id}");
    sqlx::query!(
        r#"INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $3)"#,
        team_id,
        &slug,
        &slug,
    )
    .execute(pool)
    .await
    .expect("seed team");
    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
           VALUES ($1, $2, 'owner'::team_role)"#,
        team_id,
        *owner,
    )
    .execute(pool)
    .await
    .expect("seed team owner");
    team_id
}

pub async fn seed_context_owned_by_team(pool: &PgPool, team_id: Uuid) -> Uuid {
    let context_id = Uuid::now_v7();
    let slug = format!("ctx-{context_id}");
    sqlx::query!(
        r#"INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
           VALUES ($1, 'kb_teams', $2, $3, $4)"#,
        context_id,
        team_id,
        &slug,
        &slug,
    )
    .execute(pool)
    .await
    .expect("seed context");
    sqlx::query!(
        r#"INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)"#,
        context_id,
        team_id,
    )
    .execute(pool)
    .await
    .expect("seed team_contexts");
    context_id
}

pub async fn seed_connection(
    pool: &PgPool,
    owner_team_id: Option<Uuid>,
    caller: ProfileId,
) -> Uuid {
    let req = ProvisionConnectionRequest {
        provider: "github".into(),
        name: format!("test-conn-{}", Uuid::now_v7()),
        owner_team_id,
        reach_granularity: None,
        reach_covers: None,
    };
    let conn = crate::services::connection_service::provision(pool, caller, &req)
        .await
        .expect("seed connection");
    conn.id
}

pub async fn grant_reach(pool: &PgPool, caller: ProfileId, connection_id: Uuid, team_id: Uuid) {
    crate::services::connection_service::grant_reach(pool, caller, connection_id, team_id, None)
        .await
        .expect("grant reach");
}

pub async fn create_subscription(
    pool: &PgPool,
    caller: ProfileId,
    subscriber_table: &str,
    subscriber_id: Uuid,
    authoring_team_id: Uuid,
    connection_id: Uuid,
    selector: SubscriptionSelector,
) -> Uuid {
    let req = CreateSubscriptionRequest {
        subscriber_table: subscriber_table.into(),
        subscriber_id,
        authoring_team_id,
        connection_id,
        selector,
    };
    let sub = crate::services::subscription_service::create(pool, caller, &req)
        .await
        .expect("create subscription");
    sub.id
}

/// A GitHub pull_request webhook payload for `acme/temper`.
pub fn github_pr_payload(repo: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "opened",
        "repository": { "full_name": repo },
        "pull_request": { "number": 42 }
    })
}

/// A Linear issue webhook payload for project `proj-123`.
pub fn linear_issue_payload(project_id: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "update",
        "data": { "project": { "id": project_id } }
    })
}

/// Read the references column for an event id.
pub async fn event_references(pool: &PgPool, event_id: Uuid) -> serde_json::Value {
    sqlx::query_scalar!(
        r#"SELECT "references" FROM kb_events WHERE id = $1"#,
        event_id,
    )
    .fetch_one(pool)
    .await
    .expect("read references")
}

/// Count kb_events rows for a connection's emitter.
pub async fn event_count_for_connection(pool: &PgPool, connection_id: Uuid) -> i64 {
    let count: Option<i64> = sqlx::query_scalar!(
        r#"SELECT count(*)::bigint FROM kb_events e
            JOIN kb_connections c ON c.id = $1 AND e.emitter_entity_id = c.emitter_entity_id"#,
        connection_id,
    )
    .fetch_one(pool)
    .await
    .expect("count events");
    count.unwrap_or(0)
}
/// A profile that manages nothing — the "stranger" in an authz refusal test.
pub async fn seed_plain_profile(pool: &PgPool) -> ProfileId {
    let id = Uuid::now_v7();
    let handle = format!("stranger-{id}");
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
        id,
        &handle,
        &handle,
    )
    .execute(pool)
    .await
    .expect("seed plain profile");
    ProfileId::from(id)
}

/// Register the remote event kinds a connection receives — the ledger-capable tier. Written
/// directly rather than through `connection_service`, because these tests care about the column's
/// VALUE, not about the path that sets it.
pub async fn register_webhook_events(pool: &PgPool, connection_id: Uuid, events: &[&str]) {
    let owned: Vec<String> = events.iter().map(|s| s.to_string()).collect();
    sqlx::query!(
        "UPDATE kb_connections SET webhook_events = $2 WHERE id = $1",
        connection_id,
        &owned,
    )
    .execute(pool)
    .await
    .expect("register webhook events");
}

/// Give a connection a credential, moving it off the `needs_credential` birth state. Written
/// directly: these tests care that `credential IS NOT NULL`, not about the broker seam that
/// normally sets it.
pub async fn attach_stub_credential(pool: &PgPool, connection_id: Uuid) {
    attach_credential_with_connector(pool, connection_id, "stub").await;
}

/// [`attach_stub_credential`] with a caller-chosen connector uid — the value
/// `connection_service::resolve_inbound` keys on, so a transport test can make a verified
/// attestation resolve (or deliberately fail to resolve) to a known connection.
pub async fn attach_credential_with_connector(pool: &PgPool, connection_id: Uuid, connector: &str) {
    sqlx::query!(
        r#"UPDATE kb_connections
              SET credential = jsonb_build_object('broker', 'test', 'connector', $2::text)
            WHERE id = $1"#,
        connection_id,
        connector,
    )
    .execute(pool)
    .await
    .expect("attach credential");
}
