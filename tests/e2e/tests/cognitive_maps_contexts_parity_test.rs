#![cfg(feature = "test-db")]
//! The cognitive_maps + contexts families' parity suite (beat G3d — the register's
//! largest cluster), authored green against the DIRECT binding first (the fourth
//! proof of the G3a-prime pattern) and carried through the network door in the swap
//! commit: every refusal face below was pinned against the direct callsites BEFORE
//! the tools crossed, so parity is proven, not presumed.
//!
//! Applies [the G3c discipline](temper task: the `ledger_graph_parity_test.rs` header)
//! to ten tools: cogmap read/list/create/materialize, context read/manage/materialize,
//! describe_schema, invocation read/manage. The tools take the request's `Parts` and
//! forward to their deployed routes; these drivers hand the parts straight through —
//! signatures byte-stable from the pre-swap suite, assertions carried or flipped
//! under a declared delta.
//!
//! # The refusal faces, named before they are witnessed (learning 4)
//!
//! Each face below was verified at its callsite before being pinned (arm shape, not
//! line number). Faces marked FLIPPED DELTA changed rendering at the swap, in the
//! same commit as the migration, per the G3c delta format.
//!
//! **cogmap reads** (`tools/cognitive_maps.rs`)
//! - *Garbage refs* — `parse_ref` failures render `invalid_params` at the parse
//!   callsite, prefixed `bad cogmap ref:` / `bad lens ref:`, never reaching a service.
//! - *Reads deny by posture, three different ways on purpose* — `shape` answers a 200
//!   OBJECT with `emptiness: "unreadable_or_absent"` and `population: 0`; `metrics`
//!   answers a 200 `[]`; `analytics` refuses `invalid_params` with the fixed
//!   no-leak sentence "cognitive map not found or not readable"; `show` carries the
//!   service's own identical sentence (its `NotFound` arm destructures the payload
//!   bare); `charter` is the FLIPPED projection delta below — the door denies it
//!   with the show route's sentence. A reader who cannot see the map is never told
//!   more than "not found or not readable" — and never errors where the posture is
//!   data (charter's deny-is-an-error posture is the one declared exception).
//! - *`materialize_delta`* — an unreadable anchor raises `ApiError::NotFound`, which
//!   the direct `map_api_error` has no arm for: it falls to the `internal_error`
//!   catch-all carrying the service's sentence. **DECLARED PARITY DELTA CANDIDATE**:
//!   the wire route 404s this face, so the door renders it `invalid_params` with the
//!   server's sentence — the G3c closed-invocation-409 family. The suite pins the
//!   direct `internal_error` face now; the swap flips the pin in the same commit.
//!
//! **genesis** (`cogmap_create`)
//! - *The explicit-id guard is silent, not a refusal* — a non-admin who supplies
//!   `cogmap_id`/`telos_resource_id` gets a SUCCESSFUL genesis at server-minted ids
//!   (the backend replaces the fields with `None`); pinning a refusal there would be
//!   wrong. The idempotency-at-a-supplied-id face is therefore admin-constructible
//!   only: an admin genesis at an explicit id answers `created: true`, a re-genesis
//!   at the same id answers `created: false`.
//! - *The map is born with an EMPTY charter* and the creator holds authorship (the
//!   backend mints read+write+grant) — witnessed by a successful materialize.
//!
//! **cogmap materialize**
//! - *Read-not-write* — `ForbiddenDetail` carrying the gate's own sentence
//!   ("authorship requires an explicit write grant … You can read this map; reading
//!   confers no authorship"), rendered `invalid_params` prefixed `cogmap_materialize: `.
//!   Constructed with a second identity on the L0 kernel map: auto-join membership
//!   confers READ, never write.
//! - *Absent map* — `NotFound("cognitive map {id} not found")`, same prefixed arm.
//! - *Bare `Forbidden`* — the tool has the arm, but the backend's materialize gate
//!   never produces it (read-yes/write-no is `ForbiddenDetail`, no-row is `NotFound`);
//!   named here as unreachable from this surface, not pinned.
//! - *Below threshold* — the documented no-op ack (`materialized: false`).
//!
//! **context reads + manage** (`tools/contexts.rs`)
//! - *Requirement arms are byte-exact tool constants* (`get requires \`id\`` and kin).
//! - *Garbage / unresolvable refs* — `invalid context ref: …` at the parse callsite;
//!   `context not found: …` at the resolve gate (which is the SAME predicate the
//!   analytics read gates on — verified — so the analytics `None` face is shadowed
//!   behind resolution via this tool and is pinned only on the cogmap side).
//! - *Taken names auto-suffix* — `create` never conflicts on a slug; the second
//!   create at a colliding name lands at a suffixed slug. The Conflict faces live on
//!   `rename` ("{owner} already owns a context with slug '{slug}'; pick another
//!   name") — pinned with the colliding slug named; at the door the sentence arrives
//!   bare, the `Conflict: ` label stripped and the tool prefix dropped.
//! - *The two-sided gate* (share/unshare/transfer) denies bare `Forbidden` and the
//!   tool speaks the requirement: "… administer the context and manage the target
//!   team …". Rename's one-sided gate denies 403 to a reader-non-administeror and
//!   404 ("context not found or not readable") to a non-reader; only the 404 arm is
//!   constructed here (a visible-but-not-administered context needs a shared team
//!   context + non-manager member) — the 403 arm is named, not pinned.
//! - *Idempotent no-op-safes* — `shared: false` / `unshared: false` on repeats.
//! - *`create` with a `+team` owner the caller cannot manage* — the direct tool mapped
//!   EVERY create failure through `internal_error` (owner resolution included).
//!   **FLIPPED DELTA**: the door's 403/404 render `invalid_params`; the non-manager
//!   arm now speaks the tool's requirement sentence.
//!
//! **context materialize**
//! - *Below threshold for the owner* — the documented no-op ack.
//! - *Read-not-write denies bare `Forbidden`* (the context arm keeps the
//!   argument-free refusal by design), which the direct tool's blanket
//!   `ApiError::from` + `internal_error` catch-all renders as
//!   `context_materialize failed: …`. Constructing that face needs a context the
//!   caller can read but not author — a shared team context with a non-authoring
//!   member — which this suite does not build; the authority face is named, not
//!   pinned, and the service's own unit test (`context_materialize_requires_context_write`)
//!   pins bare `Forbidden` beneath it.
//!
//! **describe_schema** (`tools/doc_types.rs`) — pure compute over embedded schemas;
//! it touches no backend and has no binding to swap. Pinned anyway: all three views
//! answer, an unknown doc-type name refuses `invalid_params` naming the asked-for
//! type, and the `doc_type` requirement arm is byte-exact.
//!
//! **invocations** (`tools/invocations.rs`)
//! - *Open* — against an authorable map answers the minted id; against a map the
//!   caller can read but not author refuses **`INVALID_REQUEST` (-32600)** with the
//!   tool's prefixed detailed sentence (the disclosure dialect; the terse
//!   "cannot author this cognitive map" arm is pinned by the module's unit test but
//!   unreachable from this backend path); against an absent map the
//!   `NotFound` arm renders `invalid_params` prefixed.
//! - *Close* — a terminal envelope refuses `Conflict`("invocation {id} is already
//!   '{status}' — close is a one-shot terminal transition"), which the direct
//!   `map_err` had no Conflict arm for: **THE FLIPPED PARITY DELTA** — pinned
//!   `internal_error` pre-swap, `invalid_params` with the server's sentence at the
//!   door, the G3c flipped delta's twin. An unknown/unreadable id collapses
//!   to `NotFound("invocation {id} not found")` → `invalid_params`, the server's
//!   sentence bare (the arm G3c's suite already wire-pinned; reused, not re-authored).
//! - *Reads deny* — `list` denies with data, never errors: the outsider's view is
//!   their own (empty) reach. `show` FLIPPED: the direct readback answered the JSON
//!   text `null`; the wire route 404s unknown and unreadable alike (the leak-safe
//!   contract), so the door renders `invalid_params` with the route's sentence.
//!
//! # The door mechanics the swap kept (declared pre-swap, argued at the PR)
//!
//! - **Charter crossed by projection, not a new route**: no standalone charter route
//!   exists, and `GET /api/cognitive-maps/{id}`'s `CogmapDetail.charter` is the
//!   identical `Vec<CharterBlock>` composed FROM the same `cogmap_charter_select` —
//!   the door's charter view is the show route plus a field projection. The declared
//!   delta REALIZED: an unreadable map's charter flips from the direct 200-empty to
//!   the show route's 404 sentence (the route family's own deny-is-an-error posture).
//! - **Context refs resolved in-process at G3d; through the door since teardown** —
//!   the orientation routes are UUID-addressed, so G3d kept the ref resolver
//!   in-process (the resources family's retained-resolver precedent). Teardown moved
//!   it onto `GET /api/contexts/resolve` (route-first #991) behind a local parse:
//!   every anchor face — the parse refusal, `@me`'s slug-naming miss, the UUID and
//!   `@<handle>` arms' uniform unreadable-equals-absent face, the `+<team>` arm's
//!   absent-team, non-member `Forbidden` and member-miss faces — was pinned
//!   byte-exact against the in-process resolver first
//!   (`every_context_anchor_face_is_pinned_byte_exact`, the table shared with the
//!   reblock suite) and carried through the swap unchanged. **One declared delta,
//!   named not pinned:** a fault behind the resolver renders `internal_error` at the
//!   door, where the in-process resolver rendered it `invalid_params` under the
//!   `context not found: ` prefix.
//!
//! # How the tools are driven
//!
//! Through the tool functions — the same hop production dispatch makes. The drivers
//! hand the request's `Parts` straight to the relayed tools: the parts carry the
//! FULL production shape (the claims the middleware injects beside the bearer), and
//! the API adjudicates Level 1 + 2 from the wire; since teardown nothing in-process
//! reads the claims. A second identity is its own
//! parts: its real token, warmed through the listener, standing-approved — no
//! synthetic claims. The suite header above still names the DIRECT faces the
//! pre-swap pins held: a refusal sentence that survives the door unchanged is the
//! proof the suite exists to give.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

