#![cfg(feature = "test-db")]

//! `public_within`: the fixture world, Phase 1 (schema, policy, helpers) and Phase 2 (the read arm
//! and the membership set for every act on T's material).
//!
//! Design: temper-artifacts `specs/2026-11-06-public-within-design.md` (P1, P4–P8, P12; rulings
//! R-a … R-o). Plan: `plans/2026-11-06-public-within.md` Phases 1 and 2, test table 2.5.
//!
//! **The world** (plan 0.3, built here because Phase 0 did not land it):
//!
//! ```text
//! temper-system (root)
//! org ─ eng ─ eng-web                 the readers: an owner, maintainer, member and watcher,
//!     │     │                         a machine client and a connection agent
//!     │     └ eng-sec  (T) ─ eng-sec-red    T's child, which stays private
//!     │                ▲
//!     └ sales          └── ops        a second parent of T
//! ```
//!
//! T owns `ctx-t`. `ctx-shared` (sales') is shared TO T; `ctx-sales` holds a resource granted to T; a
//! cogmap is bound to T with an edge homed in it. `eng-sec-red` owns `ctx-red`. The readers author in
//! `ctx-web`. The row is written directly by [`open`], the plan's fixture helper: the open door is a
//! later phase. The switch is on unless a test says otherwise.
//!
//! **Deviation from plan 0.3, recorded.** The plan's world puts an auto-join `eng-pool` under `eng`.
//! Present by default, it makes every `(eng-sec, eng)` row inert (PW-1, `anchor_subtree_holds_pool`),
//! so every reading test would read nothing. It is created by the tests that need it instead.
//!
//! Resources are created through the real write path with caller-supplied chunks (no ONNX), so the
//! events, blocks and change feed are the production ones.

use std::sync::Arc;

use bytes::Bytes;
use sha2::Digest as _;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::authorship::{ActContext, ActInput};
use temper_core::types::blob::BlobRelationAssertRequest;
use temper_core::types::graph::{EdgeKind as WireEdgeKind, Polarity};
use temper_core::types::ids::{BlockId, ProfileId};
use temper_core::types::provenance::ProvenanceSource;
use temper_services::backend::DbBackend;
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::services::{access_service, blob_service, context_service};
use temper_services::test_support;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::blob_store::InMemoryBlobStore;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::writes::{self, CreateParams};
use temper_workflow::operations::{AssertRelationship, Backend, RecordCitationAudit, Surface};

// =================================================================================================
// Fixture helpers
// =================================================================================================

/// A person: a profile with a human auth link, approved standing, and the surface emitters the
/// write path resolves. `public_within_reading` is true for exactly this shape.
async fn person(pool: &PgPool, handle: &str) -> Uuid {
    person_via(pool, handle, "test-idp").await
}

/// A person whose only auth link is `provider`'s (a Slack-linked person has `slack`).
async fn person_via(pool: &PgPool, handle: &str, provider: &str) -> Uuid {
    let id = profile_with_emitters(pool, handle).await;
    auth_link(pool, id, provider, handle).await;
    test_support::approve(pool, id).await;
    id
}

async fn profile_with_emitters(pool: &PgPool, handle: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("emitter");
    }
    id
}

async fn auth_link(pool: &PgPool, profile: Uuid, provider: &str, sub: &str) {
    sqlx::query(
        "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
         VALUES (uuid_generate_v7(), $1, $2, $3)",
    )
    .bind(profile)
    .bind(provider)
    .bind(sub)
    .execute(pool)
    .await
    .expect("auth link");
}

/// A machine client: the machine provider's link and a registration, as prod carries.
async fn machine(pool: &PgPool, handle: &str, registered_by: Uuid) -> Uuid {
    let id = profile_with_emitters(pool, handle).await;
    auth_link(pool, id, "auth0-m2m", handle).await;
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $1, $2, $3)",
    )
    .bind(handle)
    .bind(id)
    .bind(registered_by)
    .execute(pool)
    .await
    .expect("machine client");
    test_support::approve(pool, id).await;
    id
}

/// A connection's agent profile: no auth link and no machine-client row, by construction
/// (`kb_connections.profile_id`'s comment).
async fn connection_agent(pool: &PgPool, handle: &str, registered_by: Uuid, home: Uuid) -> Uuid {
    let id = profile_with_emitters(pool, handle).await;
    let entity: Uuid = sqlx::query_scalar("SELECT id FROM kb_entities WHERE name = $1")
        .bind(format!("{handle}@web"))
        .fetch_one(pool)
        .await
        .expect("agent entity");
    sqlx::query(
        "INSERT INTO kb_connections \
           (provider, slug, name, registered_by_profile_id, profile_id, emitter_entity_id, \
            home_context_id) \
         VALUES ('github', $1, $1, $2, $3, $4, $5)",
    )
    .bind(handle)
    .bind(registered_by)
    .bind(id)
    .bind(entity)
    .bind(home)
    .execute(pool)
    .await
    .expect("connection");
    id
}

async fn team(pool: &PgPool, slug: &str, parents: &[Uuid]) -> Uuid {
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .expect("team");
    for p in parents {
        link(pool, *p, id).await;
    }
    id
}

async fn link(pool: &PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("link");
}

async fn unlink(pool: &PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("DELETE FROM kb_teams_parents WHERE parent_id = $1 AND child_id = $2")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("unlink");
}

async fn join(pool: &PgPool, team: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .expect("join");
}

async fn set_team_live(pool: &PgPool, team: Uuid, live: bool) {
    sqlx::query("UPDATE kb_teams SET is_active = $2 WHERE id = $1")
        .bind(team)
        .bind(live)
        .execute(pool)
        .await
        .expect("liveness");
}

async fn forbid(pool: &PgPool, team: Uuid, forbidden: bool) {
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = $2 WHERE id = $1")
        .bind(team)
        .bind(forbidden)
        .execute(pool)
        .await
        .expect("forbid");
}

