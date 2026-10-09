#![cfg(feature = "test-db")]
//! The sweep-to-erasure bridge (build order 3c): what the erasure surveys read from the sweep's
//! stored findings, and that the acts never read it.
//!
//! Under goal *"A single resource can be erased out of a live estate"*, resource erasure spec D10, D11
//! and its *Rulings from build order 3c* (R1–R4, P5). Spec witnesses:
//!   * **17** — the block scrub survey warns while a block's current revision holds an open finding,
//!     and stops once it is edited out and swept; the scrub then closes the prior revision's finding.
//!   * **18** — each deriver's `fingerprint_match` reads `yes`, `no`, `unscanned` or `expired`; with
//!     the sweep's functions gone the surveys still render and the act's record is unchanged.
//!
//! Also the coverage edges `unscanned` rests on: a deployment that never swept, a mutable place
//! edited past the head, an oversize place, and the deriver list's own duplicates.

use sqlx::PgPool;
use temper_core::types::erasure::{DeriverFingerprint, FingerprintMatch};
use temper_core::types::ids::{ContextId, EntityId, ProfileId};
use temper_services::services::block_history_scrub_service::survey_block_history_scrub;
use temper_services::services::resource_erasure_service::survey_resource_erasure;
use temper_substrate::ids::ResourceId;
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

const SALT: &[u8] = b"witness-salt-of-sixteen-plus";
const SSN_A: &str = "219-45-6789";
const SSN_B: &str = "536-22-8147";
const NO_LAG: &str = "0 seconds";

// ── fixture helpers (duplicated per file, per this suite's convention) ──────────────────────

/// The operator's act on a deployment that opts in (Q52, Q53).
async fn enable_seeded_detectors(pool: &PgPool) {
    sqlx::query("SELECT sensitivity.enable_detectors(1, 'temper')")
        .execute(pool)
        .await
        .unwrap();
}

async fn tick(pool: &PgPool, surface: &str) {
    sqlx::query("SELECT workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep', $1)")
        .bind(serde_json::json!({ "surface": surface, "budget": 1000 }))
        .execute(pool)
        .await
        .unwrap();
    let (run, job): (Uuid, Uuid) =
        sqlx::query_as("SELECT run_id, job_id FROM sensitivity_sweep_claim()")
            .fetch_one(pool)
            .await
            .unwrap();
    let failed: bool =
        sqlx::query_scalar("SELECT failed FROM sensitivity_sweep_tick($1, $2, $3, $4::interval)")
            .bind(run)
            .bind(job)
            .bind(SALT)
            .bind(NO_LAG)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(!failed, "the tick on {surface} failed");
}

/// One tick of every enabled surface, so every place written so far is behind its cursors.
async fn sweep(pool: &PgPool) {
    let surfaces: Vec<String> = sqlx::query_scalar(
        "SELECT surface FROM sensitivity.surfaces WHERE enabled AND cursor_kind IS NOT NULL ORDER BY surface",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for surface in surfaces {
        tick(pool, &surface).await;
    }
}

struct World {
    owner: ProfileId,
    emitter: EntityId,
    home: Uuid,
}

async fn world(pool: &PgPool) -> World {
    bootseed::seed_system(pool).await.unwrap();
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
    let home: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'bridge-home', 'bridge-home') RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .unwrap();
    World {
        owner: ProfileId::from(profile),
        emitter: EntityId::from(entity),
        home,
    }
}

async fn resource(pool: &PgPool, w: &World, title: &str, body: &str) -> Uuid {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: title,
            body,
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(w.home)),
            owner: w.owner,
            originator: w.owner,
            emitter: w.emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        temper_substrate::events::EventContext::default(),
    )
    .await
    .expect("create through the write path")
    .uuid()
}

