#![cfg(feature = "artifact-tests")]
//! Invocation envelope + agent-authorship metadata. Each test resets the artifact (01+02 via psql),
//! boot-seeds the system actor, and exercises the new substrate. Serialized via the
//! `temper-next-write` nextest group (it owns the namespace).

mod common;

use temper_next::content::{PreparedBlock, PreparedChunk};
use temper_next::events::{fire, fire_with, EventContext, SeedAction};
use temper_next::ids::{BlockId, ChunkId};
use temper_next::ids::{CogmapId, EntityId, ProfileId};
use temper_next::payloads::{AgentAuthorship, ConfidenceBand};
use temper_next::replay;
use temper_next::substrate;
use uuid::Uuid;

/// Reset the artifact (01+02), connect, boot-seed the system actor. Standard write-path preamble.
async fn setup() -> sqlx::PgPool {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    pool
}

#[tokio::test]
async fn schema_has_invocations_table_and_event_column() {
    let pool = setup().await;
    // kb_events.invocation_id exists and is nullable UUID
    let col: Option<String> = sqlx::query_scalar(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema='temper_next' AND table_name='kb_events' AND column_name='invocation_id'",
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
         WHERE table_schema='temper_next' AND table_name='kb_invocations'",
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

#[tokio::test]
async fn event_append_persists_metadata_and_invocation() {
    let pool = setup().await;
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
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: content.to_string(),
            embedding,
            header_path: None,
            heading_depth: None,
        }],
    }
}

/// Genesis a cogmap, returning its id (the telos resource is created inside).
async fn genesis(pool: &sqlx::PgPool, owner: ProfileId, emitter: EntityId, name: &str) -> CogmapId {
    let charter = vec![one_chunk_block("telos charter statement")];
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
    let (cogmap, _telos) = fire(
        &mut tx,
        SeedAction::CogmapGenesis {
            name,
            telos_title: "Telos",
            charter: &charter,
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

#[tokio::test]
async fn invocation_open_projects_open_row() {
    let pool = setup().await;
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

#[tokio::test]
async fn delegation_gate_blocks_unshared_cogmaps() {
    let pool = setup().await;
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

#[tokio::test]
async fn invocation_close_sets_terminal_status() {
    let pool = setup().await;
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

#[tokio::test]
async fn authored_resource_create_stamps_metadata_and_invocation_sql() {
    let pool = setup().await;
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

#[tokio::test]
async fn fire_with_authorship_stamps_metadata_via_rust_path() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-rust").await;
    let inv = temper_next::ids::InvocationId::from(Uuid::now_v7());

    // Open the invocation through the typed Rust path.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
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
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
    fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "C",
            origin_uri: "temper://c",
            home: temper_next::payloads::AnchorRef::cogmap(cog),
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

#[tokio::test]
async fn invocation_and_authorship_survive_replay() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-replay").await;
    let inv = temper_next::ids::InvocationId::from(Uuid::now_v7());

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
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
    fire(
        &mut tx,
        SeedAction::InvocationClose {
            invocation: inv,
            disposition: temper_next::payloads::Disposition::Completed,
            outcome: serde_json::json!({"concepts": 0}),
            originating: cog,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_artifact();
    let pool2 = substrate::connect().await.unwrap();
    replay::replay(&pool2, &snap).await.unwrap();
    let after = replay::dump_projections(&pool2).await.unwrap();

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
}

#[tokio::test]
async fn authorship_is_invisible_to_affinity_inputs() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-invis").await;
    let inv = temper_next::ids::InvocationId::from(Uuid::now_v7());

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
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
    let blocks = vec![one_chunk_block("invisibility body")];
    fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "Z",
            origin_uri: "temper://z",
            home: temper_next::payloads::AnchorRef::cogmap(cog),
            owner,
            originator: None,
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
        },
        EventContext {
            authorship: Some(AgentAuthorship {
                reasoning: Some("INVIS_SENTINEL".into()),
                confidence: ConfidenceBand::Tentative,
                rationale: Some("INVIS_SENTINEL".into()),
                persona: None,
                model: None,
            }),
            invocation: Some(inv),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

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
