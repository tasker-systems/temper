#![cfg(feature = "test-db")]

//! The public_within machine fence (`public_within_reading`) asks the one shared person test,
//! `profile_is_person`, and holds at a read door for every machine shape production can build.
//!
//! Each principal is built through the path production builds it with: people through the
//! federated human door, machines through registration (`provision` under an external IdP,
//! `issue` under temper's own authorization server), connection agents through connection
//! provisioning, revocation through the revoke door. Whatever those paths persist is what the
//! fence reads. The one hand-written principal fact is a stray human link on a machine and on an
//! agent, so that each of the fence's clauses is seen deciding alone. A dynamically registered
//! client gets no profile at all, and its token is refused before one could exist: that is pinned
//! beside the machine gate, in
//! `profile_service::tests::a_dynamically_registered_client_is_refused_and_creates_no_profile`.
//!
//! The world is small: `anchor ─ readers` and `anchor ─ t`, T's row anchored on `anchor`. Every
//! reader here sits on `readers`, so the anchor is in each reader's closure, no membership reaches
//! T, and the fence is the only thing that differs between them.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::connection::ProvisionConnectionRequest;
use temper_core::types::machine::{IssueMachineRequest, ProvisionMachineRequest};
use temper_services::auth_config::AuthMode;
use temper_services::services::{
    connection_service, machine_client_service, machine_registration_service, slack_link_service,
};
use temper_services::{auth, test_support};

/// The provider the federated human door is handed in these tests.
const HUMAN_PROVIDER: &str = "test-idp";

struct World {
    /// The team every reader here sits on: T's sibling under the anchor.
    readers: Uuid,
    ctx_t: Uuid,
    admin: Uuid,
}

async fn team(pool: &PgPool, slug: &str, parent: Option<Uuid>) -> Uuid {
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .expect("team");
    if let Some(p) = parent {
        sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
            .bind(p)
            .bind(id)
            .execute(pool)
            .await
            .expect("link");
    }
    id
}

async fn join(pool: &PgPool, team: Uuid, who: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member') \
         ON CONFLICT DO NOTHING",
    )
    .bind(team)
    .bind(who)
    .execute(pool)
    .await
    .expect("join");
}

/// A person, signed in through the federated human door (`resolve_federated_human`), the seam SAML
/// and OAuth share: it creates the profile, its login link and its emitters.
async fn person(pool: &PgPool, handle: &str) -> Uuid {
    let id = auth::resolve_federated_human(
        pool,
        HUMAN_PROVIDER,
        &format!("{HUMAN_PROVIDER}|{handle}"),
        &format!("{handle}@example.test"),
        Some(true),
    )
    .await
    .expect("human door")
    .id;
    test_support::approve(pool, id).await;
    id
}