async fn context(pool: &PgPool, owner_team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(owner_team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("context")
}

async fn emitter_of(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE p.id = $1 AND e.name = p.handle || '@web'",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("web emitter")
}

/// A resource through the real create path, with caller-supplied chunks so nothing embeds.
async fn resource(pool: &PgPool, home: Uuid, owner: Uuid, title: &str) -> Uuid {
    let prose = format!("{title} prose");
    let emitter = emitter_of(pool, owner).await;
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{title}"),
            body: &prose,
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: Some(vec![IncomingChunk {
                chunk_index: 0,
                content_hash: hex::encode(sha2::Sha256::digest(&prose)),
                content: prose.clone(),
                embedding: vec![0.1; 768],
                embedded_with: Some("model-sha-1".to_string()),
                header_path: String::new(),
                heading_depth: 0,
            }]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create resource");
    sqlx::query_scalar("SELECT id FROM kb_resources WHERE origin_uri = $1")
        .bind(format!("test://{title}"))
        .fetch_one(pool)
        .await
        .expect("created resource")
}

/// An edge homed in a context, through the substrate write.
async fn edge(pool: &PgPool, src: Uuid, tgt: Uuid, home: Uuid, author: Uuid) -> Uuid {
    let emitter = emitter_of(pool, author).await;
    writes::assert_relationship(
        pool,
        writes::AssertParams {
            src: ResourceId::from(src),
            tgt: ResourceId::from(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("derived_from"),
            weight: 1.0,
            home: ContextId::from(home),
            emitter: EntityId::from(emitter),
        },
    )
    .await
    .expect("assert edge")
    .uuid()
}

/// THE PLAN'S FIXTURE HELPER: `open(T, A, instance_wide)` writes the row directly.
async fn open(pool: &PgPool, t: Uuid, a: Uuid, instance_wide: bool, by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by, instance_wide, admin_act) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(t)
    .bind(a)
    .bind(by)
    .bind(instance_wide)
    .execute(pool)
    .await
    .expect("open public_within row");
}

async fn close(pool: &PgPool, t: Uuid, a: Uuid) {
    sqlx::query("DELETE FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2")
        .bind(t)
        .bind(a)
        .execute(pool)
        .await
        .expect("close");
}

async fn switch(pool: &PgPool, on: bool) {
    sqlx::query("UPDATE kb_system_settings SET public_within_enabled = $1 WHERE id = 1")
        .bind(on)
        .execute(pool)
        .await
        .expect("switch");
}

async fn reads(pool: &PgPool, who: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(who)
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("reads")
}

async fn reads_by_membership(pool: &PgPool, who: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_by_membership($1) v \
                         WHERE v.resource_id = $2)",
    )
    .bind(who)
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("reads by membership")
}

async fn ctx_reads(pool: &PgPool, who: Uuid, ctx: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_visible_to($1, $2)")
        .bind(who)
        .bind(ctx)
        .fetch_one(pool)
        .await
        .expect("context reads")
}

async fn sees_edge(pool: &PgPool, who: Uuid, edge: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) e WHERE e.edge_id = $2)")
        .bind(who)
        .bind(edge)
        .fetch_one(pool)
        .await
        .expect("edge visibility")
}

async fn scalar_bool(pool: &PgPool, sql: &str, a: Uuid) -> bool {
    sqlx::query_scalar(sql)
        .bind(a)
        .fetch_one(pool)
        .await
        .expect("scalar")
}

struct World {
    root: Uuid,
    org: Uuid,
    eng: Uuid,
    eng_web: Uuid,
    /// T.
    eng_sec: Uuid,
    eng_sec_red: Uuid,
    sales: Uuid,
    ops: Uuid,

    admin: Uuid,
    sec_owner: Uuid,
    sec_member: Uuid,
    web_owner: Uuid,
    web_maintainer: Uuid,
    web_member: Uuid,
    web_watcher: Uuid,
    sales_member: Uuid,
    org_only: Uuid,
    robot: Uuid,
    agent: Uuid,

    ctx_t: Uuid,
    ctx_web: Uuid,
    ctx_red: Uuid,
    ctx_shared: Uuid,
    ctx_sales: Uuid,

    /// In ctx-t.
    r_t: Uuid,
    /// In ctx-t, linked to r_t by an edge homed in ctx-t.
    r_t2: Uuid,
    /// In ctx-t, linked to r_t ONLY by an edge homed in T's cogmap.
    r_t3: Uuid,
    e_t: Uuid,
    e_map: Uuid,
    /// In ctx-web, the readers' own home.
    r_web: Uuid,
    r_red: Uuid,
    r_shared: Uuid,
    r_granted: Uuid,
    map_t: Uuid,
}

