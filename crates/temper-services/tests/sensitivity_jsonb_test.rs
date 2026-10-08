#![cfg(feature = "test-db")]
//! The sensitivity sweep over its jsonb surfaces, per-path ledger remediability and derived closure
//! (build order 3a, PR C2).
//!
//! Under goal *"Personal data that lands in the corpus by accident is found"*, sensitivity-sweep spec
//! D2, D3 and rulings Q26, Q34 and Q37-Q43. Spec witnesses:
//!   * **9** — closure derives from emptied content and from a mutable place's later observation, and
//!     not from a hash disappearing. The principal-erasure clause is the act's footprint planted by
//!     hand (content emptied, hash kept), not the act. Its resource-erasure clause is witness 26's
//!     sentinels: `erased_at` alone closes nothing (D2 as amended).
//!   * **25** — the `blocked:cut-2` half, gated on the erasure trail scope (Q41). Its `remediable`
//!     half needs erasure cut 2 (Q39).
//!   * **26** — both halves, through the real write path: after `resource_erasure_execute` the title
//!     and property findings close as sentinels; after `block_history_scrub_execute` (erasure 2e) a
//!     finding confined to a prior revision closes and the current revision's stays open.
//!
//! Also the walk's guards: user-map keys are written `?`, including a resource's `anchored-at`; a row
//! is scanned whole, or passed whole when oversize (Q40); jsonb units are never memoised (Q43); an
//! event type off the tally's shape cannot stop the ledger; payment_card v2 ignores hex runs (Q42);
//! `event_resource` agrees with the erasure trail scope; the interim remediability table is the live
//! erasure survey. Ledger rows are planted with raw inserts: these are about what the scan reads.

use sqlx::{PgPool, Row};
use temper_core::types::ids::{EntityId, ProfileId};
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, SeedAction};
use temper_substrate::ids::ResourceId;
use temper_substrate::payloads::{AnchorRef, EdgePolarity, Incorporation, ProvenanceSource};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

const SALT: &[u8] = b"witness-salt-of-sixteen-plus";
const SSN_A: &str = "219-45-6789";
const SSN_B: &str = "536-22-8147";
const NO_LAG: &str = "0 seconds";

#[derive(Debug, Clone, Copy)]
struct Tick {
    rows_examined: i32,
    failed: bool,
}

async fn tick_budget(pool: &PgPool, surface: &str, budget: i32) -> Tick {
    sqlx::query("SELECT workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep', $1)")
        .bind(serde_json::json!({ "surface": surface, "budget": budget }))
        .execute(pool)
        .await
        .unwrap();
    let (run, job): (Uuid, Uuid) =
        sqlx::query_as("SELECT run_id, job_id FROM sensitivity_sweep_claim()")
            .fetch_one(pool)
            .await
            .unwrap();
    let row = sqlx::query(
        "SELECT rows_examined, failed \
           FROM sensitivity_sweep_tick($1, $2, $3, $4::interval)",
    )
    .bind(run)
    .bind(job)
    .bind(SALT)
    .bind(NO_LAG)
    .fetch_one(pool)
    .await
    .unwrap();
    Tick {
        rows_examined: row.get(0),
        failed: row.get(1),
    }
}

async fn tick(pool: &PgPool, surface: &str) -> Tick {
    tick_budget(pool, surface, 1000).await
}

/// One ledger row of `event_type`, carrying `payload` and `metadata` as given.
async fn event(
    pool: &PgPool,
    event_type: &str,
    payload: serde_json::Value,
    metadata: serde_json::Value,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, category, payload, metadata)
         SELECT t.id, (SELECT id FROM kb_entities ORDER BY id LIMIT 1), t.category, $2, $3
           FROM kb_event_types t WHERE t.name = $1
         RETURNING id",
    )
    .bind(event_type)
    .bind(payload)
    .bind(metadata)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// `(path, detector_id)` of every finding at `target`, in path order.
async fn findings_at(pool: &PgPool, target: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT path, detector_id FROM sensitivity.findings WHERE target_id = $1 ORDER BY path, detector_id",
    )
    .bind(target)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn remediability_at(pool: &PgPool, target: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT f.path, r.remediability FROM sensitivity.ledger_finding_remediability r \
           JOIN sensitivity.findings f ON f.id = r.finding_id WHERE f.target_id = $1 ORDER BY f.path",
    )
    .bind(target)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn closed_by(pool: &PgPool, target: Uuid) -> Vec<Option<String>> {
    sqlx::query_scalar(
        "SELECT c.closed_by FROM sensitivity.findings f \
           LEFT JOIN sensitivity.finding_closure c ON c.finding_id = f.id \
          WHERE f.target_id = $1 ORDER BY f.path NULLS FIRST, f.detector_id",
    )
    .bind(target)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Every row of every table in the `sensitivity` schema, and every sensitivity job, as text.
async fn everything_the_sweep_wrote(pool: &PgPool) -> String {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
          WHERE table_schema = 'sensitivity' AND table_type = 'BASE TABLE' \
            AND table_name NOT IN ('detectors', 'detector_versions')",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut all = String::new();
    for t in tables {
        let rows: Vec<String> =
            sqlx::query_scalar(&format!("SELECT t::text FROM sensitivity.{t} t"))
                .fetch_all(pool)
                .await
                .unwrap();
        all.push_str(&rows.join("\n"));
    }
    let jobs: Vec<String> =
        sqlx::query_scalar("SELECT j::text FROM kb_workflow_jobs j WHERE persona = 'sensitivity'")
            .fetch_all(pool)
            .await
            .unwrap();
    all.push_str(&jobs.join("\n"));
    all
}

fn assert_holds_none_of(haystack: &str, planted: &str, what: &str) {
    let digits: String = planted.chars().filter(char::is_ascii_digit).collect();
    for needle in [planted, digits.as_str(), &planted[..6], &planted[4..]] {
        assert!(
            !haystack.contains(needle),
            "{what} holds `{needle}`, a piece of the planted value"
        );
    }
}

// ── Q26: a jsonb finding is a structural path, and a user's keys are never written ────────────

/// The operator's act on a deployment that opts in (Q52, Q53): every seeded detector is off until
/// someone turns it on, and these witnesses are about what an enabled detector does.
async fn enable_seeded_detectors(pool: &PgPool) {
    sqlx::query("SELECT sensitivity.enable_detectors(1, 'temper')")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_user_map_key_is_written_as_a_question_mark_and_still_scanned(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let e = event(
        &pool,
        "property_set",
        serde_json::json!({
            "property_key": "notes",
            "value": { "jane_doe": format!("ssn {SSN_A}"), format!("ref {SSN_B}"): "x" },
        }),
        serde_json::json!({}),
    )
    .await;

    let t = tick(&pool, "kb_events.payload").await;

    assert!(!t.failed, "{t:?}");
    assert_eq!(
        findings_at(&pool, e).await,
        vec![
            ("/value/?".to_string(), "us_ssn_delimited".to_string()),
            ("/value/?".to_string(), "us_ssn_delimited".to_string()),
        ],
        "a value under a user key, and a user key itself, are found at `?` and never by name"
    );
    let paths: String = sqlx::query_scalar(
        "SELECT string_agg(path, ',') FROM sensitivity.findings WHERE target_id = $1",
    )
    .bind(e)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !paths.contains("jane_doe"),
        "a letters-only user key is still a user's key"
    );
    let store = everything_the_sweep_wrote(&pool).await;
    assert_holds_none_of(&store, SSN_A, "the sensitivity store");
    assert_holds_none_of(&store, SSN_B, "the sensitivity store");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_webhook_document_is_scanned_at_hidden_paths_and_tallied(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let e = event(
        &pool,
        "webhook_received",
        serde_json::json!({
            "issue": { "body": format!("my ssn is {SSN_A}"), "number": 7 },
            "resource_id": Uuid::now_v7().to_string(),
        }),
        serde_json::json!({ "provider_event_type": "issues", "provider_event_type_source": "header" }),
    )
    .await;

    tick(&pool, "kb_events.payload").await;

    assert_eq!(
        findings_at(&pool, e).await,
        vec![("/?/?".to_string(), "us_ssn_delimited".to_string())],
        "the provider's keys are its own, so none is written (Q37)"
    );
    let resource: Option<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM sensitivity.findings WHERE target_id = $1")
            .bind(e)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        resource, None,
        "a webhook belongs to no resource, whatever its provider calls a resource_id"
    );
    let tally: serde_json::Value = sqlx::query_scalar(
        "SELECT units_by_event_type -> 'webhook_received' FROM sensitivity.runs WHERE surface = 'kb_events.payload' \
          ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        tally,
        serde_json::json!(7),
        "each run counts the units each event type contributed: three leaves and four keys"
    );
}

/// The shape intake writes from payload_version 2: the remote's body under temper's `body` key.
/// It is scanned one level deeper, keeps every key hidden, and still belongs to no resource.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_wrapped_webhook_document_is_scanned_and_belongs_to_no_resource(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let e = event(
        &pool,
        "webhook_received",
        serde_json::json!({ "body": {
            "issue": { "body": format!("my ssn is {SSN_A}"), "number": 7 },
            "resource_id": Uuid::now_v7().to_string(),
        } }),
        serde_json::json!({ "provider_event_type": "issues", "provider_event_type_source": "header" }),
    )
    .await;

    tick(&pool, "kb_events.payload").await;

    assert_eq!(
        findings_at(&pool, e).await,
        vec![("/?/?/?".to_string(), "us_ssn_delimited".to_string())],
        "found under the wrap, with no key written"
    );
    let resource: Option<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM sensitivity.findings WHERE target_id = $1")
            .bind(e)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        resource, None,
        "a wrapped webhook belongs to no resource either"
    );
}

