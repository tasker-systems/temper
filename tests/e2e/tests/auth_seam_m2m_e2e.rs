#![cfg(feature = "test-db")]
//! Stage 4b: a machine (`client_credentials`) token on the MCP surface. Since G3
//! Phase A, registration is fail-closed: an unregistered client is rejected and
//! creates nothing; a registered one resolves to its pre-created agent profile.
//! temper-mcp has no gate of its own (D4): since the network door's teardown it
//! holds no in-process gate at all, so these tests drive a relayed tool act with a
//! real minted machine JWT, and the API's seam adjudicates it — the MCP rendering of
//! that answer is what is pinned here. (They used to call the in-process gate
//! `ensure_profile_from_parts`, which teardown deleted.)

mod common;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unregistered_machine_token_is_rejected_by_the_mcp_gate(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool.clone()).await;

    let err = common::mcp_act_as(&app, &common::generate_machine_jwt("steward-client-1"))
        .await
        .expect_err("an unregistered machine must be rejected through the mcp door");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("not registered"),
        "the mcp surface inherits the services-layer gate (D4): {rendered}"
    );
    // The rejection must be TERMINAL, not a retryable internal error: a permanent auth
    // denial that a Sidekiq worker would otherwise retry forever (temper-rb contract). The
    // terminal INVALID_REQUEST arm is the only one that emits "should not be retried" — the
    // retryable internal_error arm says "Failed to resolve profile" instead — so this
    // substring distinguishes the two without an rmcp dependency here.
    assert!(
        rendered.contains("should not be retried"),
        "a gate rejection must be terminal, not a transient internal_error: {rendered}"
    );

    let links = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider = 'auth0-m2m'",
    )
    .fetch_one(&pool)
    .await
    .expect("count links");
    assert_eq!(links, Some(0), "rejection creates no auth link");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn registered_machine_token_is_admitted_by_the_mcp_gate(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool.clone()).await;

    let profile_id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-steward', 'agent-steward', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ('steward-client-1', 'test', $1, $1)",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed registration");

    // D11: a machine is born Denied; approve so the mcp system gate admits it.
    common::approve(&pool, profile_id).await;

    let created = common::mcp_act_as(&app, &common::generate_machine_jwt("steward-client-1"))
        .await
        .expect("the mcp door must admit a registered machine");
    // Read the act's owner back: a registered machine resolves to ITS OWN
    // pre-created agent profile, not to some ambient identity.
    assert_eq!(
        common::created_context_owner(&pool, &created).await,
        profile_id,
        "the act lands under the machine's own registered profile: {created}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn temper_issued_machine_resolves_on_mcp(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool.clone()).await;

    // A temper-ISSUED row (issuer='temper', with a secret hash), as Phase B1's `issue` path
    // produces. The mcp gate is issuer-agnostic, so it resolves exactly like an auth0-m2m row.
    let profile_id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-tmpr-mcp', 'agent-tmpr-mcp', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients \
           (client_id, issuer, label, profile_id, registered_by_profile_id, secret_hash) \
         VALUES ('tmpr_mcp', 'temper', 'e2e', $1, $1, 'deadbeef')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed temper-issued registration");

    // D11: a machine is born Denied; approve so the mcp system gate admits it.
    common::approve(&pool, profile_id).await;

    let created = common::mcp_act_as(&app, &common::generate_machine_jwt("tmpr_mcp"))
        .await
        .expect("a temper-issued machine resolves on the MCP surface too (D4)");
    assert_eq!(
        common::created_context_owner(&pool, &created).await,
        profile_id,
        "it resolves to its own profile: {created}"
    );
}