mod parity {
    use serde::Deserialize;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// The harness principal's email — the one `approve_app_principal` provisions
    /// and standing-approves, the identity `direct_parts` carries.
    pub const EMAIL: &str = "e2e@test.example.com";

    /// The L0 kernel cognitive map reserved id (birth migration `20260625000001`) —
    /// bound to the temper-system auto-join team, so every approved profile READS it
    /// and nobody holds write without an explicit grant (`grant_cogmap_write`).
    pub const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

    pub async fn default_context_id(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "SELECT c.id \
             FROM kb_contexts c \
             JOIN kb_profiles p ON p.id = c.owner_id \
             WHERE p.email = $1 AND c.name = 'default'",
        )
        .bind(EMAIL)
        .fetch_one(pool)
        .await
        .expect("the auto-provisioned default context")
    }

    pub async fn profile_id_by_email(pool: &PgPool, email: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
            .bind(email)
            .fetch_one(pool)
            .await
            .expect("the profile")
    }

    pub async fn team_id_by_slug(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = $1")
            .bind(slug)
            .fetch_one(pool)
            .await
            .expect("the seed team")
    }

    /// A SECOND approved identity, warmed through the real listener (JIT
    /// provisioning with the correct handle, per-surface emitters, and its own
    /// default context) and standing-approved by its own email — the G3a idiom.
    /// Returns the token and the claims-carrying identity fields (the admin leg
    /// still needs the email to promote its profile).
    pub async fn second_identity(
        app: &super::common::E2eTestApp,
        pool: &PgPool,
        tag: &str,
    ) -> (String, String, String) {
        let unique = Uuid::new_v4();
        let sub = format!("{tag}-sub-{unique}");
        let email = format!("{tag}-{unique}@example.com");
        let token = super::common::generate_test_jwt(&sub, &email);
        let _ = app
            .reqwest_client
            .get(app.url("/api/profile"))
            .bearer_auth(&token)
            .send()
            .await;
        sqlx::query(
            "INSERT INTO kb_principal_standing (profile_id, state)
             SELECT id, 'approved' FROM kb_profiles WHERE email = $1
             ON CONFLICT (profile_id) DO UPDATE SET state = 'approved', updated = now()",
        )
        .bind(&email)
        .execute(pool)
        .await
        .expect("approve the second identity's standing");
        (token, sub, email)
    }

    /// The production parts shape for an ARBITRARY approved identity: the claims
    /// extension beside the bearer, exactly as the JWT middleware injects them. The
    /// door forwards on the bearer alone (since teardown nothing in-process reads the
    /// claims), so the claims ride for production fidelity, not need. Identity is
    /// consistent by construction: both halves come from the one `(token, sub, email)`
    /// triple.
    pub fn identity_parts_for(
        _app: &super::common::E2eTestApp,
        token: &str,
        sub: &str,
        email: &str,
    ) -> axum::http::request::Parts {
        axum::http::Request::builder()
            .extension(temper_mcp::middleware::BearerToken(token.to_string()))
            .extension(temper_services::auth::RawJwtClaims {
                sub: sub.to_string(),
                email: Some(email.to_string()),
                email_verified: None,
                azp: None,
                gty: None,
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
                iat: 0,
            })
            .body(())
            .expect("identity parts build")
            .into_parts()
            .0
    }

    /// Build a tool input from its WIRE shape, so the deserializer — not a struct
    /// literal — pins the field names an MCP caller actually sends.
    pub fn input<T: for<'de> Deserialize<'de>>(value: serde_json::Value) -> T {
        serde_json::from_value(value).expect("input deserializes from its wire shape")
    }

    /// The single text part a one-part tool result carries.
    pub fn one_text(res: &rmcp::model::CallToolResult) -> serde_json::Value {
        let parts = &res.content;
        assert_eq!(parts.len(), 1, "one content part, got {}", parts.len());
        serde_json::from_str(parts[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response")
    }

    /// The two parts a notice-leading tool result carries: the trust-tier notice
    /// first, the JSON body second — the charter marking is inseparable from the
    /// content it marks.
    pub fn notice_then_body(res: &rmcp::model::CallToolResult) -> (String, serde_json::Value) {
        let parts = &res.content;
        assert_eq!(parts.len(), 2, "notice + body, got {}", parts.len());
        let notice = parts[0].as_text().expect("a text notice").text.to_string();
        let body = serde_json::from_str(parts[1].as_text().expect("a text part").text.as_str())
            .expect("the second part is the tool's JSON response");
        (notice, body)
    }

    /// `rmcp::ErrorData` codes: -32600 request error (INVALID_REQUEST terminal
    /// authority arms), -32602 invalid_params, -32603 internal_error.
    pub fn code_of(err: &rmcp::ErrorData) -> i32 {
        err.code.0
    }
}

use common::E2eTestApp;
use parity::{code_of, input, notice_then_body, one_text};

/// The parity harness, once per test: the relay-ready app over this pool, the MCP
/// service, and the harness principal's parts — the full production shape, claims
/// beside bearer (the door forwards on the bearer).
async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.direct_parts();
    (app, svc, parts)
}

// ── Drivers ────────────────────────────────────────────────────────────────────
//
// `(svc, parts, params)` signatures, byte-stable across the swap: pre-swap the one
// bridging line resolves the profile from parts the way the handlers do; post-swap
// the same line becomes the parts handed to the relayed tool.

async fn run_cogmap_read(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::cognitive_maps::cogmap_read(
        svc,
        parts,
        input::<temper_mcp::tools::cognitive_maps::CogmapReadInput>(params),
    )
    .await
}

async fn run_cogmap_list(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::cognitive_maps::cogmap_list(
        svc,
        parts,
        input::<temper_mcp::tools::cognitive_maps::CogmapListInput>(params),
    )
    .await
}

