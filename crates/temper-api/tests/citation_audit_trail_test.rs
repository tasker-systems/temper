#![cfg(feature = "test-db")]
//! `citation_audit_service::list_citation_audits` — the attributed read behind
//! `GET /api/resources/{id}/citation-audits` (Set 5 per-auditor collapse, Beat 3).
//!
//! Service-direct, mirroring `evidence_handler_test.rs` rather than the HTTP-level
//! `citation_audit_handler_test.rs`: what is under test here is *who the trail names*, which needs
//! several distinct auditor principals, and a JWT-resolvable principal per auditor would be
//! scaffolding that proves nothing extra — the 401 and the routing are the `POST` sibling's
//! concern and are already covered there.
//!
//! Every audit is created through the REAL projector (`_project_citation_audited`, extended by
//! Beat 1 to resolve `audited_by_profile_id` off the owning event's emitter), never by inserting
//! into `kb_citation_audits` directly. A direct insert would let the test choose the attribution
//! it then asserts, which would pass against a projector that attributed nothing.
//!
//! `kb_events` is append-only (`kb_events_append_only`), so each audit event is constructed with
//! the `occurred_at` it needs AT INSERT TIME — nothing here updates an event.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_services::error::ApiError;
use temper_services::services::citation_audit_service::list_citation_audits;
use temper_services::services::evidential_standing_service::resource_evidence;

mod common;

/// Give `finding` one live resource-kind citation — a body block, a cited resource, and the
/// `kb_block_provenance` row joining them, which is the exact three-row set
/// `resource_live_citations` walks and therefore the set the trail read joins through. Returns
/// `(block_id, source_id)`.
///
/// Same fixture as `evidence_handler_test.rs:27-63`, returning the block as well because a citation
/// is the `(block, source)` PAIR and an audit is addressed at that grain. The contributing event is
/// borrowed from the context scaffold; nothing under test reads it.
async fn cite_one_source_on(pool: &PgPool, finding: Uuid) -> (Uuid, Uuid) {
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("the context scaffold seeded an event");
    let block = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id) \
         VALUES ($1, $2, 0, $3, $3)",
    )
    .bind(block)
    .bind(finding)
    .bind(event)
    .execute(pool)
    .await
    .expect("insert block");

    let source = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(source)
        .bind("Cited Source")
        .bind(format!("test://{source}"))
        .execute(pool)
        .await
        .expect("insert cited source");
    sqlx::query(
        "INSERT INTO kb_block_provenance \
             (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
         VALUES ($1, 'resource', $2, $3, 0)",
    )
    .bind(block)
    .bind(source)
    .bind(event)
    .execute(pool)
    .await
    .expect("cite the source on the block");

    (block, source)
}

/// An auditor principal: a profile plus the `kb_entities` row its events are emitted from. The
/// entity is what carries identity onto an event (`kb_events.emitter_entity_id` →
/// `kb_entities.profile_id`), and Beat 1's projector resolves `audited_by_profile_id` along that
/// exact chain — so ONE ENTITY PER AUDITOR is what makes two auditors two auditors.
struct Auditor {
    profile: Uuid,
    entity: Uuid,
}

async fn seed_auditor(pool: &PgPool, handle: &str) -> Auditor {
    let profile = common::seed_profile(pool, handle).await;
    let entity = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_entities (id, profile_id, name) VALUES ($1, $2, $3)")
        .bind(entity)
        .bind(profile)
        .bind(format!("{handle}-{entity}@agent"))
        .execute(pool)
        .await
        .expect("insert auditor entity");
    Auditor { profile, entity }
}

/// The one act [`audit_citation`] writes: who audited which citation, with what verdict and when.
/// A params struct rather than a long argument list — the project's rule is that more than five
/// domain-related parameters get a struct, and `#[expect(clippy::too_many_arguments)]` is a smell
/// to fix rather than suppress.
struct AuditSpec<'a> {
    /// The context anchoring the emitted event (`producing_anchor_id`).
    ctx: Uuid,
    auditor: &'a Auditor,
    block: Uuid,
    source: Uuid,
    /// The signed verdict, in `[-1.0, 1.0]`.
    value: f64,
    reason: &'a str,
    /// The emitted event's `occurred_at`. Becomes the audit's `created`, so it is both the trail's
    /// sort key and the audit's decay weight.
    ///
    /// An EXPLICIT instant chosen in Rust, not `now() - interval` computed in SQL: each audit is
    /// appended by its own statement and therefore its own transaction, so two `now()`s differ by
    /// microseconds. `trail_orders_audits_sharing_a_created_deterministically` needs two audits whose
    /// `created` is byte-identical, which only a value the caller controls can give it.
    occurred_at: DateTime<Utc>,
}

