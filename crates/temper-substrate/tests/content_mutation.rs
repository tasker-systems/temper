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
            content_block: None,
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

/// T7c Task 11: an update addressing a block explicitly (`content_block = Some(id)`) applies the
/// revision + sources to THAT block. Here the addressed block is the resource's sole body block, so
/// the outcome matches the default path — the point is that the explicit `Some(id)` selection resolves
/// to the same block and lands provenance (proving the `Some` branch, not the count-based default).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn update_with_content_block_targets_named_block(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
    use temper_substrate::writes::{self, CreateParams, UpdateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;
    let src_b = uuid::Uuid::now_v7();

    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            title: "distilled",
            origin_uri: "temper://prov/cb-target",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(uuid::Uuid::now_v7()),
                seq: 0,
            }],
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    let block_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

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
            content_block: Some(block_id),
            rehome_to: None,
            emitter: EntityId::from(emitter),
        },
        EventContext::default(),
    )
    .await
    .expect("revise addressing the block explicitly");

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_provenance p JOIN kb_content_blocks b ON b.id=p.block_id \
         WHERE b.id=$1 AND NOT p.is_corrected",
    )
    .bind(block_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        count, 2,
        "create source + explicitly-addressed revise source both land on the addressed block"
    );
}

/// T7c Task 11: addressing a `content_block` that does not belong to the resource is rejected before
/// any write — the default path would silently revise the resource's own block instead.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn update_with_foreign_content_block_is_rejected(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::AnchorRef;
    use temper_substrate::writes::{self, CreateParams, UpdateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;

    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            title: "distilled",
            origin_uri: "temper://prov/cb-foreign",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    let err = writes::update_resource_with(
        &pool,
        UpdateParams {
            resource,
            body: Some("a revised body about staged rollout and canary cadence over regions"),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            sources: vec![],
            // a block id that belongs to no resource in this DB
            content_block: Some(uuid::Uuid::now_v7()),
            rehome_to: None,
            emitter: EntityId::from(emitter),
        },
        EventContext::default(),
    )
    .await
    .expect_err("a content_block that does not belong to the resource must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("does not belong") || msg.contains("not belong"),
        "error should explain the block does not belong to the resource, got: {msg}"
    );
}

/// T7c Task 11: addressing a folded `content_block` is rejected — folding is the availability gate, and
/// a revise must not resurrect content on a folded block.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn update_with_folded_content_block_is_rejected(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::AnchorRef;
    use temper_substrate::writes::{self, CreateParams, UpdateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;

    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            title: "distilled",
            origin_uri: "temper://prov/cb-folded",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    let block_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    // Fold the block directly — we only need the folded precondition, not an event-sourced fold.
    sqlx::query("UPDATE kb_content_blocks SET is_folded = true WHERE id = $1")
        .bind(block_id)
        .execute(&pool)
        .await
        .unwrap();

    let err = writes::update_resource_with(
        &pool,
        UpdateParams {
            resource,
            body: Some("a revised body about staged rollout and canary cadence over regions"),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            sources: vec![],
            content_block: Some(block_id),
            rehome_to: None,
            emitter: EntityId::from(emitter),
        },
        EventContext::default(),
    )
    .await
    .expect_err("addressing a folded content_block must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("folded"),
        "error should explain the block is folded, got: {msg}"
    );
}

