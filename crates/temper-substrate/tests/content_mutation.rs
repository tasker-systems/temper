#![cfg(feature = "artifact-tests")]
//! Content-mutation correctness for the block-revision path.
//!
//!  - **Empty/whitespace revise → rejected** (review §1): `prepare_block("")` yields zero chunks; an
//!    unguarded revise would supersede the old chunks and insert none, silently dropping the member
//!    from its region centroid and diverging `body_hash` from create-path semantics. Both the scenario
//!    `revise` arm (clean Rust error) and the authoritative `block_mutate` SQL function reject it.
//!  - **Revert-to-same-body replays byte-identically** (review §2): revising a block back to a prior
//!    body produces two `kb_block_revisions` rows with the SAME `(block_id, block_body_hash)`; the
//!    masked-replay dump orders on `created` to break that tie, so fire and replay must still match.
//!
//! ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.
mod common;

use temper_substrate::replay;
use temper_substrate::scenario::{bootseed, model::Scenario, runner};

/// A minimal inline seed with one concept resource (`alpha`) already created — the revise target.
const SEED_WITH_ALPHA: &str = r#"
name: content-mutation-seed
seed:
  name: cm-seed
  cogmap:
    telos: { title: T, statement: "A small map for exercising content mutation.", questions: [{ question: "What groups?" }] }
    owner: pete
    emitter: "agent#1"
  world:
    profiles: [{ handle: pete, display_name: Pete, system_access: approved }]
    entities: [{ name: "agent#1", profile: pete }]
  resources: []
  uses_lenses: [telos-default]
steps:
  - { do: create_resource, key: alpha, origin_uri: "temper://cm/alpha", body: "deployment pipeline staging and rollout cadence" }
"#;

/// Build the seed scenario with extra `steps` appended (each a raw YAML `- { ... }` list item).
fn scenario_with_steps(extra_steps: &str) -> Scenario {
    let yaml = format!("{SEED_WITH_ALPHA}{extra_steps}");
    serde_yaml::from_str(&yaml).unwrap_or_else(|e| panic!("inline scenario YAML invalid: {e}"))
}

/// The runner's `revise` arm rejects an empty body before firing — a clean Rust error, no contentless
/// block written.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revise_with_empty_body_is_rejected(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let scenario = scenario_with_steps("  - { do: revise, resource: alpha, body: \"\" }\n");
    let err = runner::run_scenario(&pool, &scenario, std::path::Path::new("."))
        .await
        .expect_err("an empty-body revise must be rejected, not silently drop the block's content");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("empty") || msg.contains("no content") || msg.contains("contentless"),
        "error should explain the empty-body rejection, got: {msg}"
    );
}

/// Whitespace-only prose chunks to nothing just like the empty string — also rejected.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revise_with_whitespace_only_body_is_rejected(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let scenario =
        scenario_with_steps("  - { do: revise, resource: alpha, body: \"   \\n\\t  \" }\n");
    runner::run_scenario(&pool, &scenario, std::path::Path::new("."))
        .await
        .expect_err("a whitespace-only revise chunks to nothing and must be rejected");
}

/// The authoritative guard lives in `block_mutate` itself, so any surface (not just the scenario
/// runner) is protected: firing it with an empty `chunks` array raises, leaving the block intact.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn block_mutate_rejects_empty_chunk_set(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    // create alpha so a real block exists
    let scenario = scenario_with_steps("");
    runner::run_scenario(&pool, &scenario, std::path::Path::new("."))
        .await
        .unwrap();
    let block_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT b.id FROM kb_content_blocks b JOIN kb_resources r ON r.id=b.resource_id \
         WHERE r.origin_uri='temper://cm/alpha' AND NOT b.is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let emitter: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_entities ORDER BY id LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let payload = serde_json::json!({ "block_id": block_id, "chunks": [], "incorporated": [] });
    let res = sqlx::query_scalar::<_, uuid::Uuid>("SELECT block_mutate($1, '{}'::jsonb, $2)")
        .bind(&payload)
        .bind(emitter)
        .fetch_one(&pool)
        .await;
    let err =
        res.expect_err("block_mutate with no chunks must raise, not write a contentless block");
    assert!(
        err.to_string().contains("empty") || err.to_string().contains("no chunks"),
        "expected an empty-chunk-set exception, got: {err}"
    );
}