/// `n` days before now — the ordinary case, where the exact instant does not matter, only the order.
fn days_ago(n: i64) -> DateTime<Utc> {
    Utc::now() - Duration::days(n)
}

/// Emit one `citation_audited` event from `auditor` and project it, exactly as the SQL write path
/// does. `spec.occurred_at` sets the event's `occurred_at`, which the projector stamps onto
/// `kb_citation_audits.created` — so it is both the trail's sort key and the audit's decay weight.
/// Returns the new `kb_citation_audits.id`.
///
/// The payload's source rides NESTED (`{"source":{"kind":…,"value":…}}`), which is what the
/// projector reads (`p_payload #>> '{source,kind}'`) and what `ProvenanceSource`'s tagged serde
/// emits. A flat spelling here would project a NULL source and the assertions would drift.
async fn audit_citation(pool: &PgPool, spec: AuditSpec<'_>) -> Uuid {
    let AuditSpec {
        ctx,
        auditor,
        block,
        source,
        value,
        reason,
        occurred_at,
    } = spec;
    let event = Uuid::now_v7();
    let payload = serde_json::json!({
        "block_id": block,
        "source": { "kind": "resource", "value": source },
        "value": value,
        "reason": reason,
    });
    sqlx::query(
        "INSERT INTO kb_events \
             (id, event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, \
              occurred_at, payload) \
         SELECT $1, (SELECT id FROM kb_event_types WHERE name = 'citation_audited'), \
                $2, 'kb_contexts', $3, $4, $5",
    )
    .bind(event)
    .bind(auditor.entity)
    .bind(ctx)
    .bind(occurred_at)
    .bind(&payload)
    .execute(pool)
    .await
    .expect("append citation_audited event");

    sqlx::query_scalar::<_, Uuid>("SELECT _project_citation_audited($1, $2)")
        .bind(event)
        .bind(&payload)
        .fetch_one(pool)
        .await
        .expect("project the audit")
}

/// The trail names the auditor of each audit — the gap this beat closes. An audit with no
/// attribution is a verdict with no voter.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn trail_attributes_each_audit_to_its_auditor(pool: PgPool) {
    let (owner, ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    let (block, source) = cite_one_source_on(&pool, goal).await;
    let adversary = seed_auditor(&pool, "adversary").await;
    let audit = audit_citation(
        &pool,
        AuditSpec {
            ctx,
            auditor: &adversary,
            block,
            source,
            value: -1.0,
            reason: "the cited paragraph does not support the claim",
            occurred_at: days_ago(1),
        },
    )
    .await;

    let trail = list_citation_audits(&pool, ProfileId::from(owner), ResourceId::from(goal))
        .await
        .expect("the owner reads its own finding's audit trail");

    assert_eq!(trail.len(), 1, "one audit, one row: {trail:?}");
    let row = &trail[0];
    assert_eq!(row.audit_id, audit, "row: {row:?}");
    assert_eq!(row.block_id.uuid(), block, "row: {row:?}");
    assert_eq!(row.source_id.uuid(), source, "row: {row:?}");
    assert_eq!(row.value, -1.0, "row: {row:?}");
    assert_eq!(
        row.reason.as_deref(),
        Some("the cited paragraph does not support the claim"),
        "row: {row:?}"
    );
    // The attribution itself: the profile id AND both human-readable columns, so a caller can name
    // the voter without a second round trip.
    assert_eq!(
        row.auditor_profile_id.uuid(),
        adversary.profile,
        "the auditor is the profile behind the EMITTING entity, resolved by the projector: {row:?}"
    );
    assert!(
        row.auditor_handle.starts_with("adversary-"),
        "kb_profiles.handle is joined in: {row:?}"
    );
    assert_eq!(
        row.auditor_display_name, "adversary",
        "kb_profiles.display_name is joined in: {row:?}"
    );
}