// ── The C1 design review's trap: a row is never scanned in half ───────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_jsonb_row_is_scanned_whole_under_a_one_row_budget(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    // History first, so the one-row budget is spent by the head on this event alone.
    let e = event(
        &pool,
        "resource_created",
        serde_json::json!({ "title": format!("about {SSN_A}"), "origin_uri": format!("https://hr.example/{SSN_B}") }),
        serde_json::json!({}),
    )
    .await;
    tick(&pool, "kb_events.payload").await;
    let later = event(
        &pool,
        "resource_created",
        serde_json::json!({ "title": format!("about {SSN_A}"), "origin_uri": format!("https://hr.example/{SSN_B}") }),
        serde_json::json!({}),
    )
    .await;

    let t = tick_budget(&pool, "kb_events.payload", 1).await;

    assert_eq!(t.rows_examined, 1, "{t:?}");
    assert_eq!(
        findings_at(&pool, later).await.len(),
        2,
        "both of the row's units are scanned before its id passes under the watermark"
    );
    assert_eq!(findings_at(&pool, e).await.len(), 2);
}

// ── Q43: a payload's ids and hashes are scanned, and never remembered ─────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_jsonb_unit_is_never_memoised(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    // Every detector finds nothing here, and payment_card's `[0-9]{4}` prefilter nominates both.
    let hash = "4c1f9e0a2b7d3e5f8a6c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a";
    let id = "01a10292-201d-7645-86c8-5d5fd98a09a2";
    event(
        &pool,
        "resource_created",
        serde_json::json!({ "resource_id": id, "blocks": [{ "chunks": [{ "content_hash": hash }] }] }),
        serde_json::json!({}),
    )
    .await;

    tick(&pool, "kb_events.payload").await;

    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sensitivity.memo \
          WHERE content_hash IN (sensitivity.keyed_hash($1, $2), sensitivity.keyed_hash($1, $3))",
    )
    .bind(SALT)
    .bind(hash)
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "a jsonb unit leaves no memo row (Q43)");
}

// ── Q40: a row is bounded by its size, and passed whole when it is too big ──────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_oversize_row_is_passed_whole_and_named_and_the_next_row_is_scanned(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let mut body = serde_json::Map::new();
    for i in 0..5_000 {
        body.insert(format!("k{i}"), serde_json::json!(format!("v{i}")));
    }
    body.insert("x".into(), serde_json::json!(format!("ssn {SSN_A}")));
    let big = event(
        &pool,
        "webhook_received",
        serde_json::Value::Object(body),
        serde_json::json!({}),
    )
    .await;
    let small = event(
        &pool,
        "citation_audited",
        serde_json::json!({ "reason": format!("ssn {SSN_B}") }),
        serde_json::json!({}),
    )
    .await;
    tick(&pool, "kb_events.payload").await;

    assert!(
        findings_at(&pool, big).await.is_empty(),
        "a row of 10,002 units is not scanned in part"
    );
    let named: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM sensitivity.unscanned_places WHERE surface = 'kb_events.payload' AND target_id = $1",
    )
    .bind(big)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        named.as_deref(),
        Some("oversize_row"),
        "and never reads as clean"
    );
    assert_eq!(
        findings_at(&pool, small).await.len(),
        1,
        "the sweep moves past it"
    );
}

// ── A registered name off the tally's shape cannot stop the ledger ──────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_event_type_off_the_tally_shape_is_counted_not_fatal(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
         VALUES ('Oauth2.Linked', NULL, 1, 'domain')",
    )
    .execute(&pool)
    .await
    .unwrap();
    event(
        &pool,
        "Oauth2.Linked",
        serde_json::json!({ "note": "x" }),
        serde_json::json!({}),
    )
    .await;
    let later = event(
        &pool,
        "citation_audited",
        serde_json::json!({ "reason": format!("ssn {SSN_A}") }),
        serde_json::json!({}),
    )
    .await;

    let t = tick(&pool, "kb_events.payload").await;

    assert!(!t.failed, "{t:?}");
    assert_eq!(findings_at(&pool, later).await.len(), 1);
    let tally: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT units_by_event_type -> 'unrecognised_kind' FROM sensitivity.runs \
          WHERE surface = 'kb_events.payload' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tally, Some(serde_json::json!(1)));
}

// ── Q42: a card is never read out of a hash ───────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_hex_hash_v1_read_as_a_card_is_not_one(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let hash: String = sqlx::query_scalar(
        "SELECT x FROM (SELECT encode(sha256(g::text::bytea), 'hex') x FROM generate_series(1, 5000) g) h \
          WHERE EXISTS (SELECT 1 FROM sensitivity.detector_matches('payment_card', 1, x)) LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("v1 matched some hash");
    let e = event(
        &pool,
        "resource_created",
        serde_json::json!({ "blocks": [{ "chunks": [{ "content_hash": hash }] }], "title": "card 4111 1111 1111 1111" }),
        serde_json::json!({}),
    )
    .await;

    tick(&pool, "kb_events.payload").await;

    assert_eq!(
        findings_at(&pool, e).await,
        vec![("/title".to_string(), "payment_card".to_string())],
        "the card in prose is found; the hash is not a card"
    );
}