/// `deriver` derived_from `source`: the D8 structural lead.
async fn derives(pool: &PgPool, w: &World, deriver: Uuid, source: Uuid) {
    sqlx::query(
        "INSERT INTO kb_edges \
           (source_table, source_id, target_table, target_id, edge_kind, label, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         SELECT 'kb_resources', $1, 'kb_resources', $2, 'leads_to', 'derived_from', \
                'kb_contexts', $3, e.id, e.id \
           FROM (SELECT id FROM kb_events ORDER BY id DESC LIMIT 1) e",
    )
    .bind(deriver)
    .bind(source)
    .bind(w.home)
    .execute(pool)
    .await
    .unwrap();
}

async fn only_block(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Revise the resource's one block to `body` through the write path.
async fn revise(pool: &PgPool, w: &World, resource: Uuid, body: &str) {
    let block = only_block(pool, resource).await;
    writes::update_resource(
        pool,
        writes::UpdateParams {
            resource: ResourceId::from(resource),
            body: Some(body),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: None,
            sources: vec![],
            content_block: Some(block),
            rehome_to: None,
            emitter: w.emitter,
        },
    )
    .await
    .expect("a per-block revise");
}

async fn scrub(pool: &PgPool, w: &World, resource: Uuid) {
    let block = only_block(pool, resource).await;
    sqlx::query("SELECT block_history_scrub_execute($1, $2, $3, $4, $5)")
        .bind(resource)
        .bind(vec![block])
        .bind(w.owner.uuid())
        .bind(w.emitter)
        .bind(Uuid::now_v7())
        .execute(pool)
        .await
        .expect("the scrub completes");
}

/// Q50's window run out: the expiry clocks the closed findings, the clock is wound back past 30
/// days, and the expiry runs again.
async fn expire_erased_digests(pool: &PgPool) {
    sqlx::query("SELECT sensitivity_expire_erased_fingerprints()")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sensitivity.erased_closures SET closed_seen_at = now() - interval '31 days'",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("SELECT sensitivity_expire_erased_fingerprints()")
        .execute(pool)
        .await
        .unwrap();
}

/// The bridge's answer for R's derivers, in the order asked, through the public wrapper (the door
/// the survey service calls).
async fn matches(pool: &PgPool, r: Uuid, derivers: &[Uuid]) -> Vec<(Uuid, String)> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT deriver_id, state FROM resource_erasure_deriver_fingerprints($1, $2)",
    )
    .bind(r)
    .bind(derivers)
    .fetch_all(pool)
    .await
    .unwrap();
    let mut ordered: Vec<(Uuid, String)> = Vec::new();
    for d in derivers {
        for row in rows.iter().filter(|(id, _)| id == d) {
            if !ordered.contains(row) {
                ordered.push(row.clone());
            }
        }
    }
    assert_eq!(
        ordered.len(),
        rows.len(),
        "every row answers a deriver asked about"
    );
    ordered
}

fn states(rows: &[(Uuid, String)]) -> Vec<&str> {
    rows.iter().map(|(_, s)| s.as_str()).collect()
}

async fn flagged(pool: &PgPool, resource: Uuid, blocks: &[Uuid]) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT block_history_scrub_flagged_blocks($1, $2)")
        .bind(resource)
        .bind(blocks)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn mark(pool: &PgPool, resource: Uuid, state: &str) {
    sqlx::query(
        "INSERT INTO sensitivity.dispositions (finding_id, state) \
         SELECT id, $2 FROM sensitivity.findings WHERE resource_id = $1",
    )
    .bind(resource)
    .bind(state)
    .execute(pool)
    .await
    .unwrap();
}

// ── Witness 18: yes, no, unscanned ──────────────────────────────────────────────────────────

