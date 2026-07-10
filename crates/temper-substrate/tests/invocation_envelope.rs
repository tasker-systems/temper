#![cfg(feature = "artifact-tests")]
//! Invocation envelope + agent-authorship metadata. Each test boot-seeds the system actor and
//! exercises the new substrate. Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::affinity::EdgeKind;
use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, fire_with, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{BlockId, ChunkId};
use temper_substrate::ids::{CogmapId, EntityId, ProfileId};
use temper_substrate::payloads::{AgentAuthorship, ConfidenceBand, EdgePolarity};
use temper_substrate::replay;
use temper_substrate::scenario::bootseed;
use uuid::Uuid;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn schema_has_invocations_table_and_event_column(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    // kb_events.invocation_id exists and is nullable UUID
    let col: Option<String> = sqlx::query_scalar(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema='public' AND table_name='kb_events' AND column_name='invocation_id'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        col.as_deref(),
        Some("uuid"),
        "kb_events.invocation_id must be uuid"
    );

    // kb_invocations table exists
    let tbl: Option<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema='public' AND table_name='kb_invocations'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        tbl.as_deref(),
        Some("kb_invocations"),
        "kb_invocations table must exist"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn event_append_persists_metadata_and_invocation(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let emitter: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_entities WHERE name='system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let inv = uuid::Uuid::now_v7();
    // Call _event_append directly with named args for the two new params.
    let ev: uuid::Uuid = sqlx::query_scalar(
        "SELECT _event_append('cogmap_seeded', $1, NULL, NULL, '{}'::jsonb, \
                p_metadata => $2::jsonb, p_invocation => $3)",
    )
    .bind(emitter)
    .bind(serde_json::json!({"reasoning": "SENTINEL"}))
    .bind(inv)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (meta, got_inv): (serde_json::Value, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT metadata, invocation_id FROM kb_events WHERE id=$1")
            .bind(ev)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(meta["reasoning"], "SENTINEL");
    assert_eq!(got_inv, Some(inv));
}

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let p: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let e: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(p)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(p), EntityId::from(e))
}

fn one_chunk_block(content: &str) -> PreparedBlock {
    let mut embedding = vec![0.0_f32; 768];
    embedding[0] = 1.0;
    PreparedBlock {
        incorporated: vec![],
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: content.to_string(),
            embedding: Some(embedding),
            header_path: None,
            heading_depth: None,
        }],
    }
}

