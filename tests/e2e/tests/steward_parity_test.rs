#![cfg(feature = "test-db")]
//! The steward family's parity suite, authored against the DIRECT binding first, then
//! carried green across the network door (beat 5 — the last family on the direct
//! binding; ruled 2026-10-02, temper decision `01a0fe82-5104-79c1-a04c-ebe7eecafddd`).
//!
//! The discipline is the G3a-prime pattern's: a parity suite that never saw the old
//! binding cannot prove parity, and one that never crosses the real listener cannot
//! prove the door. The tools are driven the way the MCP function drives them: a real
//! `TemperMcpService`, per-request parts naming the harness principal, against THIS
//! process's listener.
//!
//! # The faces, named from the direct callsites before they were witnessed
//!
//! **steward_ingest_delta** (`steward_service::ingest_delta`; the route
//! `GET /api/steward/{cogmap}/delta` makes the identical call)
//! - *Bad cogmap ref* — `parse_cogmap` refuses `invalid_params` with `bad cogmap ref: …`.
//!   A pure parse, MCP-local and pre-wire on both sides of the swap.
//! - *Unreadable or absent cogmap* — the service's `anchor_readable_by_profile` gate
//!   answers NotFound with one sentence for both (no existence oracle).
//! - *The default threshold* — omitted, the service default applies and is echoed.
//! - *An explicit threshold* — echoed, and it moves only the counted comparison.
//! - *Counts* — events in the cogmap's team-context window are counted, and
//!   `max_event_id` names the newest.
//!
//! **steward_advance_watermark** (`DbBackend::advance_steward_watermark`; the route
//! `POST /api/steward/{cogmap}/watermark` makes the identical call)
//! - *Bad cogmap ref* — as above.
//! - *Readable but not authorable* — `cogmap_authorship_refusal`'s ForbiddenDetail,
//!   the disclosing 403 that names the missing write grant and the map. (The tool's
//!   terse `Forbidden` arm is unreachable from this backend, which only refuses with
//!   the detailed variant.)
//! - *Unreadable cogmap* — NotFound, `cognitive map {id} not found`.
//! - *Event outside the ingest window* — the second, distinct NotFound exit:
//!   `event {e} is not in cognitive map {c}'s ingest window`.
//! - *The ack as stored* — the ack reports what the UPDATE stored, never the input.
//! - *Boundary-only advance* — no `event_id`: the watermark holds, the boundary
//!   fingerprint is computed at write time.
//! - *A supplied fingerprint* — stored as supplied.
//! - *The ledger* — an advance writes no `kb_events` row (it moves two cursor columns
//!   and completes the active workflow job). There is nothing for attribution to
//!   attach to; the family's attribution witness pins that, beside the trusted-path
//!   leg (`steward_attribution_witness_test.rs`).
//!
//! # Declared parity delta (flipped at the swap, in the same commit)
//!
//! - **NotFound prefix drops** — on three faces: the delta's unreadable/absent cogmap,
//!   the advance's cogmap exit, and its ingest-window exit. The direct map prefixed
//!   `{action}: `; the door's `ClientError::NotFound` carries the server's own sentence,
//!   and the door does not re-apply a prefix the direct tool applied (the G3c precedent).
//!   Kind (`invalid_params`) and gate identical; the advance's two exits stay
//!   distinguishable by their sentences, and unreadable stays indistinguishable from
//!   absent.
//!
//! NOT a delta, pinned unchanged through the swap: the disclosing 403 keeps the direct
//! face byte-for-byte (`steward_advance_watermark: ` prefix, the backend's sentence,
//! INVALID_REQUEST — the reblock family's precedent for `ForbiddenDetail`), and both
//! bad-ref refusals stay MCP-local and pre-wire.
//!
//! Gate faces shared by every family — the post-edge 401 arms and the system-access
//! 403 — are pinned once in `resources_wire_arms_test.rs` and not duplicated here.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

use common::E2eTestApp;

mod parity {
    use serde::Deserialize;

    /// The harness principal's email — the profile every suite parts name.
    pub const EMAIL: &str = "e2e@test.example.com";

    /// Build a tool input from its WIRE shape, so the deserializer pins the field
    /// names an MCP caller actually sends.
    pub fn input<T: for<'de> Deserialize<'de>>(value: serde_json::Value) -> T {
        serde_json::from_value(value).expect("input deserializes from its wire shape")
    }