async fn run_cogmap_create(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::cognitive_maps::cogmap_create(
        svc,
        parts,
        input::<temper_mcp::tools::cognitive_maps::CogmapCreateInput>(params),
    )
    .await
}

async fn run_cogmap_materialize(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::cognitive_maps::cogmap_materialize(
        svc,
        parts,
        input::<temper_core::types::materialize::MaterializeTriggerInput>(params),
    )
    .await
}

async fn run_context_read(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::contexts::context_read(
        svc,
        parts,
        input::<temper_mcp::tools::contexts::ContextReadInput>(params),
    )
    .await
}

async fn run_context_manage(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::contexts::context_manage(
        svc,
        parts,
        input::<temper_mcp::tools::contexts::ContextManageInput>(params),
    )
    .await
}

async fn run_context_materialize(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::cognitive_maps::context_materialize(
        svc,
        parts,
        input::<temper_core::types::materialize::ContextMaterializeInput>(params),
    )
    .await
}

async fn run_describe_schema(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::doc_types::describe_schema(
        svc,
        parts,
        input::<temper_mcp::tools::doc_types::DescribeSchemaInput>(params),
    )
    .await
}

async fn run_invocation_read(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::invocations::invocation_read(
        svc,
        parts,
        input::<temper_mcp::tools::invocations::InvocationReadInput>(params),
    )
    .await
}

async fn run_invocation_manage(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::invocations::invocation_manage(
        svc,
        parts,
        input::<temper_mcp::tools::invocations::InvocationManageInput>(params),
    )
    .await
}

// ── Suite-local fixtures ───────────────────────────────────────────────────────

/// A second identity's parts, ready to drive — the full production shape, both
/// extensions carrying the one identity.
async fn outsider_parts(app: &E2eTestApp, tag: &str) -> axum::http::request::Parts {
    let (token, sub, email) = parity::second_identity(app, &app.pool, tag).await;
    parity::identity_parts_for(app, &token, &sub, &email)
}

/// Genesis through the tool — the family's own write path, never a struct-literal
/// command: the deserializer pins the wire fields and the tool pins the command
/// shaping (the empty charter; the `@mcp` surface rides the door's carrier).
async fn genesis(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    name: &str,
) -> (Uuid, Uuid, bool) {
    let res = run_cogmap_create(
        svc,
        parts,
        json!({ "name": name, "telos_title": format!("{name} telos") }),
    )
    .await
    .expect("genesis lands");
    let out = one_text(&res);
    (
        out["cogmap_id"]
            .as_str()
            .expect("cogmap id")
            .parse()
            .expect("uuid"),
        out["telos_resource_id"]
            .as_str()
            .expect("telos id")
            .parse()
            .expect("uuid"),
        out["created"].as_bool().expect("created flag"),
    )
}

/// Open an invocation envelope for the harness against L0 — the authoring grant the
/// F2 write gate requires is minted explicitly (`grant_cogmap_write`); auto-join
/// membership alone confers read, never write.
async fn open_invocation_for_harness(
    app: &E2eTestApp,
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
) -> Uuid {
    let harness_profile = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    common::grant_cogmap_write(&app.pool, parity::L0_COGMAP, harness_profile).await;
    let res = run_invocation_manage(
        svc,
        parts,
        json!({
            "action": "open",
            "trigger_kind": "parity_harness",
            "originating_cogmap": parity::L0_COGMAP.to_string(),
        }),
    )
    .await
    .expect("open invocation against L0");
    let ack = one_text(&res);
    ack["invocation_id"]
        .as_str()
        .expect("invocation id")
        .parse()
        .expect("uuid")
}

// ── Cogmap reads ───────────────────────────────────────────────────────────────

/// A garbage map ref is the caller's mistake and refuses at the parse callsite,
/// before any service or wire is touched.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_garbage_cogmap_ref_refuses_at_the_parse_callsite(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_cogmap_read(
        &svc,
        &parts,
        json!({ "view": "shape", "cogmap": "not-a-uuid" }),
    )
    .await
    .expect_err("a garbage ref is refused");
    assert_eq!(code_of(&err), -32602, "caller error: {err}");
    assert!(
        err.message.contains("bad cogmap ref"),
        "the parse callsite's prefix: {err}"
    );
}

/// The optional lens is parsed when present; garbage refuses the same way.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_garbage_lens_ref_refuses_at_the_parse_callsite(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_cogmap_read(
        &svc,
        &parts,
        json!({
            "view": "shape",
            "cogmap": parity::L0_COGMAP.to_string(),
            "lens": "junk-lens",
        }),
    )
    .await
    .expect_err("a garbage lens is refused");
    assert_eq!(code_of(&err), -32602, "caller error: {err}");
    assert!(err.message.contains("bad lens ref"), "{err}");
}

/// `shape`'s deny posture is DATA: a 200 object whose `emptiness` names the cause
/// and whose population counts all-lens regions — never an error, never an
/// existence oracle. The unreadable map is the harness's grant-private genesis
/// (L0 answers everyone — auto-join read — so it cannot construct this face).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn shape_answers_an_object_whose_emptiness_never_leaks_existence(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Unshapeable").await;
    let other_parts = outsider_parts(&_app, "shape").await;

    let res = run_cogmap_read(
        &svc,
        &other_parts,
        json!({ "view": "shape", "cogmap": map_id.to_string() }),
    )
    .await
    .expect("an unreadable map answers, it does not error");
    let shape = one_text(&res);
    assert!(
        shape.is_object(),
        "the schema promises an OBJECT, not an array: {shape}"
    );
    assert_eq!(
        shape["population"].as_i64(),
        Some(0),
        "the all-lens count for a non-reader: {shape}"
    );
    assert_eq!(
        shape["emptiness"].as_str(),
        Some("unreadable_or_absent"),
        "absent and unreadable are one answer on purpose: {shape}"
    );
}

/// `metrics`' deny posture is the empty array — the read answers, it does not error.
/// (The L0 peer answers its true emptiness for the same caller; the DENY face needs
/// a map the caller cannot read — the harness's grant-private genesis.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn region_metrics_answers_empty_for_an_unreadable_map(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Unmeasurable").await;
    let other_parts = outsider_parts(&_app, "metrics").await;

    let res = run_cogmap_read(
        &svc,
        &other_parts,
        json!({ "view": "metrics", "cogmap": map_id.to_string() }),
    )
    .await
    .expect("an unreadable map answers empty");
    let rows = one_text(&res);
    assert!(rows.is_array(), "metrics is the array tier: {rows}");
    assert_eq!(rows.as_array().unwrap().len(), 0, "deny is empty: {rows}");
}

/// `analytics`' deny posture is the fixed no-leak sentence — byte-exact, the same
/// for an unreadable map and an absent one, so the read stays no existence oracle.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn analytics_refuses_an_unreadable_or_absent_map_with_the_no_leak_sentence(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Unanalyzable").await;
    let other_parts = outsider_parts(&_app, "analytics").await;
    let ghost = Uuid::now_v7();

    for cogmap in [map_id.to_string(), ghost.to_string()] {
        let err = run_cogmap_read(
            &svc,
            &other_parts,
            json!({ "view": "analytics", "cogmap": cogmap }),
        )
        .await
        .expect_err("analytics denies with the sentence");
        assert_eq!(code_of(&err), -32602, "caller error: {err}");
        assert_eq!(
            err.message, "cognitive map not found or not readable",
            "the fixed sentence, byte-exact, absent and unreadable alike: {err}"
        );
    }
}

/// `charter` denies an unreadable map with the show route's sentence — the door's
/// charter view is the show route plus a field projection (no additive route), and
/// the route family's deny-is-an-error posture travels with it. THE DECLARED
/// PROJECTION DELTA, flipped in the same commit as the swap: the direct binding
/// answered a 200 empty vec beside the reading notice (pinned there pre-swap).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn charter_denies_an_unreadable_map_with_the_show_routes_sentence(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Unchartered").await;
    let other_parts = outsider_parts(&_app, "charter").await;

    let err = run_cogmap_read(
        &svc,
        &other_parts,
        json!({ "view": "charter", "cogmap": map_id.to_string() }),
    )
    .await
    .expect_err("the projection's deny is the route's error");
    assert_eq!(code_of(&err), -32602, "caller error: {err}");
    assert_eq!(
        err.message, "cognitive map not found or not readable",
        "the show route's own sentence, byte-exact: {err}"
    );
}

