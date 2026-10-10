#![cfg(feature = "test-db")]
//! The sweep meets the field scrub (field-grain scrub spec S1, S2, S9; migration 20261018100030).
//!
//! Spec witness 5, through the real act and the real sweep:
//!   * a scrubbed ledger finding closes as `sentinel`;
//!   * a finding on a current value kept in keep mode stays open (S9's "you kept the leak");
//!   * a finding on a current value cleared in clear mode closes.
//!
//! Also each projection sentinel the act writes (S7): a folded row under `scrubbed-key-<handle>`,
//! a value under a kept `tags` key in its array form (review focus 2), and a facet mark; a scrubbed
//! finding stays closed when its subject is erased later (the erasure writes no row of its own on a
//! path already at erasure's sentinel); and the family flags the listing reads (S1): `flagged`,
//! `current_flagged`, `covered`, one row per listing row, and unflagged when the sweep's function is
//! absent.
//!
//! Every witness drives the act through `resource_field_scrub_execute` and the sweep through its
//! own tick, against resources built through the real write paths. Before 20261018100030 each one
//! fails: `place_closure` closed a ledger row only under a `resource_erased` and knew only erasure's
//! projection sentinels, so every closure asserted here read open, and the flag door did not exist.

use sqlx::PgPool;
use temper_core::types::ids::{ContextId, EntityId, ProfileId};
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::ResourceId;
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

const SALT: &[u8] = b"witness-salt-of-sixteen-plus";
const SSN_A: &str = "219-45-6789";
const SSN_B: &str = "536-22-8147";
const NO_LAG: &str = "0 seconds";
const SENTINEL: Option<&str> = Some("sentinel");
const OPEN: Option<&str> = None;

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
         VALUES ('kb_profiles', $1, 'scrub-sweep-home', 'scrub-sweep-home') RETURNING id",
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

async fn resource(pool: &PgPool, w: &World, title: &str) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: "test://scrub-sweep",
            body: "clean prose",
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
}

/// Fire one write through the live write path and commit it.
async fn write(pool: &PgPool, action: SeedAction<'_>) {
    let mut tx = pool.begin().await.unwrap();
    fire(&mut tx, action).await.expect("the write fires");
    tx.commit().await.unwrap();
}

async fn retitle(pool: &PgPool, w: &World, r: ResourceId, title: &str) {
    write(
        pool,
        SeedAction::ResourceUpdate {
            resource: r,
            title: Some(title),
            origin_uri: None,
            emitter: w.emitter,
        },
    )
    .await;
}

async fn set(pool: &PgPool, w: &World, r: ResourceId, key: &str, value: serde_json::Value) {
    write(
        pool,
        SeedAction::PropertySet {
            resource: r,
            key,
            value: &value,
            weight: 1.0,
            emitter: w.emitter,
        },
    )
    .await;
}

async fn facet(pool: &PgPool, w: &World, r: ResourceId, values: serde_json::Value) {
    write(
        pool,
        SeedAction::FacetSet {
            owner: PropertyOwner::Resource { id: r },
            values: &values,
            weight: 1.0,
            emitter: w.emitter,
        },
    )
    .await;
}

/// R's `event_type` events, in walk order: a resource event by `resource_id`, a property event by
/// its owner, and with `key` only those naming it.
async fn events(pool: &PgPool, event_type: &str, r: ResourceId, key: Option<&str>) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = $1 \
            AND coalesce(e.payload ->> 'resource_id', e.payload #>> '{owner,id}') = $2::text \
            AND ($3::text IS NULL OR e.payload ->> 'property_key' = $3) \
          ORDER BY e.id",
    )
    .bind(event_type)
    .bind(r.uuid())
    .bind(key)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The one `kb_properties` row `event` asserted.