/// **The point of the beat.** Two DIFFERENT principals audit the SAME citation, and the trail keeps
/// them apart — while the aggregate read cannot.
///
/// A test in which one principal audits twice could not falsify anything: two rows would appear
/// whether or not attribution works. So both halves are asserted — two DISTINCT
/// `auditor_profile_id`s (which a projector that attributed every audit to one profile would fail),
/// and `audit_coverage == 1` on the sibling `/evidence` read, which is the exact collapse this
/// endpoint exists to open up.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn two_principals_auditing_one_citation_stay_distinguishable(pool: PgPool) {
    let (owner, ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    let (block, source) = cite_one_source_on(&pool, goal).await;
    let first = seed_auditor(&pool, "auditor-a").await;
    let second = seed_auditor(&pool, "auditor-b").await;

    // Opposite verdicts on ONE citation, ten days apart. The ledger is append-only and there is no
    // supersession: both rows are permanent, and disagreement is the signal, not an error state.
    audit_citation(
        &pool,
        AuditSpec {
            ctx,
            auditor: &first,
            block,
            source,
            value: -1.0,
            reason: "A: unsupported",
            occurred_at: days_ago(10),
        },
    )
    .await;
    audit_citation(
        &pool,
        AuditSpec {
            ctx,
            auditor: &second,
            block,
            source,
            value: 1.0,
            reason: "B: supported",
            occurred_at: days_ago(1),
        },
    )
    .await;

    let trail = list_citation_audits(&pool, ProfileId::from(owner), ResourceId::from(goal))
        .await
        .expect("the owner reads the trail");

    assert_eq!(trail.len(), 2, "both audits are permanent rows: {trail:?}");
    assert_ne!(
        trail[0].auditor_profile_id, trail[1].auditor_profile_id,
        "two auditors must not collapse into one: {trail:?}"
    );
    let auditors: Vec<Uuid> = trail.iter().map(|r| r.auditor_profile_id.uuid()).collect();
    assert!(
        auditors.contains(&first.profile) && auditors.contains(&second.profile),
        "each audit is attributed to the principal that emitted it: {trail:?}"
    );

    // Most recent first — the head of the trail carries the most weight in the decayed aggregate.
    assert_eq!(
        trail[0].auditor_profile_id.uuid(),
        second.profile,
        "ordering is most-recent-first: {trail:?}"
    );
    assert!(
        trail[0].created > trail[1].created,
        "and `created` comes from the owning event, not from now(): {trail:?}"
    );

    // The aggregate cannot see any of that. ONE distinct source is covered however many auditors
    // weighed in, and the two opposite verdicts arrive as a single blended scalar.
    let shape = resource_evidence(&pool, owner, goal)
        .await
        .expect("the owner reads the standing shape");
    assert_eq!(
        shape.audit_coverage, 1,
        "the shape counts covered SOURCES, not auditors — this is the collapse the trail opens up: \
         {shape:?}"
    );
    assert_eq!(
        shape.citation_magnitude, 1,
        "still one cited source: {shape:?}"
    );
}

/// **The `a.id DESC` tiebreak.** Two audits that share a `created` must come back in ONE order, the
/// same order every time — `migrations/20260724000220_citation_audit_trail.sql:53-62` argues that
/// case at length and nothing else in the suite reaches it: every other ordering fixture uses
/// distinct timestamps, so deleting the tiebreak would leave them all green.
///
/// The tie is REACHABLE, not hypothetical. `created` is stamped from the owning event's
/// `occurred_at` (`20260724000110:68-78`), which is a value the emitter supplies — one transaction
/// emitting several audits stamps them all with its own `now()`, so equal-`created` rows are the
/// ordinary shape there, not a corner. Both audits here are given the SAME instant, which is why
/// `AuditSpec` carries an explicit `occurred_at` rather than a day count.
///
/// Two assertions, because the header makes two claims. `ORDER BY a.id DESC` fixes WHICH order — the
/// descending-id one, whatever the ids happen to be — and repeated reads returning that same order is
/// the property a paging or diffing caller actually depends on. Drop the tiebreak and the sort is
/// unspecified for the tied rows: the planner is free to return either, and in practice returns them
/// in whatever order the scan produced, which is not the descending-id one this pins.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn trail_orders_audits_sharing_a_created_deterministically(pool: PgPool) {
    let (owner, ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    let (block, source) = cite_one_source_on(&pool, goal).await;
    let first = seed_auditor(&pool, "tie-auditor-a").await;
    let second = seed_auditor(&pool, "tie-auditor-b").await;

    // ONE instant, both audits — the tie itself.
    let same_instant = days_ago(2);
    for (auditor, value, reason) in [
        (&first, -1.0, "A: unsupported"),
        (&second, 1.0, "B: supported"),
    ] {
        audit_citation(
            &pool,
            AuditSpec {
                ctx,
                auditor,
                block,
                source,
                value,
                reason,
                occurred_at: same_instant,
            },
        )
        .await;
    }

    let trail = list_citation_audits(&pool, ProfileId::from(owner), ResourceId::from(goal))
        .await
        .expect("the owner reads the trail");
    assert_eq!(trail.len(), 2, "trail: {trail:?}");
    // Precondition: without this the fixture is not a tie and the rest proves nothing.
    assert_eq!(
        trail[0].created, trail[1].created,
        "both audits must share a `created` for this to exercise the tiebreak: {trail:?}"
    );
    assert!(
        trail[0].audit_id > trail[1].audit_id,
        "tied rows come back in descending id order, which is what `ORDER BY a.created DESC, \
         a.id DESC` specifies: {trail:?}"
    );

    // The same unchanged trail, read again, is the same list — the property the header's paging
    // caller depends on, and the one an unspecified sort does not provide.
    let ids: Vec<Uuid> = trail.iter().map(|r| r.audit_id).collect();
    for attempt in 0..3 {
        let again = list_citation_audits(&pool, ProfileId::from(owner), ResourceId::from(goal))
            .await
            .expect("re-read the trail");
        let again_ids: Vec<Uuid> = again.iter().map(|r| r.audit_id).collect();
        assert_eq!(
            again_ids, ids,
            "re-read {attempt} returned a different order for an unchanged trail: {again:?}"
        );
    }
}