async fn world(pool: &PgPool) -> World {
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(pool)
        .await
        .expect("root");
    let org = team(pool, "org", &[]).await;
    let eng = team(pool, "eng", &[org]).await;
    let eng_web = team(pool, "eng-web", &[eng]).await;
    let eng_sec = team(pool, "eng-sec", &[eng]).await;
    let eng_sec_red = team(pool, "eng-sec-red", &[eng_sec]).await;
    let sales = team(pool, "sales", &[org]).await;
    let ops = team(pool, "ops", &[]).await;
    link(pool, ops, eng_sec).await;

    let admin = person(pool, "admin").await;
    test_support::approved_admin(pool, admin).await;
    let sec_owner = person(pool, "sec-owner").await;
    let sec_member = person(pool, "sec-member").await;
    join(pool, eng_sec, sec_owner, "owner").await;
    join(pool, eng_sec, sec_member, "member").await;
    let web_owner = person(pool, "web-owner").await;
    let web_maintainer = person(pool, "web-maintainer").await;
    let web_member = person(pool, "web-member").await;
    let web_watcher = person(pool, "web-watcher").await;
    join(pool, eng_web, web_owner, "owner").await;
    join(pool, eng_web, web_maintainer, "maintainer").await;
    join(pool, eng_web, web_member, "member").await;
    join(pool, eng_web, web_watcher, "watcher").await;
    let sales_member = person(pool, "sales-member").await;
    join(pool, sales, sales_member, "member").await;
    let org_only = person(pool, "org-only").await;
    join(pool, org, org_only, "member").await;

    let ctx_t = context(pool, eng_sec, "ctx-t").await;
    let ctx_web = context(pool, eng_web, "ctx-web").await;
    let ctx_red = context(pool, eng_sec_red, "ctx-red").await;
    let ctx_shared = context(pool, sales, "ctx-shared").await;
    let ctx_sales = context(pool, sales, "ctx-sales").await;
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(ctx_shared)
        .bind(eng_sec)
        .execute(pool)
        .await
        .expect("share to T");

    let robot = machine(pool, "robot", web_owner).await;
    join(pool, eng_web, robot, "member").await;
    let agent = connection_agent(pool, "agent", web_owner, ctx_web).await;
    join(pool, eng_web, agent, "member").await;

    let r_t = resource(pool, ctx_t, sec_member, "r-t").await;
    let r_t2 = resource(pool, ctx_t, sec_member, "r-t2").await;
    let r_t3 = resource(pool, ctx_t, sec_member, "r-t3").await;
    let e_t = edge(pool, r_t2, r_t, ctx_t, sec_member).await;
    let r_web = resource(pool, ctx_web, web_member, "r-web").await;
    let r_red = resource(pool, ctx_red, sec_owner, "r-red").await;
    let r_shared = resource(pool, ctx_shared, sales_member, "r-shared").await;
    let r_granted = resource(pool, ctx_sales, sales_member, "r-granted").await;
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_teams', $2, true, $3)",
    )
    .bind(r_granted)
    .bind(eng_sec)
    .bind(sales_member)
    .execute(pool)
    .await
    .expect("grant held by T");

    // A cognitive map bound to T, and an edge homed in it between two T resources (N8).
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('map-t telos', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("telos");
    let map_t: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('map-t', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("map");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(map_t)
        .bind(eng_sec)
        .execute(pool)
        .await
        .expect("bind map to T");
    let e_map: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_edges \
           (id, source_table, source_id, target_table, target_id, edge_kind, label, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         SELECT uuid_generate_v7(), 'kb_resources', $1, 'kb_resources', $2, 'leads_to', \
                'derived_from', 'kb_cogmaps', $3, e.id, e.id \
           FROM kb_events e ORDER BY e.id LIMIT 1 RETURNING id",
    )
    .bind(r_t)
    .bind(r_t3)
    .bind(map_t)
    .fetch_one(pool)
    .await
    .expect("cogmap-homed edge");

    switch(pool, true).await;
    open(pool, eng_sec, eng, false, sec_owner).await;

    World {
        root,
        org,
        eng,
        eng_web,
        eng_sec,
        eng_sec_red,
        sales,
        ops,
        admin,
        sec_owner,
        sec_member,
        web_owner,
        web_maintainer,
        web_member,
        web_watcher,
        sales_member,
        org_only,
        robot,
        agent,
        ctx_t,
        ctx_web,
        ctx_red,
        ctx_shared,
        ctx_sales,
        r_t,
        r_t2,
        r_t3,
        e_t,
        e_map,
        r_web,
        r_red,
        r_shared,
        r_granted,
        map_t,
    }
}

// =================================================================================================
// Phase 1 — schema, policy, helpers
// =================================================================================================

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_new_setting_is_off_or_empty_by_default(pool: PgPool) {
    let enabled: bool =
        sqlx::query_scalar("SELECT public_within_enabled FROM kb_system_settings WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!enabled, "the instance switch is born off (R-e)");
    let forbidden: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_teams WHERE public_within_forbidden")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(forbidden, 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_team_public_within")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    let settings = access_service::get_system_settings(&pool).await.unwrap();
    assert!(!settings.public_within_enabled);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_trigger_and_the_check_refuse_what_p1_bars(pool: PgPool) {
    let w = world(&pool).await;
    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(w.web_member)
        .fetch_one(&pool)
        .await
        .expect("personal team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'org' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();

    for (t, a, constraint) in [
        (w.eng_sec, w.eng_sec, "kb_team_public_within_not_self"),
        (personal, w.root, "kb_team_public_within_team_personal"),
        (w.root, w.org, "kb_team_public_within_team_root_or_gating"),
        (w.org, w.root, "kb_team_public_within_team_root_or_gating"),
        (w.eng_sec, personal, "kb_team_public_within_anchor_personal"),
    ] {
        let err = sqlx::query(
            "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by) VALUES ($1, $2, $3)",
        )
        .bind(t)
        .bind(a)
        .bind(w.admin)
        .execute(&pool)
        .await
        .expect_err("the shape must be refused");
        let db = err.as_database_error().expect("a database refusal");
        assert_eq!(db.constraint(), Some(constraint), "{err}");
    }

    // Deleting the profile that opened a row keeps the row: the ledger keeps the act.
    let opener_rule: String = sqlx::query_scalar(
        "SELECT confdeltype::text FROM pg_constraint \
          WHERE conrelid = 'kb_team_public_within'::regclass AND contype = 'f' \
            AND conkey = ARRAY[(SELECT attnum FROM pg_attribute \
                                 WHERE attrelid = 'kb_team_public_within'::regclass \
                                   AND attname = 'opened_by')]",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(opener_rule, "n", "opened_by is ON DELETE SET NULL");

    // The anchor may be the root or the gating team: that is an admin's open, decided by the door.
    open(&pool, w.eng_sec, w.org, true, w.admin).await;
    open(&pool, w.eng_sec, w.root, true, w.admin).await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn team_ancestors_any_walks_through_a_dead_team_and_team_ancestors_does_not(pool: PgPool) {
    let w = world(&pool).await;
    set_team_live(&pool, w.eng, false).await;
    let any: Vec<Uuid> = sqlx::query_scalar("SELECT team_id FROM team_ancestors_any($1)")
        .bind(w.eng_sec)
        .fetch_all(&pool)
        .await
        .unwrap();
    let live: Vec<Uuid> = sqlx::query_scalar("SELECT team_id FROM team_ancestors($1)")
        .bind(w.eng_sec)
        .fetch_all(&pool)
        .await
        .unwrap();
    for t in [w.eng_sec, w.eng, w.org, w.ops] {
        assert!(any.contains(&t), "team_ancestors_any includes {t}");
    }
    assert!(!live.contains(&w.eng) && !live.contains(&w.org));
    assert!(live.contains(&w.ops), "the second parent is a live path");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_forbid_anywhere_above_t_fails_policy_closed(pool: PgPool) {
    let w = world(&pool).await;
    let allowed = |pool: PgPool, t: Uuid| async move {
        scalar_bool(&pool, "SELECT public_within_allowed_over($1)", t).await
    };
    assert!(allowed(pool.clone(), w.eng_sec).await);

    // On T's second parent's chain.
    forbid(&pool, w.ops, true).await;
    assert!(
        !allowed(pool.clone(), w.eng_sec).await,
        "a forbid on T's second parent"
    );
    forbid(&pool, w.ops, false).await;

    // Between A and T: A = org, the forbid on eng.
    forbid(&pool, w.eng, true).await;
    assert!(
        !allowed(pool.clone(), w.eng_sec).await,
        "a forbid between A and T"
    );

    // On a DEAD ancestor: soft-deleting the forbidding team does not lift it (PW-2/F5).
    set_team_live(&pool, w.eng, false).await;
    assert!(
        !allowed(pool.clone(), w.eng_sec).await,
        "a forbid on a dead ancestor"
    );
    set_team_live(&pool, w.eng, true).await;

    // Detaching T out from under the forbidding team lifts it, and only that does.
    unlink(&pool, w.eng, w.eng_sec).await;
    assert!(
        allowed(pool.clone(), w.eng_sec).await,
        "T left the forbidding subtree"
    );

    // The switch is read first.
    switch(&pool, false).await;
    assert!(!allowed(pool.clone(), w.eng_sec).await, "the switch is off");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn anchor_subtree_holds_pool_counts_live_pools_only(pool: PgPool) {
    let w = world(&pool).await;
    let holds = |pool: PgPool, a: Uuid| async move {
        scalar_bool(&pool, "SELECT anchor_subtree_holds_pool($1)", a).await
    };
    assert!(!holds(pool.clone(), w.eng).await);
    let mid = team(&pool, "eng-mid", &[w.eng]).await;
    let pool_team = team(&pool, "eng-pool", &[mid]).await;
    sqlx::query("UPDATE kb_teams SET auto_join_role = 'member' WHERE id = $1")
        .bind(pool_team)
        .execute(&pool)
        .await
        .unwrap();
    assert!(holds(pool.clone(), w.eng).await, "a pool grandchild counts");
    assert!(
        holds(pool.clone(), w.org).await,
        "and counts for every anchor above it"
    );
    set_team_live(&pool, mid, false).await;
    assert!(
        !holds(pool.clone(), w.eng).await,
        "a pool below a dead team does not count"
    );
    set_team_live(&pool, mid, true).await;
    set_team_live(&pool, pool_team, false).await;
    assert!(
        !holds(pool.clone(), w.eng).await,
        "a dead pool does not count"
    );

    // The gating team counts too.
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'eng-web' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        holds(pool.clone(), w.eng).await,
        "the gating team under the anchor counts"
    );
}

/// The machine fence (P6, PW-4) and the R-n pin: a person reads with ANY token, a Slack-minted one
/// included, because the fence is the principal, not the token.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn public_within_reading_is_for_people_only(pool: PgPool) {
    let w = world(&pool).await;
    let no_link = profile_with_emitters(&pool, "no-link").await;
    let slack_person = person_via(&pool, "slack-person", "slack").await;
    let reading = |pool: PgPool, p: Uuid| async move {
        scalar_bool(&pool, "SELECT public_within_reading($1)", p).await
    };
    assert!(!reading(pool.clone(), w.robot).await, "the machine client");
    assert!(
        !reading(pool.clone(), w.agent).await,
        "the connection agent"
    );
    assert!(
        !reading(pool.clone(), no_link).await,
        "a profile with no auth link"
    );
    assert!(reading(pool.clone(), w.web_member).await, "a person");
    switch(&pool, false).await;
    assert!(
        !reading(pool.clone(), w.web_member).await,
        "nobody reads arm 5 with the switch off"
    );
    switch(&pool, true).await;
    assert!(
        reading(pool.clone(), slack_person).await,
        "a person whose only link is Slack's: a Slack-minted token is the person (R-n)"
    );

    // A revoked machine registration is still a machine: fail closed.
    sqlx::query("UPDATE kb_machine_clients SET revoked_at = now() WHERE profile_id = $1")
        .bind(w.robot)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!reading(pool.clone(), w.robot).await, "a revoked machine");
    // A person's machine-provider link does not make them a person on its own.
    let m2m_only = profile_with_emitters(&pool, "m2m-only").await;
    auth_link(&pool, m2m_only, "auth0-m2m", "m2m-only").await;
    assert!(
        !reading(pool.clone(), m2m_only).await,
        "a machine-provider link alone"
    );
}

/// `update_system_settings` writes the switch in a transaction with its record (plan 1.13).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_instance_switch_is_recorded_once_per_change(pool: PgPool) {
    let admin = person(&pool, "switch-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let proof = test_support::system_admin_proof_for(&pool, admin).await;
    let records = |pool: PgPool| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'public_within_policy_changed' \
                AND e.payload->>'scope' = 'instance'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let on = temper_core::types::admin::UpdateSettingsRequest {
        public_within_enabled: Some(true),
        ..Default::default()
    };
    let s = access_service::update_system_settings(&pool, &proof, &on)
        .await
        .unwrap();
    assert!(s.public_within_enabled);
    assert_eq!(records(pool.clone()).await, 1);
    access_service::update_system_settings(&pool, &proof, &on)
        .await
        .unwrap();
    assert_eq!(
        records(pool.clone()).await,
        1,
        "a value already held records nothing"
    );
    let off = temper_core::types::admin::UpdateSettingsRequest {
        public_within_enabled: Some(false),
        ..Default::default()
    };
    let s = access_service::update_system_settings(&pool, &proof, &off)
        .await
        .unwrap();
    assert!(!s.public_within_enabled);
    assert_eq!(records(pool.clone()).await, 2);

    // A refused update writes nothing: the switch and its record ride the same transaction.
    let bad = temper_core::types::admin::UpdateSettingsRequest {
        public_within_enabled: Some(true),
        gating_team_slug: Some("no-such-team".into()),
        ..Default::default()
    };
    access_service::update_system_settings(&pool, &proof, &bad)
        .await
        .expect_err("a nonexistent gating team is refused");
    assert!(
        !access_service::get_system_settings(&pool)
            .await
            .unwrap()
            .public_within_enabled
    );
    assert_eq!(records(pool.clone()).await, 2);
}

// =================================================================================================
// Phase 2 — the read arm (plan 2.5)
// =================================================================================================

/// At every role on eng-web, through every read-door family.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_person_under_the_anchor_reads_ts_owned_contexts(pool: PgPool) {
    let w = world(&pool).await;
    // An artifact on r_t, so the artifact family has something to show.
    sqlx::query(
        "INSERT INTO kb_data_artifacts \
           (id, resource_id, kind_owner_table, kind_owner_id, artifact_kind, intent, \
            content_hash, content_bytes, asserted_by_event_id, last_event_id) \
         SELECT uuid_generate_v7(), $1, 'kb_profiles', $2, 'row', 'current', 'h', 0, e.id, e.id \
           FROM kb_events e ORDER BY e.id LIMIT 1",
    )
    .bind(w.r_t)
    .bind(w.sec_member)
    .execute(&pool)
    .await
    .unwrap();
    // A blob homed in ctx-t, committed by a T author.
    let store = InMemoryBlobStore::default();
    let blob = commit_blob(&pool, &store, w.ctx_t, w.sec_member, b"t-blob").await;

    for who in [w.web_owner, w.web_maintainer, w.web_member, w.web_watcher] {
        assert!(reads(&pool, who, w.r_t).await, "resources");
        assert!(ctx_reads(&pool, who, w.ctx_t).await, "contexts");
        assert!(sees_edge(&pool, who, w.e_t).await, "edges");
        let hits: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM search_exact($1, 'r-t2', NULL, NULL, NULL, 10, 0)",
        )
        .bind(who)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(hits > 0, "search");
        let feed: serde_json::Value =
            sqlx::query_scalar("SELECT context_changes_after($1, $2, '0'::xid8, 0, 100, NULL)")
                .bind(who)
                .bind(w.ctx_t)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            feed.to_string().contains(&w.r_t.to_string()),
            "the change feed names T's resource: {feed}"
        );
        let atlas: i64 =
            sqlx::query_scalar("SELECT count(*) FROM graph_atlas_nodes_visible($1, $2)")
                .bind(who)
                .bind(vec![w.r_t, w.r_t2])
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(atlas, 2, "graph/atlas");
        let artifacts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _visible_artifacts($1, $2, NULL, NULL, false)",
        )
        .bind(who)
        .bind(w.r_t)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(artifacts, 1, "artifacts");
        let lineage: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM resource_lineage($1, $2, 'ancestors', 3) \
              UNION ALL SELECT 0 LIMIT 1",
        )
        .bind(who)
        .bind(w.r_t2)
        .fetch_one(&pool)
        .await
        .unwrap();
        let lineage_desc: i64 =
            sqlx::query_scalar("SELECT count(*) FROM resource_lineage($1, $2, 'descendants', 3)")
                .bind(who)
                .bind(w.r_t2)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            lineage + lineage_desc > 0,
            "lineage walks the edge homed in ctx-t"
        );
        assert!(
            scalar_bool_2(&pool, "SELECT blob_readable_by_profile($1, $2)", who, blob).await,
            "blob"
        );
    }

    // The control for every family: sales is outside eng's subtree.
    let s = w.sales_member;
    assert!(!reads(&pool, s, w.r_t).await);
    assert!(!ctx_reads(&pool, s, w.ctx_t).await);
    assert!(!sees_edge(&pool, s, w.e_t).await);
    assert!(!scalar_bool_2(&pool, "SELECT blob_readable_by_profile($1, $2)", s, blob).await);
}