// ── Q26: only an edge's `anchored-at` is schema; a resource's is its author's document ─────────

/// One raw property row; its event columns point at any ledger row.
async fn property(
    pool: &PgPool,
    owner_table: &str,
    owner: Uuid,
    key: &str,
    value: serde_json::Value,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, \
                                    asserted_by_event_id, last_event_id) \
         SELECT $1, $2, $3, $4, e.id, e.id FROM (SELECT id FROM kb_events ORDER BY id LIMIT 1) e \
         RETURNING id",
    )
    .bind(owner_table)
    .bind(owner)
    .bind(key)
    .bind(value)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_resources_anchored_at_value_keeps_its_keys_hidden(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let doc = serde_json::json!({ "jane_doe": { "spouse": format!("ssn {SSN_A}") } });
    let on_resource = property(
        &pool,
        "kb_resources",
        Uuid::now_v7(),
        "anchored-at",
        doc.clone(),
    )
    .await;
    let on_edge = property(&pool, "kb_edges", Uuid::now_v7(), "anchored-at", doc).await;

    tick(&pool, "kb_properties.property_value").await;

    assert_eq!(
        findings_at(&pool, on_resource).await,
        vec![("/?/?".to_string(), "us_ssn_delimited".to_string())],
        "open_meta lets anyone write `anchored-at` on a resource; its keys are theirs"
    );
    assert_eq!(
        findings_at(&pool, on_edge).await,
        vec![(
            "/jane_doe/spouse".to_string(),
            "us_ssn_delimited".to_string()
        )],
        "an edge's `anchored-at` is the validated shape, written as schema"
    );
}

// ── Q41: the resource an event is attributed to is the one whose erasure trail holds it ─────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn event_resource_agrees_with_the_erasure_trail_scope(pool: PgPool) {
    let r = bare_resource(&pool, "one").await;
    let r2 = bare_resource(&pool, "two").await;
    let rev = block(&pool, r, "prose").await;
    let b: Uuid = sqlx::query_scalar("SELECT block_id FROM kb_block_revisions WHERE id = $1")
        .bind(rev)
        .fetch_one(&pool)
        .await
        .unwrap();
    let edge: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, \
                               home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         SELECT 'kb_resources', $1, 'kb_resources', $2, 'leads_to', 'kb_contexts', gen_random_uuid(), e.id, e.id \
           FROM (SELECT id FROM kb_events ORDER BY id LIMIT 1) e RETURNING id",
    )
    .bind(r)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .unwrap();
    let planted = [
        ("resource_created", serde_json::json!({ "resource_id": r })),
        (
            "property_set",
            serde_json::json!({ "owner": { "table": "kb_resources", "id": r } }),
        ),
        ("citation_audited", serde_json::json!({ "block_id": b })),
        (
            "relationship_asserted",
            serde_json::json!({ "edge_id": edge }),
        ),
        (
            "property_set",
            serde_json::json!({ "owner": { "table": "kb_edges", "id": edge } }),
        ),
        (
            "property_set",
            serde_json::json!({ "owner": { "table": "kb_content_blocks", "id": b } }),
        ),
        ("context_renamed", serde_json::json!({ "to_name": "x" })),
    ];
    for (kind, payload) in planted {
        let e = event(&pool, kind, payload.clone(), serde_json::json!({})).await;
        let (attributed, in_trail, in_own_trail): (Option<Uuid>, bool, bool) = sqlx::query_as(
            "SELECT s.resource_id, \
                    EXISTS (SELECT 1 FROM unnest($2::uuid[]) r, _resource_erasure_trail_scope(r) t WHERE t.event_id = $1), \
                    EXISTS (SELECT 1 FROM _resource_erasure_trail_scope(s.resource_id) t WHERE t.event_id = $1) \
               FROM sensitivity.src_kb_events__payload s WHERE s.target_id = $1",
        )
        .bind(e)
        .bind(vec![r, r2])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            attributed.is_some(),
            in_trail,
            "{kind} {payload}: attributed iff in some trail"
        );
        assert_eq!(
            attributed.is_some(),
            in_own_trail,
            "{kind} {payload}: in the attributed resource's trail"
        );
    }
}

// ── The registry holds what the scan needs ──────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_enabled_surface_has_a_source(pool: PgPool) {
    let missing: Vec<String> = sqlx::query_scalar(
        "SELECT surface FROM sensitivity.surfaces \
          WHERE enabled AND to_regclass(format('sensitivity.%I', 'src_' || replace(surface, '.', '__'))) IS NULL",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        missing.is_empty(),
        "an enabled surface with no source idles silently: {missing:?}"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_jsonb_surface_cannot_take_a_mutable_cursor(pool: PgPool) {
    let err = sqlx::query(
        "UPDATE sensitivity.surfaces SET cursor_kind = 'mutable_timestamp' WHERE surface = 'kb_profiles.preferences'",
    )
    .execute(&pool)
    .await
    .expect_err("the jsonb scan keeps no per-place observation");
    assert!(
        err.to_string().contains("surfaces_jsonb_is_append_only"),
        "{err}"
    );
}

// ── Witness 25: remediability is per (event_type, path) ───────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn remediability_is_read_per_event_type_and_path(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let r = Uuid::now_v7().to_string();
    let created = event(
        &pool,
        "resource_created",
        serde_json::json!({ "resource_id": r, "title": format!("Payroll for {SSN_A}"), "doc_type": format!("t {SSN_B}") }),
        serde_json::json!({ "reasoning": format!("saw {SSN_A}"), "persona": format!("p {SSN_B}") }),
    )
    .await;
    let renamed = event(
        &pool,
        "context_renamed",
        serde_json::json!({ "to_name": format!("Team {SSN_A}") }),
        serde_json::json!({}),
    )
    .await;
    let set = event(
        &pool,
        "property_set",
        serde_json::json!({ "owner": { "table": "kb_resources", "id": r }, "property_key": "notes", "value": { "k": SSN_B } }),
        serde_json::json!({}),
    )
    .await;
    let on_block = event(
        &pool,
        "property_set",
        serde_json::json!({ "owner": { "table": "kb_content_blocks", "id": Uuid::now_v7() }, "property_key": "notes", "value": SSN_A }),
        serde_json::json!({}),
    )
    .await;

    tick(&pool, "kb_events.payload").await;
    tick(&pool, "kb_events.metadata").await;

    let blocked = "blocked:cut-2".to_string();
    let never = "unremediable".to_string();
    assert_eq!(
        remediability_at(&pool, created).await,
        vec![
            ("/doc_type".to_string(), never.clone()),
            ("/persona".to_string(), never.clone()),
            ("/reasoning".to_string(), blocked.clone()),
            ("/title".to_string(), blocked.clone()),
        ],
        "a ledger_remainder path waits on cut 2; one outside it never has a remedy"
    );
    assert_eq!(
        remediability_at(&pool, renamed).await,
        vec![("/to_name".to_string(), never.clone())],
        "a context's name is permanently outside the redaction (D3)"
    );
    assert_eq!(
        remediability_at(&pool, set).await,
        vec![("/value/?".to_string(), blocked)],
        "a listed path covers its whole subtree"
    );
    assert_eq!(
        remediability_at(&pool, on_block).await,
        vec![("/value".to_string(), never)],
        "a listed path on an event in no resource's trail has no remedy either (Q41)"
    );
}