async fn row_of(pool: &PgPool, event: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_properties WHERE asserted_by_event_id = $1")
        .bind(event)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn row_state(pool: &PgPool, row: Uuid) -> (String, serde_json::Value, bool) {
    sqlx::query_as(
        "SELECT property_key, property_value, is_folded FROM kb_properties WHERE id = $1",
    )
    .bind(row)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The handle of R's family whose key text is `key` (Task 3's predicate, the listing's source).
async fn handle_of(pool: &PgPool, r: ResourceId, key: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT DISTINCT handle FROM _field_scrub_property_events($1) WHERE key_text = $2",
    )
    .bind(r.uuid())
    .bind(key)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The field scrub, as the system operator, under a fresh request reference.
async fn scrub(
    pool: &PgPool,
    w: &World,
    r: ResourceId,
    field: &str,
    family: Option<Uuid>,
    clear: bool,
) {
    sqlx::query("SELECT resource_field_scrub_execute($1, $2, $3, $4, $5, $6, $7)")
        .bind(r.uuid())
        .bind(field)
        .bind(family)
        .bind(clear)
        .bind(w.owner.uuid())
        .bind(w.emitter)
        .bind(Uuid::now_v7())
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("the scrub of {field} completes: {e}"));
}

/// How each finding at (`surface`, `target`) closes, in path order: `None` while it is open.
async fn closures(pool: &PgPool, surface: &str, target: Uuid) -> Vec<Option<String>> {
    sqlx::query_scalar(
        "SELECT c.closed_by FROM sensitivity.findings f \
           LEFT JOIN sensitivity.finding_closure c ON c.finding_id = f.id \
          WHERE f.surface = $1 AND f.target_id = $2 \
          ORDER BY f.path NULLS FIRST, f.detector_id",
    )
    .bind(surface)
    .bind(target)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Every finding at (`surface`, `target`) closes as `expected`, and there is one: an empty set
/// would pass vacuously.
async fn assert_closures(
    pool: &PgPool,
    surface: &str,
    target: Uuid,
    expected: Option<&str>,
    what: &str,
) {
    let got = closures(pool, surface, target).await;
    assert!(
        !got.is_empty(),
        "{what}: the fixture holds a finding on {surface}"
    );
    assert!(
        got.iter().all(|c| c.as_deref() == expected),
        "{what}: {got:?}, expected every one {expected:?}"
    );
}

const LEDGER: &str = "kb_events.payload";
const TITLE: &str = "kb_resources.title";
const KEY: &str = "kb_properties.property_key";
const VALUE: &str = "kb_properties.property_value";

// ── Witness 5: a scrubbed ledger finding closes ─────────────────────────────────────────────

/// FAILS IF the ledger arm still counts only a `resource_erased`'s rows: the prior title's
/// redaction row is the scrub's, under a `resource_scrubbed` that lists it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_scrubbed_ledger_finding_closes_as_sentinel(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, &format!("Payroll for {SSN_A}")).await;
    retitle(&pool, &w, r, "Payroll").await;
    sweep(&pool).await;
    let created = events(&pool, "resource_created", r, None).await[0];
    assert_closures(&pool, LEDGER, created, OPEN, "before the scrub").await;

    scrub(&pool, &w, r, "title", None, false).await;

    assert_closures(&pool, LEDGER, created, SENTINEL, "the prior title").await;
}

// ── Witness 5: a kept current value stays open ──────────────────────────────────────────────

/// FAILS IF closure reaches past what the act rewrote: keep mode leaves today's title in the
/// projection and on its event, and their findings are the "you kept the leak" signal (S9). Also
/// FAILS, before 20261018100030, on the prior title, which must close beside them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_finding_on_a_kept_current_value_stays_open(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, &format!("Payroll for {SSN_A}")).await;
    retitle(&pool, &w, r, &format!("Report on {SSN_B}")).await;
    sweep(&pool).await;
    let created = events(&pool, "resource_created", r, None).await[0];
    let updated = events(&pool, "resource_updated", r, None).await[0];

    scrub(&pool, &w, r, "title", None, false).await;

    assert_closures(&pool, LEDGER, created, SENTINEL, "the prior title").await;
    assert_closures(&pool, LEDGER, updated, OPEN, "today's title on its event").await;
    assert_closures(
        &pool,
        TITLE,
        r.uuid(),
        OPEN,
        "today's title in the projection",
    )
    .await;
}

