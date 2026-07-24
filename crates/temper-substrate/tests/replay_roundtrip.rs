#![cfg(feature = "artifact-tests")]
//! Proof obligation 2 (replay) + 4 (CAS retention), payload spec §7: run a scenario, snapshot,
//! reset the namespace, walk the ledger through the SAME _project_* halves, and prove the
//! projections come back byte-identical (masked-surrogate rule). Regions re-prove by
//! re-materialization matching the payload's recorded membership fingerprint.
//!
//! ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.

mod common;

use temper_substrate::ids::{BlockId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ProvenanceSource};
use temper_substrate::writes::{self, CitationAuditParams, CreateParams};
use temper_substrate::{
    replay, scenario::bootseed, scenario::model::Scenario, scenario::runner, write,
};
use uuid::Uuid;

const ONBOARDING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/scenarios/onboarding-cogmap.yaml"
);

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reproduces_projections_byte_identically(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();

    let scenario: Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING).unwrap()).unwrap();
    let base_dir = std::path::Path::new(ONBOARDING).parent().unwrap();
    runner::run_scenario(&pool, &scenario, base_dir)
        .await
        .unwrap();

    // capture: projections (the diff baseline), inputs + sidecars (the replay substrate), and the
    // recorded materialization acts (the region re-proof targets)
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    let recorded = replay::recorded_materializations(&pool).await.unwrap();
    assert!(
        !recorded.is_empty(),
        "scenario must have materialized at least once"
    );

    // reset to a clean, UN-seeded namespace; replay the ledger through the projection halves
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();

    let after = replay::dump_projections(&pool).await.unwrap();
    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }

    // regions: second-order derived — re-materialize and match the recorded fingerprints. A lens
    // whose substrate gained formation-affecting events AFTER its recorded watermark is legitimately
    // stale (the drift-detection concept) and is skipped; at least one lens must be provably fresh.
    let mut proven = 0usize;
    for (anchor, lens_id, watermark, fingerprint) in recorded {
        if replay::formation_touched_since(&pool, anchor, watermark)
            .await
            .unwrap()
        {
            continue; // recorded act is stale relative to the substrate — re-proof not applicable
        }
        let lens_name: String =
            sqlx::query_scalar("SELECT name FROM kb_cogmap_lenses WHERE id = $1")
                .bind(lens_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let emitter: uuid::Uuid =
            sqlx::query_scalar("SELECT id FROM kb_entities ORDER BY id LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let out = write::materialize(&pool, anchor, &lens_name, emitter.into())
            .await
            .unwrap();
        assert_eq!(
            out.membership_fingerprint, fingerprint,
            "re-materialization under lens {lens_name} must reproduce the recorded fingerprint"
        );
        proven += 1;
    }
    assert!(
        proven >= 1,
        "at least one recorded materialization must be fresh enough to re-prove"
    );
}

// ── Task 2 — citation_audited survives replay ────────────────────────────────────────────────
//
// Local fixture helpers (duplicated per file rather than shared — established convention per
// `citation_audits.rs`'s own header: `block_content.rs`, `invocation_envelope.rs`,
// `search_index.rs`, `search_surface_a.rs`, and `write_path_mutations.rs` each carry their own
// copy). `kb_profiles`/`kb_contexts`/`kb_entities` are `INPUT_TABLES` (restored verbatim by
// `replay::replay`); `kb_resources`/`kb_content_blocks`/`kb_citation_audits` are projections
// rebuilt by walking events — so this test's resources and audit must come back from the SAME
// `_project_resource_created`/`_project_citation_audited` halves replay always runs.

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn make_home(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    title: &str,
    uri: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            title,
            origin_uri: uri,
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        temper_substrate::events::EventContext::default(),
    )
    .await
    .unwrap()
}