async fn scalar_bool_2(pool: &PgPool, sql: &str, a: Uuid, b: Uuid) -> bool {
    sqlx::query_scalar(sql)
        .bind(a)
        .bind(b)
        .fetch_one(pool)
        .await
        .expect("scalar")
}

fn blob_cfg() -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes: 1 << 20,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 64 * 1024,
    }
}

async fn commit_blob(
    pool: &PgPool,
    store: &InMemoryBlobStore,
    home: Uuid,
    caller: Uuid,
    bytes: &[u8],
) -> Uuid {
    blob_service::commit_blob(
        pool,
        store,
        &blob_cfg(),
        blob_service::BlobCommitCommand {
            caller: ProfileId::from(caller),
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(home.to_string()),
            content_type: "image/png".to_string(),
            bytes: Bytes::copy_from_slice(bytes),
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("commit blob")
    .blob_id
    .uuid()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_audience_is_the_anchors_subtree_only(pool: PgPool) {
    let w = world(&pool).await;
    assert!(
        !reads(&pool, w.sales_member, w.r_t).await,
        "sales is beside eng"
    );
    assert!(
        !reads(&pool, w.org_only, w.r_t).await,
        "a member only of org is above eng"
    );

    close(&pool, w.eng_sec, w.eng).await;
    open(&pool, w.eng_sec, w.org, false, w.sec_owner).await;
    assert!(
        reads(&pool, w.sales_member, w.r_t).await,
        "A = org: sales is under it"
    );
    assert!(
        reads(&pool, w.org_only, w.r_t).await,
        "A = org: its own roster"
    );
    assert!(reads(&pool, w.web_member, w.r_t).await);
}

/// N10, and the R-n pin.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn machines_and_agents_never_read_through_the_arm(pool: PgPool) {
    let w = world(&pool).await;
    for (who, what) in [
        (w.robot, "the machine client"),
        (w.agent, "the connection agent"),
    ] {
        assert!(!reads(&pool, who, w.r_t).await, "{what} reads r-t");
        assert!(!ctx_reads(&pool, who, w.ctx_t).await, "{what} reads ctx-t");
        assert!(!sees_edge(&pool, who, w.e_t).await, "{what} sees T's edge");
        assert!(
            reads(&pool, who, w.r_web).await,
            "{what} still reads by membership"
        );
    }

    // A person reading with any token does read T: the Slack mention path mints the person's own
    // token, and the principal is the person (R-n). The token never reaches SQL, so the pin is that
    // a person whose only link is Slack's reads T exactly as any person does.
    let slack_person = person_via(&pool, "slack-reader", "slack").await;
    join(&pool, w.eng_web, slack_person, "member").await;
    assert!(reads(&pool, slack_person, w.r_t).await);
}

/// N8: shares, grants and maps held by T, and the edges homed in a map, are not passed on.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn shares_grants_maps_and_their_edges_are_not_passed_on(pool: PgPool) {
    let w = world(&pool).await;
    // T's own members read all of them: the fixtures are live.
    for r in [w.r_shared, w.r_granted] {
        assert!(reads(&pool, w.sec_member, r).await);
    }
    assert!(sees_edge(&pool, w.sec_member, w.e_map).await);

    let p = w.web_member;
    assert!(!reads(&pool, p, w.r_shared).await, "a context shared TO T");
    assert!(!ctx_reads(&pool, p, w.ctx_shared).await);
    assert!(!reads(&pool, p, w.r_granted).await, "a grant held BY T");
    let map_reads: bool = scalar_bool_2(
        &pool,
        "SELECT cogmap_readable_by_profile($1, $2)",
        p,
        w.map_t,
    )
    .await;
    assert!(!map_reads, "a map bound to T");
    assert!(
        reads(&pool, p, w.r_t3).await,
        "both endpoints of the map's edge are readable"
    );
    assert!(
        !sees_edge(&pool, p, w.e_map).await,
        "an edge homed in T's map"
    );

    // follow_from / search_graph_expand, seeded on a T resource, does not walk the map's edge.
    let expanded: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM search_graph_expand($1, $2, 2, NULL, 0.5)")
            .bind(p)
            .bind(vec![w.r_t])
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        !expanded.contains(&w.r_t3),
        "follow_from reached r-t3 through the edge homed in T's map: {expanded:?}"
    );
    assert!(
        expanded.contains(&w.r_t2),
        "the edge homed in ctx-t is walked, so the probe is live: {expanded:?}"
    );
}