/// `show` carries the service's own sentence through its bare `NotFound` arm —
/// absent and unreadable collapse into the one no-leak sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_refuses_an_unreadable_map_with_the_servers_own_sentence(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Unshowable").await;
    let other_parts = outsider_parts(&_app, "show").await;

    let err = run_cogmap_read(
        &svc,
        &other_parts,
        json!({ "view": "show", "cogmap": map_id.to_string() }),
    )
    .await
    .expect_err("show denies a non-reader");
    assert_eq!(code_of(&err), -32602, "caller error: {err}");
    assert_eq!(
        err.message, "cognitive map not found or not readable",
        "the service's sentence, un-prefixed, byte-exact: {err}"
    );
}

/// `list` is self-scoped: the harness sees the kernel map and its own genesis; the
/// second identity sees the kernel but NOT the harness's grant-private genesis. Both
/// answers lead with the excerpt notice.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_answers_only_maps_the_caller_can_see_beside_the_excerpt_notice(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let (_id, _telos, created) = genesis(&svc, &parts, "Solo Map").await;
    assert!(created, "first genesis creates");
    let (token, sub, email) = parity::second_identity(&app, &app.pool, "list").await;
    let other_parts = parity::identity_parts_for(&app, &token, &sub, &email);

    let (notice, mine) = notice_then_body(
        &run_cogmap_list(&svc, &parts, json!({}))
            .await
            .expect("list answers"),
    );
    assert!(
        notice.starts_with("Charter text"),
        "the excerpt tier rides list: {notice}"
    );
    let mine_names: Vec<&str> = mine
        .as_array()
        .expect("a JSON array of rows")
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(
        mine_names.contains(&"Solo Map"),
        "the creator sees their genesis: {mine:?}"
    );

    let (_notice, theirs) = notice_then_body(
        &run_cogmap_list(&svc, &other_parts, json!({}))
            .await
            .expect("list answers for the second identity"),
    );
    let their_names: Vec<&str> = theirs
        .as_array()
        .expect("a JSON array of rows")
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(
        !their_names.contains(&"Solo Map"),
        "a grant-private map is invisible to a non-holder: {their_names:?}"
    );
    // L0's charter statement rides every row that can see it — the excerpt tier's payload.
    let l0 = parity::L0_COGMAP.to_string();
    assert!(
        their_names.contains(&"L0 Kernel")
            || theirs
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"].as_str() == Some(l0.as_str())),
        "the kernel map is readable to every approved profile: {theirs:?}"
    );
}

/// `materialize_delta` answers a normal delta object for a readable map: the
/// threshold echoes and the exceeds flag is a bool, whatever the counts are.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_delta_reports_the_formation_delta_for_a_readable_map(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let res = run_cogmap_read(
        &svc,
        &parts,
        json!({
            "view": "materialize_delta",
            "cogmap": parity::L0_COGMAP.to_string(),
            "threshold": 3,
        }),
    )
    .await
    .expect("a readable map answers a delta");
    let delta = one_text(&res);
    assert_eq!(
        delta["threshold"].as_i64(),
        Some(3),
        "the threshold echoes: {delta}"
    );
    assert!(
        delta["formation_events"].is_i64(),
        "the formation count is present: {delta}"
    );
    assert!(
        delta["exceeds_threshold"].is_boolean(),
        "the clears-flag is present: {delta}"
    );
}

/// THE FLIPPED DELTA FACE: an unreadable anchor raises the service's `NotFound`,
/// which the direct `map_api_error` had no arm for — the catch-all rendered it
/// `internal_error` carrying the service's sentence (pinned there by this suite
/// pre-swap). The wire route 404s the face, and the door renders it `invalid_params`
/// with the server's own sentence — the G3c closed-invocation family, flipped in the
/// same commit as the swap.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_delta_refuses_an_unreadable_map_with_the_service_sentence(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Undeltable").await;
    let other_parts = outsider_parts(&_app, "delta").await;

    let err = run_cogmap_read(
        &svc,
        &other_parts,
        json!({
            "view": "materialize_delta",
            "cogmap": map_id.to_string(),
        }),
    )
    .await
    .expect_err("an unreadable anchor refuses");
    assert_eq!(
        code_of(&err),
        -32602,
        "the caller-actionable 404 arm — the flipped delta: {err}"
    );
    assert_eq!(
        err.message, "cognitive map not found or not readable",
        "the server's own sentence, un-prefixed, byte-exact: {err}"
    );
}

// ── Genesis ────────────────────────────────────────────────────────────────────

/// Genesis creates the map + its telos charter resource, grants its creator
/// authorship (witnessed by a successful materialize), and the map is born with an
/// EMPTY charter — the charter is authored prose delivered later by reconcile.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn genesis_creates_a_map_with_an_empty_charter_its_creator_can_author(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (id, telos, created) = genesis(&svc, &parts, "Charterless").await;
    assert!(created, "first genesis creates");
    assert_ne!(
        id,
        parity::L0_COGMAP,
        "a fresh id, not a collision with the kernel"
    );

    let (_notice, detail) = notice_then_body(
        &run_cogmap_read(
            &svc,
            &parts,
            json!({ "view": "show", "cogmap": id.to_string() }),
        )
        .await
        .expect("the creator reads their map"),
    );
    assert_eq!(
        detail["cogmap"]["id"].as_str(),
        Some(id.to_string().as_str())
    );
    assert_eq!(
        detail["charter"].as_array().map(Vec::len),
        Some(0),
        "born with an EMPTY charter: {detail}"
    );
    assert_eq!(
        detail["cogmap"]["telos_resource_id"].as_str(),
        Some(telos.to_string().as_str()),
        "the telos resource is realized: {detail}"
    );

    // The charter VIEW on a readable map still leads with the trust-tier marking —
    // the projection's deny face flipped at the swap, but the notice must ride the
    // content wherever the content arrives (the MCP witness for the marking; the
    // deny face no longer carries content, so it can no longer pin it).
    let (notice, blocks) = notice_then_body(
        &run_cogmap_read(
            &svc,
            &parts,
            json!({ "view": "charter", "cogmap": id.to_string() }),
        )
        .await
        .expect("the creator reads their own charter"),
    );
    assert!(
        notice.starts_with("READING NOTICE"),
        "the charter trust tier is stated where the content arrives: {notice}"
    );
    assert_eq!(
        blocks.as_array().map(Vec::len),
        Some(0),
        "born with an EMPTY charter, notice beside it: {blocks}"
    );

    let ack = one_text(
        &run_cogmap_materialize(
            &svc,
            &parts,
            json!({ "cogmap": id.to_string(), "threshold": 1_000_000 }),
        )
        .await
        .expect("the creator authors their map"),
    );
    assert_eq!(
        ack["materialized"].as_bool(),
        Some(false),
        "below threshold: {ack}"
    );
}

/// THE EXPLICIT-ID GUARD IS SILENT: a non-admin who supplies ids gets a successful
/// genesis at SERVER-MINTED ids — never a refusal. Pinning a refusal here would be
/// wrong; the pin is the replacement.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_admins_explicit_ids_are_replaced_by_server_minted_ones(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let wanted = Uuid::now_v7();
    let res = run_cogmap_create(
        &svc,
        &parts,
        json!({
            "name": "Chosen",
            "telos_title": "Chosen telos",
            "cogmap_id": wanted.to_string(),
        }),
    )
    .await
    .expect("genesis succeeds");
    let out = one_text(&res);
    assert_eq!(out["created"].as_bool(), Some(true));
    assert_ne!(
        out["cogmap_id"].as_str(),
        Some(wanted.to_string().as_str()),
        "a non-admin never places a map at a chosen id: {out}"
    );
}