/// FAILS IF the fingerprint join is lost (D1 not `yes`), if an unread deriver reads as clean (D3
/// not `unscanned`), or if a read, clean deriver is not `no`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_deriver_reads_yes_no_or_unscanned(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "quote", &format!("as the payroll said, {SSN_A}")).await;
    let d2 = resource(&pool, &w, "summary", "a summary with nothing in it").await;
    derives(&pool, &w, d1, r).await;
    derives(&pool, &w, d2, r).await;
    sweep(&pool).await;
    let d3 = resource(&pool, &w, "later", "written after the sweep").await;
    derives(&pool, &w, d3, r).await;

    assert_eq!(
        states(&matches(&pool, r, &[d1, d2, d3]).await),
        vec!["yes", "no", "unscanned"]
    );
}

// ── Witness 18 and R1, R3: R's closed findings count until their fingerprints expire ────────

/// FAILS IF R's side is limited to open findings (the first `yes`), or if a lost fingerprint reads
/// `no` rather than `expired`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn r_side_closed_findings_count_then_expire(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "quote", &format!("as the payroll said, {SSN_A}")).await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;
    revise(&pool, &w, r, "the record is clean now").await;
    scrub(&pool, &w, r).await;
    sweep(&pool).await;

    assert_eq!(
        states(&matches(&pool, r, &[d1]).await),
        vec!["yes"],
        "R's only finding is closed by the scrub, and still matches"
    );

    expire_erased_digests(&pool).await;
    assert_eq!(
        states(&matches(&pool, r, &[d1]).await),
        vec!["expired"],
        "the sweep can no longer compare, so it does not say no"
    );
}

/// FAILS IF a deriver's closed finding still counts: it would read `yes` for a deriver that no
/// longer holds the value.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_derivers_closed_finding_does_not_count(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "quote", &format!("as the payroll said, {SSN_A}")).await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;
    revise(&pool, &w, d1, "the quote was removed").await;
    scrub(&pool, &w, d1).await;
    sweep(&pool).await;

    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["no"]);
}

/// FAILS IF a finding ruled `false_positive` still matches, on either side.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_false_positive_counts_on_neither_side(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "quote", &format!("as the payroll said, {SSN_A}")).await;
    let d2 = resource(&pool, &w, "also", &format!("and again, {SSN_A}")).await;
    derives(&pool, &w, d1, r).await;
    derives(&pool, &w, d2, r).await;
    sweep(&pool).await;

    mark(&pool, d1, "false_positive").await;
    mark(&pool, d2, "acknowledged").await;
    assert_eq!(
        states(&matches(&pool, r, &[d1, d2]).await),
        vec!["no", "yes"],
        "the deriver's ruling silences it; an acknowledgement does not"
    );

    mark(&pool, r, "false_positive").await;
    assert_eq!(
        states(&matches(&pool, r, &[d1, d2]).await),
        vec!["no", "no"],
        "R's ruling silences every match"
    );
}

// ── Coverage edges (ruling R2) ──────────────────────────────────────────────────────────────

/// FAILS IF an empty detector set reads as covered: a deployment that never swept would say `no`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn never_swept_is_unscanned_and_swept_clean_is_no(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "plain", "nothing to find here").await;
    let d1 = resource(&pool, &w, "quote", &format!("an unrelated {SSN_B}")).await;
    derives(&pool, &w, d1, r).await;

    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["unscanned"]);
    sweep(&pool).await;
    assert_eq!(
        states(&matches(&pool, r, &[d1]).await),
        vec!["no"],
        "R holds no detected value, so no deriver can share one"
    );
}

/// FAILS IF, with no finding on R, a disabled detector is waited on: it stops reading, so a deriver
/// written since would read `unscanned` for good on a deployment that has turned one off.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_disabled_detector_does_not_hold_a_clean_resource_open(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "plain", "nothing to find here").await;
    sweep(&pool).await;
    sqlx::query("SELECT sensitivity.disable_detector('payment_card')")
        .execute(&pool)
        .await
        .unwrap();
    let d1 = resource(
        &pool,
        &w,
        "later",
        "written after the card detector was turned off",
    )
    .await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;

    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["no"]);
}

