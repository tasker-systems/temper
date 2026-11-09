#![cfg(feature = "test-db")]

//! `public_within`: the fixture world, the schema, policy and helpers, and the read arm with the
//! membership set for every act on T's material.
//!
//! **The world** (the plan's fixture world, built here):
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
//! `ctx-web`. The row is written directly by [`open`]: no door that opens a row exists yet. There is
//! no instance setting: a forbid on the root is the instance-wide off.
//!
//! **Deviation from the plan's world, recorded.** The plan's world puts an auto-join `eng-pool`
//! under `eng`. Present by default, it makes every `(eng-sec, eng)` row inert
//! (`anchor_subtree_holds_pool`), so every reading test would read nothing. It is created by the tests that need it instead.
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

    // A cognitive map bound to T, and an edge homed in it between two T resources: none of it is
    // passed on.
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
// Schema, policy, helpers
// =================================================================================================

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_forbid_and_no_row_exist_by_default(pool: PgPool) {
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
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_trigger_and_the_checks_refuse_the_barred_shapes(pool: PgPool) {
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

    // A row records who opened it.
    let err = sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by) VALUES ($1, $2, NULL)",
    )
    .bind(w.eng_sec_red)
    .bind(w.eng)
    .execute(&pool)
    .await
    .expect_err("an insert with no opener is refused");
    assert_eq!(
        err.as_database_error().and_then(|e| e.constraint()),
        Some("kb_team_public_within_opened_by")
    );

    // Replay restores a row as it stands, even one the guard would refuse today.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by) VALUES ($1, $2, NULL)",
    )
    .bind(w.eng_sec_red)
    .bind(w.eng)
    .execute(&mut *tx)
    .await
    .expect("replay restores an opener-less row");
    tx.rollback().await.unwrap();

    // An instance_wide row is an admin's act, on insert and on update.
    let err = sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by, instance_wide, admin_act) \
         VALUES ($1, $2, $3, true, false)",
    )
    .bind(w.eng_sec_red)
    .bind(w.eng)
    .bind(w.admin)
    .execute(&pool)
    .await
    .expect_err("instance_wide without admin_act is refused on insert");
    assert_eq!(
        err.as_database_error().and_then(|e| e.constraint()),
        Some("kb_team_public_within_instance_wide_is_admin")
    );
    open(&pool, w.eng_sec_red, w.eng, false, w.admin).await;
    let err = sqlx::query(
        "UPDATE kb_team_public_within SET instance_wide = true WHERE team_id = $1 AND anchor_id = $2",
    )
    .bind(w.eng_sec_red)
    .bind(w.eng)
    .execute(&pool)
    .await
    .expect_err("instance_wide without admin_act is refused on update");
    assert_eq!(
        err.as_database_error().and_then(|e| e.constraint()),
        Some("kb_team_public_within_instance_wide_is_admin")
    );

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

    // On a DEAD ancestor: soft-deleting the forbidding team does not lift it (a forbid fails
    // closed).
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

/// The machine fence, and the token pin: a person reads with ANY token, a Slack-minted one
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
    assert!(
        reading(pool.clone(), slack_person).await,
        "a person whose only link is Slack's: a Slack-minted token is the person"
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

// =================================================================================================
// The read arm
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

    // The change feed withholds what is newer than the oldest transaction still open anywhere on
    // the server, so wait for the horizon to pass this world's writes.
    settle(&pool).await;
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

/// Wait until every transaction that could precede this point has finished: the change feed's
/// horizon (the same helper as `edge_conjunction_harness.rs`).
async fn settle(pool: &PgPool) {
    let target: String = sqlx::query_scalar("SELECT pg_snapshot_xmax(pg_current_snapshot())::text")
        .fetch_one(pool)
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let settled: bool =
            sqlx::query_scalar("SELECT pg_snapshot_xmin(pg_current_snapshot()) >= $1::text::xid8")
                .bind(&target)
                .fetch_one(pool)
                .await
                .unwrap();
        if settled {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the horizon did not pass xid {target} within 120 s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
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

/// Machines and connection agents never read through public_within; a person does, with any token.
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
    // token, and the principal is the person. The token never reaches SQL, so the pin is that
    // a person whose only link is Slack's reads T exactly as any person does.
    let slack_person = person_via(&pool, "slack-reader", "slack").await;
    join(&pool, w.eng_web, slack_person, "member").await;
    assert!(reads(&pool, slack_person, w.r_t).await);
}

/// Shares, grants and maps held by T, and the edges homed in a map, are not passed on.
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

/// A row is not inherited: T's children stay private.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ts_children_stay_private(pool: PgPool) {
    let w = world(&pool).await;
    // Reading inherits up, never down: not even T's own members read T's child's context, so a row
    // on T, which opens what T owns, cannot open it either.
    assert!(!reads(&pool, w.sec_member, w.r_red).await);
    assert!(!reads(&pool, w.web_member, w.r_red).await);
    assert!(!ctx_reads(&pool, w.web_member, w.ctx_red).await);
    // The row on T does not open T's child's context: the arm itself never yields it.
    let opened = scalar_bool_2(
        &pool,
        "SELECT $2 IN (SELECT context_id FROM contexts_public_within($1, \
           ARRAY(SELECT team_id FROM profile_reachable_teams($1))))",
        w.web_member,
        w.ctx_red,
    )
    .await;
    assert!(!opened, "contexts_public_within opens T's child's context");
    // ...while it does open T's own context, so the probe is live.
    assert!(
        scalar_bool_2(
            &pool,
            "SELECT $2 IN (SELECT context_id FROM contexts_public_within($1, \
               ARRAY(SELECT team_id FROM profile_reachable_teams($1))))",
            w.web_member,
            w.ctx_t,
        )
        .await
    );
}

/// Read is conferred only on material the granter reads through membership. No grant row reaches
/// the fence today (`kb_access_grants_check` makes every grant authority carry `can_read`, and a grant
/// row is itself a membership arm), so this drops the CHECK in its own database to put a
/// grant-capable holder whose only read of `r_t` is `public_within` in front of the fence.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn read_is_conferred_only_on_what_the_granter_reads_by_membership(pool: PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .expect("bootseed");
    let w = world(&pool).await;
    let holder = w.web_member;
    assert!(reads(&pool, holder, w.r_t).await, "precondition: reads T");
    assert!(
        !scalar_bool_2(
            &pool,
            "SELECT can_read_by_membership($1, 'kb_resources', $2)",
            holder,
            w.r_t
        )
        .await,
        "precondition: only through public_within"
    );
    sqlx::query("ALTER TABLE kb_access_grants DROP CONSTRAINT kb_access_grants_check")
        .execute(&pool)
        .await
        .unwrap();
    for (subject, read) in [(w.r_t, false), (w.r_web, true)] {
        sqlx::query(
            "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
               can_read, can_grant, granted_by_profile_id) \
             VALUES ('kb_resources', $1, 'kb_profiles', $2, $3, true, $4)",
        )
        .bind(subject)
        .bind(holder)
        .bind(read)
        .bind(w.admin)
        .execute(&pool)
        .await
        .unwrap();
    }
    let human = test_support::human_principal_for(&pool, holder).await;
    let confer = |subject: Uuid| temper_core::types::cognitive_maps::GrantCapabilityRequest {
        subject_table: "kb_resources".into(),
        subject_id: subject,
        principal_table: "kb_profiles".into(),
        principal_id: w.sales_member,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    };
    let refused = access_service::grant_capability(&pool, &human, &confer(w.r_t)).await;
    assert!(
        matches!(refused, Err(temper_services::error::ApiError::Forbidden)),
        "conferring read on T material read only through public_within: {refused:?}"
    );
    access_service::grant_capability(&pool, &human, &confer(w.r_web))
        .await
        .expect("the same holder confers read on material it reads by membership");
}

/// For a public reader, every act ON T's material refuses; an audit of the reader's own side's
/// finding that CITES T is admitted, because citing is a reference, not an act.
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
    assert!(
        matches!(grant, Err(temper_services::error::ApiError::Forbidden)),
        "grant on a T resource: {grant:?}"
    );

    // Reassign (a two-sided act) of T's context to the reader's team.
    let reassign = context_service::reassign(&pool, &human, w.ctx_t, w.eng_web).await;
    assert!(
        matches!(reassign, Err(temper_services::error::ApiError::Forbidden)),
        "reassign ctx-t: {reassign:?}"
    );

    // An edge whose SOURCE is a T resource: T's home is not authorable.
    let from_t = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .assert_relationship(assert_cmd(w.r_t, w.r_web))
        .await;
    assert!(
        matches!(from_t, Err(temper_core::error::TemperError::Forbidden)),
        "an edge sourced on a T resource: {from_t:?}"
    );
    // ... while the same edge the other way, from the owner's own home, is a reference.
    DbBackend::new(pool.clone(), ProfileId::from(owner))
        .assert_relationship(assert_cmd(w.r_web, w.r_t))
        .await
        .expect("the reverse edge, sourced in the reader's own home, is admitted");

    let backend = DbBackend::new(pool.clone(), ProfileId::from(p));

    // An audit of a T finding refuses: T's finding cites r-web; the reader can read both, by
    // reading.
    let t_finding_block = cite(&pool, w.r_t2, w.r_web).await;
    let audit_t = backend
        .record_citation_audit(audit_cmd(t_finding_block, w.r_web))
        .await;
    assert!(
        matches!(audit_t, Err(temper_core::error::TemperError::NotFound(_))),
        "auditing T's own finding: {audit_t:?}"
    );

    // An audit of the reader's own side's finding, CITING T, is admitted. The finding is
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

/// References from the reader's own home are admitted, and homed there. A machine on the
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

/// A reference to T's material is visible exactly under the conjunction.
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

    // A lapse: T is moved out from under A. The lapse will delete the row; here the read-time
    // condition already makes it confer nothing, and only a fresh open restores it once T is back.
    unlink(&pool, w.eng, w.eng_sec).await;
    assert!(
        !sees_edge(&pool, w.web_member, e).await,
        "lapsed: A is no longer above T"
    );
    link(&pool, w.eng, w.eng_sec).await;
    close(&pool, w.eng_sec, w.eng).await; // what the lapse does to the row
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