/// The idempotency face is admin-constructible: an admin genesis at an explicit id
/// answers `created: true` AT that id; the re-genesis answers `created: false` with
/// the same realized identity — reproducible genesis, not a duplicate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_admins_explicit_ids_are_honored_and_genesis_is_idempotent_there(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let (token, sub, email) = parity::second_identity(&app, &app.pool, "admin").await;
    let admin_parts = parity::identity_parts_for(&app, &token, &sub, &email);
    let admin_profile = parity::profile_id_by_email(&app.pool, &email).await;
    common::make_system_admin(&app.pool, admin_profile).await;

    let chosen = Uuid::now_v7();
    let first = one_text(
        &run_cogmap_create(
            &svc,
            &admin_parts,
            json!({
                "name": "Chosen Admin Map",
                "telos_title": "Chosen telos",
                "cogmap_id": chosen.to_string(),
            }),
        )
        .await
        .expect("admin genesis succeeds"),
    );
    assert_eq!(first["created"].as_bool(), Some(true));
    assert_eq!(
        first["cogmap_id"].as_str(),
        Some(chosen.to_string().as_str()),
        "an admin's explicit id is honored: {first}"
    );

    let second = one_text(
        &run_cogmap_create(
            &svc,
            &admin_parts,
            json!({
                "name": "Chosen Admin Map",
                "telos_title": "Chosen telos",
                "cogmap_id": chosen.to_string(),
            }),
        )
        .await
        .expect("re-genesis succeeds"),
    );
    assert_eq!(
        second["created"].as_bool(),
        Some(false),
        "idempotent at a supplied id: {second}"
    );
    assert_eq!(
        second["cogmap_id"].as_str(),
        Some(chosen.to_string().as_str())
    );
}

// ── Cogmap materialize ─────────────────────────────────────────────────────────

/// Below threshold the trigger is the documented no-op: a truthful ack, no regions.
/// (The harness authors its own genesis map; L0 needs the explicit write grant.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_below_threshold_is_a_documented_no_op(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Quiet Map").await;
    let ack = one_text(
        &run_cogmap_materialize(
            &svc,
            &parts,
            json!({
                "cogmap": map_id.to_string(),
                "threshold": 1_000_000,
            }),
        )
        .await
        .expect("a no-op answers"),
    );
    assert_eq!(ack["materialized"].as_bool(), Some(false), "{ack}");
    assert_eq!(
        ack["anchor_table"].as_str(),
        Some("kb_cogmaps"),
        "the anchor is named: {ack}"
    );
}

/// Read-but-not-write is the gate's own detailed sentence, prefixed by the tool —
/// the disclosure dialect: the caller reads the map, and the sentence tells them
/// what authorship would take. Constructed with the second identity on L0
/// (auto-join read, no explicit write grant).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_names_the_missing_detail_for_a_reader_who_cannot_author(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let (token, sub, email) = parity::second_identity(&app, &app.pool, "matwrite").await;
    let other_parts = parity::identity_parts_for(&app, &token, &sub, &email);

    let err = run_cogmap_materialize(
        &svc,
        &other_parts,
        json!({ "cogmap": parity::L0_COGMAP.to_string(), "threshold": 1 }),
    )
    .await
    .expect_err("a reader without a write grant cannot author");
    assert_eq!(
        code_of(&err),
        -32602,
        "the ForbiddenDetail arm renders invalid_params: {err}"
    );
    assert!(
        err.message.starts_with("cogmap_materialize: "),
        "the tool's prefix: {err}"
    );
    assert!(
        err.message
            .contains("authorship requires an explicit write grant"),
        "the gate's own sentence: {err}"
    );
    assert!(
        err.message.contains("You can read this map"),
        "the disclosure clause that distinguishes this arm from the terse one: {err}"
    );
}

/// An absent map is the NotFound arm — the door carries the backend's own sentence
/// un-prefixed (the direct binding's `{action}: ` prefix dropped at the swap; kind
/// and gate identical).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_refuses_an_absent_map_with_the_not_found_arm(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = Uuid::now_v7();
    let err = run_cogmap_materialize(
        &svc,
        &parts,
        json!({ "cogmap": ghost.to_string(), "threshold": 1 }),
    )
    .await
    .expect_err("an absent map refuses");
    assert_eq!(code_of(&err), -32602, "caller error: {err}");
    assert_eq!(
        err.message,
        format!("cognitive map {ghost} not found"),
        "the server's own sentence, un-prefixed: {err}"
    );
}

// ── Context reads + manage ─────────────────────────────────────────────────────

/// The view/action requirement arms are the tool's own byte-exact constants.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_context_requirement_arms_are_byte_exact(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    for (params, expected) in [
        (json!({ "view": "get" }), "get requires `id`"),
        (json!({ "view": "shape" }), "shape requires `context`"),
        (json!({ "view": "metrics" }), "metrics requires `context`"),
        (
            json!({ "view": "analytics" }),
            "analytics requires `context`",
        ),
        (json!({ "action": "create" }), "create requires `name`"),
        (json!({ "action": "rename" }), "rename requires `context`"),
        (
            json!({ "action": "rename", "context": Uuid::now_v7() }),
            "rename requires `name`",
        ),
        (json!({ "action": "share" }), "share requires `context`"),
        (
            json!({ "action": "share", "context": Uuid::now_v7() }),
            "share requires `team`",
        ),
        (json!({ "action": "unshare" }), "unshare requires `context`"),
        (
            json!({ "action": "unshare", "context": Uuid::now_v7() }),
            "unshare requires `team`",
        ),
        (
            json!({ "action": "transfer" }),
            "transfer requires `context`",
        ),
        (
            json!({ "action": "transfer", "context": Uuid::now_v7() }),
            "transfer requires `team`",
        ),
    ] {
        let runner: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<rmcp::model::CallToolResult, rmcp::ErrorData>,
                    > + Send,
            >,
        > = if params["view"].is_null() {
            Box::pin(run_context_manage(&svc, &parts, params))
        } else {
            Box::pin(run_context_read(&svc, &parts, params))
        };
        let err = runner
            .await
            .expect_err("a missing key is the caller's mistake");
        assert_eq!(code_of(&err), -32602, "caller error: {err}");
        assert_eq!(err.message, expected, "the byte-exact requirement arm");
    }
}

/// A garbage context ref refuses at the parse callsite.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_garbage_context_ref_refuses_at_the_parse_callsite(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_context_read(
        &svc,
        &parts,
        json!({ "view": "shape", "context": "not a ref" }),
    )
    .await
    .expect_err("a garbage ref is refused");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.contains("invalid context ref"), "{err}");
}

/// An unresolvable ref refuses at the resolve gate carrying the resolver's
/// sentence — the gate that shadows the analytics `None` face behind this tool
/// (same predicate, verified).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unresolvable_context_ref_refuses_as_not_found(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_context_read(
        &svc,
        &parts,
        json!({ "view": "shape", "context": "@me/no-such-slug" }),
    )
    .await
    .expect_err("an unresolvable ref refuses");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.contains("context not found"), "{err}");
}

/// A context the caller cannot READ is invisible at the same resolve gate even
/// when addressed by its real UUID — addressing is not disclosure.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unreadable_context_ref_refuses_at_the_resolve_gate(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let (token, sub, email) = parity::second_identity(&app, &app.pool, "ctxread").await;
    let other_parts = parity::identity_parts_for(&app, &token, &sub, &email);
    let harness_context = parity::default_context_id(&app.pool).await;

    let err = run_context_read(
        &svc,
        &other_parts,
        json!({ "view": "shape", "context": harness_context.to_string() }),
    )
    .await
    .expect_err("another principal's private context is invisible");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.contains("context not found"), "{err}");
}