/// FAILS IF a mutable place's key is read from anything but `(updated, id)`: a title edited after
/// the head passed would read as covered.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_title_edited_past_the_head_is_unscanned(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "summary", "clean").await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;
    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["no"]);

    sqlx::query("UPDATE kb_resources SET title = 'renamed', updated = now() WHERE id = $1")
        .bind(d1)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["unscanned"]);
}

/// FAILS IF R's own coverage is not read: with the deriver read in full, R edited past the head
/// may hold a value the sweep has not seen, so `no` cannot be said (R2 applies to R too).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn r_edited_past_the_head_makes_its_derivers_unscanned(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "summary", "clean").await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;
    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["no"]);

    sqlx::query("UPDATE kb_resources SET title = 'renamed', updated = now() WHERE id = $1")
        .bind(r)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["unscanned"]);
}

/// FAILS IF the backfill lane is not read: content older than a detector's bound is covered only
/// once its backfill has passed it, however far the head has gone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn content_the_backfill_has_not_reached_is_unscanned(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "summary", "clean").await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;
    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["no"]);

    // The backfill as it stands before its first step: at its floor, not yet started.
    sqlx::query(
        "UPDATE sensitivity.cursors SET watermark_at = NULL, watermark_id = NULL, \
                backfill_completed_at = NULL \
          WHERE lane = 'backfill' AND surface = 'kb_block_content.content'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["unscanned"]);
}

/// FAILS IF `unscanned_places` is not read: an oversize place passed by the sweep would read clean.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_oversize_place_is_unscanned(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "summary", "clean").await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;
    sqlx::query(
        "INSERT INTO sensitivity.unscanned_places (surface, target_id, reason) \
         SELECT 'kb_block_content.content', b.current_revision_id, 'oversize' \
           FROM kb_content_blocks b WHERE b.resource_id = $1",
    )
    .bind(d1)
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(states(&matches(&pool, r, &[d1]).await), vec!["unscanned"]);
}

/// FAILS IF R is answered as its own deriver, or a deriver named twice is answered twice.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn r_itself_and_a_repeated_deriver_are_answered_once(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "quote", &format!("as the payroll said, {SSN_A}")).await;
    sweep(&pool).await;

    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT deriver_id, state FROM resource_erasure_deriver_fingerprints($1, $2)",
    )
    .bind(r)
    .bind(vec![r, d1, d1])
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows, vec![(d1, "yes".to_string())]);
}

// ── Witness 17: the scrub survey's current-revision warning (ruling R4) ─────────────────────

/// FAILS IF the warning reads anything but an open finding on the current revision: a closed or
/// prior-revision finding would keep warning after the edit and sweep.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_scrub_warning_follows_the_current_revision(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let block = only_block(&pool, r).await;
    sweep(&pool).await;
    assert_eq!(
        flagged(&pool, r, &[block]).await,
        vec![block],
        "not edited out yet"
    );

    revise(&pool, &w, r, "the record is clean now").await;
    assert_eq!(
        flagged(&pool, r, &[block]).await,
        Vec::<Uuid>::new(),
        "R2's stated gap: an unswept current revision gives no warning"
    );
    sweep(&pool).await;
    assert_eq!(flagged(&pool, r, &[block]).await, Vec::<Uuid>::new());

    scrub(&pool, &w, r).await;
    let closed: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT c.closed_by FROM sensitivity.findings f \
           LEFT JOIN sensitivity.finding_closure c ON c.finding_id = f.id \
          WHERE f.resource_id = $1 AND f.surface = 'kb_block_content.content'",
    )
    .bind(r)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        closed,
        vec![Some("content_empty".to_string())],
        "the scrub closes it"
    );
}

/// FAILS IF a folded block, which has no current revision to keep, can warn (P6).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_folded_block_never_warns(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let block = only_block(&pool, r).await;
    sweep(&pool).await;
    sqlx::query("UPDATE kb_content_blocks SET is_folded = true WHERE id = $1")
        .bind(block)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(flagged(&pool, r, &[block]).await, Vec::<Uuid>::new());
}