/// The queue never offers what the gate refuses, and offers what it admits; otherwise an auditor
/// livelocks on work the gate refuses.
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
    let refused = backend
        .record_citation_audit(audit_cmd(t_block, w.r_web))
        .await;
    assert!(
        matches!(refused, Err(temper_core::error::TemperError::NotFound(_))),
        "the gate refuses T's finding: {refused:?}"
    );
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

/// Each cause makes the row inert; undoing it restores reading, except the structural lapse.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_row_is_inert_when(pool: PgPool) {
    let w = world(&pool).await;
    let p = w.web_member;
    assert!(reads(&pool, p, w.r_t).await);

    // The instance-wide off: a forbid on the root.
    link(&pool, w.root, w.org).await;
    forbid(&pool, w.root, true).await;
    assert!(!reads(&pool, p, w.r_t).await, "the root forbids");
    forbid(&pool, w.root, false).await;
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

    // T becomes the gating team after the row was opened. The shape trigger refuses that at write
    // time; the read-time check keeps a later slug change from making the row confer. An
    // instance-wide row is the witness, because a gating team under A already makes every other row
    // inert through the pool check.
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'eng-sec' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !reads(&pool, w.web_member, w.r_t).await,
        "T is the gating team"
    );
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = NULL WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        reads(&pool, w.web_member, w.r_t).await,
        "the gating slug moved away"
    );

    // With T the gating team, the row's other columns still update, and deleting its opener still
    // cascades: the guard judges only an insert or a change of team or anchor.
    let opener: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('opener', 'opener') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE kb_team_public_within SET opened_by = $3 WHERE team_id = $1 AND anchor_id = $2",
    )
    .bind(w.eng_sec)
    .bind(w.eng)
    .bind(opener)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'eng-sec' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_team_public_within SET admin_act = true WHERE team_id = $1")
        .bind(w.eng_sec)
        .execute(&pool)
        .await
        .expect("a flag change on a row whose T became gating");
    sqlx::query("DELETE FROM kb_profiles WHERE id = $1")
        .bind(opener)
        .execute(&pool)
        .await
        .expect("deleting the opener cascades SET NULL");
    let opened_by: Option<Uuid> = sqlx::query_scalar(
        "SELECT opened_by FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2",
    )
    .bind(w.eng_sec)
    .bind(w.eng)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(opened_by, None);
}

/// A resource re-homed into T shows a public reader only the trail produced in T.
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

    // Born in T's own intake (a connection's home), then re-homed into ctx-t: the intake's history
    // stays out of the public reader's trail, though T owns it.
    let intake = context(&pool, w.eng_sec, "ctx-intake").await;
    connection_agent(&pool, "intake-agent", w.sec_member, intake).await;
    let from_intake = resource(&pool, intake, w.sec_member, "from-intake").await;
    sqlx::query("UPDATE kb_resource_homes SET anchor_id = $2 WHERE resource_id = $1")
        .bind(from_intake)
        .bind(w.ctx_t)
        .execute(&pool)
        .await
        .unwrap();
    writes::set_property(
        &pool,
        ResourceId::from(from_intake),
        "status",
        &serde_json::json!("filed-in-t"),
        EntityId::from(emitter_of(&pool, w.sec_member).await),
    )
    .await
    .expect("an event in T");
    let trail_of = |pool: PgPool, who: Uuid| async move {
        sqlx::query_as::<_, (Uuid, Option<String>, Option<Uuid>)>(
            "SELECT t.event_id, e.producing_anchor_table, e.producing_anchor_id \
               FROM element_trail_node($1, $2) t JOIN kb_events e ON e.id = t.event_id",
        )
        .bind(who)
        .bind(from_intake)
        .fetch_all(&pool)
        .await
        .unwrap()
    };
    let member_trail = trail_of(pool.clone(), w.sec_member).await;
    assert!(
        member_trail.iter().any(|(_, _, a)| *a == Some(intake)),
        "a T member sees the intake's events: {member_trail:?}"
    );
    let public_trail = trail_of(pool.clone(), w.web_member).await;
    assert!(
        !public_trail.is_empty() && public_trail.iter().all(|(_, _, a)| *a == Some(w.ctx_t)),
        "the public reader sees only the events produced after the move: {public_trail:?}"
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

/// The root is a pool: every personal team sits under it, so a row anchored on the root reads
/// instance-wide and confers only as an admin's instance_wide row — whatever the gating slug says.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_root_anchored_row_confers_only_when_instance_wide(pool: PgPool) {
    let w = world(&pool).await;
    link(&pool, w.root, w.org).await;
    // No gating team and no auto-join team: the root must be a pool on its own account.
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = NULL WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_teams SET auto_join_role = NULL")
        .execute(&pool)
        .await
        .unwrap();
    assert!(scalar_bool(&pool, "SELECT anchor_subtree_holds_pool($1)", w.root).await);
    let anyone = person(&pool, "anyone").await;

    open(&pool, w.eng_sec, w.root, false, w.admin).await;
    assert!(
        !reads(&pool, anyone, w.r_t).await,
        "a root-anchored row that is not instance_wide is inert, with no gating team configured"
    );
    close(&pool, w.eng_sec, w.root).await;

    open(&pool, w.eng_sec, w.root, true, w.admin).await;
    assert!(
        reads(&pool, anyone, w.r_t).await,
        "an admin's instance_wide root row confers"
    );
}

/// A connection's home (its intake) is never opened, in any connection state, while an ordinary
/// context of the same team is; T's own members read the intake as before.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_connections_intake_context_is_never_opened(pool: PgPool) {
    let w = world(&pool).await;
    let intake = context(&pool, w.eng_sec, "ctx-intake").await;
    assert!(
        ctx_reads(&pool, w.web_member, intake).await,
        "precondition: before it is a connection's home, the arm opens it"
    );
    connection_agent(&pool, "intake-agent", w.sec_member, intake).await;
    assert!(
        !ctx_reads(&pool, w.web_member, intake).await,
        "a connection's intake is not opened to the anchor's subtree"
    );
    assert!(
        ctx_reads(&pool, w.web_member, w.ctx_t).await,
        "while T's ordinary context still is"
    );
    assert!(
        ctx_reads(&pool, w.sec_member, intake).await,
        "T's own members read the intake by membership, as before"
    );

    sqlx::query(
        "UPDATE kb_connections SET revoked_at = now(), revoked_by_profile_id = $2 \
          WHERE home_context_id = $1",
    )
    .bind(intake)
    .bind(w.sec_member)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        !ctx_reads(&pool, w.web_member, intake).await,
        "a revoked connection's intake stays closed"
    );
    assert!(ctx_reads(&pool, w.sec_member, intake).await);
}

/// The admin's instance-wide off is a forbid on the root. With org linked under the root, it makes
/// every row inert, an admin's instance_wide row anchored on the root included. Clearing it revives
/// them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_root_forbid_turns_every_row_off_and_clearing_it_revives_them(pool: PgPool) {
    let w = world(&pool).await;
    link(&pool, w.root, w.org).await;
    open(&pool, w.eng_sec_red, w.root, true, w.admin).await;
    let anyone = person(&pool, "anyone").await;
    assert!(
        reads(&pool, w.web_member, w.r_t).await,
        "precondition: the (T, eng) row"
    );
    assert!(
        reads(&pool, anyone, w.r_red).await,
        "precondition: the root row"
    );

    forbid(&pool, w.root, true).await;
    assert!(
        !reads(&pool, w.web_member, w.r_t).await,
        "the (T, eng) row is off"
    );
    assert!(
        !reads(&pool, anyone, w.r_red).await,
        "the admin's instance_wide root row is off too"
    );
    for (t, a, iw) in [(w.eng_sec, w.eng, false), (w.eng_sec_red, w.root, true)] {
        assert!(
            !scalar_bool_2(
                &pool,
                &format!("SELECT public_within_row_effective($1, $2, {iw})"),
                t,
                a
            )
            .await
        );
    }

    forbid(&pool, w.root, false).await;
    assert!(
        reads(&pool, w.web_member, w.r_t).await,
        "clearing it revives the (T, eng) row"
    );
    assert!(reads(&pool, anyone, w.r_red).await, "and the root row");
}

/// Anyone may create a parentless team, so a tree need not sit under the root. A root forbid still
/// turns its rows off: the root's flag is read directly, not only along the ancestor walk.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_root_forbid_reaches_a_tree_not_under_the_root(pool: PgPool) {
    let w = world(&pool).await;
    assert!(
        reads(&pool, w.web_member, w.r_t).await,
        "precondition: the (T, eng) row, in a tree with no link to the root"
    );

    forbid(&pool, w.root, true).await;
    assert!(
        !reads(&pool, w.web_member, w.r_t).await,
        "a root forbid turns off a row in a parentless tree"
    );
    assert!(
        !scalar_bool_2(
            &pool,
            "SELECT public_within_row_effective($1, $2, false)",
            w.eng_sec,
            w.eng
        )
        .await
    );

    forbid(&pool, w.root, false).await;
    assert!(
        reads(&pool, w.web_member, w.r_t).await,
        "clearing it revives the row"
    );
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
    // The row is inert: a forbid above T.
    forbid(&pool, w.org, true).await;
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
    // And with the forbid lifted, only the row's own audience gains, and only T's owned contexts.
    forbid(&pool, w.org, false).await;
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

// =================================================================================================
// The doors: open, close, the forbid, the detail, discovery, the ledger and a hard delete
// =================================================================================================
//
// The world above opens `(eng-sec, eng)` directly; every door test starts by closing it, so the row
// that exists is the one a door wrote. `eng` gets an owner here, the anchor side's manager.

use temper_core::types::team::{PublicWithinOpenKind, PublicWithinRef};
use temper_services::auth::HumanPrincipal;
use temper_services::error::ApiError;
use temper_services::services::admin_ledger_service;
use temper_services::services::team_service::{
    self, PUBLIC_WITHIN_ADMIN_ANCHOR, PUBLIC_WITHIN_FORBIDDEN_HERE,
};
use temper_substrate::payloads::{AnchorTable, RefTarget};