    /// The single text part a one-part tool result carries, as JSON.
    pub fn one_text(res: &rmcp::model::CallToolResult) -> serde_json::Value {
        let parts = &res.content;
        assert_eq!(parts.len(), 1, "one content part, got {}", parts.len());
        serde_json::from_str(parts[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response")
    }

    /// `rmcp::ErrorData` codes: -32600 request error, -32602 invalid_params,
    /// -32603 internal_error.
    pub fn code_of(err: &rmcp::ErrorData) -> i32 {
        err.code.0
    }
}

use parity::{code_of, input, one_text, EMAIL};

/// The parity harness, once per test: the relay-ready app, the MCP service, and the
/// parts the family's tools are driven with.
async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

// ── Drivers ────────────────────────────────────────────────────────────────────
//
// `(svc, parts, params)` signatures, byte-stable across the swap: pre-swap (94d4e1d) one
// bridging line resolved the profile from parts the way service.rs's dispatch did; at the
// swap the same drivers hand the relayed parts to the tool, whose act the API adjudicates.

async fn run_delta(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::steward::steward_ingest_delta(svc, parts, input(params)).await
}

async fn run_advance(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::steward::steward_advance_watermark(svc, parts, input(params)).await
}

// ── Fixtures ───────────────────────────────────────────────────────────────────
//
// The steward service tests' seed, re-homed onto the harness principal: a team the
// principal is a member of, a cogmap joined to that team, a team-owned context whose
// events the cogmap ingests, and an emitter entity for seeding events. Membership
// confers READ of the cogmap; authorship needs an explicit write grant, which only
// the tests that need it take.

struct Seeded {
    principal: Uuid,
    cogmap: Uuid,
    ctx: Uuid,
    entity: Uuid,
}

async fn principal_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind(EMAIL)
        .fetch_one(pool)
        .await
        .expect("the harness principal is provisioned")
}

async fn new_cogmap(pool: &PgPool, name: &str) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(format!("{name} telos"))
    .fetch_one(pool)
    .await
    .expect("telos resource");
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("cogmap")
}

async fn seed(pool: &PgPool) -> Seeded {
    let principal = principal_id(pool).await;
    let team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('steward-parity', 'Steward parity') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("team");
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(principal)
    .execute(pool)
    .await
    .expect("membership");
    let cogmap = new_cogmap(pool, "steward parity map").await;
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("team joins the map");
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'steward-parity', 'Steward parity') RETURNING id",
    )
    .bind(team)
    .fetch_one(pool)
    .await
    .expect("team context");
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'steward-parity-emitter') RETURNING id",
    )
    .bind(principal)
    .fetch_one(pool)
    .await
    .expect("emitter entity");
    Seeded {
        principal,
        cogmap,
        ctx,
        entity,
    }
}

async fn add_event(pool: &PgPool, entity: Uuid, type_name: &str, ctx: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         VALUES ((SELECT id FROM kb_event_types WHERE name = $1), $2, 'kb_contexts', $3) RETURNING id",
    )
    .bind(type_name)
    .bind(entity)
    .bind(ctx)
    .fetch_one(pool)
    .await
    .expect("event")
}

/// The two steward cursors as stored — read back after a refusal to prove the refusal
/// wrote nothing (a fresh cogmap's are both NULL; a write would settle the fingerprint).
async fn cursors(pool: &PgPool, cogmap: Uuid) -> (Option<Uuid>, Option<String>) {
    sqlx::query_as(
        "SELECT steward_watermark_event_id, steward_boundary_fingerprint FROM kb_cogmaps WHERE id = $1",
    )
    .bind(cogmap)
    .fetch_one(pool)
    .await
    .expect("cursors")
}