// ── Witness 5: a cleared current value closes ───────────────────────────────────────────────

/// FAILS IF the projection arm does not know the placeholder clear mode writes (S2): the title
/// is `scrubbed-<resource id>`, and no later tick has observed it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_finding_on_a_cleared_current_value_closes(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, &format!("Payroll for {SSN_A}")).await;
    sweep(&pool).await;
    let created = events(&pool, "resource_created", r, None).await[0];
    assert_closures(&pool, TITLE, r.uuid(), OPEN, "before the scrub").await;

    scrub(&pool, &w, r, "title", None, true).await;

    let title: String = sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(r.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        title,
        format!("scrubbed-{}", r.uuid()),
        "the placeholder title"
    );
    assert_closures(&pool, TITLE, r.uuid(), SENTINEL, "the cleared title").await;
    assert_closures(
        &pool,
        LEDGER,
        created,
        SENTINEL,
        "its event, prior once cleared",
    )
    .await;
}

// ── S7: each projection sentinel the act writes ─────────────────────────────────────────────

/// FAILS IF a folded row's renamed key or its value sentinel is not recognised: a live key cleared,
/// whose row is folded by the act's own unset and rewritten under `scrubbed-key-<handle>`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_folded_row_finding_closes_under_its_renamed_key(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "Payroll").await;
    let key = format!("ssn {SSN_B}");
    set(
        &pool,
        &w,
        r,
        &key,
        serde_json::json!(format!("ssn {SSN_A}")),
    )
    .await;
    sweep(&pool).await;
    let set_event = events(&pool, "property_set", r, Some(&key)).await[0];
    let row = row_of(&pool, set_event).await;
    let handle = handle_of(&pool, r, &key).await;
    assert_closures(&pool, KEY, row, OPEN, "the key, before").await;
    assert_closures(&pool, VALUE, row, OPEN, "the value, before").await;

    scrub(&pool, &w, r, "property", Some(handle), true).await;

    assert_eq!(
        row_state(&pool, row).await,
        (
            format!("scrubbed-key-{handle}"),
            serde_json::json!(format!("erased:{set_event}")),
            true
        ),
        "the fixture reached the act's folded row"
    );
    assert_closures(&pool, KEY, row, SENTINEL, "the renamed key").await;
    assert_closures(&pool, VALUE, row, SENTINEL, "the value").await;
    assert_closures(
        &pool,
        LEDGER,
        set_event,
        SENTINEL,
        "the ledger's key and value",
    )
    .await;
}

/// Review focus 2. FAILS IF only the string form of the value sentinel closes: under the kept key
/// `tags` the projector stores it as the one-element array.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_tags_value_closes_in_its_array_form(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "Payroll").await;
    set(
        &pool,
        &w,
        r,
        "tags",
        serde_json::json!(format!("ssn {SSN_A}")),
    )
    .await;
    sweep(&pool).await;
    set(&pool, &w, r, "tags", serde_json::json!("clean")).await;
    let first = events(&pool, "property_set", r, Some("tags")).await[0];
    let row = row_of(&pool, first).await;
    assert_closures(&pool, VALUE, row, OPEN, "before the scrub").await;

    scrub(
        &pool,
        &w,
        r,
        "property",
        Some(handle_of(&pool, r, "tags").await),
        false,
    )
    .await;

    assert_eq!(
        row_state(&pool, row).await,
        (
            "tags".to_string(),
            serde_json::json!([format!("erased:{first}")]),
            true
        ),
        "the fixture reached the array form"
    );
    assert_closures(&pool, VALUE, row, SENTINEL, "the folded tags value").await;
}