struct Doors {
    w: World,
    /// A direct owner of the anchor `eng`, and nothing else.
    eng_owner: Uuid,
    /// A direct owner of T and of `eng`: both sides in one person.
    both: Uuid,
    /// A direct owner of `org`, above both sides, managing neither.
    org_owner: Uuid,
    /// A direct owner of `ops`, T's other parent: an ancestor manager of T on another branch.
    ops_owner: Uuid,
}

async fn doors(pool: &PgPool) -> Doors {
    let w = world(pool).await;
    close(pool, w.eng_sec, w.eng).await;
    let eng_owner = person(pool, "eng-owner").await;
    join(pool, w.eng, eng_owner, "owner").await;
    let both = person(pool, "both-sides").await;
    join(pool, w.eng, both, "owner").await;
    join(pool, w.eng_sec, both, "maintainer").await;
    let org_owner = person(pool, "org-owner").await;
    join(pool, w.org, org_owner, "owner").await;
    let ops_owner = person(pool, "ops-owner").await;
    join(pool, w.ops, ops_owner, "owner").await;
    Doors {
        w,
        eng_owner,
        both,
        org_owner,
        ops_owner,
    }
}

async fn hp(pool: &PgPool, who: Uuid) -> HumanPrincipal {
    test_support::human_principal_for(pool, who).await
}

async fn open_by(
    pool: &PgPool,
    who: Uuid,
    t: Uuid,
    a: Uuid,
) -> Result<temper_core::types::team::PublicWithinOutcome, ApiError> {
    team_service::open_public_within(pool, &hp(pool, who).await, t, a).await
}

async fn close_by(pool: &PgPool, who: Uuid, t: Uuid, a: Uuid) -> Result<(), ApiError> {
    team_service::close_public_within(pool, &hp(pool, who).await, t, a).await
}

async fn policy_by(
    pool: &PgPool,
    who: Uuid,
    team: Uuid,
    forbidden: bool,
) -> Result<temper_core::types::team::PublicWithinPolicyOutcome, ApiError> {
    team_service::set_public_within_policy(pool, &hp(pool, who).await, team, forbidden).await
}

async fn row_exists(pool: &PgPool, t: Uuid, a: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2)",
    )
    .bind(t)
    .bind(a)
    .fetch_one(pool)
    .await
    .expect("row exists")
}

/// How many admin records of `event_type` name both `t` and `a` (the row records), or `t` alone
/// when `a` is `None` (the policy record).
async fn records(pool: &PgPool, event_type: &str, t: Uuid, a: Option<Uuid>) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = $1 AND (e.payload ->> 'team_id')::uuid = $2 \
            AND ($3::uuid IS NULL OR (e.payload ->> 'anchor_id')::uuid = $3)",
    )
    .bind(event_type)
    .bind(t)
    .bind(a)
    .fetch_one(pool)
    .await
    .expect("records")
}

fn forbidden_here(err: &ApiError) -> bool {
    matches!(err, ApiError::ForbiddenDetail(m) if m == PUBLIC_WITHIN_FORBIDDEN_HERE)
}

fn admin_anchor(err: &ApiError) -> bool {
    matches!(err, ApiError::ForbiddenDetail(m) if m == PUBLIC_WITHIN_ADMIN_ANCHOR)
}

/// Wait until a backend in this test's database is blocked on a lock held by `holder`.
async fn await_blocked_by(pool: &PgPool, holder: i32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)))",
        )
        .bind(holder)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the contender never blocked on the holder"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

async fn backend_pid(conn: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .expect("pid")
}

// ── Open ──────────────────────────────────────────────────────────────────────────────────────────

/// The open's rules, each refusal in its order, and then the open itself: the row, its one record,
/// and the audience.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_refuses_in_rule_order_and_then_opens(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;

    // Rule 1: the root's forbid, the instance-wide off, refuses even a caller on both sides.
    forbid(&pool, w.root, true).await;
    let err = open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap_err();
    assert!(forbidden_here(&err), "rule 1: {err:?}");
    forbid(&pool, w.root, false).await;

    // Rule 2: the anchor's owner alone is not the team's consent.
    let err = open_by(&pool, d.eng_owner, w.eng_sec, w.eng)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "rule 2: {err:?}");
    // Rule 3: the team's owner alone is not the anchor's.
    let err = open_by(&pool, w.sec_owner, w.eng_sec, w.eng)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "rule 3: {err:?}");

    // Rule 4: a leftover owner row on the root is not an admin.
    join(&pool, w.root, d.both, "owner").await;
    let err = open_by(&pool, d.both, w.eng_sec, w.root).await.unwrap_err();
    assert!(admin_anchor(&err), "rule 4: {err:?}");

    // Rule 5: an auto-join team under the anchor makes it an admin's open.
    let pool_team = team(&pool, "eng-pool", &[w.eng]).await;
    sqlx::query("UPDATE kb_teams SET auto_join_role = 'member' WHERE id = $1")
        .bind(pool_team)
        .execute(&pool)
        .await
        .unwrap();
    let err = open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap_err();
    assert!(admin_anchor(&err), "rule 5: {err:?}");
    sqlx::query("UPDATE kb_teams SET auto_join_role = NULL WHERE id = $1")
        .bind(pool_team)
        .execute(&pool)
        .await
        .unwrap();

    // Rule 6: a forbid above the team.
    forbid(&pool, w.org, true).await;
    let err = open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap_err();
    assert!(forbidden_here(&err), "rule 6: {err:?}");
    forbid(&pool, w.org, false).await;

    // Rule 7: the anchor is not above the team.
    join(&pool, w.sales, d.both, "owner").await;
    let err = open_by(&pool, d.both, w.eng_sec, w.sales)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ApiError::BadRequest(m) if m.contains("above this one")),
        "rule 7: {err:?}"
    );

    // Rule 8: a personal team is never opened (its owner manages it; the root anchor needs the
    // admin, so the caller is one).
    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(w.admin)
        .fetch_one(&pool)
        .await
        .unwrap();
    let err = open_by(&pool, w.admin, personal, w.root).await.unwrap_err();
    assert!(
        matches!(&err, ApiError::BadRequest(m) if m.contains("personal team")),
        "rule 8: {err:?}"
    );

    // Nothing was written by any refusal.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_team_public_within")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        records(&pool, "team_public_within_changed", w.eng_sec, None).await,
        0
    );

    // Both sides: opened, recorded once, and the people under eng now read T.
    let out = open_by(&pool, d.both, w.eng_sec, w.eng)
        .await
        .expect("open");
    assert_eq!(out.outcome, PublicWithinOpenKind::Opened);
    assert!(!out.instance_wide);
    assert!(
        out.audience.teams >= 3,
        "eng, eng-web, eng-sec: {:?}",
        out.audience
    );
    assert!(out.audience.principals >= 4, "{:?}", out.audience);
    assert!(reads(&pool, w.web_member, w.r_t).await);
    assert_eq!(
        records(&pool, "team_public_within_changed", w.eng_sec, Some(w.eng)).await,
        1
    );
    let admin_act: bool = sqlx::query_scalar(
        "SELECT admin_act FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2",
    )
    .bind(w.eng_sec)
    .bind(w.eng)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!admin_act, "two managers opened it; no admin stood in");

    // Again: exists, nothing written.
    let again = open_by(&pool, d.both, w.eng_sec, w.eng)
        .await
        .expect("reopen");
    assert_eq!(again.outcome, PublicWithinOpenKind::Exists);
    assert_eq!(
        records(&pool, "team_public_within_changed", w.eng_sec, Some(w.eng)).await,
        1
    );
}

/// A caller who manages neither side learns nothing: the same bare 403 for a real row, a team that
/// does not exist, and a team under a forbid.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_caller_on_no_side_gets_one_identical_refusal(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    forbid(&pool, w.ops, true).await;
    for (t, a) in [
        (w.eng_sec, w.eng),
        (Uuid::now_v7(), w.eng),
        (w.eng_sec_red, w.eng_sec),
        (w.eng_web, w.eng),
    ] {
        for who in [w.web_member, w.org_only, w.sales_member, d.org_owner] {
            let err = open_by(&pool, who, t, a).await.unwrap_err();
            assert!(matches!(err, ApiError::Forbidden), "{t} {a}: {err:?}");
        }
    }
}

/// An admin never stands in for the team side, and does stand in for the anchor side alone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_stands_in_for_the_anchor_side_only(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    // The admin manages neither side: refused at the team side, though they may stand in for A.
    let err = open_by(&pool, w.admin, w.eng_sec, w.eng).await.unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);

    // T's owner, made an admin, opens with no role on the anchor: the admin is the anchor side.
    test_support::approved_admin(&pool, w.sec_owner).await;
    let out = open_by(&pool, w.sec_owner, w.eng_sec, w.eng)
        .await
        .expect("open");
    assert_eq!(out.outcome, PublicWithinOpenKind::Opened);
    let admin_act: bool = sqlx::query_scalar(
        "SELECT admin_act FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2",
    )
    .bind(w.eng_sec)
    .bind(w.eng)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(admin_act, "the admin stood in for the anchor");
}

/// PINS CURRENT BEHAVIOUR: an admin promoted with `--team T` holds an owner row on T that nothing
/// tells apart from a real one, so it satisfies the team side. A later fix that makes the `--team`
/// promotion write no team role flips this: the open is then refused, and this test is inverted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_promoted_onto_the_team_satisfies_the_team_side_today(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let promoted = person(&pool, "promoted").await;
    let proof = test_support::system_admin_proof_for(&pool, w.admin).await;
    access_service::promote_admin(&pool, &proof, promoted, Some(w.eng_sec))
        .await
        .expect("promote onto T");
    let out = open_by(&pool, promoted, w.eng_sec, w.eng)
        .await
        .expect("today: the promoted admin is the team side and stands in for the anchor");
    assert_eq!(out.outcome, PublicWithinOpenKind::Opened);
}