// ── steward_ingest_delta ───────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delta_bad_cogmap_ref_refuses_invalid_params(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_delta(&svc, &parts, json!({ "cogmap": "not-a-ref" }))
        .await
        .expect_err("a malformed ref refuses");
    assert_eq!(code_of(&err), -32602, "{err:?}");
    assert!(
        err.message.starts_with("bad cogmap ref: "),
        "the MCP-local parse refusal: {err:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delta_unreadable_cogmap_refuses_with_the_uniform_not_found(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    // A map no team of the principal's is joined to: unreadable, and indistinguishable
    // from absent.
    let unreadable = new_cogmap(&app.pool, "someone else's map").await;
    let absent = Uuid::now_v7();

    let unreadable_err = run_delta(&svc, &parts, json!({ "cogmap": unreadable.to_string() }))
        .await
        .expect_err("unreadable refuses");
    let absent_err = run_delta(&svc, &parts, json!({ "cogmap": absent.to_string() }))
        .await
        .expect_err("absent refuses");

    assert_eq!(code_of(&unreadable_err), -32602, "{unreadable_err:?}");
    assert_eq!(
        unreadable_err.message,
        "cognitive map not found or not readable"
    );
    assert_eq!(
        (code_of(&unreadable_err), unreadable_err.message.as_ref()),
        (code_of(&absent_err), absent_err.message.as_ref()),
        "unreadable and absent are one face — no existence oracle"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delta_default_and_explicit_threshold_round_trip(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let s = seed(&app.pool).await;

    let default = one_text(
        &run_delta(&svc, &parts, json!({ "cogmap": s.cogmap.to_string() }))
            .await
            .expect("the member reads the delta"),
    );
    assert_eq!(default["cogmap_id"], json!(s.cogmap));
    assert_eq!(
        default["threshold"],
        json!(temper_core::types::steward::DEFAULT_STEWARD_INGEST_THRESHOLD)
    );

    let explicit = one_text(
        &run_delta(
            &svc,
            &parts,
            json!({ "cogmap": s.cogmap.to_string(), "threshold": 1 }),
        )
        .await
        .expect("an explicit threshold reads"),
    );
    assert_eq!(explicit["threshold"], json!(1));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delta_counts_the_team_window_and_names_the_newest_event(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let s = seed(&app.pool).await;
    add_event(&app.pool, s.entity, "resource_created", s.ctx).await;
    add_event(&app.pool, s.entity, "resource_created", s.ctx).await;
    let newest = add_event(&app.pool, s.entity, "relationship_asserted", s.ctx).await;

    let delta = one_text(
        &run_delta(&svc, &parts, json!({ "cogmap": s.cogmap.to_string() }))
            .await
            .expect("the member reads the delta"),
    );
    assert_eq!(delta["new_resources"], json!(2), "{delta}");
    assert_eq!(delta["new_events"], json!(3), "{delta}");
    assert_eq!(delta["max_event_id"], json!(newest), "{delta}");
    assert_eq!(delta["watermark"], json!(null), "never advanced: {delta}");
}

// ── steward_advance_watermark ──────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn advance_bad_cogmap_ref_refuses_invalid_params(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_advance(&svc, &parts, json!({ "cogmap": "not-a-ref" }))
        .await
        .expect_err("a malformed ref refuses");
    assert_eq!(code_of(&err), -32602, "{err:?}");
    assert!(
        err.message.starts_with("bad cogmap ref: "),
        "the MCP-local parse refusal: {err:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn advance_readable_but_not_authorable_refuses_with_the_disclosing_403(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let s = seed(&app.pool).await;
    let e = add_event(&app.pool, s.entity, "resource_created", s.ctx).await;

    let err = run_advance(
        &svc,
        &parts,
        json!({ "cogmap": s.cogmap.to_string(), "event_id": e }),
    )
    .await
    .expect_err("membership reads but does not author");
    assert_eq!(code_of(&err), -32600, "{err:?}");
    assert_eq!(
        err.message,
        format!(
            "steward_advance_watermark: cannot author cognitive map {}: authorship requires an \
             explicit write grant on the map, which you do not hold. You can read this map; \
             reading confers no authorship.",
            s.cogmap
        )
    );
    assert_eq!(
        cursors(&app.pool, s.cogmap).await,
        (None, None),
        "the refusal wrote nothing — the gate runs before the UPDATE"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn advance_unreadable_cogmap_refuses_not_found(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let unreadable = new_cogmap(&app.pool, "someone else's map").await;

    let err = run_advance(
        &svc,
        &parts,
        json!({ "cogmap": unreadable.to_string(), "event_id": Uuid::now_v7() }),
    )
    .await
    .expect_err("unreadable refuses");
    assert_eq!(code_of(&err), -32602, "{err:?}");
    assert_eq!(err.message, format!("cognitive map {unreadable} not found"));

    // An absent map answers the same face, modulo the id the caller supplied — no
    // existence oracle on the write path either.
    let absent = Uuid::now_v7();
    let absent_err = run_advance(
        &svc,
        &parts,
        json!({ "cogmap": absent.to_string(), "event_id": Uuid::now_v7() }),
    )
    .await
    .expect_err("absent refuses");
    assert_eq!(code_of(&absent_err), code_of(&err), "{absent_err:?}");
    assert_eq!(
        absent_err.message,
        format!("cognitive map {absent} not found"),
        "unreadable and absent are one face"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn advance_to_an_event_outside_the_window_refuses_with_the_second_not_found(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let s = seed(&app.pool).await;
    common::grant_cogmap_write(&app.pool, s.cogmap, s.principal).await;
    // An event anchored to the principal's personal default context — no team the
    // cogmap is joined to owns or shares it, so it is outside the window.
    let personal_ctx: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_contexts WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND name = 'default'",
    )
    .bind(s.principal)
    .fetch_one(&app.pool)
    .await
    .expect("the auto-provisioned default context");
    let outside = add_event(&app.pool, s.entity, "resource_created", personal_ctx).await;

    let err = run_advance(
        &svc,
        &parts,
        json!({ "cogmap": s.cogmap.to_string(), "event_id": outside }),
    )
    .await
    .expect_err("an out-of-window event refuses");
    assert_eq!(code_of(&err), -32602, "{err:?}");
    assert_eq!(
        err.message,
        format!(
            "event {outside} is not in cognitive map {}'s ingest window",
            s.cogmap
        ),
        "the window exit, distinct from the cogmap exit"
    );
    assert_eq!(
        cursors(&app.pool, s.cogmap).await,
        (None, None),
        "the refusal wrote nothing — the window check runs before the UPDATE"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn advance_acks_what_it_stored_and_the_delta_shrinks(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let s = seed(&app.pool).await;
    common::grant_cogmap_write(&app.pool, s.cogmap, s.principal).await;
    add_event(&app.pool, s.entity, "resource_created", s.ctx).await;
    let e2 = add_event(&app.pool, s.entity, "resource_created", s.ctx).await;
    let e3 = add_event(&app.pool, s.entity, "relationship_asserted", s.ctx).await;

    let ack = one_text(
        &run_advance(
            &svc,
            &parts,
            json!({ "cogmap": s.cogmap.to_string(), "event_id": e2 }),
        )
        .await
        .expect("an author advances"),
    );
    assert_eq!(ack["cogmap_id"], json!(s.cogmap), "{ack}");
    assert_eq!(
        ack["watermark"],
        json!(e2),
        "the ack is what was stored: {ack}"
    );
    assert!(
        ack["boundary_fingerprint"].is_string(),
        "no fingerprint supplied → the server settles it at write time: {ack}"
    );

    let delta = one_text(
        &run_delta(&svc, &parts, json!({ "cogmap": s.cogmap.to_string() }))
            .await
            .expect("the delta reads after the advance"),
    );
    assert_eq!(delta["watermark"], json!(e2), "{delta}");
    assert_eq!(
        delta["new_events"],
        json!(1),
        "only e3 is after e2: {delta}"
    );
    assert_eq!(delta["max_event_id"], json!(e3), "{delta}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_boundary_only_advance_holds_the_watermark(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let s = seed(&app.pool).await;
    common::grant_cogmap_write(&app.pool, s.cogmap, s.principal).await;
    let e1 = add_event(&app.pool, s.entity, "resource_created", s.ctx).await;

    run_advance(
        &svc,
        &parts,
        json!({ "cogmap": s.cogmap.to_string(), "event_id": e1 }),
    )
    .await
    .expect("the first advance lands");

    let ack = one_text(
        &run_advance(&svc, &parts, json!({ "cogmap": s.cogmap.to_string() }))
            .await
            .expect("a boundary-only advance lands"),
    );
    assert_eq!(
        ack["watermark"],
        json!(e1),
        "no event id → the stored watermark holds: {ack}"
    );
    assert!(ack["boundary_fingerprint"].is_string(), "{ack}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_supplied_fingerprint_is_stored_as_supplied(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let s = seed(&app.pool).await;
    common::grant_cogmap_write(&app.pool, s.cogmap, s.principal).await;

    let ack = one_text(
        &run_advance(
            &svc,
            &parts,
            json!({ "cogmap": s.cogmap.to_string(), "boundary_fingerprint": "parity-fp" }),
        )
        .await
        .expect("an advance with a fingerprint lands"),
    );
    assert_eq!(ack["boundary_fingerprint"], json!("parity-fp"), "{ack}");
    assert_eq!(
        ack["watermark"],
        json!(null),
        "never advanced to an event: {ack}"
    );
}