// ── Witness 18, last clause: the act never consumes either value ────────────────────────────

/// Two identical estates in one database; the first is erased with the sweep's functions present,
/// the second after they are dropped. FAILS IF the act reads either value (the records differ), or
/// if a survey stops rendering when the functions are gone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_act_never_consumes_the_annotation(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let mut estates = Vec::new();
    for n in 0..2 {
        let r = resource(
            &pool,
            &w,
            &format!("payroll {n}"),
            &format!("holds {SSN_A}"),
        )
        .await;
        let yes = resource(&pool, &w, &format!("quote {n}"), &format!("quotes {SSN_A}")).await;
        let no = resource(&pool, &w, &format!("summary {n}"), "clean").await;
        derives(&pool, &w, yes, r).await;
        derives(&pool, &w, no, r).await;
        estates.push((r, yes, no));
    }
    sweep(&pool).await;
    // An unswept deriver in each, so all three states reach the act.
    for (n, (r, _, _)) in estates.iter().enumerate() {
        let late = resource(&pool, &w, &format!("late {n}"), "after the sweep").await;
        derives(&pool, &w, late, *r).await;
    }

    let mut records = Vec::new();
    for (n, (r, yes, no)) in estates.iter().copied().enumerate() {
        if n == 1 {
            sqlx::query("DROP FUNCTION sensitivity.deriver_fingerprint_matches(uuid, uuid[])")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("DROP FUNCTION sensitivity.block_current_revision_flagged(uuid)")
                .execute(&pool)
                .await
                .unwrap();
            assert_eq!(
                states(&matches(&pool, r, &[yes, no]).await),
                vec!["unscanned", "unscanned"],
                "the survey still renders without the sweep"
            );
            let block = only_block(&pool, r).await;
            assert_eq!(flagged(&pool, r, &[block]).await, Vec::<Uuid>::new());
        } else {
            assert_eq!(
                states(&matches(&pool, r, &[yes, no]).await),
                vec!["yes", "no"]
            );
        }
        sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
            .bind(r)
            .bind(w.emitter)
            .bind(Uuid::now_v7())
            .execute(&pool)
            .await
            .expect("the act completes");
        let record: String = sqlx::query_scalar(
            "SELECT regexp_replace(e.payload::text, \
                    '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}', 'U', 'g') \
               FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'resource_erased' AND e.payload ->> 'subject_id' = $1::text",
        )
        .bind(r)
        .fetch_one(&pool)
        .await
        .unwrap();
        records.push(record);
    }
    assert_eq!(
        records[0], records[1],
        "the act's record does not depend on the annotation"
    );
}

// ── Through the survey doors (the services the admin routes call) ───────────────────────────

async fn scrub_flags(
    pool: &PgPool,
    admin: &temper_services::auth::SystemAdmin,
    r: Uuid,
    block: Uuid,
) -> Vec<bool> {
    survey_block_history_scrub(pool, admin, ResourceId::from(r), &[block])
        .await
        .expect("the survey renders")
        .plan
        .expect("a live resource has a plan")
        .blocks
        .iter()
        .map(|b| b.current_revision_flagged)
        .collect()
}

/// FAILS IF the scrub survey stops carrying the warning per block (Witness 17 at the door).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_scrub_survey_carries_the_warning(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let block = only_block(&pool, r).await;
    sweep(&pool).await;
    let admin = temper_services::test_support::system_admin_proof(&pool).await;
    assert_eq!(scrub_flags(&pool, &admin, r, block).await, vec![true]);

    revise(&pool, &w, r, "the record is clean now").await;
    sweep(&pool).await;
    assert_eq!(scrub_flags(&pool, &admin, r, block).await, vec![false]);
}

fn annotated(rows: &[DeriverFingerprint]) -> Vec<(Uuid, FingerprintMatch)> {
    rows.iter()
        .map(|d| (d.deriver.uuid(), d.fingerprint_match))
        .collect()
}