/// R-a: not inherited.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ts_children_stay_private(pool: PgPool) {
    let w = world(&pool).await;
    // Reading inherits up, never down: not even T's own members read T's child's context, so a row
    // on T, which opens what T owns, cannot open it either.
    assert!(!reads(&pool, w.sec_member, w.r_red).await);
    assert!(!reads(&pool, w.web_member, w.r_red).await);
    assert!(!ctx_reads(&pool, w.web_member, w.ctx_red).await);
    // A row on T confers nothing on T's child, even as T's own team: a separate team needs its own.
    let child_row_effective = scalar_bool_2(
        &pool,
        "SELECT EXISTS (SELECT 1 FROM kb_team_public_within WHERE team_id = $1 AND $2 IS NOT NULL)",
        w.eng_sec_red,
        w.eng_sec_red,
    )
    .await;
    assert!(!child_row_effective, "no row was opened for the child");
}

/// N2, N8: for a public reader, every act ON T's material refuses; an audit of the reader's own
/// side's finding that CITES T is admitted (R-m).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reading_authorizes_no_act_on_t(pool: PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .expect("bootseed");
    let w = world(&pool).await;
    // The watcher: it reads its own team's material but cannot modify it, so an audit it records is
    // not refused as self-grading (the gate's `Author` proxy), and every refusal below is about T.
    let p = w.web_watcher;
    assert!(reads(&pool, p, w.r_t).await, "precondition: p reads T");

    // The point gates: write, delete, grant, author, modify — and read by membership.
    for (sql, what) in [
        (
            "SELECT can('kb_profiles', $1, 'write', 'kb_resources', $2)",
            "write r-t",
        ),
        (
            "SELECT can('kb_profiles', $1, 'delete', 'kb_resources', $2)",
            "delete r-t",
        ),
        (
            "SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)",
            "grant on r-t",
        ),
        ("SELECT can_modify_resource($1, $2)", "modify r-t"),
        (
            "SELECT can_read_by_membership($1, 'kb_resources', $2)",
            "attenuated read on r-t",
        ),
    ] {
        assert!(!scalar_bool_2(&pool, sql, p, w.r_t).await, "{what}");
    }
    for (sql, what) in [
        (
            "SELECT can('kb_profiles', $1, 'write', 'kb_contexts', $2)",
            "write ctx-t",
        ),
        (
            "SELECT context_authorable_by_profile($1, $2)",
            "author into ctx-t",
        ),
        (
            "SELECT can('kb_profiles', $1, 'grant', 'kb_contexts', $2)",
            "grant on ctx-t",
        ),
        (
            "SELECT can_read_by_membership($1, 'kb_contexts', $2)",
            "attenuated read on ctx-t",
        ),
    ] {
        assert!(!scalar_bool_2(&pool, sql, p, w.ctx_t).await, "{what}");
    }

    // The door-level acts run as eng-web's OWNER, who manages the reader's side: each refusal below
    // is therefore about T, not about authority the caller lacks on its own side.
    let owner = w.web_owner;
    assert!(
        reads(&pool, owner, w.r_t).await,
        "precondition: the owner reads T"
    );
    for (sql, what) in [
        (
            "SELECT can('kb_profiles', $1, 'write', 'kb_resources', $2)",
            "write r-t",
        ),
        ("SELECT can_modify_resource($1, $2)", "modify r-t"),
        (
            "SELECT can_read_by_membership($1, 'kb_resources', $2)",
            "attenuated read on r-t",
        ),
    ] {
        assert!(
            !scalar_bool_2(&pool, sql, owner, w.r_t).await,
            "owner: {what}"
        );
    }
    let human = test_support::human_principal_for(&pool, owner).await;

    // Grant (and the attenuated grant): conferring read on T material through the grant door.
    let grant = access_service::grant_capability(
        &pool,
        &human,
        &temper_core::types::cognitive_maps::GrantCapabilityRequest {
            subject_table: "kb_resources".into(),
            subject_id: w.r_t,
            principal_table: "kb_profiles".into(),
            principal_id: w.sales_member,
            can_read: true,
            can_write: false,
            can_delete: false,
            can_grant: false,
        },
    )
    .await;
    assert!(grant.is_err(), "grant on a T resource: {grant:?}");

    // Reassign (a two-sided act) of T's context to the reader's team.
    let reassign = context_service::reassign(&pool, &human, w.ctx_t, w.eng_web).await;
    assert!(reassign.is_err(), "reassign ctx-t: {reassign:?}");

    // An edge whose SOURCE is a T resource: T's home is not authorable.
    let from_t = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .assert_relationship(assert_cmd(w.r_t, w.r_web))
        .await;
    assert!(from_t.is_err(), "an edge sourced on a T resource");
    // ... while the same edge the other way, from the owner's own home, is a reference.
    DbBackend::new(pool.clone(), ProfileId::from(owner))
        .assert_relationship(assert_cmd(w.r_web, w.r_t))
        .await
        .expect("the reverse edge, sourced in the reader's own home, is admitted");

    let backend = DbBackend::new(pool.clone(), ProfileId::from(p));

    // An audit of a T finding (R-o): T's finding cites r-web; the reader can read both, by reading.
    let t_finding_block = cite(&pool, w.r_t2, w.r_web).await;
    let audit_t = backend
        .record_citation_audit(audit_cmd(t_finding_block, w.r_web))
        .await;
    assert!(audit_t.is_err(), "auditing T's own finding: {audit_t:?}");

    // An audit of the reader's own side's finding, CITING T, is admitted (R-m). The finding is
    // authored by another member, so p is not its author.
    let own_finding = resource(&pool, w.ctx_web, w.web_member, "own-finding").await;
    let own_block = cite(&pool, own_finding, w.r_t).await;
    backend
        .record_citation_audit(audit_cmd(own_block, w.r_t))
        .await
        .expect("an audit of the reader's own team's finding citing T is admitted");
}