/// Every `context_anchor` refusal face, byte-exact, through each of the four tools that
/// address a context by ref — the three orientation reads and the trigger. The table is shared
/// with reblock's `scope=context` suite (`common::context_anchor_faces`), so the two anchors
/// answer one dialect. Pinned green against the in-process resolver first, then carried through
/// the relay to `GET /api/contexts/resolve` unchanged (teardown).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_context_anchor_face_is_pinned_byte_exact(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    for face in common::context_anchor_faces(&app).await {
        for view in ["shape", "metrics", "analytics"] {
            let err = run_context_read(
                &svc,
                &face.parts,
                json!({ "view": view, "context": face.context_ref }),
            )
            .await
            .expect_err(face.label);
            assert_eq!(code_of(&err), -32602, "{} ({view}): {err}", face.label);
            assert_eq!(err.message, face.expected, "{} ({view})", face.label);
        }
        let err = run_context_materialize(
            &svc,
            &face.parts,
            json!({ "context": face.context_ref, "threshold": 1 }),
        )
        .await
        .expect_err(face.label);
        assert_eq!(code_of(&err), -32602, "{} (materialize): {err}", face.label);
        assert_eq!(err.message, face.expected, "{} (materialize)", face.label);
    }
}

/// `list` answers the caller's own visible contexts.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_read_list_answers_the_callers_contexts(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let rows = one_text(
        &run_context_read(&svc, &parts, json!({ "view": "list" }))
            .await
            .expect("list answers"),
    );
    let arr = rows.as_array().expect("a JSON array of context rows");
    assert!(
        arr.iter().any(|r| r["name"].as_str() == Some("default")),
        "the auto-provisioned default context is the caller's: {rows}"
    );
}

/// Create answers the realized row: the caller's name echoed, a derived slug.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_manage_create_answers_the_row_with_a_derived_slug(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let row = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Beat 3 Scratch" }),
        )
        .await
        .expect("create answers the row"),
    );
    assert_eq!(row["name"].as_str(), Some("Beat 3 Scratch"));
    let slug = row["slug"].as_str().expect("a slug is derived");
    assert!(!slug.is_empty(), "the slug is the addressable form: {row}");
    assert_eq!(
        row["can_write"].as_bool(),
        Some(true),
        "the creator authors: {row}"
    );
}

/// `create` NEVER conflicts on a taken name — the slug auto-suffixes and both
/// contexts live. The Conflict faces live on rename, not here.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_taken_name_auto_suffixes_a_unique_slug(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let first = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Dup Notes" }),
        )
        .await
        .expect("first create"),
    );
    let second = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Dup Notes" }),
        )
        .await
        .expect("second create at the same name"),
    );
    let a = first["slug"].as_str().expect("slug");
    let b = second["slug"].as_str().expect("slug");
    assert_ne!(
        a, b,
        "the slugs differ — the second was suffixed: {first} vs {second}"
    );
    assert_ne!(
        first["id"].as_str(),
        second["id"].as_str(),
        "two real contexts, not one row twice"
    );
}

/// Rename re-addresses: the outcome carries the composed `context_ref` to use from
/// now on, and `renamed` is true for an actual change.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn rename_readdresses_the_context(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let created = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Rename Me" }),
        )
        .await
        .expect("create"),
    );
    let id = created["id"].as_str().expect("context id");

    let out = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "rename", "context": id, "name": "Renamed locus" }),
        )
        .await
        .expect("rename answers"),
    );
    assert_eq!(out["renamed"].as_bool(), Some(true), "{out}");
    let new_ref = out["context_ref"]
        .as_str()
        .expect("the composed ref to use");
    assert!(
        new_ref.contains("renamed-locus"),
        "the ref carries the NEW slug — the old address stops resolving: {out}"
    );
}

/// The rename Conflict face names the colliding slug — caller-fixable, rendered
/// `invalid_params` with the server's own sentence, the `Conflict: ` status label
/// stripped and the direct binding's `{context}: ` prefix dropped (the declared
/// delta; the pinned sentence and the named slug carry green).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn rename_conflicts_name_the_colliding_slug(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let first = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Held Name" }),
        )
        .await
        .expect("first"),
    );
    let second = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Other Context" }),
        )
        .await
        .expect("second"),
    );
    let second_id = second["id"].as_str().expect("id");

    let err = run_context_manage(
        &svc,
        &parts,
        json!({ "action": "rename", "context": second_id, "name": "Held Name" }),
    )
    .await
    .expect_err("the slug is already held by the same owner");
    assert_eq!(
        code_of(&err),
        -32602,
        "a taken slug is caller-fixable, not an internal error: {err}"
    );
    assert!(
        !err.message.contains("rename_context"),
        "the door carries the server's sentence bare — no tool prefix: {err}"
    );
    assert!(
        err.message.contains("already owns a context with slug"),
        "the conflict sentence: {err}"
    );
    let held_slug = first["slug"].as_str().expect("the held slug");
    assert!(
        err.message.contains(held_slug),
        "the colliding slug is the actionable half: {err}"
    );
}

/// The two-sided gate speaks its requirement to a caller who administers the
/// context but not the target team: the harness owns a context and is a mere
/// watcher on the auto-join team, so the share refuses with the requirement —
/// and idempotent no-op-safes hold once an admin shares.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_two_sided_gate_speaks_its_requirement_then_idempotence_holds(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let created = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Share Target" }),
        )
        .await
        .expect("create"),
    );
    let context = created["id"].as_str().expect("id");
    let team = parity::team_id_by_slug(&app.pool, "temper-system").await;

    let err = run_context_manage(
        &svc,
        &parts,
        json!({ "action": "share", "context": context, "team": team.to_string() }),
    )
    .await
    .expect_err("a watcher on the target team cannot share");
    assert_eq!(
        code_of(&err),
        -32602,
        "bare Forbidden renders invalid_params: {err}"
    );
    assert!(
        err.message.starts_with("share_context requires that "),
        "the tool speaks the requirement, keyed by the rule: {err}"
    );
    assert!(
        err.message
            .contains("administer the context and manage the target team"),
        "the two-sided clause: {err}"
    );

    // With the harness promoted, the gate passes: shared, re-shared (false), unshared, re-unshared (false).
    let harness_profile = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    common::make_system_admin(&app.pool, harness_profile).await;
    let first = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "share", "context": context, "team": team.to_string() }),
        )
        .await
        .expect("admin share"),
    );
    assert_eq!(first["shared"].as_bool(), Some(true), "{first}");
    let again = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "share", "context": context, "team": team.to_string() }),
        )
        .await
        .expect("re-share"),
    );
    assert_eq!(
        again["shared"].as_bool(),
        Some(false),
        "idempotent: {again}"
    );
    let off = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "unshare", "context": context, "team": team.to_string() }),
        )
        .await
        .expect("unshare"),
    );
    assert_eq!(off["unshared"].as_bool(), Some(true), "{off}");
    let off_again = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "unshare", "context": context, "team": team.to_string() }),
        )
        .await
        .expect("re-unshare"),
    );
    assert_eq!(
        off_again["unshared"].as_bool(),
        Some(false),
        "no-op safe: {off_again}"
    );
}

/// Rename's one-sided gate denies a NON-READER with the service's 404 sentence —
/// invisible means "not found or not readable", never a leak. (The 403
/// reader-non-administeror arm needs a shared team context with a non-manager
/// member and is named in the header, not constructed here.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn rename_by_a_non_reader_refuses_with_the_not_found_sentence(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let (token, sub, email) = parity::second_identity(&app, &app.pool, "renread").await;
    let other_parts = parity::identity_parts_for(&app, &token, &sub, &email);
    let harness_context = parity::default_context_id(&app.pool).await;

    let err = run_context_manage(
        &svc,
        &other_parts,
        json!({
            "action": "rename",
            "context": harness_context.to_string(),
            "name": "Not Yours",
        }),
    )
    .await
    .expect_err("a non-reader cannot rename what it cannot see");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message, "context not found or not readable",
        "the 404 denial's carried sentence, un-prefixed: {err}"
    );
}