async fn first_block(pool: &sqlx::PgPool, resource: ResourceId) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq LIMIT 1",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// LOAD-BEARING: falsifies the EXACT bug the brief names — a missing `EventKind::
/// from_canonical_name` arm for `citation_audited` compiles clean and hard-fails only at replay
/// time, with "replay: no projector for event type citation_audited", against any database that
/// ever recorded an audit (the `resource_finalized` precedent, `events.rs`'s own `EventKind` doc
/// comment). Fires one citation audit through the Rust write path, snapshots, resets to a clean
/// UN-seeded namespace, replays, and asserts the row comes back byte-identical — identified by its
/// emitting event, not by its surrogate id, per the masked-surrogate rule (see below).
/// Proof obligation 2 (payload spec §7), extended to this event type.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reprojects_a_citation_audit(pool: sqlx::PgPool) {
    // NO `common::reset_schema` in the setup half, unlike the sibling above. reset_schema
    // re-applies every migration and then TRUNCATEs the data tables — including `kb_event_types`
    // (`common/mod.rs:16-25`). `citation_audited` is registered by a migration INSERT
    // (`20260723000010_citation_audits.sql`), not by the canonical seed, so the truncate drops it
    // and `bootseed::seed_system` — which re-seeds only the canonical types — never restores it:
    // `_event_append` then raises `event_type citation_audited not seeded`
    // (`20260624000002_canonical_functions.sql:777`). The fresh `MIGRATOR` database this test is
    // handed already carries the registration, which is what Task 1's tests rely on too.
    //
    // The reset BEFORE replay (below) is still correct and is the point of the test: `snapshot`
    // dumps `kb_event_types` as an INPUT_TABLE (`replay.rs:112`), so replay restores the
    // registration along with the ledger.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-replay-t2").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/replay-finding",
    )
    .await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "source",
        "temper://ca/replay-source",
    )
    .await;
    let block = BlockId::from(first_block(&pool, finding).await);

    // The `(block, source)` pair must be a LIVE citation — `citation_audit` refuses an audit of a
    // non-citation (`20260723000010_citation_audits.sql`, `citation_is_live`). `make_resource`
    // creates with no sources, so the provenance row is added here, borrowing the block's own
    // genesis event as the contributing act (nothing under test reads that event's content).
    sqlx::query(
        "INSERT INTO kb_block_provenance \
           (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
         SELECT $1, 'resource', $2, b.genesis_event_id, 0 \
           FROM kb_content_blocks b WHERE b.id = $1",
    )
    .bind(block.uuid())
    .bind(source.uuid())
    .execute(&pool)
    .await
    .unwrap();

    let audit_id = writes::record_citation_audit(
        &pool,
        CitationAuditParams {
            block,
            source: ProvenanceSource::Resource(source.uuid()),
            value: 0.75,
            reason: Some("replay proof"),
            emitter,
        },
    )
    .await
    .unwrap();

    // Identify the row by its EMITTING EVENT, never by its surrogate `id`. `kb_citation_audits.id`
    // carries no inbound references, which makes it a masked-surrogate table by this module's own
    // rule (`replay.rs:3`): the namespace reset deletes the row, so replay's INSERT mints a FRESH
    // `uuid_generate_v7()` and the original id is gone by design. `audited_by_event_id` is the
    // replay-stable identity — it comes off the ledger, which is the whole point.
    // EVERY column, not a chosen four. The tuple used to stop at `value` while the assertion below
    // claimed "byte-identical", so `reason` and — the one that mattered — `created` were outside the
    // comparison. `created` is the audit's DECAY CLOCK: `resource_citation_quality` weights each
    // verdict by `pow(0.5, age/30d)`, so a projector that leaves `created` to `DEFAULT now()`
    // rewrites every weight on replay and can INVERT a band, while this test stayed green. `id` is
    // deliberately still excluded — see the masked-surrogate note above.
    const BY_EVENT: &str = "SELECT block_id, source_kind::text, source_id, value, reason, created \
                            FROM kb_citation_audits WHERE audited_by_event_id=$1";
    type AuditRow = (
        Uuid,
        String,
        Uuid,
        f64,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    );

    let event_id: Uuid =
        sqlx::query_scalar("SELECT audited_by_event_id FROM kb_citation_audits WHERE id=$1")
            .bind(audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    // Age the trail before snapshotting, exactly as `evidential_standing.rs` does to exercise decay
    // without sleeping — and it is what makes the `created` comparison FALSIFYING rather than
    // decorative. A same-second write reprojects a `now()` within microseconds of the original, so
    // the broken projector would pass; a year-old audit reprojects a year off. Both the event's
    // `occurred_at` (replay's source of truth) and the row's `created` move together, which is the
    // invariant the projector is supposed to maintain.
    sqlx::query("UPDATE kb_events SET occurred_at = now() - interval '365 days' WHERE id=$1")
        .bind(event_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE kb_citation_audits SET created = (SELECT occurred_at FROM kb_events WHERE id=$1) \
          WHERE audited_by_event_id=$1",
    )
    .bind(event_id)
    .execute(&pool)
    .await
    .unwrap();

    let before: AuditRow = sqlx::query_as(BY_EVENT)
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap)
        .await
        .expect("replay must find a projector for citation_audited — no 'no projector' error");

    let after: AuditRow = sqlx::query_as(BY_EVENT)
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .expect("the citation audit must be reprojected from its event by replay");

    assert_eq!(
        before, after,
        "the reprojected row must be byte-identical to the original — INCLUDING `created`, which \
         is the decay clock the standing projection weights every verdict by"
    );

    // ...and exactly once. `ON CONFLICT (audited_by_event_id) DO NOTHING` is what makes a second
    // replay pass a no-op rather than a duplicate; without the unique constraint behind it, an
    // append-only trail would silently double every time the ledger is walked again.
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE audited_by_event_id=$1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "replay must reproject the audit exactly once");
}