/// An admin's open over the root is `instance_wide`: every person reads T, and the root's forbid
/// turns it off like any other row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_open_over_the_root_is_instance_wide(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    link(&pool, w.root, w.org).await;
    test_support::approved_admin(&pool, w.sec_owner).await;
    let out = open_by(&pool, w.sec_owner, w.eng_sec, w.root)
        .await
        .expect("open");
    assert!(out.instance_wide);
    let (iw, aa): (bool, bool) = sqlx::query_as(
        "SELECT instance_wide, admin_act FROM kb_team_public_within \
          WHERE team_id = $1 AND anchor_id = $2",
    )
    .bind(w.eng_sec)
    .bind(w.root)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(iw && aa);
    assert!(
        reads(&pool, w.sales_member, w.r_t).await,
        "every person reads T"
    );
    assert!(!reads(&pool, w.robot, w.r_t).await, "never a machine");

    policy_by(&pool, w.admin, w.root, true)
        .await
        .expect("instance-wide off");
    assert!(!reads(&pool, w.sales_member, w.r_t).await);
}

/// Authority runs twice: a T manager demoted while the open waits on the team's lock opens nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_caller_demoted_while_the_open_waits_opens_nothing(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(w.eng_sec)
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE kb_team_members SET role = 'member' WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(w.eng_sec)
    .bind(d.both)
    .execute(&mut *holder)
    .await
    .unwrap();

    let p2 = pool.clone();
    let (both, t, a) = (d.both, w.eng_sec, w.eng);
    let contender = tokio::spawn(async move { open_by(&p2, both, t, a).await });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();

    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    assert_eq!(
        records(&pool, "team_public_within_changed", w.eng_sec, None).await,
        0
    );
}

/// The race with the instance-wide off, forbid first: a root forbid committed while the open waits
/// is seen by the open's re-check, and nothing is opened.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_root_forbid_committed_first_refuses_a_concurrent_open(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let admin_emitter = emitter_of(&pool, w.admin).await;

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    let _: serde_json::Value =
        sqlx::query_scalar("SELECT _admin_public_within_policy_write($1, $2, $3, true, NULL)")
            .bind(admin_emitter)
            .bind(w.admin)
            .bind(w.root)
            .fetch_one(&mut *holder)
            .await
            .unwrap();

    let p2 = pool.clone();
    let (both, t, a) = (d.both, w.eng_sec, w.eng);
    let contender = tokio::spawn(async move { open_by(&p2, both, t, a).await });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();

    let err = contender.await.unwrap().unwrap_err();
    assert!(forbidden_here(&err), "{err:?}");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
}

/// The same race, open first: an open committed while the root forbid waits is in the forbid's
/// scan, so it goes inert with its `public_within_effect_changed` recorded, and the answer names it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_open_committed_first_is_made_inert_and_recorded_by_the_root_forbid(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let emitter = emitter_of(&pool, d.both).await;

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query("SELECT _public_within_open_locks($1, $2)")
        .bind(w.eng_sec)
        .bind(w.eng)
        .execute(&mut *holder)
        .await
        .unwrap();
    let _: String = sqlx::query_scalar(
        "SELECT _admin_public_within_write($1, $2, $3, $4, 'opened', false, false, NULL)",
    )
    .bind(emitter)
    .bind(d.both)
    .bind(w.eng_sec)
    .bind(w.eng)
    .fetch_one(&mut *holder)
    .await
    .unwrap();

    let p2 = pool.clone();
    let (admin, root) = (w.admin, w.root);
    let contender = tokio::spawn(async move { policy_by(&p2, admin, root, true).await });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();

    let out = contender.await.unwrap().expect("forbid");
    assert!(out.changed);
    assert_eq!(out.dormant.len(), 1, "{out:?}");
    assert_eq!(out.dormant[0].team.id, w.eng_sec);
    assert_eq!(out.dormant[0].anchor.id, w.eng);
    assert_eq!(
        records(
            &pool,
            "public_within_effect_changed",
            w.eng_sec,
            Some(w.eng)
        )
        .await,
        1
    );
    assert!(!reads(&pool, w.web_member, w.r_t).await);
}

// ── Close ─────────────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn either_side_closes_alone_and_a_non_manager_learns_nothing(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();

    // Nobody else closes, and the refusal is the same whether or not a row exists.
    for who in [w.web_member, w.sec_member, d.org_owner, d.ops_owner] {
        for (t, a) in [
            (w.eng_sec, w.eng),
            (w.eng_web, w.eng),
            (Uuid::now_v7(), w.eng),
        ] {
            let err = close_by(&pool, who, t, a).await.unwrap_err();
            assert!(matches!(err, ApiError::Forbidden), "{err:?}");
        }
    }
    assert!(row_exists(&pool, w.eng_sec, w.eng).await);

    // The team side.
    close_by(&pool, w.sec_owner, w.eng_sec, w.eng)
        .await
        .expect("T side");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    // A side manager on a missing row: 404.
    let err = close_by(&pool, d.eng_owner, w.eng_sec, w.eng)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::NotFound(_)), "{err:?}");
    // The anchor side.
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    close_by(&pool, d.eng_owner, w.eng_sec, w.eng)
        .await
        .expect("A side");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    assert_eq!(
        records(&pool, "team_public_within_changed", w.eng_sec, Some(w.eng)).await,
        4,
        "two opens and two closes, one record each"
    );
}

/// Closing works whatever either side's liveness: the live side closes when the other is dead, and
/// an admin when both are.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dead_side_does_not_stop_a_close(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    set_team_live(&pool, w.eng, false).await;
    // A dead team confers nothing, its managers' standing included...
    let err = close_by(&pool, d.eng_owner, w.eng_sec, w.eng)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    // ...but the live side closes.
    close_by(&pool, w.sec_owner, w.eng_sec, w.eng)
        .await
        .expect("T side");

    set_team_live(&pool, w.eng, true).await;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    set_team_live(&pool, w.eng, false).await;
    set_team_live(&pool, w.eng_sec, false).await;
    close_by(&pool, w.admin, w.eng_sec, w.eng)
        .await
        .expect("admin");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
}

/// On a root anchor, a leftover owner row on the root closes nothing; the team side and an admin do.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_root_anchor_is_closed_from_the_anchor_side_by_an_admin_only(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    link(&pool, w.root, w.org).await;
    test_support::approved_admin(&pool, w.sec_owner).await;
    open_by(&pool, w.sec_owner, w.eng_sec, w.root)
        .await
        .unwrap();
    let leftover = person(&pool, "root-leftover").await;
    join(&pool, w.root, leftover, "owner").await;
    let err = close_by(&pool, leftover, w.eng_sec, w.root)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    close_by(&pool, w.admin, w.eng_sec, w.root)
        .await
        .expect("admin");
    assert!(!row_exists(&pool, w.eng_sec, w.root).await);
}

// ── The forbid ────────────────────────────────────────────────────────────────────────────────────

/// Who may forbid: the direct owner (not a maintainer), an admin; on the root an admin only, a
/// non-admin owner row there included; never on a personal team.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_forbid_is_the_owners_and_on_the_root_an_admins_alone(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let org_maint = person(&pool, "org-maint").await;
    join(&pool, w.org, org_maint, "maintainer").await;
    let err = policy_by(&pool, org_maint, w.org, true).await.unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    let err = policy_by(&pool, w.web_member, w.org, true)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(
        policy_by(&pool, d.org_owner, w.org, true)
            .await
            .unwrap()
            .changed
    );
    assert!(
        policy_by(&pool, w.admin, w.org, false)
            .await
            .unwrap()
            .changed
    );

    // A non-admin who holds an owner row on the root cannot turn the instance off.
    let root_owner = person(&pool, "root-owner").await;
    join(&pool, w.root, root_owner, "owner").await;
    let err = policy_by(&pool, root_owner, w.root, true)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(
        policy_by(&pool, w.admin, w.root, true)
            .await
            .unwrap()
            .changed
    );
    let unchanged = policy_by(&pool, w.admin, w.root, true).await.unwrap();
    assert!(!unchanged.changed, "already so: nothing recorded");
    assert_eq!(
        records(&pool, "public_within_policy_changed", w.root, None).await,
        1
    );

    // A personal team has no forbid, for its owner or an admin.
    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(w.web_member)
        .fetch_one(&pool)
        .await
        .unwrap();
    for who in [w.web_member, w.admin] {
        let err = policy_by(&pool, who, personal, true).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
    }
}

/// A forbid at `org` makes `(eng-sec, eng)` inert and answers it: named to an admin, counted for an
/// owner who manages neither side; clearing it revives the row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_forbid_above_makes_the_row_inert_and_answers_it(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    assert!(reads(&pool, w.web_member, w.r_t).await);

    let out = policy_by(&pool, d.org_owner, w.org, true).await.unwrap();
    assert!(
        out.dormant.is_empty(),
        "org's owner manages neither side: {out:?}"
    );
    assert_eq!(out.dormant_unnamed, 1);
    assert!(!reads(&pool, w.web_member, w.r_t).await);
    assert_eq!(
        records(
            &pool,
            "public_within_effect_changed",
            w.eng_sec,
            Some(w.eng)
        )
        .await,
        1
    );

    let out = policy_by(&pool, w.admin, w.org, false).await.unwrap();
    assert_eq!(out.revived.len(), 1, "{out:?}");
    assert_eq!(out.revived[0].team.id, w.eng_sec);
    assert!(reads(&pool, w.web_member, w.r_t).await);
    assert_eq!(
        records(
            &pool,
            "public_within_effect_changed",
            w.eng_sec,
            Some(w.eng)
        )
        .await,
        2
    );
}