/// Transfer reassigns ownership to a team (administered here by the promoted
/// harness): the outcome names the new `+team-slug` owner ref, once.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn transfer_reassigns_ownership_to_a_team(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let harness_profile = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    common::make_system_admin(&app.pool, harness_profile).await;
    let created = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "create", "name": "Give Away" }),
        )
        .await
        .expect("create"),
    );
    let context = created["id"].as_str().expect("id");
    let team = parity::team_id_by_slug(&app.pool, "temper-system").await;

    let out = one_text(
        &run_context_manage(
            &svc,
            &parts,
            json!({ "action": "transfer", "context": context, "team": team.to_string() }),
        )
        .await
        .expect("transfer answers"),
    );
    assert_eq!(out["reassigned"].as_bool(), Some(true), "{out}");
    assert!(
        out["owner_ref"]
            .as_str()
            .unwrap_or_default()
            .starts_with("+temper-system"),
        "the owner ref is the team's decorated form: {out}"
    );
}

/// `create` with a `+team` owner the caller cannot manage is caller-actionable at
/// the door: the FLIPPED DELTA — the direct tool mapped the resolver's `Forbidden`
/// to the internal catch-all carrying "Failed to resolve owner: " (pinned there
/// pre-swap); the wire route answers 403 and the tool renders the requirement the
/// caller can act on, `invalid_params`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_team_owned_create_by_a_non_manager_refuses_with_the_requirement(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_context_manage(
        &svc,
        &parts,
        json!({
            "action": "create",
            "name": "Team Owned",
            "owner": { "Team": "temper-system" },
        }),
    )
    .await
    .expect_err("a watcher cannot create team-owned contexts");
    assert_eq!(
        code_of(&err),
        -32602,
        "the flipped delta: caller-actionable, not a fault: {err}"
    );
    assert_eq!(
        err.message,
        "create_context requires that you manage the team that will own it (owner/maintainer)",
        "the tool's requirement sentence, byte-exact: {err}"
    );
}

// ── Context materialize ────────────────────────────────────────────────────────

/// Below threshold, for the owner, the context trigger is the documented no-op.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_materialize_is_a_no_op_below_threshold_for_the_owner(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let context = parity::default_context_id(&app.pool).await;
    let ack = one_text(
        &run_context_materialize(
            &svc,
            &parts,
            json!({ "context": context.to_string(), "threshold": 1_000_000 }),
        )
        .await
        .expect("the owner authors their context"),
    );
    assert_eq!(ack["materialized"].as_bool(), Some(false), "{ack}");
}

/// A ghost context ref refuses at the resolve gate before any trigger runs.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_materialize_refuses_a_ghost_context_at_the_resolve_gate(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_context_materialize(
        &svc,
        &parts,
        json!({ "context": "@me/no-such-locus", "threshold": 1 }),
    )
    .await
    .expect_err("an unresolvable context refuses");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.contains("context not found"), "{err}");
}

// ── describe_schema ────────────────────────────────────────────────────────────

/// All three views answer — pure compute over the embedded schemas, no backend.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn describe_schema_answers_all_three_views(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let kinds = one_text(
        &run_describe_schema(&svc, &parts, json!({ "view": "doc_types" }))
            .await
            .expect("doc_types answers"),
    );
    assert!(
        kinds.as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "the embedded schema set is non-empty: {kinds}"
    );

    let task = one_text(
        &run_describe_schema(&svc, &parts, json!({ "view": "doc_type", "name": "task" }))
            .await
            .expect("the task type answers"),
    );
    assert!(task.is_object(), "a full schema description: {task}");

    let open_meta = one_text(
        &run_describe_schema(&svc, &parts, json!({ "view": "open_meta" }))
            .await
            .expect("open_meta answers"),
    );
    assert!(open_meta.is_object(), "the conventions answer: {open_meta}");
}

/// An unknown doc-type name is a CALLER error naming what was asked for — never
/// an internal fault.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_doc_type_name_refuses_as_a_caller_error(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_describe_schema(
        &svc,
        &parts,
        json!({ "view": "doc_type", "name": "widget" }),
    )
    .await
    .expect_err("widget is not a doc type");
    assert_eq!(code_of(&err), -32602, "{err}");
    assert!(
        err.message.contains("Unknown doc type 'widget'"),
        "the refusal names the asked-for type: {err}"
    );
}

// ── Invocations ────────────────────────────────────────────────────────────────

/// The open/close requirement arms are the tool's byte-exact constants.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_invocation_requirement_arms_are_byte_exact(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    for (params, expected) in [
        (json!({ "action": "open" }), "open requires `trigger_kind`"),
        (
            json!({ "action": "open", "trigger_kind": "manual" }),
            "open requires `originating_cogmap`",
        ),
        (json!({ "action": "close" }), "close requires `invocation`"),
        (
            json!({ "action": "close", "invocation": Uuid::now_v7().to_string() }),
            "close requires `disposition`",
        ),
    ] {
        let err = run_invocation_manage(&svc, &parts, params)
            .await
            .expect_err("a missing key is the caller's mistake");
        assert_eq!(code_of(&err), -32602, "caller error: {err}");
        assert_eq!(err.message, expected, "the byte-exact requirement arm");
    }

    let err = run_invocation_read(&svc, &parts, json!({ "view": "show" }))
        .await
        .expect_err("show without an invocation");
    assert_eq!(err.message, "show requires `invocation`", "{err}");
}

/// A garbage map ref in the open input refuses at the parse callsite.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_garbage_map_ref_in_open_refuses_at_the_parse_callsite(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_invocation_manage(
        &svc,
        &parts,
        json!({ "action": "open", "trigger_kind": "manual", "originating_cogmap": "junk" }),
    )
    .await
    .expect_err("a garbage ref is refused");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.contains("bad cogmap ref"), "{err}");
}

/// Open against an AUTHORABLE map answers the minted envelope — and `show` reads
/// it back with the trigger kind echoed and an empty acts list.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invocation_open_against_l0_answers_the_minted_id(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let invocation = open_invocation_for_harness(&app, &svc, &parts).await;

    let view = one_text(
        &run_invocation_read(
            &svc,
            &parts,
            json!({ "view": "show", "invocation": invocation.to_string() }),
        )
        .await
        .expect("the opener reads their envelope"),
    );
    assert_eq!(view["id"].as_str(), Some(invocation.to_string().as_str()));
    assert_eq!(view["status"].as_str(), Some("open"), "{view}");
    assert_eq!(
        view["trigger_kind"].as_str(),
        Some("parity_harness"),
        "{view}"
    );
    // The open itself is on the ledger: the envelope's first act is its own
    // `delegated_launch` event — the write's evidence row.
    let acts = view["acts"].as_array().expect("the acts list");
    assert_eq!(acts.len(), 1, "the open event is the first act: {view}");
    assert_eq!(
        acts[0]["event_kind"].as_str(),
        Some("delegated_launch"),
        "{view}"
    );
}

/// Open against a map the caller can READ but not author refuses with the
/// DETAILED authorship sentence under INVALID_REQUEST — the disclosure dialect:
/// the gate decides the caller has standing to hear the reason. The second
/// identity auto-joins temper-system (read on L0, no write grant).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invocation_open_refuses_a_reader_with_the_detailed_authorship_sentence(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let (token, sub, email) = parity::second_identity(&app, &app.pool, "invopen").await;
    let other_parts = parity::identity_parts_for(&app, &token, &sub, &email);

    let err = run_invocation_manage(
        &svc,
        &other_parts,
        json!({
            "action": "open",
            "trigger_kind": "manual",
            "originating_cogmap": parity::L0_COGMAP.to_string(),
        }),
    )
    .await
    .expect_err("a reader without a write grant cannot open");
    assert_eq!(
        code_of(&err),
        -32600,
        "the ForbiddenDetail arm is the INVALID_REQUEST disclosure dialect: {err}"
    );
    assert!(
        err.message.starts_with("invocation_open: "),
        "prefixed by the action: {err}"
    );
    assert!(
        err.message
            .contains("authorship requires an explicit write grant"),
        "the gate's own sentence reaches the agent intact: {err}"
    );
    assert!(
        !err.message.contains("this invocation"),
        "the refusal names the MAP — the invocation does not exist at gate time: {err}"
    );
}