/// A machine client registered under an external IdP: an `auth0-m2m` registration.
async fn provisioned_machine(pool: &PgPool, admin: Uuid, client_id: &str) -> (Uuid, Uuid) {
    let client = machine_registration_service::provision(
        pool,
        &test_support::human_principal_for(pool, admin).await,
        &ProvisionMachineRequest {
            client_id: client_id.to_owned(),
            label: client_id.to_owned(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        AuthMode::ExternalIdp,
    )
    .await
    .expect("provision machine");
    (client.id, client.profile_id)
}

/// A machine client temper's own authorization server minted.
async fn issued_machine(pool: &PgPool, admin: Uuid, label: &str) -> Uuid {
    machine_registration_service::issue(
        pool,
        &test_support::human_principal_for(pool, admin).await,
        &IssueMachineRequest {
            label: label.to_owned(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        AuthMode::TemperAs,
    )
    .await
    .expect("issue machine")
    .client
    .profile_id
}

/// A connection's agent profile, through connection provisioning.
async fn connection_agent(pool: &PgPool, admin: Uuid, name: &str) -> Uuid {
    let conn = connection_service::provision(
        pool,
        &test_support::human_principal_for(pool, admin).await,
        &ProvisionConnectionRequest {
            provider: "github".into(),
            name: name.into(),
            owner_team_id: None,
            reach_granularity: None,
            reach_covers: None,
        },
    )
    .await
    .expect("provision connection");
    sqlx::query_scalar("SELECT profile_id FROM kb_connections WHERE id = $1")
        .bind(conn.id)
        .fetch_one(pool)
        .await
        .expect("agent profile")
}

async fn world(pool: &PgPool) -> World {
    let anchor = team(pool, "anchor", None).await;
    let readers = team(pool, "readers", Some(anchor)).await;
    let t = team(pool, "t", Some(anchor)).await;
    let ctx_t: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'ctx-t', 'ctx-t') RETURNING id",
    )
    .bind(t)
    .fetch_one(pool)
    .await
    .expect("ctx-t");
    let admin = person(pool, "admin").await;
    test_support::approved_admin(pool, admin).await;
    sqlx::query(
        "INSERT INTO kb_team_public_within \
           (team_id, anchor_id, opened_by, instance_wide, admin_act) \
         VALUES ($1, $2, $3, false, false)",
    )
    .bind(t)
    .bind(anchor)
    .bind(admin)
    .execute(pool)
    .await
    .expect("open public_within row");
    World {
        readers,
        ctx_t,
        admin,
    }
}

async fn is_machine(pool: &PgPool, who: Uuid) -> bool {
    sqlx::query_scalar("SELECT is_machine_profile($1)")
        .bind(who)
        .fetch_one(pool)
        .await
        .expect("is_machine_profile")
}

/// `who`, on the readers' team: (the fence's answer, whether the context door reads T's context).
async fn reads_t(pool: &PgPool, w: &World, who: Uuid) -> (bool, bool) {
    join(pool, w.readers, who).await;
    let fence: bool = sqlx::query_scalar("SELECT public_within_reading($1)")
        .bind(who)
        .fetch_one(pool)
        .await
        .expect("fence");
    let door: bool = sqlx::query_scalar("SELECT context_visible_to($1, $2)")
        .bind(who)
        .bind(w.ctx_t)
        .fetch_one(pool)
        .await
        .expect("context door");
    (fence, door)
}

/// Every machine production can register is a machine to the shared test, and reads nothing
/// through public_within: a provisioned client, an issued client, a revoked client, and a
/// connection's agent. As built, each of these also lacks a human auth link, which alone keeps
/// the fence false; [`each_clause_fences_on_its_own`] pins the machine and connection clauses.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_machine_shape_is_fenced(pool: PgPool) {
    let w = world(&pool).await;

    let (_, provisioned) = provisioned_machine(&pool, w.admin, "fence-provisioned").await;
    assert!(is_machine(&pool, provisioned).await);
    assert_eq!(
        reads_t(&pool, &w, provisioned).await,
        (false, false),
        "a provisioned (external IdP) machine"
    );

    let issued = issued_machine(&pool, w.admin, "fence-issued").await;
    assert!(is_machine(&pool, issued).await);
    assert_eq!(
        reads_t(&pool, &w, issued).await,
        (false, false),
        "an issued (temper's own authorization server) machine"
    );

    let (client, revoked) = provisioned_machine(&pool, w.admin, "fence-revoked").await;
    machine_client_service::revoke(
        &pool,
        client,
        &test_support::human_principal_for(&pool, w.admin).await,
    )
    .await
    .expect("revoke");
    assert!(
        is_machine(&pool, revoked).await,
        "a revoked machine is still a machine"
    );
    assert_eq!(
        reads_t(&pool, &w, revoked).await,
        (false, false),
        "a revoked machine"
    );

    let agent = connection_agent(&pool, w.admin, "fence-conn").await;
    assert!(
        !is_machine(&pool, agent).await,
        "a connection agent holds no client row"
    );
    assert_eq!(
        reads_t(&pool, &w, agent).await,
        (false, false),
        "a connection agent"
    );
}

/// No door gives a machine or a connection agent a human auth link, but a hand-written row could.
/// With one, the human-link clause passes, so the person test decides: its machine arm for a
/// machine, revoked or not, and its connection arm for an agent. Fails if either arm is dropped
/// from `profile_is_person`, or if the fence stops asking it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn each_clause_fences_on_its_own(pool: PgPool) {
    let w = world(&pool).await;
    let stray_link = |who: Uuid, sub: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO kb_profile_auth_links \
                   (id, profile_id, auth_provider, auth_provider_user_id) \
                 VALUES (uuid_generate_v7(), $1, $2, $3)",
            )
            .bind(who)
            .bind(HUMAN_PROVIDER)
            .bind(sub)
            .execute(&pool)
            .await
            .expect("stray human link");
        }
    };

    let (client, machine) = provisioned_machine(&pool, w.admin, "fence-linked-machine").await;
    stray_link(machine, "stray|machine").await;
    assert_eq!(
        reads_t(&pool, &w, machine).await,
        (false, false),
        "a machine with a human link: the person test's machine arm decides"
    );
    machine_client_service::revoke(
        &pool,
        client,
        &test_support::human_principal_for(&pool, w.admin).await,
    )
    .await
    .expect("revoke");
    assert_eq!(
        reads_t(&pool, &w, machine).await,
        (false, false),
        "revoked, still a machine"
    );

    let agent = connection_agent(&pool, w.admin, "fence-linked-conn").await;
    stray_link(agent, "stray|agent").await;
    assert_eq!(
        reads_t(&pool, &w, agent).await,
        (false, false),
        "a connection agent with a human link: the person test's connection arm decides"
    );
}