/// FAILS IF the erasure survey's plan stops naming each deriver's match, in the remainder's order
/// (Witness 18 at the door).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_erasure_survey_annotates_each_deriver(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "quote", &format!("as the payroll said, {SSN_A}")).await;
    let d2 = resource(&pool, &w, "summary", "a summary with nothing in it").await;
    derives(&pool, &w, d1, r).await;
    derives(&pool, &w, d2, r).await;
    sweep(&pool).await;
    let d3 = resource(&pool, &w, "later", "written after the sweep").await;
    derives(&pool, &w, d3, r).await;
    let admin = temper_services::test_support::system_admin_proof(&pool).await;

    let plan = survey_resource_erasure(&pool, &admin, ResourceId::from(r))
        .await
        .expect("the survey renders")
        .plan
        .expect("a live resource has a plan");
    let order: Vec<Uuid> = plan
        .remainder
        .iter()
        .filter(|e| e.target == "deriver")
        .filter_map(|e| {
            e.outcome
                .split(' ')
                .nth(1)
                .and_then(|id| Uuid::parse_str(id).ok())
        })
        .collect();
    let mut named = order.clone();
    named.sort();
    let mut planted = vec![d1, d2, d3];
    planted.sort();
    assert_eq!(named, planted, "the plan names the three derivers");
    let expected = |d: Uuid| {
        if d == d1 {
            FingerprintMatch::Yes
        } else if d == d2 {
            FingerprintMatch::No
        } else {
            FingerprintMatch::Unscanned
        }
    };
    assert_eq!(
        annotated(&plan.deriver_fingerprints),
        order.iter().map(|d| (*d, expected(*d))).collect::<Vec<_>>(),
        "each deriver's match, in the remainder's order"
    );
}

/// After the act the husk's survey still answers for the derivers its erasure named, from R's
/// closed findings, until Q50's window ends (P5). FAILS IF the husk branch drops the annotation,
/// re-derives the derivers from edges the act folded, or limits R's side to open findings.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_husks_survey_annotates_the_derivers_its_erasure_named(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "payroll", &format!("the record holds {SSN_A}")).await;
    let d1 = resource(&pool, &w, "quote", &format!("as the payroll said, {SSN_A}")).await;
    derives(&pool, &w, d1, r).await;
    sweep(&pool).await;
    sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(r)
        .bind(w.emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the act completes");
    sweep(&pool).await;
    let admin = temper_services::test_support::system_admin_proof(&pool).await;

    let husk = survey_resource_erasure(&pool, &admin, ResourceId::from(r))
        .await
        .expect("the husk's survey renders");
    assert!(husk.plan.is_none());
    assert_eq!(
        annotated(&husk.deriver_fingerprints),
        vec![(d1, FingerprintMatch::Yes)]
    );

    // A completion pass's record carries no remainder (D12); the first record still names them.
    sqlx::query(
        "SELECT _event_append('resource_erased', $2, NULL, NULL, \
                jsonb_build_object('subject_table', 'kb_resources', 'subject_id', $1, \
                                   'actor', $3, 'redacted_fields', '[]'::jsonb))",
    )
    .bind(r)
    .bind(w.emitter)
    .bind(w.owner.uuid())
    .execute(&pool)
    .await
    .unwrap();
    let husk = survey_resource_erasure(&pool, &admin, ResourceId::from(r))
        .await
        .expect("the husk's survey renders");
    assert_eq!(
        annotated(&husk.deriver_fingerprints),
        vec![(d1, FingerprintMatch::Yes)],
        "the first record is the one that named the derivers"
    );

    expire_erased_digests(&pool).await;
    let husk = survey_resource_erasure(&pool, &admin, ResourceId::from(r))
        .await
        .expect("the husk's survey renders");
    assert_eq!(
        annotated(&husk.deriver_fingerprints),
        vec![(d1, FingerprintMatch::Expired)]
    );
}