/// An unreadable finding does not leak: the trail refuses in exactly the dialect `/evidence` uses,
/// so a caller cannot diff the two endpoints into an existence oracle. Absent and denied are
/// likewise indistinguishable on both.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unreadable_finding_is_not_found_exactly_as_evidence_is(pool: PgPool) {
    let (owner, ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    let (block, source) = cite_one_source_on(&pool, goal).await;
    let adversary = seed_auditor(&pool, "adversary").await;
    // A real audit exists — so an unguarded read would have something to leak.
    audit_citation(
        &pool,
        AuditSpec {
            ctx,
            auditor: &adversary,
            block,
            source,
            value: -1.0,
            reason: "unsupported",
            occurred_at: days_ago(1),
        },
    )
    .await;

    let stranger = common::seed_profile(&pool, "stranger").await;

    let trail =
        list_citation_audits(&pool, ProfileId::from(stranger), ResourceId::from(goal)).await;
    assert!(
        matches!(trail, Err(ApiError::NotFound)),
        "an existing but unreadable finding is NotFound, never an empty 200: {trail:?}"
    );
    let evidence = resource_evidence(&pool, stranger, goal).await;
    assert!(
        matches!(evidence, Err(ApiError::NotFound)),
        "the sibling read refuses identically — same status, no diffable signal: {evidence:?}"
    );

    let absent_id = Uuid::now_v7();
    let absent_trail = list_citation_audits(
        &pool,
        ProfileId::from(stranger),
        ResourceId::from(absent_id),
    )
    .await;
    assert!(
        matches!(absent_trail, Err(ApiError::NotFound)),
        "an absent finding is NotFound too: {absent_trail:?}"
    );
    let absent_evidence = resource_evidence(&pool, stranger, absent_id).await;
    assert!(
        matches!(absent_evidence, Err(ApiError::NotFound)),
        "…and so is it on /evidence: {absent_evidence:?}"
    );

    // And the owner still reads it, so the deny above is the gate working, not the fixture failing.
    let owner_trail = list_citation_audits(&pool, ProfileId::from(owner), ResourceId::from(goal))
        .await
        .expect("the owner still reads its own trail");
    assert_eq!(owner_trail.len(), 1, "trail: {owner_trail:?}");
}

/// A READABLE finding with no audits is an empty list, not an error — the only case in which empty
/// is the right answer. This is the half that makes the 404 above meaningful: if every empty trail
/// 404'd, the endpoint would be telling an honest caller its own finding does not exist.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn readable_finding_with_no_audits_is_empty_not_an_error(pool: PgPool) {
    let (owner, _ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    // Cited, but nobody has audited it — the state the auditor's queue exists to clear.
    cite_one_source_on(&pool, goal).await;

    let trail = list_citation_audits(&pool, ProfileId::from(owner), ResourceId::from(goal))
        .await
        .expect("a readable finding with no audits is Ok, not NotFound");
    assert!(trail.is_empty(), "trail: {trail:?}");

    // A readable finding with no CITATIONS at all is likewise empty rather than an error.
    let (other_owner, _c, uncited) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    let uncited_trail = list_citation_audits(
        &pool,
        ProfileId::from(other_owner),
        ResourceId::from(uncited),
    )
    .await
    .expect("no citations, no audits, still Ok");
    assert!(uncited_trail.is_empty(), "trail: {uncited_trail:?}");
}