/// FAILS IF a facet mark the act rewrote does not close: its row holds the one-key object of its
/// `scrubbed-facet-<event>-<position>` inner key and `"erased"`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_scrubbed_facet_mark_closes(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "Payroll").await;
    facet(
        &pool,
        &w,
        r,
        serde_json::json!({"status": format!("ssn {SSN_A}")}),
    )
    .await;
    sweep(&pool).await;
    // The whole-facet fold, after which the mark is prior (S3, facets).
    set(&pool, &w, r, "facet", serde_json::json!({"phase": "one"})).await;
    let asserted = events(&pool, "property_asserted", r, Some("facet")).await[0];
    let row = row_of(&pool, asserted).await;
    assert_closures(&pool, VALUE, row, OPEN, "before the scrub").await;

    scrub(
        &pool,
        &w,
        r,
        "property",
        Some(handle_of(&pool, r, "facet").await),
        false,
    )
    .await;

    assert_eq!(
        row_state(&pool, row).await,
        (
            "facet".to_string(),
            serde_json::json!({format!("scrubbed-facet-{asserted}-1"): "erased"}),
            true
        ),
        "the fixture reached the act's mark row"
    );
    assert_closures(&pool, VALUE, row, SENTINEL, "the scrubbed mark").await;
}

// ── S9: the scrub's precondition at closure ─────────────────────────────────────────────────

/// FAILS IF a scrub row closes only while its subject is live. The erasure lists only paths whose
/// value it changes, and the scrubbed title already holds erasure's sentinel, so the scrub's row is
/// the path's only row after the erasure too.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_scrubbed_finding_stays_closed_when_its_subject_is_erased(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, &format!("Payroll for {SSN_A}")).await;
    retitle(&pool, &w, r, "Payroll").await;
    sweep(&pool).await;
    let created = events(&pool, "resource_created", r, None).await[0];
    scrub(&pool, &w, r, "title", None, false).await;

    sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(r.uuid())
        .bind(w.emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the erasure completes");

    let authorities: Vec<String> = sqlx::query_scalar(
        "SELECT authority FROM kb_event_field_redactions WHERE event_id = $1 AND path = 'title'",
    )
    .bind(created)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        authorities,
        vec!["scrub".to_string()],
        "the erasure found the path done and wrote no row of its own"
    );
    assert_closures(
        &pool,
        LEDGER,
        created,
        SENTINEL,
        "the prior title, after the erasure",
    )
    .await;
}

// ── S1, S2: the family flags ────────────────────────────────────────────────────────────────

type Flags = (String, Option<Uuid>, bool, bool, bool);