/// What does not lift a forbid: a lower owner's own allow, and soft-deleting the forbidding team.
/// What does: moving T out of the forbidding team's subtree.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn only_leaving_the_subtree_lifts_a_forbid(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    policy_by(&pool, d.org_owner, w.org, true).await.unwrap();

    // eng's owner forbids and allows their own team: one more restriction, never a lift.
    policy_by(&pool, d.eng_owner, w.eng, true).await.unwrap();
    let out = policy_by(&pool, d.eng_owner, w.eng, false).await.unwrap();
    assert!(
        out.revived.is_empty() && out.revived_unnamed == 0,
        "{out:?}"
    );
    assert!(!reads(&pool, w.web_member, w.r_t).await);

    // Soft-deleting the forbidding team does not lift it.
    set_team_live(&pool, w.org, false).await;
    assert!(!reads(&pool, w.web_member, w.r_t).await);
    set_team_live(&pool, w.org, true).await;
    policy_by(&pool, d.org_owner, w.org, false).await.unwrap();

    // A forbid on T's other parent; detaching T from it lifts it, since T is no longer beneath, and
    // the detach records the row's change of effect and answers it to T's manager.
    policy_by(&pool, d.ops_owner, w.ops, true).await.unwrap();
    assert!(!reads(&pool, w.web_member, w.r_t).await);
    let before = records(
        &pool,
        "public_within_effect_changed",
        w.eng_sec,
        Some(w.eng),
    )
    .await;
    let out = team_service::detach_child(&pool, &hp(&pool, w.sec_owner).await, w.ops, w.eng_sec)
        .await
        .expect("T's owner detaches T from its other parent");
    assert_eq!(out.revived.len(), 1, "{out:?}");
    assert_eq!(out.revived[0].anchor.id, w.eng);
    assert_eq!(
        records(
            &pool,
            "public_within_effect_changed",
            w.eng_sec,
            Some(w.eng)
        )
        .await,
        before + 1
    );
    assert!(reads(&pool, w.web_member, w.r_t).await);
}

/// A forbid reaches a row below a dead team: the forbid fails closed, so its transition scan walks
/// through dead teams as the forbid itself does.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_forbid_records_a_row_below_a_dead_team(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    // top → mid (dead) → org → eng → eng-sec: top's forbid reaches the row through the dead team.
    let top = team(&pool, "top", &[]).await;
    let top_owner = person(&pool, "top-owner").await;
    join(&pool, top, top_owner, "owner").await;
    let mid = team(&pool, "mid", &[top]).await;
    link(&pool, mid, w.org).await;
    set_team_live(&pool, mid, false).await;
    let out = policy_by(&pool, top_owner, top, true).await.unwrap();
    assert_eq!(out.dormant_unnamed, 1, "{out:?}");
    assert!(!reads(&pool, w.web_member, w.r_t).await);
}

// ── The detail ────────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_detail_shows_each_side_to_its_own_audience_only(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let eng_member = person(&pool, "eng-member").await;
    join(&pool, w.eng, eng_member, "member").await;

    // T's members (any role) and admins see `public_within`.
    for who in [w.sec_member, w.sec_owner, w.admin] {
        let caller = test_support::caller_for(&pool, who).await;
        let detail = team_service::team_detail(&pool, &caller, w.eng_sec)
            .await
            .unwrap();
        assert_eq!(detail.public_within.len(), 1, "{who}");
        assert_eq!(detail.public_within[0].anchor.id, w.eng);
        assert!(detail.public_within[0].effective);
    }
    // An ancestor's manager on another branch does not, whether or not they read the detail.
    let caller = test_support::caller_for(&pool, d.ops_owner).await;
    let detail = team_service::team_detail(&pool, &caller, w.eng_sec)
        .await
        .expect("an ancestor's manager reads the detail");
    assert!(detail.public_within.is_empty());
    // Nor does a machine member of T: it never reads through a row.
    join(&pool, w.eng_sec, w.robot, "member").await;
    let robot = test_support::machine_caller_for(&pool, w.robot).await;
    let detail = team_service::team_detail(&pool, &robot, w.eng_sec)
        .await
        .expect("a member reads the detail");
    assert!(detail.public_within.is_empty());
    // Nor a connection agent, which classifies as a human caller but is outside the read arm's
    // fence: the detail follows the fence, not the caller kind.
    join(&pool, w.eng_sec, w.agent, "member").await;
    let agent = test_support::caller_for(&pool, w.agent).await;
    assert!(matches!(agent, temper_services::auth::Caller::Human(_)));
    let detail = team_service::team_detail(&pool, &agent, w.eng_sec)
        .await
        .expect("a member reads the detail");
    assert!(detail.public_within.is_empty());

    // A's managers and admins see `public_within_in`; A's plain member does not.
    for (who, sees) in [(d.eng_owner, true), (w.admin, true), (eng_member, false)] {
        let caller = test_support::caller_for(&pool, who).await;
        let detail = team_service::team_detail(&pool, &caller, w.eng)
            .await
            .unwrap();
        assert_eq!(detail.public_within_in.len(), usize::from(sees), "{who}");
        assert!(detail.public_within.is_empty());
    }

    // `effective` is a boolean and names nothing: under a forbid the entry's keys are unchanged.
    forbid(&pool, w.org, true).await;
    let caller = test_support::caller_for(&pool, w.sec_member).await;
    let detail = team_service::team_detail(&pool, &caller, w.eng_sec)
        .await
        .unwrap();
    let entry: &PublicWithinRef = &detail.public_within[0];
    assert!(!entry.effective);
    let v = serde_json::to_value(entry).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "anchor",
            "audience",
            "effective",
            "instance_wide",
            "opened_at"
        ]
    );
    assert!(v["effective"].is_boolean());
}

// ── Discovery ─────────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn discovery_lists_only_rows_that_confer_on_the_caller(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let mine = team_service::list_public_within(&pool, &hp(&pool, w.web_member).await)
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!((mine[0].team.id, mine[0].anchor.id), (w.eng_sec, w.eng));
    // Outside the closure.
    let theirs = team_service::list_public_within(&pool, &hp(&pool, w.sales_member).await)
        .await
        .unwrap();
    assert!(theirs.is_empty());
    // Inert.
    forbid(&pool, w.org, true).await;
    let mine = team_service::list_public_within(&pool, &hp(&pool, w.web_member).await)
        .await
        .unwrap();
    assert!(mine.is_empty());
}

// ── The ledger ────────────────────────────────────────────────────────────────────────────────────

/// Both sides' managers read all three record types on their own team; nobody else does.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn both_sides_managers_read_the_records_and_nobody_else(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    policy_by(&pool, w.sec_owner, w.eng_sec, true)
        .await
        .unwrap();
    policy_by(&pool, w.sec_owner, w.eng_sec, false)
        .await
        .unwrap();

    let types_seen = |entries: &[admin_ledger_service::AdminLedgerEntry]| {
        let mut t: Vec<String> = entries.iter().map(|e| e.event_type.clone()).collect();
        t.sort();
        t.dedup();
        t
    };
    let read = |who: Uuid, team: Uuid| {
        let pool = pool.clone();
        async move {
            admin_ledger_service::list_by_subject(
                &pool,
                &hp(&pool, who).await,
                RefTarget {
                    kind: AnchorTable::Teams,
                    id: team,
                },
                100,
                0,
            )
            .await
        }
    };

    let t_side = read(w.sec_owner, w.eng_sec).await.unwrap();
    let seen = types_seen(&t_side);
    for ty in [
        "public_within_effect_changed",
        "public_within_policy_changed",
        "team_public_within_changed",
    ] {
        assert!(seen.iter().any(|s| s == ty), "T side reads {ty}: {seen:?}");
    }
    let a_side = read(d.eng_owner, w.eng).await.unwrap();
    let seen = types_seen(&a_side);
    for ty in ["public_within_effect_changed", "team_public_within_changed"] {
        assert!(seen.iter().any(|s| s == ty), "A side reads {ty}: {seen:?}");
    }

    for (who, team) in [
        (w.sec_member, w.eng_sec),
        (w.web_member, w.eng),
        (d.org_owner, w.eng_sec),
        (d.ops_owner, w.eng_sec),
    ] {
        let err = read(who, team).await.unwrap_err();
        assert!(matches!(err, ApiError::NotFound(_)), "{who}: {err:?}");
    }
}

// ── Hard delete ───────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_hard_deleted_team_records_each_row_before_the_cascade(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let t2 = team(&pool, "eng-tmp", &[w.eng]).await;
    join(&pool, t2, d.both, "owner").await;
    open_by(&pool, d.both, t2, w.eng).await.unwrap();
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    sqlx::query("DELETE FROM kb_teams WHERE id = $1")
        .bind(t2)
        .execute(&pool)
        .await
        .expect("hard delete");
    assert!(!row_exists(&pool, t2, w.eng).await);
    let acts: Vec<String> = sqlx::query_scalar(
        "SELECT e.payload ->> 'act' FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'team_public_within_changed' AND (e.payload ->> 'team_id')::uuid = $1 \
          ORDER BY e.id",
    )
    .bind(t2)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(acts, ["opened", "cascade_deleted"]);
    assert!(
        row_exists(&pool, w.eng_sec, w.eng).await,
        "another team's row is untouched"
    );
}

/// The open's lock set is computed before it waits: a link committed while it waited gives T an
/// ancestor whose forbid would not serialize with it. The open sees the grown set and refuses with a
/// retryable conflict, writing nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_ancestor_gained_while_the_open_waits_is_a_retryable_conflict(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let q = team(&pool, "q", &[]).await;

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(w.eng_sec)
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(q)
        .bind(w.eng_sec)
        .execute(&mut *holder)
        .await
        .unwrap();

    let p2 = pool.clone();
    let (both, t, a) = (d.both, w.eng_sec, w.eng);
    let contender = tokio::spawn(async move { open_by(&p2, both, t, a).await });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();

    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Conflict(_)), "{err:?}");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    // Retried, it opens.
    open_by(&pool, d.both, w.eng_sec, w.eng)
        .await
        .expect("retry");
}

/// The forbid's answer counts the rows it may not name; its effect records are the system's, so the
/// forbidder's own history in the ledger does not name them either.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_forbidders_ledger_history_does_not_name_withheld_rows(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let out = policy_by(&pool, d.org_owner, w.org, true).await.unwrap();
    assert_eq!(out.dormant_unnamed, 1);
    let mine = admin_ledger_service::list_by_actor(
        &pool,
        &hp(&pool, d.org_owner).await,
        ProfileId::from(d.org_owner),
        100,
        0,
    )
    .await
    .expect("own history");
    assert!(mine
        .iter()
        .any(|e| e.event_type == "public_within_policy_changed"));
    assert!(
        !mine
            .iter()
            .any(|e| e.event_type == "public_within_effect_changed"),
        "{mine:?}"
    );
    assert_eq!(
        records(
            &pool,
            "public_within_effect_changed",
            w.eng_sec,
            Some(w.eng)
        )
        .await,
        1,
        "the record exists, on the subject axis of both sides"
    );
}