/// The interim table is the live `ledger_remainder` CASE, arm for arm. When erasure changes the
/// CASE, this fails until the table follows.
///
/// The plan's other arm, the `telos_centroid` copies on a goal's home context's
/// `region_materialized` / `salience_refreshed` events (a `UNION ALL`, not a CASE arm), is outside
/// this table on purpose: those events sit in no resource's erasure trail, so under Q41 they read
/// `unremediable` whether listed or not.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_interim_remediability_table_is_the_live_erasure_survey(pool: PgPool) {
    let live: std::collections::BTreeSet<(Option<String>, String)> = sqlx::query_as(
        r#"WITH def AS (SELECT pg_get_functiondef('resource_erasure_survey_plan'::regproc) AS d)
           SELECT m[1], jsonb_array_elements_text(m[2]::jsonb)
             FROM def, regexp_matches(d, $$WHEN '([a-z_]+)'\s+THEN '(\[[^']*\])'::jsonb$$, 'g') m
           UNION
           SELECT NULL, jsonb_array_elements_text(m[1]::jsonb)
             FROM def, regexp_matches(d, $$THEN '(\["metadata\.[^']*\])'::jsonb$$, 'g') m"#,
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let table: std::collections::BTreeSet<(Option<String>, String)> =
        sqlx::query_as("SELECT event_type, erasure_path FROM sensitivity.ledger_redact_paths")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert!(live.len() > 20, "the parse found the CASE: {live:?}");
    assert_eq!(table, live);
}

// ── Witness 9: closure derives, and never from a hash disappearing ────────────────────────────

/// One block on `resource` holding `content`; returns the revision id.
async fn block(pool: &PgPool, resource: Uuid, content: &str) -> Uuid {
    sqlx::query_scalar(
        "WITH ev AS (
             INSERT INTO kb_events (event_type_id, emitter_entity_id, category)
             SELECT t.id, (SELECT id FROM kb_entities ORDER BY id LIMIT 1), t.category
               FROM kb_event_types t WHERE t.name = 'resource_updated' RETURNING id),
         b AS (
             INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
             SELECT $1, (SELECT count(*) FROM kb_content_blocks WHERE resource_id = $1), ev.id, ev.id
               FROM ev RETURNING id),
         rev AS (
             INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count)
             SELECT id, 'witness', 0 FROM b RETURNING id)
         INSERT INTO kb_block_content (block_revision_id, content, content_hash)
         SELECT id, $2, encode(sha256(convert_to($2, 'UTF8')), 'hex') FROM rev
         RETURNING block_revision_id",
    )
    .bind(resource)
    .bind(content)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn bare_resource(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, 'test://witness') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn closure_reads_emptied_content_and_a_changed_title_never_a_missing_hash(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let r = bare_resource(&pool, &format!("Payroll for {SSN_A}")).await;
    let emptied = block(&pool, r, &format!("SSN {SSN_A}")).await;
    let kept = block(&pool, r, &format!("SSN {SSN_B}")).await;
    tick(&pool, "kb_block_content.content").await;
    tick(&pool, "kb_resources.title").await;
    assert_eq!(closed_by(&pool, emptied).await, vec![None]);
    assert_eq!(closed_by(&pool, r).await, vec![None]);

    // Principal erasure's footprint on content (20260911000000:69, :143): the content emptied, its
    // hash kept in the row and recorded in kb_erased_content.
    sqlx::query("UPDATE kb_block_content SET content = '' WHERE block_revision_id = $1")
        .bind(emptied)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE kb_resources SET title = 'Payroll', updated = clock_timestamp() WHERE id = $1",
    )
    .bind(r)
    .execute(&pool)
    .await
    .unwrap();
    tick(&pool, "kb_resources.title").await;

    let hash_kept: bool = sqlx::query_scalar(
        "SELECT content_hash <> '' FROM kb_block_content WHERE block_revision_id = $1",
    )
    .bind(emptied)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        hash_kept,
        "the source keeps its hash, so a hash-absence rule would not close"
    );
    assert_eq!(
        closed_by(&pool, emptied).await,
        vec![Some("content_empty".to_string())]
    );
    assert_eq!(
        closed_by(&pool, r).await,
        vec![Some("changed".to_string())],
        "the sweep's later observation of the title closes it"
    );
    assert_eq!(
        closed_by(&pool, kept).await,
        vec![None],
        "a live place stays open"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_enabled_surface_has_a_closure_rule(pool: PgPool) {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT surface, sensitivity.place_closure(surface, gen_random_uuid(), repeat('0', 64)) \
           FROM sensitivity.surfaces WHERE enabled ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (surface, closed) in rows {
        let expected = if surface.starts_with("kb_events.") {
            None
        } else {
            Some("row_missing".to_string())
        };
        assert_eq!(
            closed, expected,
            "{surface}: a missing place closes, and the ledger never closes here (Q38)"
        );
    }
}

// ── Witness 26: sentinel closure, through the real act ────────────────────────────────────────

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
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

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_resource_erasure_closes_its_title_and_property_findings_and_not_its_ledger(
    pool: PgPool,
) {
    enable_seeded_detectors(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'sweep-home', 'sweep-home') RETURNING id",
    )
    .bind(owner.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let title = format!("Payroll for {SSN_A}");
    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: &title,
            origin_uri: "test://sweep",
            body: "clean prose",
            doc_type: "research",
            home: AnchorRef::context(temper_core::types::ids::ContextId::from(home)),
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
    .expect("create through the write path");
    let key = format!("ssn {SSN_B}");
    writes::set_property(
        &pool,
        resource,
        &key,
        &serde_json::json!(format!("ssn {SSN_A}")),
        emitter,
    )
    .await
    .unwrap();
    let block: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1 LIMIT 1")
            .bind(resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let on_block = property(
        &pool,
        "kb_content_blocks",
        block,
        "block_note",
        serde_json::json!(format!("ssn {SSN_A}")),
    )
    .await;
    let property: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_properties WHERE owner_id = $1 AND property_key = $2",
    )
    .bind(resource.uuid())
    .bind(&key)
    .fetch_one(&pool)
    .await
    .unwrap();
    for surface in [
        "kb_resources.title",
        "kb_properties.property_key",
        "kb_properties.property_value",
        "kb_events.payload",
    ] {
        assert!(!tick(&pool, surface).await.failed, "{surface}");
    }
    let created: Uuid = sqlx::query_scalar(
        "SELECT f.target_id FROM sensitivity.findings f WHERE f.surface = 'kb_events.payload' \
            AND f.path = '/title' AND f.resource_id = $1",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .expect("the ledger's copy of the title is found");

    sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(resource.uuid())
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the act completes");

    assert_eq!(
        closed_by(&pool, resource.uuid()).await,
        vec![Some("sentinel".to_string())],
        "the husk's title is a sentinel, though the surface is not empty"
    );
    assert_eq!(
        closed_by(&pool, property).await,
        vec![Some("sentinel".to_string()), Some("sentinel".to_string())],
        "the property's key and value are sentinels"
    );
    assert_eq!(
        closed_by(&pool, on_block).await,
        vec![Some("sentinel".to_string())],
        "a property owned by one of the resource's blocks is a sentinel too (step 9d′)"
    );
    let ledger: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT c.closed_by FROM sensitivity.findings f \
           LEFT JOIN sensitivity.finding_closure c ON c.finding_id = f.id \
          WHERE f.target_id = $1 AND f.path = '/title'",
    )
    .bind(created)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        ledger,
        vec![None],
        "cut 1 leaves the title in the trail, so its finding stays open (Q38, a guard on the view)"
    );
}