/// The flags through the door the listing calls.
async fn flags(pool: &PgPool, r: ResourceId) -> Vec<Flags> {
    sqlx::query_as(
        "SELECT field, family, flagged, current_flagged, covered \
           FROM resource_field_scrub_family_flags($1)",
    )
    .bind(r.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn listing(pool: &PgPool, r: ResourceId) -> Vec<(String, Option<Uuid>)> {
    sqlx::query_as("SELECT field, family FROM resource_field_scrub_families($1)")
        .bind(r.uuid())
        .fetch_all(pool)
        .await
        .unwrap()
}

/// `(flagged, current_flagged, covered)` of the row keyed (`field`, `family`).
fn flags_of(rows: &[Flags], field: &str, family: Option<Uuid>) -> (bool, bool, bool) {
    let row = rows
        .iter()
        .find(|(f, h, ..)| f == field && *h == family)
        .unwrap_or_else(|| panic!("a row for {field} {family:?}"));
    (row.2, row.3, row.4)
}

/// FAILS IF the rows drift from the listing's, if `flagged` misses a family whose leak is only in
/// its history, if `current_flagged` reads anything but the live place, if a closed finding still
/// flags, or if `covered` reads true before the sweep or after a write it has not read.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_family_flags_read_findings_live_places_and_coverage(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "Payroll").await;
    set(
        &pool,
        &w,
        r,
        "notes",
        serde_json::json!(format!("ssn {SSN_A}")),
    )
    .await;
    set(
        &pool,
        &w,
        r,
        "memo",
        serde_json::json!(format!("ssn {SSN_B}")),
    )
    .await;
    set(&pool, &w, r, "colour", serde_json::json!("blue")).await;
    let (notes, memo, colour) = (
        handle_of(&pool, r, "notes").await,
        handle_of(&pool, r, "memo").await,
        handle_of(&pool, r, "colour").await,
    );
    let doc_type = events(&pool, "resource_created", r, None).await[0];

    let before = flags(&pool, r).await;
    assert!(
        before.iter().all(|row| !row.2 && !row.3 && !row.4),
        "never swept: nothing flagged, nothing covered: {before:?}"
    );

    sweep(&pool).await;
    set(&pool, &w, r, "notes", serde_json::json!("clean")).await;
    sweep(&pool).await;

    let rows = flags(&pool, r).await;
    assert_eq!(
        rows.iter()
            .map(|(f, h, ..)| (f.clone(), *h))
            .collect::<Vec<_>>(),
        listing(&pool, r).await,
        "one row per listing row, in the listing's order"
    );
    assert_eq!(
        flags_of(&rows, "property", Some(notes)),
        (true, false, true),
        "notes: history only"
    );
    assert_eq!(
        flags_of(&rows, "property", Some(memo)),
        (true, true, true),
        "memo: live"
    );
    assert_eq!(
        flags_of(&rows, "property", Some(colour)),
        (false, false, true),
        "colour: clean"
    );
    assert_eq!(
        flags_of(&rows, "property", Some(doc_type)),
        (false, false, true),
        "doc_type"
    );
    assert_eq!(
        flags_of(&rows, "title", None),
        (false, false, true),
        "title"
    );
    assert_eq!(
        flags_of(&rows, "origin_uri", None),
        (false, false, true),
        "origin URI"
    );

    scrub(&pool, &w, r, "property", Some(notes), false).await;
    let rows = flags(&pool, r).await;
    assert_eq!(
        (
            flags_of(&rows, "property", Some(notes)).0,
            flags_of(&rows, "property", Some(memo)).0
        ),
        (false, true),
        "the scrubbed family's findings closed; the other's did not"
    );

    set(&pool, &w, r, "colour", serde_json::json!("red")).await;
    let rows = flags(&pool, r).await;
    assert!(
        rows.iter().all(|row| !row.4),
        "a write the sweep has not read: not covered: {rows:?}"
    );
}

/// FAILS IF the door stops rendering without the sweep's function, or answers anything but a
/// sweep that read nothing would: every listing row, unflagged and uncovered.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn without_the_sweeps_function_the_door_reads_nothing_flagged(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let w = world(&pool).await;
    let r = resource(&pool, &w, "Payroll").await;
    set(
        &pool,
        &w,
        r,
        "memo",
        serde_json::json!(format!("ssn {SSN_B}")),
    )
    .await;
    sweep(&pool).await;
    let memo = handle_of(&pool, r, "memo").await;
    assert_eq!(
        flags_of(&flags(&pool, r).await, "property", Some(memo)),
        (true, true, true),
        "with the function, the live leak is flagged"
    );

    sqlx::query("DROP FUNCTION sensitivity.field_scrub_family_flags(uuid)")
        .execute(&pool)
        .await
        .unwrap();

    let rows = flags(&pool, r).await;
    assert_eq!(
        rows.iter()
            .map(|(f, h, ..)| (f.clone(), *h))
            .collect::<Vec<_>>(),
        listing(&pool, r).await
    );
    assert!(
        rows.iter().all(|row| !row.2 && !row.3 && !row.4),
        "{rows:?}"
    );
}