// =================================================================================================
// Structural and admin acts carry their effect
// =================================================================================================
//
// Every act that can change whether a row confers records the change (system-emitted, both teams as
// subject), answers it filtered, and deletes a row whose anchor is no longer above its team.

use temper_core::types::team::{PublicWithinEffects, ReparentRequest, TeamCreateRequest, TeamRole};

/// The payload of every admin record of `event_type` naming row `(t, a)`, oldest first.
async fn row_records(pool: &PgPool, event_type: &str, t: Uuid, a: Uuid) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = $1 AND (e.payload ->> 'team_id')::uuid = $2 \
            AND (e.payload ->> 'anchor_id')::uuid = $3 ORDER BY e.id",
    )
    .bind(event_type)
    .bind(t)
    .bind(a)
    .fetch_all(pool)
    .await
    .expect("row records")
}

/// Does `(t, a)` confer right now?
async fn effective(pool: &PgPool, t: Uuid, a: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT public_within_row_effective(team_id, anchor_id, instance_wide) \
           FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2",
    )
    .bind(t)
    .bind(a)
    .fetch_optional(pool)
    .await
    .expect("effective")
    .unwrap_or(false)
}

fn names(effects: &PublicWithinEffects, list: &str) -> Vec<(Uuid, Uuid)> {
    let rows = match list {
        "lapsed" => &effects.lapsed,
        "dormant" => &effects.dormant,
        _ => &effects.revived,
    };
    rows.iter().map(|r| (r.team.id, r.anchor.id)).collect()
}

async fn reparent_by(
    pool: &PgPool,
    who: Uuid,
    child: Uuid,
    from: Uuid,
    to: Uuid,
) -> temper_core::types::team::ReparentOutcome {
    team_service::reparent(
        pool,
        &hp(pool, who).await,
        child,
        &ReparentRequest {
            from: from.to_string(),
            to: to.to_string(),
        },
    )
    .await
    .expect("reparent")
}

/// Moving T out from under A deletes the row (`lapsed`), recorded on both sides' subject axis and
/// answered by name to an admin; moving T back does not revive it, and nobody reads T again until
/// both sides reopen.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn moving_t_out_from_under_a_deletes_the_row_and_moving_it_back_does_not_revive_it(
    pool: PgPool,
) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    assert!(reads(&pool, w.web_member, w.r_t).await);

    let out = reparent_by(&pool, w.admin, w.eng_sec, w.eng, w.sales).await;
    assert_eq!(names(&out.public_within, "lapsed"), [(w.eng_sec, w.eng)]);
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    let lapsed = row_records(&pool, "team_public_within_changed", w.eng_sec, w.eng).await;
    assert_eq!(lapsed.last().unwrap()["act"], "lapsed", "{lapsed:?}");
    assert!(
        lapsed.last().unwrap().get("actor").is_none(),
        "a lapse is the system's record, naming no actor"
    );
    assert!(!reads(&pool, w.web_member, w.r_t).await);

    // Both sides read the lapse on their subject axis.
    for (who, team) in [(w.sec_owner, w.eng_sec), (d.eng_owner, w.eng)] {
        let seen = admin_ledger_service::list_by_subject(
            &pool,
            &hp(&pool, who).await,
            RefTarget {
                kind: AnchorTable::Teams,
                id: team,
            },
            100,
            0,
        )
        .await
        .expect("subject axis");
        assert!(
            seen.iter()
                .any(|e| e.event_type == "team_public_within_changed"
                    && e.payload["act"] == "lapsed"),
            "{who} reads the lapse"
        );
    }

    // A third party moving T back revives nothing: the row is gone, and so is the reading.
    let back = reparent_by(&pool, w.admin, w.eng_sec, w.sales, w.eng).await;
    assert_eq!(back.public_within, PublicWithinEffects::default());
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    assert!(!reads(&pool, w.web_member, w.r_t).await);
}

/// Deleting a team between A and T makes the row dormant; restoring it revives it. Both are
/// recorded with cause liveness and answered.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn deleting_a_mid_tier_team_makes_rows_dormant_and_restoring_revives_them(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    join(&pool, w.org, d.both, "owner").await;
    open_by(&pool, d.both, w.eng_sec, w.org).await.unwrap();
    assert!(reads(&pool, w.org_only, w.r_t).await);

    let deleted = team_service::delete_team(&pool, &hp(&pool, w.admin).await, w.eng)
        .await
        .expect("delete eng");
    assert_eq!(names(&deleted, "dormant"), [(w.eng_sec, w.org)]);
    assert!(
        row_exists(&pool, w.eng_sec, w.org).await,
        "liveness never deletes"
    );
    assert!(!reads(&pool, w.org_only, w.r_t).await);

    let restored = team_service::restore_team(&pool, &hp(&pool, w.admin).await, w.eng)
        .await
        .expect("restore eng");
    assert_eq!(
        names(&restored.public_within, "revived"),
        [(w.eng_sec, w.org)]
    );
    assert!(reads(&pool, w.org_only, w.r_t).await);

    let recs = row_records(&pool, "public_within_effect_changed", w.eng_sec, w.org).await;
    let shape: Vec<(String, String, String)> = recs
        .iter()
        .map(|r| {
            (
                r["from"].as_str().unwrap().to_string(),
                r["to"].as_str().unwrap().to_string(),
                r["cause"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("effective".into(), "dormant".into(), "liveness".into()),
            ("dormant".into(), "effective".into(), "liveness".into()),
        ]
    );
    assert!(recs
        .iter()
        .all(|r| r["act"] == "team_lifecycle_changed" && r.get("act_ref").is_some()));
}

/// An admin's auto-join on a team under A makes a non-instance-wide row dormant (cause policy),
/// answered and recorded; clearing it revives the row. An admin's instance-wide row is untouched.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admins_auto_join_under_a_makes_non_instance_wide_rows_dormant(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let admin = test_support::system_admin_proof_for(&pool, w.admin).await;

    let set = team_service::set_auto_join(&pool, &admin, w.eng_web, Some(TeamRole::Member))
        .await
        .expect("auto-join");
    assert_eq!(names(&set.public_within, "dormant"), [(w.eng_sec, w.eng)]);
    assert!(!effective(&pool, w.eng_sec, w.eng).await);
    let recs = row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng).await;
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["cause"], "policy");

    let cleared = team_service::set_auto_join(&pool, &admin, w.eng_web, None)
        .await
        .expect("clear");
    assert_eq!(
        names(&cleared.public_within, "revived"),
        [(w.eng_sec, w.eng)]
    );
    assert!(effective(&pool, w.eng_sec, w.eng).await);
}

/// Pointing the gating slug into A's subtree makes the row dormant, in the settings write's own
/// transaction, recorded; pointing it away revives it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn pointing_the_gating_slug_into_as_subtree_makes_rows_dormant(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let admin = test_support::system_admin_proof_for(&pool, w.admin).await;
    let previous: Option<String> =
        sqlx::query_scalar("SELECT gating_team_slug FROM kb_system_settings WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let to = |slug: &str| temper_core::types::admin::UpdateSettingsRequest {
        gating_team_slug: Some(slug.to_string()),
        instance_name: None,
        terms_version: None,
        terms_resource_uri: None,
    };

    access_service::update_system_settings(&pool, &admin, &to("eng-web"))
        .await
        .expect("repoint");
    assert!(!effective(&pool, w.eng_sec, w.eng).await);
    let recs = row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng).await;
    assert_eq!(recs.len(), 1, "{recs:?}");
    assert_eq!(recs[0]["to"], "dormant");
    assert_eq!(recs[0]["cause"], "policy");

    access_service::update_system_settings(
        &pool,
        &admin,
        &to(previous.as_deref().unwrap_or("sales")),
    )
    .await
    .expect("repoint away");
    assert!(effective(&pool, w.eng_sec, w.eng).await);
    assert_eq!(
        row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng)
            .await
            .len(),
        2
    );
}

/// A caller who manages only the child of a cut gets counts, never refs: a lapse caused by a
/// middle team's owner detaching it is counted in their answer, and their own history in the ledger
/// names no row either.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_caller_who_manages_only_the_child_gets_counts_never_refs(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    // eng → mid → t2, the row (t2, eng); mid's owner manages mid alone.
    let mid = team(&pool, "mid", &[w.eng]).await;
    let t2 = team(&pool, "t2", &[mid]).await;
    join(&pool, t2, d.both, "owner").await;
    let mid_owner = person(&pool, "mid-owner").await;
    join(&pool, mid, mid_owner, "owner").await;
    open_by(&pool, d.both, t2, w.eng).await.unwrap();

    let out = team_service::detach_child(&pool, &hp(&pool, mid_owner).await, w.eng, mid)
        .await
        .expect("the child's owner detaches it");
    assert!(out.lapsed.is_empty(), "{out:?}");
    assert_eq!(out.lapsed_unnamed, 1);
    assert!(!row_exists(&pool, t2, w.eng).await);

    let mine = admin_ledger_service::list_by_actor(
        &pool,
        &hp(&pool, mid_owner).await,
        ProfileId::from(mid_owner),
        100,
        0,
    )
    .await
    .expect("own history");
    assert!(mine
        .iter()
        .any(|e| e.event_type == "team_parent_link_changed"));
    assert!(
        !mine
            .iter()
            .any(|e| e.event_type == "team_public_within_changed"
                || e.event_type == "public_within_effect_changed"),
        "{mine:?}"
    );

    // Attach: the new child's roster reads the row through its parent, counted to a caller who
    // manages the child and the parent but neither side of the row.
    let u = team(&pool, "u", &[]).await;
    let u_owner = person(&pool, "u-owner").await;
    join(&pool, u, u_owner, "owner").await;
    join(&pool, w.eng_web, u_owner, "maintainer").await;
    let out = team_service::attach_child(&pool, &hp(&pool, u_owner).await, w.eng_web, u)
        .await
        .expect("attach");
    assert!(out.public_within_in_reach.rows.is_empty(), "{out:?}");
    assert_eq!(
        out.public_within_in_reach.unnamed, 0,
        "t2's row lapsed; (eng-sec, eng) is closed"
    );
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let v = team(&pool, "v", &[]).await;
    join(&pool, v, u_owner, "owner").await;
    let out = team_service::attach_child(&pool, &hp(&pool, u_owner).await, w.eng_web, v)
        .await
        .expect("attach");
    assert!(out.public_within_in_reach.rows.is_empty(), "{out:?}");
    assert_eq!(out.public_within_in_reach.unnamed, 1);
    // T's manager attaching the same way is told which row.
    let x = team(&pool, "x", &[]).await;
    join(&pool, x, w.sec_owner, "owner").await;
    join(&pool, w.eng_web, w.sec_owner, "maintainer").await;
    let out = team_service::attach_child(&pool, &hp(&pool, w.sec_owner).await, w.eng_web, x)
        .await
        .expect("attach");
    assert_eq!(out.public_within_in_reach.rows.len(), 1, "{out:?}");
    assert_eq!(out.public_within_in_reach.rows[0].team.id, w.eng_sec);
}