// ── Witness 26, the scrub half: a finding confined to a prior revision closes ───────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_block_history_scrub_closes_the_prior_revisions_finding_and_not_the_current(
    pool: PgPool,
) {
    enable_seeded_detectors(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'scrub-home', 'scrub-home') RETURNING id",
    )
    .bind(owner.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let first = format!("the first draft quoted {SSN_A}");
    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "notes",
            origin_uri: "test://scrub",
            body: &first,
            doc_type: "research",
            home: AnchorRef::context(temper_core::types::ids::ContextId::from(home)),
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
    .expect("create through the write path");
    let block: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1 LIMIT 1")
            .bind(resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let second = format!("the revision still quotes {SSN_B}");
    writes::update_resource(
        &pool,
        writes::UpdateParams {
            resource,
            body: Some(&second),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: None,
            sources: vec![],
            content_block: Some(block),
            rehome_to: None,
            emitter,
        },
    )
    .await
    .expect("a per-block revise");
    let (prior, current): (Uuid, Uuid) = sqlx::query_as(
        "SELECT (SELECT br.id FROM kb_block_revisions br WHERE br.block_id = b.id AND br.id <> b.current_revision_id), \
                b.current_revision_id \
           FROM kb_content_blocks b WHERE b.id = $1",
    )
    .bind(block)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!tick(&pool, "kb_block_content.content").await.failed);
    assert_eq!(closed_by(&pool, prior).await, vec![None]);
    assert_eq!(closed_by(&pool, current).await, vec![None]);

    sqlx::query("SELECT block_history_scrub_execute($1, $2, $3, $4, $5)")
        .bind(resource.uuid())
        .bind(vec![block])
        .bind(owner.uuid())
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the scrub completes");

    assert_eq!(
        closed_by(&pool, prior).await,
        vec![Some("content_empty".to_string())],
        "the scrub empties the prior revision, so its finding closes"
    );
    assert_eq!(
        closed_by(&pool, current).await,
        vec![None],
        "the current revision is kept, and so is its finding"
    );
}

// ── Places the erasure act reached give up their digests after 30 days (D11, ruled 2026-10-03) ─

/// `(fingerprint_state, content_hash, fingerprints held)` for each finding on `target`'s `surface`.
async fn digests_at(pool: &PgPool, surface: &str, target: Uuid) -> Vec<(String, String, i64)> {
    sqlx::query_as(
        "SELECT f.fingerprint_state, f.content_hash, \
                (SELECT count(*) FROM sensitivity.finding_fingerprints ff WHERE ff.finding_id = f.id) \
           FROM sensitivity.findings f WHERE f.surface = $1 AND f.target_id = $2 \
          ORDER BY f.path NULLS FIRST, f.detector_id",
    )
    .bind(surface)
    .bind(target)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Every recorded window started `days` ago.
async fn windows_started_days_ago(pool: &PgPool, days: i32) {
    sqlx::query(
        "UPDATE sensitivity.erased_closures SET closed_seen_at = now() - make_interval(days => $1)",
    )
    .bind(days)
    .execute(pool)
    .await
    .unwrap();
}

async fn tick_all(pool: &PgPool) {
    for surface in [
        "kb_resources.title",
        "kb_events.payload",
        "kb_edges.label",
        "kb_properties.property_value",
        "kb_remote_sources.uri",
    ] {
        assert!(!tick(pool, surface).await.failed, "{surface}");
    }
}

async fn created_resource(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: Uuid,
    title: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: title,
            body: "clean prose",
            doc_type: "research",
            home: AnchorRef::context(temper_core::types::ids::ContextId::from(home)),
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
    .expect("create through the write path")
}

/// A closed finding on any place the act reached gives up its fingerprints and its keyed hash 30
/// days after the sweep first sees it so; the row stays, closed. The places include those the scan
/// does not attribute to R: the label of an edge INTO R (attributed to its source), a property on
/// that edge (attributed to nothing), and R's remote source, which the act deletes. Each conjunct
/// has a case that keeps its digests: a closed finding on a resource never erased, the erased
/// places at 29 days, and the ledger's copy of R's title, which stays open until cut 2.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_place_the_act_reached_gives_up_its_digests_after_thirty_days(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'expiry-home', 'expiry-home') RETURNING id",
    )
    .bind(owner.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let title = format!("Payroll for {SSN_A}");
    let erased = created_resource(&pool, owner, emitter, home, &title).await;
    let neighbour = created_resource(&pool, owner, emitter, home, "Neighbour").await;
    // An edge from a live resource INTO the one erased: the scan attributes its label to the source.
    let mut tx = pool.begin().await.unwrap();
    let edge = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(neighbour),
            tgt: AnchorRef::resource(erased),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(&format!("met {SSN_A}")),
            weight: 1.0,
            home: EdgeHome::Context(temper_core::types::ids::ContextId::from(home)),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap()
    .uuid();
    tx.commit().await.unwrap();
    let on_edge = property(
        &pool,
        "kb_edges",
        edge,
        "note",
        serde_json::json!(format!("ssn {SSN_B}")),
    )
    .await;
    let url = format!("https://example.com/case/{SSN_A}");
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: erased,
            sources: vec![Incorporation {
                source: ProvenanceSource::Remote(url.clone()),
                seq: 0,
            }],
            content_block: None,
            emitter,
        },
    )
    .await
    .expect("the erased resource cites the URL");
    let remote: Uuid = sqlx::query_scalar("SELECT id FROM kb_remote_sources WHERE uri = $1")
        .bind(&url)
        .fetch_one(&pool)
        .await
        .unwrap();
    // Closed by a later observation, never erased.
    let live = bare_resource(&pool, &format!("Payroll for {SSN_B}")).await;
    // An edge between two live resources: its label and its property close below without any
    // erasure, so the erased-end condition of the edge arms is the only thing that keeps them.
    let other = created_resource(&pool, owner, emitter, home, "Other").await;
    let mut tx = pool.begin().await.unwrap();
    let live_edge = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(neighbour),
            tgt: AnchorRef::resource(other),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(&format!("met {SSN_B}")),
            weight: 1.0,
            home: EdgeHome::Context(temper_core::types::ids::ContextId::from(home)),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap()
    .uuid();
    tx.commit().await.unwrap();
    let on_live_edge = property(
        &pool,
        "kb_edges",
        live_edge,
        "note",
        serde_json::json!(format!("ssn {SSN_B}")),
    )
    .await;
    tick_all(&pool).await;
    let ledger: Uuid = sqlx::query_scalar(
        "SELECT f.target_id FROM sensitivity.findings f WHERE f.surface = 'kb_events.payload' \
            AND f.path = '/title' AND f.resource_id = $1",
    )
    .bind(erased.uuid())
    .fetch_one(&pool)
    .await
    .expect("the ledger's copy of the title is found");

    let places = [
        ("kb_resources.title", erased.uuid()),
        ("kb_edges.label", edge),
        ("kb_properties.property_value", on_edge),
        ("kb_remote_sources.uri", remote),
    ];
    let mut before = Vec::new();
    for (surface, target) in places {
        let d = digests_at(&pool, surface, target).await;
        assert!(
            !d.is_empty() && d.iter().all(|(s, _, n)| s == "complete" && *n > 0),
            "{surface} is found and fingerprinted before the act: {d:?}"
        );
        before.push(d);
    }
    let ledger_before = digests_at(&pool, "kb_events.payload", ledger).await;
    let live_before = digests_at(&pool, "kb_resources.title", live).await;
    let live_edge_places = [
        ("kb_edges.label", live_edge),
        ("kb_properties.property_value", on_live_edge),
    ];
    let mut live_edge_before = Vec::new();
    for (surface, target) in live_edge_places {
        let d = digests_at(&pool, surface, target).await;
        assert!(
            !d.is_empty() && d.iter().all(|(s, _, n)| s == "complete" && *n > 0),
            "{surface} on the live edge is found and fingerprinted: {d:?}"
        );
        live_edge_before.push(d);
    }

    sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(erased.uuid())
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the act completes");
    sqlx::query(
        "UPDATE kb_resources SET title = 'Payroll', updated = clock_timestamp() WHERE id = $1",
    )
    .bind(live)
    .execute(&pool)
    .await
    .unwrap();
    // The live edge's label and property emptied by hand: closed as content_empty, never erased.
    sqlx::query("UPDATE kb_edges SET label = NULL WHERE id = $1")
        .bind(live_edge)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_properties SET property_value = '\"\"'::jsonb WHERE id = $1")
        .bind(on_live_edge)
        .execute(&pool)
        .await
        .unwrap();
    tick_all(&pool).await;
    for (_, target) in live_edge_places {
        assert_eq!(
            closed_by(&pool, target).await,
            vec![Some("content_empty".to_string())],
            "the live edge's place is closed"
        );
    }
    for (surface, target) in places {
        assert!(
            closed_by(&pool, target).await.iter().all(Option::is_some),
            "{surface} is closed by the act"
        );
    }
    assert!(
        closed_by(&pool, live).await.iter().all(Option::is_some),
        "the live title is closed by its change"
    );

    windows_started_days_ago(&pool, 29).await;
    tick_all(&pool).await;
    for ((surface, target), b) in places.iter().zip(&before) {
        assert_eq!(
            &digests_at(&pool, surface, *target).await,
            b,
            "at 29 days {surface} keeps its digests"
        );
    }

    windows_started_days_ago(&pool, 31).await;
    tick_all(&pool).await;
    for ((surface, target), b) in places.iter().zip(&before) {
        let after = digests_at(&pool, surface, *target).await;
        assert_eq!(after.len(), b.len(), "{surface}: the finding rows stay");
        for ((state, hash, n), (_, old_hash, _)) in after.iter().zip(b) {
            assert_eq!(state, "expired", "{surface}");
            assert_eq!(*n, 0, "{surface}: no fingerprint is left");
            assert_ne!(
                hash, old_hash,
                "{surface}: the keyed hash of the unit is gone"
            );
        }
        assert!(
            closed_by(&pool, *target).await.iter().all(Option::is_some),
            "{surface}: the rewrite leaves the finding closed"
        );
    }
    let confirms_title: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM sensitivity.findings \
                  WHERE surface = 'kb_resources.title' AND target_id = $1 \
                    AND content_hash = sensitivity.keyed_hash($2, $3)) \
              + (SELECT count(*) FROM sensitivity.place_observations \
                  WHERE surface = 'kb_resources.title' AND target_id = $1 \
                    AND content_hash = sensitivity.keyed_hash($2, $3)) \
              + (SELECT count(*) FROM sensitivity.memo \
                  WHERE content_hash = sensitivity.keyed_hash($2, $3))",
    )
    .bind(erased.uuid())
    .bind(SALT)
    .bind(&title)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        confirms_title, 0,
        "the salt no longer confirms the erased title anywhere the sweep stores a hash"
    );
    assert_eq!(
        digests_at(&pool, "kb_events.payload", ledger).await,
        ledger_before,
        "an open finding keeps its digests: the ledger still holds the title until cut 2"
    );
    assert_eq!(
        digests_at(&pool, "kb_resources.title", live).await,
        live_before,
        "a closed finding on a resource that was never erased keeps its digests"
    );
    for ((surface, target), b) in live_edge_places.iter().zip(&live_edge_before) {
        assert_eq!(
            &digests_at(&pool, surface, *target).await,
            b,
            "a closed {surface} on an edge with no erased end keeps its digests"
        );
    }
}