fn assert_cmd(source: Uuid, target: Uuid) -> AssertRelationship {
    AssertRelationship {
        source: ResourceId::from(source),
        target: ResourceId::from(target),
        target_table: Default::default(),
        edge_kind: WireEdgeKind::Express,
        polarity: Polarity::Forward,
        label: "relates_to".to_string(),
        weight: 1.0,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

fn audit_cmd(block: Uuid, source: Uuid) -> RecordCitationAudit {
    RecordCitationAudit {
        block: BlockId::from(block),
        source: ProvenanceSource::Resource(source),
        value: 0.5,
        reason: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// Make `finding`'s first block cite `source`; returns the block. Raw provenance, as a projection
/// would write it, contributed by the finding's own create event.
async fn cite(pool: &PgPool, finding: Uuid, source: Uuid) -> Uuid {
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(finding)
    .fetch_one(pool)
    .await
    .expect("block");
    sqlx::query(
        "INSERT INTO kb_block_provenance \
           (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
         SELECT $1, 'resource', $2, b.genesis_event_id, 0 FROM kb_content_blocks b WHERE b.id = $1",
    )
    .bind(block)
    .bind(source)
    .execute(pool)
    .await
    .expect("provenance");
    block
}

/// C6, R-j: references from the reader's own home are admitted, and homed there. A machine on the
/// same team is refused all three, because T is not readable to it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_person_references_t_from_their_own_home(pool: PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .expect("bootseed");
    let w = world(&pool).await;
    let store = InMemoryBlobStore::default();

    for (who, admitted) in [(w.web_member, true), (w.robot, false)] {
        let backend = DbBackend::new(pool.clone(), ProfileId::from(who));
        let source = resource(&pool, w.ctx_web, who, &format!("ref-src-{who}")).await;

        // An edge to a T resource.
        let edge = backend.assert_relationship(assert_cmd(source, w.r_t)).await;
        // A goal link (advances) to a T resource.
        let goal = backend
            .update_resource(temper_workflow::operations::UpdateResource {
                resource: ResourceId::from(source),
                title: None,
                slug: None,
                body: None,
                managed_meta: None,
                open_meta: None,
                open_meta_add: None,
                goal: Some(temper_workflow::operations::GoalPatch::Set(
                    ResourceId::from(w.r_t2),
                )),
                move_to: None,
                context_ref: None,
                act: ActContext::default(),
                origin: Surface::ApiHttp,
            })
            .await;
        // A blob relation whose peer is a T resource, from a blob in the reader's home.
        let blob = commit_blob(&pool, &store, w.ctx_web, who, who.as_bytes()).await;
        let relation = relate(&pool, who, blob, w.r_t3).await;

        if admitted {
            edge.expect("an edge from the reader's own resource to T");
            goal.expect("a goal link to a T resource");
            relation.expect("a blob relation with a T peer");
            let homes: Vec<Uuid> = sqlx::query_scalar(
                "SELECT home_anchor_id FROM kb_edges \
                  WHERE NOT is_folded AND target_id IN ($1, $2, $3) \
                    AND (source_id = $4 OR source_id = $5)",
            )
            .bind(w.r_t)
            .bind(w.r_t2)
            .bind(w.r_t3)
            .bind(source)
            .bind(blob)
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(homes.len(), 3, "three references written");
            assert!(
                homes.iter().all(|h| *h == w.ctx_web),
                "every reference is homed in the reader's own context: {homes:?}"
            );
        } else {
            assert!(edge.is_err(), "the machine's edge to T");
            assert!(goal.is_err(), "the machine's goal link to T");
            assert!(
                relation.is_err(),
                "the machine's blob relation with a T peer"
            );
        }
    }
}

async fn relate(
    pool: &PgPool,
    caller: Uuid,
    blob: Uuid,
    peer: Uuid,
) -> Result<temper_core::types::blob::BlobRelationAck, temper_services::error::ApiError> {
    blob_service::relate_blob(
        pool,
        ProfileId::from(caller),
        temper_core::types::ids::BlobId::from(blob),
        &BlobRelationAssertRequest {
            direction: temper_core::types::blob::BlobRelationDirection::BlobAsSource,
            peer_table: "kb_resources".to_string(),
            peer_id: peer,
            edge_kind: WireEdgeKind::Express,
            polarity: Polarity::Forward,
            label: "figure_of".to_string(),
            weight: 1.0,
            act: ActInput::default(),
        },
        ActContext::default(),
        Surface::ApiHttp,
    )
    .await
}

/// N12: the reference is visible exactly under the conjunction.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_reference_is_visible_only_under_the_conjunction(pool: PgPool) {
    let w = world(&pool).await;
    // web-member's reference from ctx-web into T.
    let e = edge(&pool, w.r_web, w.r_t, w.ctx_web, w.web_member).await;
    assert!(sees_edge(&pool, w.web_member, e).await, "the author");
    assert!(
        sees_edge(&pool, w.web_watcher, e).await,
        "a reader of the home and of T"
    );

    // A reader of the author's context who is outside A's subtree: share ctx-web to sales.
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(w.ctx_web)
        .bind(w.sales)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        reads(&pool, w.sales_member, w.r_web).await,
        "sales reads the home"
    );
    assert!(
        !sees_edge(&pool, w.sales_member, e).await,
        "but not T, so not the edge"
    );

    // T's members do not see the backlink: its home is a context T does not read.
    assert!(!ctx_reads(&pool, w.sec_member, w.ctx_web).await);
    assert!(
        !sees_edge(&pool, w.sec_member, e).await,
        "T is not annotated against its will"
    );

    // Homed in an ancestor context T reads, T's members see it.
    let ctx_eng = context(&pool, w.eng, "ctx-eng").await;
    let r_eng = resource(&pool, ctx_eng, w.web_owner, "r-eng").await;
    let backlink = edge(&pool, r_eng, w.r_t, ctx_eng, w.web_owner).await;
    assert!(
        sees_edge(&pool, w.sec_member, backlink).await,
        "a backlink homed where T reads"
    );
}

/// Close/reopen, a forbid, and a lapse: the reference disappears and reappears with no row written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_reference_vanishes_with_access_and_returns_with_it(pool: PgPool) {
    let w = world(&pool).await;
    let e = edge(&pool, w.r_web, w.r_t, w.ctx_web, w.web_member).await;
    let edges_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_edges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(sees_edge(&pool, w.web_member, e).await);

    close(&pool, w.eng_sec, w.eng).await;
    assert!(!sees_edge(&pool, w.web_member, e).await, "closed");
    open(&pool, w.eng_sec, w.eng, false, w.sec_owner).await;
    assert!(sees_edge(&pool, w.web_member, e).await, "reopened");

    forbid(&pool, w.org, true).await;
    assert!(
        !sees_edge(&pool, w.web_member, e).await,
        "dormant: a forbid above T"
    );
    forbid(&pool, w.org, false).await;
    assert!(sees_edge(&pool, w.web_member, e).await, "the forbid lifted");

    // A lapse: T is moved out from under A. Phase 4 deletes the row; here the read-time condition
    // already makes it confer nothing, and only a fresh open restores it once T is back.
    unlink(&pool, w.eng, w.eng_sec).await;
    assert!(
        !sees_edge(&pool, w.web_member, e).await,
        "lapsed: A is no longer above T"
    );
    link(&pool, w.eng, w.eng_sec).await;
    close(&pool, w.eng_sec, w.eng).await; // what Phase 4's lapse does to the row
    assert!(
        !sees_edge(&pool, w.web_member, e).await,
        "a lapsed row does not revive"
    );
    open(&pool, w.eng_sec, w.eng, false, w.sec_owner).await;
    assert!(sees_edge(&pool, w.web_member, e).await, "a fresh open");

    let edges_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_edges")
        .fetch_one(&pool)
        .await
        .unwrap();
    let folded: bool = sqlx::query_scalar("SELECT is_folded FROM kb_edges WHERE id = $1")
        .bind(e)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(edges_before, edges_after, "no edge row was written");
    assert!(!folded, "the reference was never folded");
}