/// Genesis a cogmap, returning its id (the telos resource is created inside).
async fn genesis(pool: &sqlx::PgPool, owner: ProfileId, emitter: EntityId, name: &str) -> CogmapId {
    let charter = vec![one_chunk_block("telos charter statement")];
    let mut tx = pool.begin().await.unwrap();
    let (cogmap, _telos) = fire(
        &mut tx,
        SeedAction::CogmapGenesis {
            name,
            telos_title: "Telos",
            charter: &charter,
            cogmap_id: None,
            telos_resource_id: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap()
    .cogmap_genesis()
    .unwrap();
    tx.commit().await.unwrap();
    cogmap
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invocation_open_projects_open_row(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-a").await;
    let inv = Uuid::now_v7();

    let returned: Uuid = sqlx::query_scalar("SELECT invocation_open($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "trigger_kind": "manual",
            "originating_cogmap_id": cog.uuid(), "scoped_entity_id": emitter.uuid(),
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(returned, inv, "invocation_open returns the invocation id");

    let (status, trig, orig, telos_present): (String, String, Uuid, bool) = sqlx::query_as(
        "SELECT status, trigger_kind, originating_cogmap_id, telos_resource_id IS NOT NULL \
         FROM kb_invocations WHERE id=$1",
    )
    .bind(inv)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "open");
    assert_eq!(trig, "manual");
    assert_eq!(orig, cog.uuid());
    assert!(telos_present, "telos resolved from the cogmap");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn delegation_gate_blocks_unshared_cogmaps(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let parent = genesis(&pool, owner, emitter, "parent").await;
    let child = genesis(&pool, owner, emitter, "child").await; // no shared team
    let inv = Uuid::now_v7();
    let res = sqlx::query_scalar::<_, Uuid>("SELECT invocation_open($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "trigger_kind": "delegated",
            "originating_cogmap_id": child.uuid(), "parent_cogmap_id": parent.uuid(),
            "scoped_entity_id": emitter.uuid(),
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await;
    let err = res.expect_err("delegation gate must reject cogmaps with no shared team");
    assert!(err.to_string().contains("delegation gate"), "got: {err}");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invocation_close_sets_terminal_status(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-c").await;
    let inv = Uuid::now_v7();
    sqlx::query_scalar::<_, Uuid>("SELECT invocation_open($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "trigger_kind": "manual",
            "originating_cogmap_id": cog.uuid(), "scoped_entity_id": emitter.uuid(),
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

    sqlx::query_scalar::<_, Uuid>("SELECT invocation_close($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "disposition": "completed",
            "outcome": {"concepts": 3, "edges": 2},
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

    let (status, outcome, closed): (String, serde_json::Value, bool) = sqlx::query_as(
        "SELECT status, outcome, closed_at IS NOT NULL FROM kb_invocations WHERE id=$1",
    )
    .bind(inv)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "completed");
    assert_eq!(outcome["concepts"], 3);
    assert!(closed, "closed_at set");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invocation_close_rejects_open_disposition(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-reject").await;
    let inv = Uuid::now_v7();
    sqlx::query_scalar::<_, Uuid>("SELECT invocation_open($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "trigger_kind": "manual",
            "originating_cogmap_id": cog.uuid(), "scoped_entity_id": emitter.uuid(),
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

    // A non-terminal disposition must be rejected before any event is appended.
    let res = sqlx::query_scalar::<_, Uuid>("SELECT invocation_close($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "disposition": "open",
            "outcome": {"concepts": 0},
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await;
    let err = res.expect_err("invocation_close must reject a non-terminal disposition");
    assert!(
        err.to_string().contains("invalid disposition"),
        "got: {err}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn authored_resource_create_stamps_metadata_and_invocation_sql(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-auth").await;
    let inv = Uuid::now_v7();
    let res_id = Uuid::now_v7();
    // resource_create with the two new args (named) — minimal payload + empty content sidecar.
    sqlx::query_scalar::<_, Uuid>(
        "SELECT resource_create($1::jsonb, '{}'::jsonb, $2, p_metadata => $3::jsonb, p_invocation => $4)",
    )
    .bind(serde_json::json!({
        "resource_id": res_id, "title": "Concept X", "origin_uri": "temper://x",
        "home": {"table": "kb_cogmaps", "id": cog.uuid()},
        "owner_profile_id": owner.uuid(), "blocks": [],
    }))
    .bind(emitter.uuid())
    .bind(serde_json::json!({"reasoning": "AUTHORSHIP_SENTINEL", "confidence": "probable"}))
    .bind(inv)
    .fetch_one(&pool).await.unwrap();

    let (meta, got_inv): (serde_json::Value, Option<Uuid>) = sqlx::query_as(
        "SELECT metadata, invocation_id FROM kb_events \
         WHERE event_type_id = (SELECT id FROM kb_event_types WHERE name='resource_created')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(meta["reasoning"], "AUTHORSHIP_SENTINEL");
    assert_eq!(meta["confidence"], "probable");
    assert_eq!(got_inv, Some(inv));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fire_with_authorship_stamps_metadata_via_rust_path(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-rust").await;
    let inv = temper_substrate::ids::InvocationId::from(Uuid::now_v7());

    // Open the invocation through the typed Rust path.
    let mut tx = pool.begin().await.unwrap();
    let opened = fire(
        &mut tx,
        SeedAction::InvocationOpen {
            invocation: inv,
            trigger_kind: "manual",
            originating: cog,
            parent: None,
            scoped_entity: emitter,
            emitter,
        },
    )
    .await
    .unwrap()
    .invocation()
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(opened, inv);

    // Author a resource under the invocation with authorship metadata.
    let blocks = vec![one_chunk_block("concept body")];
    let mut tx = pool.begin().await.unwrap();
    fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "C",
            origin_uri: "temper://c",
            resource_id: None,
            home: temper_substrate::payloads::AnchorRef::cogmap(cog),
            owner,
            originator: None,
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
        },
        EventContext {
            authorship: Some(AgentAuthorship {
                reasoning: Some("RUST_SENTINEL".into()),
                confidence: ConfidenceBand::Confident,
                rationale: None,
                persona: Some("steward".into()),
                model: None,
            }),
            invocation: Some(inv),
            correlation: None,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (meta, got_inv): (serde_json::Value, Option<Uuid>) = sqlx::query_as(
        "SELECT metadata, invocation_id FROM kb_events \
         WHERE event_type_id=(SELECT id FROM kb_event_types WHERE name='resource_created')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(meta["reasoning"], "RUST_SENTINEL");
    assert_eq!(meta["confidence"], "confident");
    assert_eq!(got_inv, Some(inv.uuid()));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invocation_and_authorship_survive_replay(pool: sqlx::PgPool) {
    const REPLAY_SENTINEL: &str = "REPLAY_AUTHORSHIP_SENTINEL";
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-replay").await;
    let inv = temper_substrate::ids::InvocationId::from(Uuid::now_v7());

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::InvocationOpen {
            invocation: inv,
            trigger_kind: "manual",
            originating: cog,
            parent: None,
            scoped_entity: emitter,
            emitter,
        },
    )
    .await
    .unwrap();
    // Author a real act UNDER the invocation so the replay actually exercises authorship metadata.
    let blocks = vec![one_chunk_block("replayed concept body")];
    fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "R",
            origin_uri: "temper://r",
            resource_id: None,
            home: temper_substrate::payloads::AnchorRef::cogmap(cog),
            owner,
            originator: None,
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
        },
        EventContext {
            authorship: Some(AgentAuthorship {
                reasoning: Some(REPLAY_SENTINEL.into()),
                confidence: ConfidenceBand::Confident,
                rationale: None,
                persona: None,
                model: None,
            }),
            invocation: Some(inv),
            correlation: None,
        },
    )
    .await
    .unwrap();
    fire(
        &mut tx,
        SeedAction::InvocationClose {
            invocation: inv,
            disposition: temper_substrate::payloads::Disposition::Completed,
            outcome: serde_json::json!({"concepts": 1}),
            originating: cog,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    let inv_before = before
        .iter()
        .find(|(t, _)| t == "kb_invocations")
        .map(|(_, v)| v);
    let inv_after = after
        .iter()
        .find(|(t, _)| t == "kb_invocations")
        .map(|(_, v)| v);
    assert!(
        inv_before.is_some(),
        "kb_invocations must be in the projection dump set"
    );
    assert_eq!(
        inv_before, inv_after,
        "kb_invocations must replay byte-identically"
    );

    // The authored act's metadata must survive replay onto the fresh pool.
    let survived: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_events WHERE metadata->>'reasoning' = $1")
            .bind(REPLAY_SENTINEL)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        survived >= 1,
        "authorship metadata must survive replay onto the rebuilt pool"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn authorship_is_invisible_to_affinity_inputs(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-invis").await;
    let inv = temper_substrate::ids::InvocationId::from(Uuid::now_v7());

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::InvocationOpen {
            invocation: inv,
            trigger_kind: "manual",
            originating: cog,
            parent: None,
            scoped_entity: emitter,
            emitter,
        },
    )
    .await
    .unwrap();
    // The same authorship rides every authored act below so each affinity-input projection
    // (resources, edges, properties) is non-empty when we later assert the sentinel is absent.
    let authorship = AgentAuthorship {
        reasoning: Some("INVIS_SENTINEL".into()),
        confidence: ConfidenceBand::Tentative,
        rationale: Some("INVIS_SENTINEL".into()),
        persona: None,
        model: None,
    };
    let blocks_a = vec![one_chunk_block("invisibility body a")];
    let res_a = fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "Z",
            origin_uri: "temper://z",
            resource_id: None,
            home: temper_substrate::payloads::AnchorRef::cogmap(cog),
            owner,
            originator: None,
            blocks: &blocks_a,
            doc_type: Some("concept"),
            emitter,
        },
        EventContext {
            authorship: Some(authorship.clone()),
            invocation: Some(inv),
            correlation: None,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap();
    // A second resource in the same cogmap so the edge arm has a real target.
    let blocks_b = vec![one_chunk_block("invisibility body b")];
    let res_b = fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "Z2",
            origin_uri: "temper://z2",
            resource_id: None,
            home: temper_substrate::payloads::AnchorRef::cogmap(cog),
            owner,
            originator: None,
            blocks: &blocks_b,
            doc_type: Some("concept"),
            emitter,
        },
        EventContext {
            authorship: Some(authorship.clone()),
            invocation: Some(inv),
            correlation: None,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap();
    // An edge between them, carrying the SAME authorship — makes the kb_edges arm non-vacuous.
    fire_with(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: res_a,
            tgt: res_b,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: None,
            weight: 1.0,
            home: EdgeHome::Cogmap(cog),
            emitter,
        },
        EventContext {
            authorship: Some(authorship.clone()),
            invocation: Some(inv),
            correlation: None,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // The kb_edges arm below is only meaningful if a row actually exists.
    let edge_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_edges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        edge_rows >= 1,
        "kb_edges must be non-empty for a non-vacuous invisibility check"
    );

    // Authorship IS in the ledger metadata.
    let in_meta: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events WHERE metadata->>'reasoning' = 'INVIS_SENTINEL'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        in_meta >= 1,
        "authorship must be recorded in kb_events.metadata"
    );

    // Authorship is NOT in ANY affinity-input projection (resources / edges / properties).
    for table in ["kb_resources", "kb_edges", "kb_properties"] {
        let leaked: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} t WHERE to_jsonb(t)::text LIKE '%INVIS_SENTINEL%'",
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            leaked, 0,
            "{table} must not contain authorship text (invisible to affinity)"
        );
    }
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn writes_open_then_close_round_trips(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-writes").await;

    let inv = temper_substrate::writes::open_invocation(
        &pool,
        temper_substrate::writes::OpenParams {
            trigger_kind: "manual".to_string(),
            originating: cog,
            parent: None,
            scoped_entity: emitter,
            emitter,
        },
    )
    .await
    .unwrap();

    let status: String = sqlx::query_scalar("SELECT status FROM kb_invocations WHERE id=$1")
        .bind(inv.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "open");

    temper_substrate::writes::close_invocation(
        &pool,
        inv,
        cog,
        temper_substrate::payloads::Disposition::Completed,
        serde_json::json!({"concepts": 2}),
        emitter,
    )
    .await
    .unwrap();

    let (status, closed): (String, bool) =
        sqlx::query_as("SELECT status, closed_at IS NOT NULL FROM kb_invocations WHERE id=$1")
            .bind(inv.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "completed");
    assert!(closed, "closed_at set");
}

// ── Steward tick correlation (task 019f4be3) ─────────────────────────────────
//
// One steward tick is one dispatch act plus N run-grain sessions. The tick id reaches the data layer
// by being stamped on each claimed job; `invocation_open` inherits it server-side, so nothing is asked
// of the agent. These tests pin that inheritance, its absence, and its replay-stability.

/// Stand in for `workflow_job_claim(…, p_correlation)`: an ACTIVE job for `cog`, stamped by a tick.
async fn claimed_job(pool: &sqlx::PgPool, cog: CogmapId, correlation: Option<Uuid>) {
    sqlx::query(
        "INSERT INTO kb_workflow_jobs (cogmap_id, persona, dispatch_type, status, leased_at, correlation_id)
         VALUES ($1, 'steward', 'steward', 'in_progress', now(), $2)",
    )
    .bind(cog.uuid())
    .bind(correlation)
    .execute(pool)
    .await
    .unwrap();
}

async fn open_invocation(pool: &sqlx::PgPool, cog: CogmapId, emitter: EntityId) -> Uuid {
    let inv = temper_substrate::ids::InvocationId::from(Uuid::now_v7());
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::InvocationOpen {
            invocation: inv,
            trigger_kind: "delegated",
            originating: cog,
            parent: None,
            scoped_entity: emitter,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    inv.uuid()
}

async fn invocation_correlation(pool: &sqlx::PgPool, inv: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT correlation_id FROM kb_invocations WHERE id = $1")
        .bind(inv)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invocation_open_inherits_the_tick_from_its_active_claimed_job(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-tick").await;

    // The cron's id is a v4 UUID (crypto.randomUUID), deliberately not v7 — the column has no
    // sortability requirement, and the NULLIF-vs-self-root logic must not depend on version.
    let tick = Uuid::parse_str("6f1e5a2c-9d3b-4c7e-8a10-2b4d6e8f0a12").unwrap();
    claimed_job(&pool, cog, Some(tick)).await;

    let inv = open_invocation(&pool, cog, emitter).await;

    assert_eq!(
        invocation_correlation(&pool, inv).await,
        Some(tick),
        "the run inherits the tick that claimed its job — no model involvement"
    );

    // …and the opening event carries it too, so the ledger alone can rebuild the projection.
    let event_correlation: Option<Uuid> = sqlx::query_scalar(
        "SELECT e.correlation_id FROM kb_events e
           JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE t.name = 'delegated_launch' AND e.invocation_id = $1",
    )
    .bind(inv)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(event_correlation, Some(tick));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn manual_invocation_open_with_no_active_job_gets_no_correlation(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-manual").await;

    // No job at all: a human opening an envelope by hand. There is no tick to correlate to.
    let inv = open_invocation(&pool, cog, emitter).await;
    assert_eq!(invocation_correlation(&pool, inv).await, None);

    // The delegated_launch event still self-roots (correlation_id = its own id), exactly as before
    // this migration — which is precisely what the projection's NULLIF maps back to NULL.
    let self_rooted: bool = sqlx::query_scalar(
        "SELECT e.correlation_id = e.id FROM kb_events e
           JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE t.name = 'delegated_launch' AND e.invocation_id = $1",
    )
    .bind(inv)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(self_rooted, "an uncorrelated event roots at itself");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_job_that_is_no_longer_active_is_not_inherited(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-stale").await;

    // A finished job from an EARLIER tick must not leak its id into a later, manually opened run.
    let stale = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
    claimed_job(&pool, cog, Some(stale)).await;
    sqlx::query("UPDATE kb_workflow_jobs SET status = 'done', completed_at = now()")
        .execute(&pool)
        .await
        .unwrap();

    let inv = open_invocation(&pool, cog, emitter).await;
    assert_eq!(
        invocation_correlation(&pool, inv).await,
        None,
        "only an in_progress job is inherited from"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn tick_correlation_survives_replay_without_the_job_table(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-tick-replay").await;

    let tick = Uuid::parse_str("6f1e5a2c-9d3b-4c7e-8a10-2b4d6e8f0a12").unwrap();
    claimed_job(&pool, cog, Some(tick)).await;
    let inv = open_invocation(&pool, cog, emitter).await;
    assert_eq!(invocation_correlation(&pool, inv).await, Some(tick));

    // Differential: rebuild the projections from the ledger alone. `reset_schema` wipes
    // kb_workflow_jobs too, so a projection that re-read the job table (rather than the event's own
    // correlation_id) would silently rebuild this run as uncorrelated. Replay must be ledger-pure.
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    let pick = |d: &Vec<(String, serde_json::Value)>| {
        d.iter()
            .find(|(t, _)| t == "kb_invocations")
            .map(|(_, v)| v.clone())
            .expect("kb_invocations must be in the projection dump set")
    };
    assert_eq!(
        pick(&before),
        pick(&after),
        "kb_invocations (correlation_id included) must replay byte-identically from the ledger"
    );
    assert_eq!(
        invocation_correlation(&pool, inv).await,
        Some(tick),
        "the rebuilt run still names its tick, with no job row in existence"
    );
}