/// Places an erasure act empties while the resource lives on give up their digests too: the block
/// history scrub's prior revision (through the real scrub), and the principal act's governed block
/// content (its footprint planted by hand, as Witness 9 does: content emptied, hash kept). The
/// scrub's kept current revision stays open, and keeps its digests.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn content_a_scrub_or_principal_erasure_emptied_gives_up_its_digests_after_thirty_days(
    pool: PgPool,
) {
    enable_seeded_detectors(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'scrub-expiry', 'scrub-expiry') RETURNING id",
    )
    .bind(owner.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "notes",
            origin_uri: "test://scrub-expiry",
            body: &format!("the first draft quoted {SSN_A}"),
            doc_type: "research",
            home: AnchorRef::context(temper_core::types::ids::ContextId::from(home)),
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
    .expect("create through the write path");
    let scrubbed: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1 LIMIT 1")
            .bind(resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    writes::update_resource(
        &pool,
        writes::UpdateParams {
            resource,
            body: Some(&format!("the revision still quotes {SSN_B}")),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: None,
            sources: vec![],
            content_block: Some(scrubbed),
            rehome_to: None,
            emitter,
        },
    )
    .await
    .expect("a per-block revise");
    let (prior, current): (Uuid, Uuid) = sqlx::query_as(
        "SELECT (SELECT br.id FROM kb_block_revisions br WHERE br.block_id = b.id AND br.id <> b.current_revision_id), \
                b.current_revision_id \
           FROM kb_content_blocks b WHERE b.id = $1",
    )
    .bind(scrubbed)
    .fetch_one(&pool)
    .await
    .unwrap();
    let principal = bare_resource(&pool, "a governed note").await;
    let governed = block(&pool, principal, &format!("SSN {SSN_A}")).await;
    let surface = "kb_block_content.content";
    assert!(!tick(&pool, surface).await.failed);
    let emptied = [prior, governed];
    let mut before = Vec::new();
    for target in emptied {
        let d = digests_at(&pool, surface, target).await;
        assert!(
            !d.is_empty() && d.iter().all(|(s, _, n)| s == "complete" && *n > 0),
            "found and fingerprinted: {d:?}"
        );
        before.push(d);
    }
    let current_before = digests_at(&pool, surface, current).await;

    sqlx::query("SELECT block_history_scrub_execute($1, $2, $3, $4, $5)")
        .bind(resource.uuid())
        .bind(vec![scrubbed])
        .bind(owner.uuid())
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the scrub completes");
    sqlx::query("UPDATE kb_block_content SET content = '' WHERE block_revision_id = $1")
        .bind(governed)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!tick(&pool, surface).await.failed);
    for target in emptied {
        assert_eq!(
            closed_by(&pool, target).await,
            vec![Some("content_empty".to_string())]
        );
    }

    windows_started_days_ago(&pool, 29).await;
    assert!(!tick(&pool, surface).await.failed);
    for (target, b) in emptied.iter().zip(&before) {
        assert_eq!(
            &digests_at(&pool, surface, *target).await,
            b,
            "at 29 days an emptied place keeps its digests"
        );
    }

    windows_started_days_ago(&pool, 31).await;
    assert!(!tick(&pool, surface).await.failed);
    for (target, b) in emptied.iter().zip(&before) {
        for ((state, hash, n), (_, old_hash, _)) in
            digests_at(&pool, surface, *target).await.iter().zip(b)
        {
            assert_eq!(state, "expired");
            assert_eq!(*n, 0, "no fingerprint is left");
            assert_ne!(hash, old_hash, "the keyed hash of the unit is gone");
            let left: i64 = sqlx::query_scalar(
                "SELECT (SELECT count(*) FROM sensitivity.memo WHERE content_hash = $1) \
                      + (SELECT count(*) FROM sensitivity.findings WHERE content_hash = $1)",
            )
            .bind(old_hash)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(
                left, 0,
                "no memo row or finding still carries the retired hash"
            );
        }
    }
    assert_eq!(
        closed_by(&pool, current).await,
        vec![None],
        "the current revision stays open"
    );
    assert_eq!(
        digests_at(&pool, surface, current).await,
        current_before,
        "the scrub's kept current revision keeps its digests"
    );
}