/// A person reads T, and linking Slack (a person's act, which the mention path's act-as-the-human
/// mint relies on) changes nothing: the fence is keyed on the profile, so any token that resolves
/// to the person reads as the person.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_person_reads_and_a_slack_link_changes_nothing(pool: PgPool) {
    let w = world(&pool).await;
    let dana = person(&pool, "dana").await;
    assert_eq!(reads_t(&pool, &w, dana).await, (true, true), "a person");

    let linker = test_support::human_principal_for(&pool, dana).await;
    let mut conn = pool.acquire().await.unwrap();
    let outcome =
        slack_link_service::link_slack_principal(&mut conn, &linker, "slack:T0FENCE:UDANA")
            .await
            .expect("link slack");
    drop(conn);
    assert!(matches!(
        outcome,
        slack_link_service::SlackLinkOutcome::Linked
    ));
    assert_eq!(
        reads_t(&pool, &w, dana).await,
        (true, true),
        "a Slack-linked person"
    );
}

/// Drift guard: the fence's person test is the shared one, not a copy. Fails if
/// `public_within_reading` stops calling `profile_is_person` or reads `kb_machine_clients` or
/// `kb_connections` itself, or if `profile_is_person` stops excluding both.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_fence_asks_the_shared_person_test(pool: PgPool) {
    let def = |sig: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(&format!(
                "SELECT pg_get_functiondef('{sig}'::regprocedure)"
            ))
            .fetch_one(&pool)
            .await
            .expect("definition")
        }
    };
    let fence = def("public_within_reading(uuid)").await;
    assert!(
        fence.contains("profile_is_person(p_profile)"),
        "public_within_reading must call profile_is_person:\n{fence}"
    );
    for copy in ["kb_machine_clients", "kb_connections", "is_machine_profile"] {
        assert!(
            !fence.contains(copy),
            "public_within_reading must not test `{copy}` itself:\n{fence}"
        );
    }
    assert!(
        fence.contains("search_path"),
        "public_within_reading pins its search_path:\n{fence}"
    );
    let person = def("profile_is_person(uuid)").await;
    assert!(
        person.contains("is_machine_profile(p_profile)") && person.contains("kb_connections"),
        "profile_is_person excludes machines and connection agents:\n{person}"
    );
}