/// Revert regression (review §2): revising a block to a new body then BACK to its original yields two
/// `kb_block_revisions` rows with the same `(block_id, block_body_hash)`. The masked-replay dump orders
/// on `created` (the replay-stable event occurred_at) to break that tie deterministically; this drives
/// the path and proves fire and replay still produce byte-identical projections.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revert_to_prior_body_replays_byte_identically(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let original = "deployment pipeline staging and rollout cadence";
    let extra = format!(
        "  - {{ do: revise, resource: alpha, body: \"an unrelated note about tea brewing temperature and steeping time\" }}\n  \
         - {{ do: revise, resource: alpha, body: \"{original}\" }}\n"
    );
    let scenario = scenario_with_steps(&extra);
    runner::run_scenario(&pool, &scenario, std::path::Path::new("."))
        .await
        .unwrap();

    // sanity: the revert really produced a (block_id, block_body_hash) tie — else the test proves nothing.
    let tied_pairs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT block_id FROM kb_block_revisions \
         GROUP BY block_id, block_body_hash HAVING count(*) > 1) d",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        tied_pairs >= 1,
        "revert must yield ≥1 (block_id, block_body_hash) pair with two revisions (got {tied_pairs})"
    );

    // fire-side projections, then reset + replay the ledger and prove byte-identical projections.
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(
            a, b,
            "projection table {table_a} diverged under replay after a body revert (same-hash tie)"
        );
    }
    temper_substrate::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger payload roundtrip after revert");
}

/// T7a Task 2: a create carrying a resource-level provenance source fires a `resource_created` whose
/// block manifest carries the incorporation. (The `kb_block_provenance` INSERT is Task 3's projector
/// change — here we prove only that the source is threaded into the fired payload.)
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_with_sources_carries_incorporation_in_payload(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
    use temper_substrate::writes::{self, CreateParams};

    common::seed_system(&pool).await;
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let emitter: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap();
    let home = common::insert_context(&pool, "kb_profiles", owner, "prov", "Prov")
        .await
        .unwrap();
    let src = uuid::Uuid::now_v7();

    writes::create_resource_with(
        &pool,
        CreateParams {
            title: "distilled concept",
            origin_uri: "temper://prov/distilled",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(src),
                seq: 0,
            }],
        },
        EventContext::default(),
    )
    .await
    .expect("create with sources");

    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types et ON et.id=e.event_type_id \
         WHERE et.name='resource_created' ORDER BY e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let inc = &payload["blocks"][0]["incorporated"][0];
    assert_eq!(
        inc["source"]["kind"], "resource",
        "source kind tagged 'resource'"
    );
    assert_eq!(
        inc["source"]["value"],
        src.to_string(),
        "source id is the distilled-from resource"
    );
    assert_eq!(inc["seq"], 0, "accretion seq preserved");
}

/// Seed the system actor + a home context; return `(owner_profile, emitter_entity, home_context)`.
async fn prov_fixture(pool: &sqlx::PgPool) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
    common::seed_system(pool).await;
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let emitter: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(owner)
            .fetch_one(pool)
            .await
            .unwrap();
    let home = common::insert_context(pool, "kb_profiles", owner, "prov", "Prov")
        .await
        .unwrap();
    (owner, emitter, home)
}

/// T7a Task 3: a create carrying a resource-level source writes a `kb_block_provenance` row
/// (source_kind='resource', the caller's `seq` as accretion_seq) — the projector INSERT is live.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_with_sources_writes_block_provenance(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
    use temper_substrate::writes::{self, CreateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;
    let src = uuid::Uuid::now_v7();

    writes::create_resource_with(
        &pool,
        CreateParams {
            title: "distilled",
            origin_uri: "temper://prov/c",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(src),
                seq: 0,
            }],
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    let (kind, sid, seq): (String, uuid::Uuid, i32) = sqlx::query_as(
        "SELECT source_kind::text, source_id, accretion_seq FROM kb_block_provenance \
         WHERE NOT is_corrected ORDER BY created DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(kind, "resource");
    assert_eq!(sid, src);
    assert_eq!(seq, 0);
}

/// T7a Task 3: a create-with-source-A then a revise-with-source-B accretes TWO provenance rows on the
/// same block, and the already-wired `resource_blocks.reinforce_count` read reflects them (2).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revise_accretes_a_second_source(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
    use temper_substrate::writes::{self, CreateParams, UpdateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;
    let src_a = uuid::Uuid::now_v7();
    let src_b = uuid::Uuid::now_v7();

    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            title: "distilled",
            origin_uri: "temper://prov/accrete",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(src_a),
                seq: 0,
            }],
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    writes::update_resource_with(
        &pool,
        UpdateParams {
            resource,
            body: Some("a revised body about staged rollout and canary cadence over regions"),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(src_b),
                seq: 1,
            }],
            rehome_to: None,
            emitter: EntityId::from(emitter),
        },
        EventContext::default(),
    )
    .await
    .expect("revise");

    // Two provenance rows on the resource's block, both source_ids present.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_provenance p JOIN kb_content_blocks b ON b.id=p.block_id \
         WHERE b.resource_id=$1 AND NOT p.is_corrected",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        count, 2,
        "create source A + revise source B accrete two rows"
    );

    // The already-wired read signal reflects the accretion.
    let reinforce: i64 = sqlx::query_scalar(
        "SELECT reinforce_count FROM resource_blocks($1, 'profile', $2, NULL) LIMIT 1",
    )
    .bind(resource.uuid())
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        reinforce, 2,
        "resource_blocks.reinforce_count reflects both sources"
    );
}