/// issue #355: annotate-only. A create then an `annotate_block_sources` records a NEW provenance row
/// WITHOUT a body revise — `block_body_hash`, the chunk rows, and the revision count are all unchanged
/// (no re-chunk, no re-embed), and the annotate replays byte-identically through `_project_block_annotated`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn annotate_records_provenance_without_touching_chunks(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
    use temper_substrate::writes::{self, AnnotateParams, CreateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;

    let resource = writes::create_resource_deferred_with(
        &pool,
        CreateParams {
            title: "distilled",
            origin_uri: "temper://prov/annotate",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![], // imported with NO provenance — the corpus-backfill precondition
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    let block_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    // Snapshot the content-derived state BEFORE annotating: body hash, revision count, chunk rows +
    // their embeddings. An annotate must leave every one of these untouched.
    let hash_before: String = sqlx::query_scalar(
        "SELECT block_body_hash FROM kb_block_revisions WHERE block_id=$1 ORDER BY created DESC LIMIT 1",
    )
    .bind(block_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let revisions_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_block_revisions WHERE block_id=$1")
            .bind(block_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    // (chunk_id, version, embedding-as-text) for every chunk row of the block.
    let chunks_before: Vec<(uuid::Uuid, i32, Option<String>)> = sqlx::query_as(
        "SELECT id, version, embedding::text FROM kb_chunks WHERE block_id=$1 ORDER BY id",
    )
    .bind(block_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    let src = uuid::Uuid::now_v7();
    let returned = writes::annotate_block_sources_with(
        &pool,
        AnnotateParams {
            resource,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(src),
                seq: 0,
            }],
            content_block: None,
            emitter: EntityId::from(emitter),
        },
        EventContext::default(),
    )
    .await
    .expect("annotate");
    assert_eq!(
        returned.uuid(),
        block_id,
        "annotate returns the addressed block id"
    );

    // The provenance row landed, attributed to a `block_provenance_annotated` event.
    let (kind, sid, seq, ev_type): (String, uuid::Uuid, i32, String) = sqlx::query_as(
        "SELECT p.source_kind::text, p.source_id, p.accretion_seq, et.name \
           FROM kb_block_provenance p \
           JOIN kb_events e ON e.id = p.contributed_by_event_id \
           JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE p.block_id=$1 AND NOT p.is_corrected ORDER BY p.created DESC LIMIT 1",
    )
    .bind(block_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(kind, "resource");
    assert_eq!(sid, src);
    assert_eq!(seq, 0);
    assert_eq!(
        ev_type, "block_provenance_annotated",
        "the row is attributed to the annotate event, not a block_mutated revise"
    );

    // NO body revise: hash, revision count, and chunk rows (+ embeddings) are byte-for-byte unchanged.
    let hash_after: String = sqlx::query_scalar(
        "SELECT block_body_hash FROM kb_block_revisions WHERE block_id=$1 ORDER BY created DESC LIMIT 1",
    )
    .bind(block_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let revisions_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_block_revisions WHERE block_id=$1")
            .bind(block_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let chunks_after: Vec<(uuid::Uuid, i32, Option<String>)> = sqlx::query_as(
        "SELECT id, version, embedding::text FROM kb_chunks WHERE block_id=$1 ORDER BY id",
    )
    .bind(block_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        hash_before, hash_after,
        "block_body_hash unchanged by annotate"
    );
    assert_eq!(
        revisions_before, revisions_after,
        "annotate writes NO new kb_block_revisions row"
    );
    assert_eq!(
        chunks_before, chunks_after,
        "chunk rows + embeddings unchanged (no re-chunk / no re-embed)"
    );

    // Replay symmetry: the annotate reprojects byte-identically through `_project_block_annotated`.
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(
            a, b,
            "projection table {table_a} diverged under replay after annotate"
        );
    }
    temper_substrate::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger payload roundtrip after annotate");
}

/// issue #355 (span locators): annotate with a REMOTE URL carrying a `#L<start>-L<end>` fragment. The
/// locator rides the URL verbatim — `normalize_remote_uri` preserves the fragment, so the read fn
/// surfaces the exact `…#L120-L180` URL. Zero schema change; the fragment IS the locator.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn annotate_with_remote_locator_round_trips(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
    use temper_substrate::writes::{self, AnnotateParams, CreateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;
    let resource = writes::create_resource_deferred_with(
        &pool,
        CreateParams {
            title: "chapter 11",
            origin_uri: "temper://prov/locator",
            body: "part eleven of a sixteen chunk series about staged rollout",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    let located = "https://example.com/long-source.md#L120-L180";
    writes::annotate_block_sources_with(
        &pool,
        AnnotateParams {
            resource,
            sources: vec![Incorporation {
                source: ProvenanceSource::Remote(located.to_owned()),
                seq: 0,
            }],
            content_block: None,
            emitter: EntityId::from(emitter),
        },
        EventContext::default(),
    )
    .await
    .expect("annotate with a located remote source");

    let (kind, uri): (String, Option<String>) = sqlx::query_as(
        "SELECT source_kind, source_uri FROM resource_block_provenance($1, 'profile', $2) LIMIT 1",
    )
    .bind(resource.uuid())
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(kind, "remote");
    assert_eq!(
        uri.as_deref(),
        Some(located),
        "the span-locator fragment survives verbatim into --provenance / get_block_provenance"
    );
}

/// issue #355: annotate with no sources is rejected — an annotate that attaches nothing is a caller
/// error, not a silent no-op. The guard lives in `block_annotate` (any surface is protected).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn annotate_with_empty_sources_is_rejected(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::AnchorRef;
    use temper_substrate::writes::{self, AnnotateParams, CreateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;
    let resource = writes::create_resource_deferred_with(
        &pool,
        CreateParams {
            title: "distilled",
            origin_uri: "temper://prov/annotate-empty",
            body: "deployment pipeline staging and rollout cadence",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create");

    let err = writes::annotate_block_sources_with(
        &pool,
        AnnotateParams {
            resource,
            sources: vec![],
            content_block: None,
            emitter: EntityId::from(emitter),
        },
        EventContext::default(),
    )
    .await
    .expect_err("an annotate with no sources must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("source"),
        "error should explain there are no sources to attach, got: {msg}"
    );
}

/// T7c Task 9: a create carrying a REMOTE (URL) source mints a `kb_remote_sources` row, writes a
/// provenance row (`source_kind='remote'`, `source_id` = the minted id), the read fn surfaces the raw
/// URL, and two normalization-equivalent URLs (case / default-port) collapse to a single row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_with_remote_source_mints_dedups_and_surfaces_url(pool: sqlx::PgPool) {
    use temper_substrate::events::EventContext;
    use temper_substrate::ids::{ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
    use temper_substrate::writes::{self, CreateParams};

    let (owner, emitter, home) = prov_fixture(&pool).await;

    let mk = |title: &'static str, origin: &'static str, url: &'static str| CreateParams {
        title,
        origin_uri: origin,
        body: "deployment pipeline staging and rollout cadence",
        doc_type: "research",
        home: AnchorRef::context(ContextId::from(home)),
        owner: ProfileId::from(owner),
        originator: ProfileId::from(owner),
        emitter: EntityId::from(emitter),
        properties: &[],
        chunks: None,
        sources: vec![Incorporation {
            source: ProvenanceSource::Remote(url.to_owned()),
            seq: 0,
        }],
    };

    // First create: raw casing/path preserved; scheme+host lowercased in the normalized key.
    let resource = writes::create_resource_with(
        &pool,
        mk("a", "temper://prov/remote-a", "https://Example.com/Issue/1"),
        EventContext::default(),
    )
    .await
    .expect("create with remote source");

    let (kind, sid): (String, uuid::Uuid) = sqlx::query_as(
        "SELECT source_kind::text, source_id FROM kb_block_provenance \
         WHERE NOT is_corrected ORDER BY created DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(kind, "remote", "source kind tagged 'remote'");

    let (uri, normalized): (String, String) =
        sqlx::query_as("SELECT uri, uri_normalized FROM kb_remote_sources WHERE id=$1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        uri, "https://Example.com/Issue/1",
        "raw URL preserved verbatim"
    );
    assert_eq!(
        normalized, "https://example.com/Issue/1",
        "scheme+host lowercased; path casing untouched"
    );

    // Second create with a normalization-equivalent URL (default port) dedups to the same source row.
    writes::create_resource_with(
        &pool,
        mk(
            "b",
            "temper://prov/remote-b",
            "https://example.com:443/Issue/1",
        ),
        EventContext::default(),
    )
    .await
    .expect("create with equivalent remote source");

    let remote_count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_remote_sources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        remote_count, 1,
        "normalization-equivalent URLs collapse to one kb_remote_sources row"
    );

    // The read fn surfaces the raw URL for the remote row (source_uri), scoped to a reader who can see it.
    let (read_kind, read_uri): (String, Option<String>) = sqlx::query_as(
        "SELECT source_kind, source_uri FROM resource_block_provenance($1, 'profile', $2) LIMIT 1",
    )
    .bind(resource.uuid())
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(read_kind, "remote");
    assert_eq!(
        read_uri.as_deref(),
        Some("https://Example.com/Issue/1"),
        "read fn surfaces the human URL, not the minted uuid"
    );
}