/// The queue never offers what the gate refuses, and offers what it admits (R-m, PW-5/F1).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_audit_queue_matches_the_gate(pool: PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .expect("bootseed");
    let w = world(&pool).await;
    // The watcher, so that the gate's author proxy does not refuse its own team's finding.
    let p = w.web_watcher;
    let t_block = cite(&pool, w.r_t2, w.r_web).await;
    let own = resource(&pool, w.ctx_web, w.web_member, "queue-own").await;
    let own_block = cite(&pool, own, w.r_t).await;

    let queued = |pool: PgPool, finding: Uuid| async move {
        sqlx::query_scalar::<_, Uuid>("SELECT block_id FROM resource_auditable_citations($1, $2)")
            .bind(finding)
            .bind(p)
            .fetch_all(&pool)
            .await
            .unwrap()
    };
    let backend = DbBackend::new(pool.clone(), ProfileId::from(p));

    // T's finding: the queue does not offer it, and the gate refuses it.
    assert!(
        queued(pool.clone(), w.r_t2).await.is_empty(),
        "the queue offers T's finding"
    );
    assert!(backend
        .record_citation_audit(audit_cmd(t_block, w.r_web))
        .await
        .is_err());
    // The reader's own side's finding citing T: offered, and admitted.
    assert_eq!(queued(pool.clone(), own).await, vec![own_block]);
    backend
        .record_citation_audit(audit_cmd(own_block, w.r_t))
        .await
        .expect("admitted");
    // Once weighed, it is withheld until it goes stale.
    assert!(queued(pool.clone(), own).await.is_empty());

    // The finding probe is membership; the source probe is reading.
    assert!(!reads_by_membership(&pool, p, w.r_t2).await);
    assert!(reads(&pool, p, w.r_t2).await);
}