// ── The memo and the place observations are bounded by the expiry too (security review §4.3) ───

/// What the expiry door runs on every cron call, whether or not the deployment sweeps (Q53).
async fn expiry_door(pool: &PgPool) -> i32 {
    sqlx::query_scalar("SELECT sensitivity_expire_erased_fingerprints()")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// How many rows of each store still confirm `unit` to a salt holder: `(memo, place_observations)`.
async fn confirmations(pool: &PgPool, unit: &str) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM sensitivity.memo \
                  WHERE content_hash = sensitivity.keyed_hash($1, $2)), \
                (SELECT count(*) FROM sensitivity.place_observations \
                  WHERE content_hash = sensitivity.keyed_hash($1, $2))",
    )
    .bind(SALT)
    .bind(unit)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// `(surface, content_hash)` of each place observation of `resource`.
async fn observations_of(pool: &PgPool, resource: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT surface, content_hash FROM sensitivity.place_observations \
          WHERE target_id = $1 ORDER BY surface",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// A clean title leaves no finding, so the finding expiry never reaches its hash. With every
/// detector off before the act, no head tick re-reads the place, so nothing but the expiry can take
/// the pre-erasure observation away. The door takes it on its next call; a live resource's
/// observations stay. Once a detector is back on and the head records the husk, the door takes that
/// observation too.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_erased_resources_place_observations_go_on_the_next_expiry_call(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'observation-expiry', 'observation-expiry') RETURNING id",
    )
    .bind(owner.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let title = "Quarterly roadmap for the harbour project";
    let erased = created_resource(&pool, owner, emitter, home, title).await;
    let live = created_resource(&pool, owner, emitter, home, "Kept roadmap").await;
    for surface in ["kb_resources.title", "kb_resources.origin_uri"] {
        assert!(!tick(&pool, surface).await.failed, "{surface}");
    }
    assert_eq!(
        observations_of(&pool, erased.uuid()).await.len(),
        2,
        "the head observes the erased resource's title and origin_uri before the act"
    );
    let findings: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sensitivity.findings WHERE resource_id = $1")
            .bind(erased.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        findings, 0,
        "the title is clean, so no finding carries its hash"
    );
    let live_before = observations_of(&pool, live.uuid()).await;
    assert_eq!(live_before.len(), 2);

    sqlx::query("SELECT sensitivity.disable_detectors(4, 'temper')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(erased.uuid())
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the act completes");
    assert_eq!(
        confirmations(&pool, title).await.1,
        2,
        "with no tick after the act, both pre-erasure observations are still there (the fixture's \
         origin_uri is its title)"
    );

    assert_eq!(expiry_door(&pool).await, 0, "no finding is expired");
    assert_eq!(
        observations_of(&pool, erased.uuid()).await,
        vec![],
        "the door takes the erased resource's observations"
    );
    assert_eq!(
        confirmations(&pool, title).await.1,
        0,
        "no observation confirms the erased title"
    );
    assert_eq!(
        observations_of(&pool, live.uuid()).await,
        live_before,
        "a live resource keeps its observations"
    );

    enable_seeded_detectors(&pool).await;
    assert!(!tick(&pool, "kb_resources.title").await.failed);
    assert_eq!(
        observations_of(&pool, erased.uuid()).await.len(),
        1,
        "the head re-reads the moved place and observes the husk"
    );
    expiry_door(&pool).await;
    assert_eq!(
        observations_of(&pool, erased.uuid()).await,
        vec![],
        "the husk's observation goes too"
    );
}

/// A memo row is deleted 30 days after it was written, whatever it hashes: a memo row names no
/// place, so its age is the only bound that reaches an erased unit. A row the scan writes carries
/// its age (and so survives the call that follows it); one 29 days old stays; one 31 days old goes,
/// and so does a row from before the column existed, which reads as -infinity.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_memo_row_is_deleted_thirty_days_after_it_was_written(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    let young = "A memo row the window keeps";
    let old = "A memo row past the window";
    let unaged = "A memo row older than its column";
    for title in [young, old, unaged] {
        bare_resource(&pool, title).await;
    }
    assert!(!tick(&pool, "kb_resources.title").await.failed);
    for title in [young, old, unaged] {
        assert!(
            confirmations(&pool, title).await.0 > 0,
            "the scan memoises the clean title {title:?}"
        );
    }
    expiry_door(&pool).await;
    for title in [young, old, unaged] {
        assert!(
            confirmations(&pool, title).await.0 > 0,
            "a row the scan just wrote is not due: {title:?}"
        );
    }

    for (title, age) in [
        (young, "now() - interval '29 days'"),
        (old, "now() - interval '31 days'"),
        (unaged, "'-infinity'::timestamptz"),
    ] {
        sqlx::query(&format!(
            "UPDATE sensitivity.memo SET memoized_at = {age} \
              WHERE content_hash = sensitivity.keyed_hash($1, $2)"
        ))
        .bind(SALT)
        .bind(title)
        .execute(&pool)
        .await
        .unwrap();
    }
    expiry_door(&pool).await;
    assert!(
        confirmations(&pool, young).await.0 > 0,
        "a row 29 days old stays"
    );
    assert_eq!(
        confirmations(&pool, old).await.0,
        0,
        "a row 31 days old goes"
    );
    assert_eq!(
        confirmations(&pool, unaged).await.0,
        0,
        "a row with no recorded age goes"
    );
}

/// One call deletes at most 50,000 memo rows, oldest first, so a memo that predates the column
/// clears over several door calls rather than in one statement, and the next call takes the rest.
/// The one row the first call leaves is the youngest due row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_memo_is_cleared_in_bounded_batches_oldest_first(pool: PgPool) {
    sqlx::query(
        "INSERT INTO sensitivity.memo (content_hash, detector_id, detector_version, memoized_at) \
         SELECT encode(sha256(convert_to(i::text, 'UTF8')), 'hex'), v.detector_id, v.version, \
                CASE WHEN i = 1 THEN now() - interval '31 days' ELSE '-infinity' END \
           FROM generate_series(1, 50001) i \
          CROSS JOIN (SELECT detector_id, version FROM sensitivity.detector_versions \
                       ORDER BY detector_id, version LIMIT 1) v",
    )
    .execute(&pool)
    .await
    .unwrap();
    let left = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sensitivity.memo")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    expiry_door(&pool).await;
    assert_eq!(left().await, 1, "one call takes 50,000");
    let kept: String = sqlx::query_scalar("SELECT content_hash FROM sensitivity.memo")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        kept, "6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b",
        "the row the first call leaves is the youngest, sha256('1')"
    );
    expiry_door(&pool).await;
    assert_eq!(left().await, 0, "the next call takes the rest");
}