/// Open against an ABSENT map speaks the TERSE sentence — the open gate does not
/// disclose existence: absent and unauthorable-collapse are one
/// `INVALID_REQUEST` refusal naming the map, never the invocation that does not
/// exist. (The detailed sentence is reserved for a caller who can read the map —
/// the prior test.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invocation_open_refuses_an_absent_map_with_the_terse_sentence(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = Uuid::now_v7();
    let err = run_invocation_manage(
        &svc,
        &parts,
        json!({
            "action": "open",
            "trigger_kind": "manual",
            "originating_cogmap": ghost.to_string(),
        }),
    )
    .await
    .expect_err("an absent map refuses");
    assert_eq!(
        code_of(&err),
        -32600,
        "the terse arm is the INVALID_REQUEST dialect: {err}"
    );
    assert_eq!(
        err.message, "invocation_open: cannot author this cognitive map",
        "byte-exact: the gate names the map and nothing it did not verify: {err}"
    );
}

/// Close is a one-shot terminal transition: the happy close answers the ack, the
/// re-close refuses — with the door's caller-actionable 409 arm. THE FLIPPED
/// PARITY DELTA: the direct binding's `map_err` had no Conflict arm and rendered
/// this face `internal_error` (pinned there pre-swap); the swap flips the pin to
/// `invalid_params` with the server's own sentence, the `Conflict: ` label
/// stripped — the G3c flipped delta's twin, in the same commit.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_closed_invocation_refuses_with_the_conflict_arm(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let invocation = open_invocation_for_harness(&app, &svc, &parts).await;

    let ack = one_text(
        &run_invocation_manage(
            &svc,
            &parts,
            json!({
                "action": "close",
                "invocation": invocation.to_string(),
                "disposition": "completed",
                "outcome": { "note": "done" },
            }),
        )
        .await
        .expect("the first close lands"),
    );
    assert_eq!(
        ack["invocation_id"].as_str(),
        Some(invocation.to_string().as_str())
    );
    assert_eq!(ack["disposition"].as_str(), Some("completed"));

    let err = run_invocation_manage(
        &svc,
        &parts,
        json!({
            "action": "close",
            "invocation": invocation.to_string(),
            "disposition": "completed",
        }),
    )
    .await
    .expect_err("a terminal envelope takes no second close");
    assert_eq!(
        code_of(&err),
        -32602,
        "a caller-actionable 409, not a fault — the flipped delta: {err}"
    );
    assert_eq!(
        err.message,
        format!(
            "invocation {invocation} is already 'completed' — close is a one-shot \
             terminal transition"
        ),
        "the server's own sentence, label stripped, byte-exact: {err}"
    );
}

/// A close of an unknown invocation is the uniform not-found arm — the shape
/// G3c's suite already wire-pinned for the act gate; reused, not re-authored.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_close_of_an_unknown_invocation_refuses_with_the_not_found_arm(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = Uuid::now_v7();
    let err = run_invocation_manage(
        &svc,
        &parts,
        json!({
            "action": "close",
            "invocation": ghost.to_string(),
            "disposition": "failed",
        }),
    )
    .await
    .expect_err("a correlation to nowhere is refused");
    assert_eq!(code_of(&err), -32602, "{err}");
    assert!(
        err.message
            .contains(&format!("invocation {ghost} not found")),
        "the uniform not-found sentence: {err}"
    );
}

/// A garbage invocation ref refuses at the parse callsite, on read and write alike.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_garbage_invocation_ref_refuses_at_the_parse_callsite(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_invocation_read(
        &svc,
        &parts,
        json!({ "view": "show", "invocation": "junk" }),
    )
    .await
    .expect_err("a garbage ref is refused");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.contains("bad invocation ref"), "{err}");
}

/// `list` is the caller's own reach: narrowed by status, filtered by originating
/// map, and the closed envelope leaves the open view.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invocation_read_list_narrows_by_status_and_by_cogmap(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let invocation = open_invocation_for_harness(&app, &svc, &parts).await;

    let open_rows = one_text(
        &run_invocation_read(
            &svc,
            &parts,
            json!({ "view": "list", "status": "open", "cogmap": parity::L0_COGMAP.to_string() }),
        )
        .await
        .expect("list answers"),
    );
    let inv = invocation.to_string();
    assert!(
        open_rows
            .as_array()
            .expect("array")
            .iter()
            .any(|r| r["id"].as_str() == Some(inv.as_str())),
        "the open envelope is in the caller's open view: {open_rows}"
    );

    run_invocation_manage(
        &svc,
        &parts,
        json!({
            "action": "close",
            "invocation": invocation.to_string(),
            "disposition": "abandoned",
        }),
    )
    .await
    .expect("close lands");

    let after = one_text(
        &run_invocation_read(
            &svc,
            &parts,
            json!({ "view": "list", "status": "open", "cogmap": parity::L0_COGMAP.to_string() }),
        )
        .await
        .expect("list answers"),
    );
    assert!(
        !after
            .as_array()
            .expect("array")
            .iter()
            .any(|r| r["id"].as_str() == Some(inv.as_str())),
        "a closed envelope leaves the open view: {after}"
    );
    let closed = one_text(
        &run_invocation_read(
            &svc,
            &parts,
            json!({ "view": "list", "status": "abandoned" }),
        )
        .await
        .expect("list answers"),
    );
    assert!(
        closed
            .as_array()
            .expect("array")
            .iter()
            .any(|r| r["id"].as_str() == Some(inv.as_str())),
        "and appears under its disposition: {closed}"
    );
}

/// Reads deny on TWO postures at the door: the outsider's `list` stays DATA —
/// their own (empty) view, never an error — while their `show` of an envelope
/// originating on a map they cannot read refuses with the route's uniform 404
/// sentence (deny and absent indistinguishable, leak-safe). THE FLIPPED DELTA on
/// `show`: the direct readback answered null (pinned there pre-swap); the wire
/// route 404s the face and the door carries the sentence. (The envelope's read
/// gate rides the ORIGINATING MAP's readability, so the L0-opened envelopes of
/// the earlier tests are visible to every approved profile — the private map is
/// what makes the deny arm constructible.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_outsiders_list_denies_with_data_and_show_with_the_route_404(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    // The envelope originates on the harness's grant-private map, not on L0.
    let (map_id, _telos, _created) = genesis(&svc, &parts, "Private Run Map").await;
    let harness_profile = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    common::grant_cogmap_write(&app.pool, map_id, harness_profile).await;
    let opened = one_text(
        &run_invocation_manage(
            &svc,
            &parts,
            json!({
                "action": "open",
                "trigger_kind": "parity_harness",
                "originating_cogmap": map_id.to_string(),
            }),
        )
        .await
        .expect("open invocation against the private map"),
    );
    let invocation: Uuid = opened["invocation_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");

    let (token, sub, email) = parity::second_identity(&app, &app.pool, "invread").await;
    let other_parts = parity::identity_parts_for(&app, &token, &sub, &email);

    let err = run_invocation_read(
        &svc,
        &other_parts,
        json!({ "view": "show", "invocation": invocation.to_string() }),
    )
    .await
    .expect_err("an outsider's show is refused at the route's 404");
    assert_eq!(
        code_of(&err),
        -32602,
        "the route's uniform 404 is caller-actionable: {err}"
    );
    assert_eq!(
        err.message, "invocation not found or not readable",
        "the route's own sentence, byte-exact — deny and absent indistinguishable: {err}"
    );

    let rows = one_text(
        &run_invocation_read(&svc, &other_parts, json!({ "view": "list" }))
            .await
            .expect("an outsider's list answers"),
    );
    assert_eq!(
        rows.as_array().map(Vec::len),
        Some(0),
        "their own reach is empty here: {rows}"
    );
}