/// A maintainer who attaches their team under a root team they own and forbid on makes the team's
/// rows inert: the answer and the ledger name them, so the forbid's reach is never silent.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn attaching_under_ones_own_forbidding_team_makes_rows_inert_and_names_them(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let top = team(&pool, "top", &[]).await;
    let maint = person(&pool, "maint").await;
    join(&pool, top, maint, "owner").await;
    join(&pool, w.eng_sec, maint, "maintainer").await;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    policy_by(&pool, maint, top, true).await.unwrap();
    assert!(effective(&pool, w.eng_sec, w.eng).await);

    let out = team_service::attach_child(&pool, &hp(&pool, maint).await, top, w.eng_sec)
        .await
        .expect("attach under the forbidding team");
    assert_eq!(names(&out.public_within, "dormant"), [(w.eng_sec, w.eng)]);
    assert!(!effective(&pool, w.eng_sec, w.eng).await);
    let recs = row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng).await;
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["cause"], "policy");
    assert_eq!(recs[0]["act"], "team_parent_link_changed");
}

/// Creating an auto-join team under A (an admin's act) makes the row dormant, answered and recorded;
/// the new team's roster's reach is answered too.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn creating_an_auto_join_team_under_a_makes_rows_dormant(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    // A child of eng needs a manager of eng; auto-join needs a system admin.
    join(&pool, w.eng, w.admin, "owner").await;
    let out = team_service::create_team_with_effects(
        &pool,
        &hp(&pool, w.admin).await,
        &TeamCreateRequest {
            slug: "eng-pool".to_string(),
            parent: Some(w.eng.to_string()),
            auto_join_role: Some(TeamRole::Member),
            ..Default::default()
        },
    )
    .await
    .expect("create");
    assert_eq!(names(&out.public_within, "dormant"), [(w.eng_sec, w.eng)]);
    assert!(
        out.public_within_in_reach.rows.is_empty(),
        "the row no longer confers"
    );
    assert_eq!(
        row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng)
            .await
            .len(),
        1
    );

    // A plain child of eng, by eng's owner: no effect change.
    let out = team_service::create_team_with_effects(
        &pool,
        &hp(&pool, d.eng_owner).await,
        &TeamCreateRequest {
            slug: "eng-new".to_string(),
            parent: Some(w.eng.to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("create plain");
    assert_eq!(out.public_within, PublicWithinEffects::default());
}

/// The riskiest pairing: an auto-join set below the anchor changes a row's effect through a team the
/// open does not lock. An open committed while the auto-join waits is in the auto-join's snapshot, so
/// its change of effect is recorded rather than lost.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_open_committed_first_is_recorded_by_a_concurrent_auto_join_below_the_anchor(
    pool: PgPool,
) {
    let d = doors(&pool).await;
    let w = &d.w;
    let emitter = emitter_of(&pool, d.both).await;

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    let stable: bool = sqlx::query_scalar("SELECT _public_within_open_locks($1, $2)")
        .bind(w.eng_sec)
        .bind(w.eng)
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    assert!(stable);
    let _: String = sqlx::query_scalar(
        "SELECT _admin_public_within_write($1, $2, $3, $4, 'opened', false, false, NULL)",
    )
    .bind(emitter)
    .bind(d.both)
    .bind(w.eng_sec)
    .bind(w.eng)
    .fetch_one(&mut *holder)
    .await
    .unwrap();

    let p2 = pool.clone();
    let (admin_id, eng_web) = (w.admin, w.eng_web);
    let contender = tokio::spawn(async move {
        let admin = test_support::system_admin_proof_for(&p2, admin_id).await;
        team_service::set_auto_join(&p2, &admin, eng_web, Some(TeamRole::Member)).await
    });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();

    let out = contender.await.unwrap().expect("auto-join");
    assert_eq!(names(&out.public_within, "dormant"), [(w.eng_sec, w.eng)]);
    assert_eq!(
        row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng)
            .await
            .len(),
        1
    );
}

/// The other order: an auto-join committed while an open waits is seen by the open's re-check, which
/// then needs an admin (the anchor's subtree holds a pool) and refuses a non-admin.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_auto_join_committed_first_is_seen_by_the_opens_recheck(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let emitter = emitter_of(&pool, w.admin).await;

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    let raw: serde_json::Value = sqlx::query_scalar(
        "SELECT _admin_team_lifecycle_act($1, $2, $3, 'auto_join_changed', 'member', NULL)",
    )
    .bind(emitter)
    .bind(w.admin)
    .bind(w.eng_web)
    .fetch_one(&mut *holder)
    .await
    .unwrap();
    assert_eq!(raw["outcome"], "changed");

    let p2 = pool.clone();
    let (both, t, a) = (d.both, w.eng_sec, w.eng);
    let contender = tokio::spawn(async move { open_by(&p2, both, t, a).await });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();

    let err = contender.await.unwrap().unwrap_err();
    assert!(admin_anchor(&err), "{err:?}");
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
}

/// The invariant over the world: after every structural act, each row's recorded effect (its
/// effect when opened, folded through every public_within_effect_changed since) equals
/// public_within_row_effective, and every row gone without a close has a lapsed record.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn after_every_structural_act_the_recorded_effect_equals_the_effect(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    join(&pool, w.org, d.both, "owner").await;
    join(&pool, w.sales, d.both, "owner").await;
    let rows = [(w.eng_sec, w.eng), (w.eng_sec, w.org), (w.eng_web, w.org)];
    join(&pool, w.eng_web, d.both, "owner").await;
    let mut recorded: std::collections::HashMap<(Uuid, Uuid), bool> = Default::default();
    for (t, a) in rows {
        open_by(&pool, d.both, t, a).await.expect("open");
        recorded.insert((t, a), effective(&pool, t, a).await);
    }
    let admin = test_support::system_admin_proof_for(&pool, w.admin).await;
    let ap = hp(&pool, w.admin).await;

    let mut seen = 0i64;
    macro_rules! after {
        ($label:literal) => {
            check_recorded_effects(&pool, $label, &mut recorded, &mut seen).await
        };
    }
    after!("opened");

    team_service::delete_team(&pool, &ap, w.eng).await.unwrap();
    after!("delete eng");
    team_service::restore_team(&pool, &ap, w.eng).await.unwrap();
    after!("restore eng");
    team_service::set_auto_join(&pool, &admin, w.sales, Some(TeamRole::Watcher))
        .await
        .unwrap();
    after!("auto-join sales under org");
    team_service::set_auto_join(&pool, &admin, w.sales, None)
        .await
        .unwrap();
    after!("clear auto-join");
    policy_by(&pool, d.ops_owner, w.ops, true).await.unwrap();
    after!("forbid ops");
    team_service::detach_child(&pool, &ap, w.ops, w.eng_sec)
        .await
        .unwrap();
    after!("detach T from ops");
    reparent_by(&pool, w.admin, w.eng, w.org, w.sales).await;
    after!("reparent eng under sales");
    team_service::attach_child(&pool, &ap, w.org, w.eng)
        .await
        .unwrap();
    after!("attach eng back under org");
    team_service::detach_child(&pool, &ap, w.sales, w.eng)
        .await
        .unwrap();
    after!("detach eng from sales");
    team_service::detach_child(&pool, &ap, w.eng, w.eng_sec)
        .await
        .unwrap();
    after!("detach T from eng");
    let to = temper_core::types::admin::UpdateSettingsRequest {
        gating_team_slug: Some("eng-web".to_string()),
        instance_name: None,
        terms_version: None,
        terms_resource_uri: None,
    };
    access_service::update_system_settings(&pool, &admin, &to)
        .await
        .unwrap();
    after!("gating into org");
}

/// Fold every public_within record since `seen` into `recorded`, then check it against the rows.
async fn check_recorded_effects(
    pool: &PgPool,
    label: &str,
    recorded: &mut std::collections::HashMap<(Uuid, Uuid), bool>,
    seen: &mut i64,
) {
    let new: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT e.payload || jsonb_build_object('type', et.name) FROM kb_events e \
           JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name IN ('public_within_effect_changed', 'team_public_within_changed') \
          ORDER BY e.id OFFSET $1",
    )
    .bind(*seen)
    .fetch_all(pool)
    .await
    .unwrap();
    *seen += new.len() as i64;
    for rec in &new {
        let key = (
            rec["team_id"].as_str().unwrap().parse::<Uuid>().unwrap(),
            rec["anchor_id"].as_str().unwrap().parse::<Uuid>().unwrap(),
        );
        match (rec["type"].as_str().unwrap(), rec["act"].as_str()) {
            ("team_public_within_changed", Some("lapsed")) => {
                recorded.remove(&key);
            }
            ("team_public_within_changed", _) => {}
            _ => {
                recorded.insert(key, rec["to"] == "effective");
            }
        }
    }
    for (&(t, a), &was) in recorded.iter() {
        assert!(
            row_exists(pool, t, a).await,
            "{label}: ({t}, {a}) vanished unrecorded"
        );
        assert_eq!(effective(pool, t, a).await, was, "{label}: ({t}, {a})");
    }
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_team_public_within")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        live as usize,
        recorded.len(),
        "{label}: a row appeared unrecorded"
    );
}