/// A memo row written before the column existed reads as -infinity, so it is due on the first call
/// after the migration rather than 30 days later. Read from the catalog, because a test database
/// migrates an empty memo: the value a pre-existing row takes is the column's stored missing value.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_memo_row_from_before_the_column_has_no_age(pool: PgPool) {
    let missing: Option<String> = sqlx::query_scalar(
        "SELECT attmissingval::text FROM pg_attribute \
          WHERE attrelid = 'sensitivity.memo'::regclass AND attname = 'memoized_at'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(missing.as_deref(), Some("{-infinity}"));
}

/// The planner keeps no statistics on a keyed-hash column, so pg_stats never holds a copy of a hash
/// the expiry deleted. Swept, erased-free rows in all four tables, then ANALYZE: none of the four
/// columns gains a pg_stats row. The migration's removal of statistics gathered before it ran is
/// not witnessed here, because a test database migrates empty tables.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_hash_column_keeps_planner_statistics(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
    for i in 0..20 {
        bare_resource(&pool, &format!("Payroll {i} for {SSN_A}")).await;
        bare_resource(&pool, &format!("Clean title {i}")).await;
    }
    assert!(!tick(&pool, "kb_resources.title").await.failed);
    for table in [
        "memo",
        "place_observations",
        "findings",
        "finding_fingerprints",
    ] {
        let rows: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM sensitivity.{table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(rows > 0, "the tick wrote sensitivity.{table}");
        sqlx::query(&format!("ANALYZE sensitivity.{table}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let gathered: Vec<(String, String)> = sqlx::query_as(
        "SELECT tablename::text, attname::text FROM pg_stats WHERE schemaname = 'sensitivity' \
            AND (tablename, attname) IN (('memo', 'content_hash'), \
                 ('place_observations', 'content_hash'), ('findings', 'content_hash'), \
                 ('finding_fingerprints', 'fingerprint'))",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(gathered, vec![], "ANALYZE gathers no hash column");
    let analysed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stats WHERE schemaname = 'sensitivity' \
            AND tablename = 'place_observations' AND attname = 'surface'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        analysed, 1,
        "the ANALYZE ran: the other columns have statistics"
    );
}

/// The expiry's observation arm names the kb_resources surfaces, because only they are
/// mutable_timestamp and only a mutable surface writes an observation. A surface that becomes
/// mutable fails here until the arm covers it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_mutable_surface_is_a_resource_surface(pool: PgPool) {
    let mutable: Vec<String> = sqlx::query_scalar(
        "SELECT surface FROM sensitivity.surfaces WHERE cursor_kind = 'mutable_timestamp' \
          ORDER BY surface",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        mutable,
        vec!["kb_resources.origin_uri", "kb_resources.title"],
        "a new mutable surface needs an arm in expire_erased_fingerprints' observation DELETE"
    );
}

// ── Q51: payment_card's noise closes as false positives, never a card ───────────────────────────

/// Moves payment_card to its next version under `validator`, as a migration would.
async fn bump_payment_card(pool: &PgPool, validator: &str) {
    sqlx::query(
        "UPDATE sensitivity.detectors SET version = version + 1, validator = $1 WHERE id = 'payment_card'",
    )
    .bind(validator)
    .execute(pool)
    .await
    .unwrap();
    // A bump turns the detector off (Q53); the operator enables the version they reviewed.
    sqlx::query(
        "SELECT sensitivity.enable_detector(id, version) FROM sensitivity.detectors WHERE id = $1",
    )
    .bind("payment_card")
    .execute(pool)
    .await
    .unwrap();
}

/// `(path, state)` of every payment_card finding at `target` and its dispositions, in path order.
async fn card_dispositions_at(
    pool: &PgPool,
    target: Uuid,
) -> Vec<(Option<String>, Option<String>)> {
    sqlx::query_as(
        "SELECT f.path, d.state FROM sensitivity.findings f \
           LEFT JOIN sensitivity.dispositions d ON d.finding_id = f.id \
          WHERE f.target_id = $1 AND f.detector_id = 'payment_card' ORDER BY f.path NULLS FIRST, d.state",
    )
    .bind(target)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// v2's validator was Luhn alone, and production's first sweep found no card with it. The noise
/// closes as false positives; a place where the current version still finds a card stays open,
/// because Q27 would carry the ruling onto that card's re-detection.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_card_finding_the_current_version_would_not_make_closes_as_a_false_positive(
    pool: PgPool,
) {
    enable_seeded_detectors(&pool).await;
    const STAMP: &str = "20261003000050"; // a migration stamp that passes Luhn
    const CARD: &str = "4111 1111 1111 1111";
    bump_payment_card(&pool, "luhn_valid").await;

    let r = bare_resource(&pool, &format!("Release {STAMP}")).await;
    let stamp = block(&pool, r, &format!("ran {STAMP}")).await;
    let card = block(&pool, r, &format!("card {CARD}")).await;
    let ruled = block(&pool, r, &format!("again {STAMP}")).await;
    let doc = event(
        &pool,
        "resource_created",
        serde_json::json!({ "title": format!("card {CARD}"), "body": format!("at {STAMP}") }),
        serde_json::json!({}),
    )
    .await;
    tick(&pool, "kb_block_content.content").await;
    tick(&pool, "kb_resources.title").await;
    tick(&pool, "kb_events.payload").await;
    for place in [stamp, card, ruled, r, doc] {
        assert!(
            !card_dispositions_at(&pool, place).await.is_empty(),
            "the Luhn-only version found something at {place}, or nothing below witnesses"
        );
    }

    // An operator already ruled on `ruled`. The title changes, and the sweep's next look at it
    // closes its place as changed.
    sqlx::query(
        "INSERT INTO sensitivity.dispositions (finding_id, state) \
         SELECT id, 'acknowledged' FROM sensitivity.findings \
          WHERE target_id = $1 AND detector_id = 'payment_card'",
    )
    .bind(ruled)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE kb_resources SET title = 'Release', updated = clock_timestamp() WHERE id = $1",
    )
    .bind(r)
    .execute(&pool)
    .await
    .unwrap();
    tick(&pool, "kb_resources.title").await;

    bump_payment_card(&pool, "card_valid").await;
    let closed: i32 = sqlx::query_scalar("SELECT sensitivity.close_cardless_card_findings()")
        .fetch_one(&pool)
        .await
        .unwrap();

    let fp = || Some("false_positive".to_string());
    assert_eq!(
        card_dispositions_at(&pool, stamp).await,
        vec![(None, fp())],
        "noise closes"
    );
    assert_eq!(
        card_dispositions_at(&pool, card).await,
        vec![(None, None)],
        "a card stays open"
    );
    assert_eq!(
        card_dispositions_at(&pool, ruled).await,
        vec![(None, Some("acknowledged".to_string()))],
        "an operator's ruling is not joined by another"
    );
    assert_eq!(
        card_dispositions_at(&pool, r).await,
        vec![(None, None)],
        "a place that closed as changed takes no disposition"
    );
    assert_eq!(
        card_dispositions_at(&pool, doc).await,
        vec![
            (Some("/body".to_string()), fp()),
            (Some("/title".to_string()), None),
        ],
        "a document is read at each finding's path"
    );
    assert_eq!(closed, 2);
    let again: i32 = sqlx::query_scalar("SELECT sensitivity.close_cardless_card_findings()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(again, 0, "a second call closes nothing more");
}