/// N9: each cause makes the row inert; undoing it restores reading, except the structural lapse.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_row_is_inert_when(pool: PgPool) {
    let w = world(&pool).await;
    let p = w.web_member;
    assert!(reads(&pool, p, w.r_t).await);

    switch(&pool, false).await;
    assert!(!reads(&pool, p, w.r_t).await, "the switch is off");
    switch(&pool, true).await;
    assert!(reads(&pool, p, w.r_t).await);

    for (forbidder, what) in [
        (w.org, "a live ancestor of A"),
        (w.ops, "T's second parent"),
        (w.eng, "A itself"),
        (w.eng_sec, "T itself"),
    ] {
        forbid(&pool, forbidder, true).await;
        assert!(!reads(&pool, p, w.r_t).await, "a forbid on {what}");
        forbid(&pool, forbidder, false).await;
        assert!(reads(&pool, p, w.r_t).await, "lifting the forbid on {what}");
    }

    // A forbid between A and T, and on a dead ancestor: A = org, the forbid on eng, then eng dead.
    close(&pool, w.eng_sec, w.eng).await;
    open(&pool, w.eng_sec, w.org, false, w.sec_owner).await;
    forbid(&pool, w.eng, true).await;
    assert!(!reads(&pool, p, w.r_t).await, "a forbid between A and T");
    forbid(&pool, w.eng, false).await;
    assert!(reads(&pool, p, w.r_t).await);
    // A dead team between A and T on the only live path, then T reached only through ops.
    set_team_live(&pool, w.eng, false).await;
    assert!(
        !reads(&pool, w.sales_member, w.r_t).await,
        "a dead team between A and T"
    );
    set_team_live(&pool, w.eng, true).await;
    assert!(reads(&pool, w.sales_member, w.r_t).await);
    close(&pool, w.eng_sec, w.org).await;
    open(&pool, w.eng_sec, w.eng, false, w.sec_owner).await;

    set_team_live(&pool, w.eng_sec, false).await;
    assert!(!reads(&pool, p, w.r_t).await, "T dead");
    set_team_live(&pool, w.eng_sec, true).await;
    assert!(reads(&pool, p, w.r_t).await, "T restored");

    // A dead: the reader's own reach through eng goes too, so the witness is the row itself.
    set_team_live(&pool, w.eng, false).await;
    let effective = |pool: PgPool| async move {
        scalar_bool_2(
            &pool,
            "SELECT public_within_row_effective($1, $2, false)",
            w.eng_sec,
            w.eng,
        )
        .await
    };
    assert!(!effective(pool.clone()).await, "A dead");
    set_team_live(&pool, w.eng, true).await;
    assert!(effective(pool.clone()).await, "A restored");

    // A pool team under A, and the row is not instance_wide.
    let pool_team = team(&pool, "eng-pool", &[w.eng]).await;
    sqlx::query("UPDATE kb_teams SET auto_join_role = 'member' WHERE id = $1")
        .bind(pool_team)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!reads(&pool, p, w.r_t).await, "a pool team under A");
    sqlx::query("UPDATE kb_teams SET auto_join_role = NULL WHERE id = $1")
        .bind(pool_team)
        .execute(&pool)
        .await
        .unwrap();
    assert!(reads(&pool, p, w.r_t).await, "the pool cleared");

    // T becomes the gating team after the row was opened: the shape trigger refused that at write
    // time, and the read-time condition keeps a later slug change from making the row confer.
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'eng-sec' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!reads(&pool, p, w.r_t).await, "T is the gating team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = NULL WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(reads(&pool, p, w.r_t).await, "the gating slug moved away");

    // The structural lapse is the exception: moving T out ends reading at once.
    unlink(&pool, w.eng, w.eng_sec).await;
    assert!(
        !reads(&pool, p, w.r_t).await,
        "A is no longer an ancestor of T"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_instance_wide_admin_row_survives_a_pool_under_a(pool: PgPool) {
    let w = world(&pool).await;
    close(&pool, w.eng_sec, w.eng).await;
    open(&pool, w.eng_sec, w.eng, true, w.admin).await;
    let pool_team = team(&pool, "eng-pool", &[w.eng]).await;
    sqlx::query("UPDATE kb_teams SET auto_join_role = 'member' WHERE id = $1")
        .bind(pool_team)
        .execute(&pool)
        .await
        .unwrap();
    assert!(reads(&pool, w.web_member, w.r_t).await);
}

/// PW-7: a resource re-homed into T shows a public reader only the trail produced in T.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_trail_of_a_rehomed_resource_starts_in_t(pool: PgPool) {
    let w = world(&pool).await;
    // Born in sales' private context, then re-homed into ctx-t.
    let moved = resource(&pool, w.ctx_sales, w.sales_member, "moved").await;
    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_id = $2, owner_profile_id = $3 WHERE resource_id = $1",
    )
    .bind(moved)
    .bind(w.ctx_t)
    .bind(w.sec_member)
    .execute(&pool)
    .await
    .unwrap();
    // One event produced in T, after the move.
    writes::set_property(
        &pool,
        ResourceId::from(moved),
        "status",
        &serde_json::json!("filed-in-t"),
        EntityId::from(emitter_of(&pool, w.sec_member).await),
    )
    .await
    .expect("an event in T");

    let trail = |pool: PgPool, who: Uuid| async move {
        sqlx::query_as::<_, (Uuid, Option<String>, Option<Uuid>)>(
            "SELECT t.event_id, e.producing_anchor_table, e.producing_anchor_id \
               FROM element_trail_node($1, $2) t JOIN kb_events e ON e.id = t.event_id",
        )
        .bind(who)
        .bind(moved)
        .fetch_all(&pool)
        .await
        .unwrap()
    };
    let member_trail = trail(pool.clone(), w.sec_member).await;
    assert!(
        member_trail.iter().any(|(_, _, a)| *a == Some(w.ctx_sales)),
        "a T member sees the whole trail, the private context's events included: {member_trail:?}"
    );
    let public_trail = trail(pool.clone(), w.web_member).await;
    assert!(
        !public_trail.is_empty(),
        "the public reader sees the events produced in T"
    );
    assert!(
        public_trail.iter().all(|(_, _, a)| *a == Some(w.ctx_t)),
        "the public reader sees only T's part of the trail: {public_trail:?}"
    );
}

/// An admin row anchored on the root reads for every person, because every closure holds the root:
/// a row names its anchor, and only that anchor's subtree reads. Never a machine.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_root_row_reaches_everyone(pool: PgPool) {
    let w = world(&pool).await;
    // T must be under the root for the root to be its ancestor.
    link(&pool, w.root, w.org).await;
    open(&pool, w.eng_sec, w.root, true, w.admin).await;
    let anyone = person(&pool, "anyone").await;
    assert!(
        reads(&pool, anyone, w.r_t).await,
        "every person's personal team sits under the root"
    );
    assert!(!reads(&pool, w.robot, w.r_t).await, "but never a machine");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_row_means_no_sideways_read(pool: PgPool) {
    let w = world(&pool).await;
    close(&pool, w.eng_sec, w.eng).await;
    for who in [
        w.web_owner,
        w.web_member,
        w.web_watcher,
        w.org_only,
        w.sales_member,
    ] {
        assert!(!reads(&pool, who, w.r_t).await);
        assert!(!ctx_reads(&pool, who, w.ctx_t).await);
    }
}

/// The siblings of `read_never_flows_sideways_or_down` (`context_read_predicate_test.rs`) and of
/// `team_child_oversight_test.rs`'s ancestor-reach pins, asserted WITH an inert row in place: a row
/// that does not confer changes nothing about how reading flows.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reading_never_flows_sideways_or_down_with_an_inert_row(pool: PgPool) {
    let w = world(&pool).await;
    switch(&pool, false).await; // the row is inert
                                // Sideways: eng-web does not read its sibling's context.
    assert!(!ctx_reads(&pool, w.web_member, w.ctx_t).await);
    // Down: eng's own roster does not read its child's context, and T does not read eng-web's.
    let eng_member = person(&pool, "eng-member").await;
    join(&pool, w.eng, eng_member, "member").await;
    assert!(!ctx_reads(&pool, eng_member, w.ctx_t).await);
    assert!(!ctx_reads(&pool, w.sec_member, w.ctx_web).await);
    // Up still flows: a T member reads an ancestor's context.
    let ctx_eng = context(&pool, w.eng, "ctx-eng").await;
    assert!(ctx_reads(&pool, w.sec_member, ctx_eng).await);
    // And with the switch on, only the row's own audience gains, and only T's owned contexts.
    switch(&pool, true).await;
    assert!(
        !ctx_reads(&pool, w.sec_member, w.ctx_web).await,
        "the arm is one-way"
    );
    assert!(
        !ctx_reads(&pool, eng_member, w.ctx_shared).await,
        "and owned-only"
    );
    assert!(
        ctx_reads(&pool, eng_member, w.ctx_t).await,
        "A's own roster is the audience"
    );
}