/// The re-checks under the act's locks run on the transaction's own connection: with a pool of one
/// connection, an admin's create under the root and rename of the gating team complete instead of
/// waiting forever for a second connection.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_rechecks_under_lock_need_no_second_connection(pool: PgPool) {
    let w = world(&pool).await;
    let one = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("one-connection pool");
    let admin = hp(&one, w.admin).await;
    team_service::create_team(
        &one,
        &admin,
        &TeamCreateRequest {
            slug: "under-root".to_string(),
            parent: Some(w.root.to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("create under the root on one connection");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'org' WHERE id = 1")
        .execute(&one)
        .await
        .unwrap();
    team_service::rename_slug(
        &one,
        &admin,
        w.org,
        &temper_core::types::team::RenameTeamSlugRequest {
            slug: "org-renamed".to_string(),
        },
    )
    .await
    .expect("rename the gating team on one connection");
    let attached = team(&one, "attached", &[]).await;
    team_service::attach_child(&one, &admin, w.root, attached)
        .await
        .expect("attach under the root on one connection");
}

/// An owner deleting their own team is told which of its rows went dormant: the delete makes the
/// team dead, but the owner is still the manager of that side for this answer.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_owner_deleting_their_own_team_is_told_its_rows_by_name(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let out = team_service::delete_team(&pool, &hp(&pool, w.sec_owner).await, w.eng_sec)
        .await
        .expect("T's owner deletes T");
    assert_eq!(names(&out, "dormant"), [(w.eng_sec, w.eng)], "{out:?}");
    assert_eq!(out.dormant_unnamed, 0);
}

/// A close and an act that flips the same row serialize on the effects lock: with the close
/// committed first, the act records nothing for the row it no longer finds.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_close_committed_first_leaves_no_record_from_a_concurrent_act(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();

    // Hold the close's locks and its delete, uncommitted.
    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE")
        .bind(vec![w.eng_sec, w.eng])
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query("SELECT _public_within_effects_lock()")
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query("DELETE FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2")
        .bind(w.eng_sec)
        .bind(w.eng)
        .execute(&mut *holder)
        .await
        .unwrap();

    let p2 = pool.clone();
    let (admin_id, eng_web) = (w.admin, w.eng_web);
    let contender = tokio::spawn(async move {
        let admin = test_support::system_admin_proof_for(&p2, admin_id).await;
        team_service::set_auto_join(&p2, &admin, eng_web, Some(TeamRole::Member)).await
    });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();

    let out = contender.await.unwrap().expect("auto-join");
    assert_eq!(out.public_within, PublicWithinEffects::default());
    assert!(
        row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng)
            .await
            .is_empty()
    );
}

// ── An opening adds no membership, and a lapsed row stays gone ───────────────────────────────────

/// An authorization refusal (a bare 403 or a detailed one), never some other failure that would
/// pass for one.
fn refused<T>(r: &Result<T, ApiError>) -> bool {
    matches!(r, Err(ApiError::Forbidden | ApiError::ForbiddenDetail(_)))
}

/// Every `kb_team_members` row, as a sorted set: what an opening, or a read through one, must never
/// change.
async fn membership_snapshot(pool: &PgPool) -> Vec<(Uuid, Uuid, String)> {
    sqlx::query_as(
        "SELECT team_id, profile_id, role::text FROM kb_team_members \
          ORDER BY team_id, profile_id, role",
    )
    .fetch_all(pool)
    .await
    .expect("membership snapshot")
}

/// An opening confers reading only. Opening `(eng-sec, eng)` and reading T through it leaves
/// `kb_team_members` exactly as it was, so the reader holds no role on T, and every act that
/// membership on T authorizes (inviting, adding a member, changing a role, attaching under T) is
/// refused to them as to any stranger.
///
/// FAILS IF an open, or a read through it, writes a membership row, or any act gated on a role on T
/// admits a reader who reaches T only through the row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_opening_adds_no_membership_and_admits_no_membership_act(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    let before = membership_snapshot(&pool).await;

    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    assert_eq!(
        membership_snapshot(&pool).await,
        before,
        "the open writes no membership"
    );

    // The reader reaches T only through the row.
    assert!(reads(&pool, w.web_member, w.r_t).await);
    assert!(!reads_by_membership(&pool, w.web_member, w.r_t).await);
    assert!(ctx_reads(&pool, w.web_member, w.ctx_t).await);
    assert_eq!(
        membership_snapshot(&pool).await,
        before,
        "reading through the row writes no membership"
    );

    // Membership acts on T are refused to the reader, a manager of a team under the anchor included.
    for who in [w.web_member, w.web_owner] {
        let p = hp(&pool, who).await;
        let invite = temper_services::services::invitation_service::create_invitation(
            &pool,
            &p,
            w.eng_sec,
            temper_services::services::invitation_service::CreateInvitationParams {
                invited_email: format!("pw-invitee-{who}@example.com"),
                role: TeamRole::Watcher,
            },
        )
        .await;
        assert!(refused(&invite), "{who} invites onto T: {invite:?}");
        let add = team_service::add_member(
            &pool,
            &p,
            w.eng_sec,
            &temper_core::types::team::AddMemberRequest {
                profile_id: w.sales_member,
                role: TeamRole::Watcher,
            },
        )
        .await;
        assert!(refused(&add), "{who} adds a member to T: {add:?}");
        let role =
            team_service::change_role(&pool, &p, w.eng_sec, w.sec_member, TeamRole::Watcher).await;
        assert!(refused(&role), "{who} changes a role on T: {role:?}");
        let attach = team_service::attach_child(&pool, &p, w.eng_sec, w.eng_web).await;
        assert!(refused(&attach), "{who} attaches under T: {attach:?}");
    }
    assert_eq!(
        membership_snapshot(&pool).await,
        before,
        "no refused act wrote membership"
    );
}

/// The snapshot above catches an open that writes membership: with a trigger standing in for an
/// open path that enrolls the anchor's readers on T, the same open changes the snapshot. A control
/// for the witness, so a check that compares nothing cannot pass for one.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_membership_snapshot_catches_an_open_that_enrolls(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    sqlx::query(&format!(
        "CREATE FUNCTION pw_test_enroll() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
           INSERT INTO kb_team_members (team_id, profile_id, role) \
           VALUES (NEW.team_id, '{}', 'watcher') ON CONFLICT DO NOTHING; RETURN NEW; END $$",
        w.web_member
    ))
    .execute(&pool)
    .await
    .expect("enrolling function");
    sqlx::query(
        "CREATE TRIGGER pw_test_enroll AFTER INSERT ON kb_team_public_within \
         FOR EACH ROW EXECUTE FUNCTION pw_test_enroll()",
    )
    .execute(&pool)
    .await
    .expect("enrolling trigger");

    let before = membership_snapshot(&pool).await;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    let after = membership_snapshot(&pool).await;
    assert_ne!(after, before, "the snapshot sees the enrolled row");
    assert!(after.contains(&(w.eng_sec, w.web_member, "watcher".to_string())));
    assert!(reads_by_membership(&pool, w.web_member, w.r_t).await);
}

/// A row that lapsed is never revived by a third party. Detaching T from its only path to the
/// anchor deletes `(eng-sec, eng)`. A manager of neither side cannot put T back: attaching or moving
/// T needs a manager of T, so a manager of a team under the anchor (eng-web) and of the anchor's
/// parent (org) are both refused. A system admin, who manages neither, can attach T back, and that
/// revives nothing: no row, no reading, no effect record.
///
/// FAILS IF a non-manager of T can re-link it, or a re-link by anyone brings the row, the reading or
/// a `public_within_effect_changed` record back.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_third_party_never_revives_a_lapsed_row(pool: PgPool) {
    let d = doors(&pool).await;
    let w = &d.w;
    open_by(&pool, d.both, w.eng_sec, w.eng).await.unwrap();
    assert!(reads(&pool, w.web_member, w.r_t).await);

    // T keeps its second parent (ops), so it lives on; eng is no longer above it by any path.
    let lapsed = team_service::detach_child(&pool, &hp(&pool, w.sec_owner).await, w.eng, w.eng_sec)
        .await
        .expect("T's manager detaches it from eng");
    assert_eq!(names(&lapsed, "lapsed"), [(w.eng_sec, w.eng)]);
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    assert!(!reads(&pool, w.web_member, w.r_t).await);
    let effect_records = row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng).await;

    // Nobody who does not manage T can re-link it, under the anchor or a team beneath it: a manager
    // of a team under the anchor, of the anchor's parent, of T's other parent, or of the anchor alone.
    for (who, parent) in [
        (w.web_owner, w.eng_web),
        (w.web_owner, w.eng),
        (d.org_owner, w.eng),
        (d.ops_owner, w.eng),
        (d.eng_owner, w.eng),
    ] {
        let p = hp(&pool, who).await;
        let attach = team_service::attach_child(&pool, &p, parent, w.eng_sec).await;
        assert!(
            matches!(attach, Err(ApiError::Forbidden)),
            "{who} attaches T under {parent}: {attach:?}"
        );
        let moved = team_service::reparent(
            &pool,
            &p,
            w.eng_sec,
            &ReparentRequest {
                from: w.ops.to_string(),
                to: parent.to_string(),
            },
        )
        .await;
        assert!(refused(&moved), "{who} moves T under {parent}: {moved:?}");
    }

    // A system admin, who manages neither side, re-links T under the anchor: nothing revives.
    let back = team_service::attach_child(&pool, &hp(&pool, w.admin).await, w.eng, w.eng_sec)
        .await
        .expect("an admin attaches T back");
    assert_eq!(back.public_within, PublicWithinEffects::default());
    assert!(!row_exists(&pool, w.eng_sec, w.eng).await);
    assert!(!reads(&pool, w.web_member, w.r_t).await);
    assert_eq!(
        row_records(&pool, "public_within_effect_changed", w.eng_sec, w.eng).await,
        effect_records,
        "no effect change is recorded for a row that no longer exists"
    );
}
