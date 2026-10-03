#![cfg(feature = "test-db")]
//! The reblock + blobs + segmented-ingest + data_artifacts(+shapes) families' parity
//! suite (beat G4 — the last mixed direct cluster, 12 handlers), authored green
//! against the DIRECT binding first (the fifth proof of the G3a-prime pattern) and
//! carried through the network door in the swap commit: every refusal face below was
//! pinned against the direct callsites BEFORE the tools crossed, so parity is proven,
//! not presumed.
//!
//! Applies [the G3c discipline](ledger_graph_parity_test.rs's header) to ten tools
//! (twelve handlers): `resource_reblock`; `blob_read` (actions read|list);
//! `blob_manage` (commit|relate); the consolidated `segmented_ingest` tool (begin /
//! append / finalize / blocks); `list_data_artifacts` / `get_data_artifact` /
//! `commit_data_artifact`; `list_data_artifact_shapes` / `get_data_artifact_shape` /
//! `declare_data_artifact_shape`. The tools take the request's `Parts` and forward to
//! their deployed routes; these drivers hand the parts straight through — signatures
//! byte-stable from the pre-swap suite, assertions carried or flipped under a declared
//! delta.
//!
//! # The refusal faces, named before they are witnessed (learning 4)
//!
//! Each face below was verified at its callsite before being pinned (arm shape, not
//! line number). Faces marked FLIPPED DELTA change rendering at the swap, in the same
//! commit as the migration, per the G3c delta format.
//!
//! **resource_reblock** (`tools/reblock.rs`)
//! - *Requirement arms are byte-exact tool constants* — `scope=resource requires
//!   \`resource\``, `scope=context requires \`context\``, both `invalid_params`.
//! - *Garbage refs* — a malformed resource ref hits `parse_ref` and renders
//!   `invalid_params` "invalid ref: ..."; a malformed context ref renders
//!   `invalid_params` prefixed `invalid context ref: `; a context the caller cannot
//!   resolve renders prefixed `context not found: `. All at the resolve callsite, never
//!   reaching the backend. **Teardown:** the resolver relays to
//!   `GET /api/contexts/resolve` (one `context_anchor`, shared with the context
//!   orientation tools); every face was pinned byte-exact against the in-process
//!   resolver first (`every_context_anchor_face_is_pinned_byte_exact`, the shared
//!   table) and carried unchanged. The one declared delta — a fault behind the
//!   resolver renders `internal_error`, not `invalid_params` — is named, not pinned.
//! - *Declared deltas, carried-forward note* — the module header declares a NotFound
//!   prefix-drop and a Conflict-arm addition for this family; neither face is
//!   pinned here (the direct suite never constructed a reblock NotFound or 409 —
//!   the receipt's `Denied` rows and cursor resume are the family's own surface).
//!   The mapper carries them per the G3c idiom; the register's beat G4 row counts
//!   the family's deltas as declared-not-pinned.
//! - *Below threshold* — scope=context dry_run=true on a fresh context answers the
//!   documented no-op receipt (`reblocked: 0` / `no_op: 0` — one row per candidate,
//!   so an empty candidate set is an empty `outcomes`); pins that the answer is a
//!   receipt, not an error.
//! - *The deployment-wide arm gates at the backend* — scope=all by a non-system-admin
//!   answers INVALID_REQUEST (-32600) with the tool's system-administrator sentence
//!   (the `map_err` Forbidden arm), named rather than pinned here since the harness's
//!   default identity is a system admin in some setups — named, not pinned: the unit
//!   test `the_forbidden_mapper_names_the_system_administrator_gate` covers the mapper.
//!
//! **blob_read** (`tools/blobs.rs`) — needs the app's blob_store configured.
//! - *Requirement arm* — read without `blob_id` answers `invalid_params` "read
//!   requires `blob_id`".
//! - *Absent or invisible blob* — **FLIPPED DELTA** (the blob family's prefix-drop,
//!   shared with commit/relate): the direct binding rendered the visibility gate's
//!   `NotFound` as `invalid_params` prefixed `{action}: `; the door's
//!   `ClientError::NotFound` carries the server's own sentence bare. Named here; the
//!   prefix-drop's pin lives on the blob_commit invisible-home face.
//! - *Half-paired home scope (list)* — a `home_table` without `home_id` (or vice
//!   versa) refuses with the service's own pair sentence, restated MCP-locally (the
//!   wire client's list carries only scoped-or-unscoped, so the service guard could
//!   not answer it) — carried face, never silently unscoped.
//! - *The read ceiling* — a blob over the single-request threshold refuses
//!   `invalid_params` naming the streaming doors: "this blob is N bytes against a
//!   blob_read ceiling of M bytes — read it through the API (GET /api/blobs/{id}) or
//!   the CLI (`temper blob get`), which stream". Byte-stable, never a delta — the
//!   sentence is the tool's own.
//! - *Store disabled* — the `NullBroker` posture: `blob_parts` maps
//!   `AppState::blob_refusal()` to a shared refusal; named, not pinned (the harness
//!   configures a store).
//!
//! **blob_manage** (`tools/blobs.rs`)
//! - *Per-field requirement arms* — commit without `home_table` / `home_id` /
//!   `content_type` / `content` each answer their own `invalid_params` sentence; relate
//!   without `blob_id` / `peer_table` / `peer_id` / `edge_kind` / `polarity` / `label` /
//!   `weight` likewise. Tool-door sentences, byte-exact.
//! - *Bad base64* — commit's decode failure is `invalid_params` with "…`content` is not
//!   valid base64 — the bytes ride base64 because MCP carries JSON only: …".
//! - *Home authority* — commit/relate against a home the caller can read but not author
//!   answers INVALID_REQUEST with the tool's home-authorable sentence (the
//!   `check_home_authorable` Forbidden arm); a home the caller cannot read at all
//!   answers `invalid_params` with `blob_commit: ` / `blob_relate: ` prefixed
//!   NotFound (the disclosure distinction the mapper's doc comment names).
//! - *peer vocabulary* — relate with `peer_table` anything but `kb_resources` is the
//!   service's `BadRequest` with `blob_relate: ` prefix — the pair constraint lives in
//!   the service, never in the tool.
//!
//! **segmented_ingest** (`tools/ingest.rs`) — the consolidated tool; the four call
//! steps, each with its required-field arms, named in the service.rs docs.
//! - *begin* — a first segment with a matching hash lands a resource and answers
//!   `SegmentedBeginResponse` (resource id, block 0 landed); a `content_hash` that
//!   does not match `content` refuses `invalid_params` with the surface-side check
//!   "content_hash does not match content"; begin with empty content refuses
//!   `invalid_params` "ingest_begin requires content — segment 0's text".
//! - *append* — a matching hash at seq 1 lands and answers `BlocksResponse`; a content
//!   hash mismatch refuses `invalid_params` bare with the backend's sentence
//!   "content_hash mismatch for seq N: declared X, computed Y" (BadRequest
//!   arm — the surface's own check at begin is not on the append path; the server
//!   guards it). An occupied seq re-write refuses INTERNAL ERROR on both sides —
//!   the direct catch-all and the wire's generic error bridge agree (the append
//!   route never types the raise as Conflict; NOT a delta — a stable-class face
//!   with a message-shape delta, named at the pin).
//! - *finalize* — answers the success text `Finalized <ref> (N blocks).` on a
//!   correct pair; a wrong `expected_blocks` fires the TF002 constraint →
//!   **FLIPPED DELTA** — direct catches nothing for Conflict → `internal_error`
//!   prefixed, wire answers 409 → `invalid_params` with the server's own sentence;
//!   a wrong `expected_body_hash` (TF003)
//!   fires ContentIntegrity → direct falls through to `internal_error`; wire answers
//!   422 with code CONTENT_INTEGRITY → `invalid_params` with the server's own
//!   sentence.
//! - *blocks* — the resume read answers `BlocksResponse` listing the landed set; the
//!   consolidated tool's dispatch refusal arms (`begin requires …`, `append requires
//!   …`, `finalize requires …`, `blocks requires …`) are tool-door, byte-exact.
//!
//! **data_artifacts** (`tools/data_artifacts.rs`)
//! - *list* — the tool answers an array of ArtifactViews; an invisible/absent
//!   resource is NOT refused — the read is visibility-gated and answers `[]` (a flat
//!   visibility passthrough, no NotFound arm — the read never names the home).
//! - *get* — a live artifact answers the ArtifactView body; a folded artifact answers
//!   the same body with `is_folded: true`; an absent/invisible artifact answers the
//!   200-text posture "Artifact not found or not visible to you." — direct answers
//!   SUCCESS TEXT, the wire answers 404 — **FLIPPED DELTA**: the door's flat route
//!   404s, the parity witness reads `invalid_params` "artifact not found" (the
//!   server's sentence).
//! - *commit* — an authored artifact answers `ArtifactCommitResponse`
//!   (`artifact_id` + the hydrated view); the per-act envelope fields (confidence,
//!   persona, reasoning, invocation_id, correlation_id) are forwarded to the wire
//!   request — never defaulted to an empty act. A read-on-write commit against an
//!   unwritable resource refuses `invalid_params` with the tool's sentence
//!   "Not authorized to commit artifacts to this resource: write access required." An
//!   artifact commit against an enforcing-shape violation is refused through
//!   `DataArtifactRefusal` → `invalid_params` with the refusal's own sentence.
//!
//! **data_artifact_shapes** (`tools/data_artifact_shapes.rs`)
//! - *list_shapes* — cogmap-home and context-home both answer arrays; an
//!   invisible/absent home answers `[]`; a garbage `home_type` refuses
//!   `invalid_params` "unrecognized home_type '…'; expected 'context' or 'cogmap'".
//! - *get_shape* — a live shape answers its body; a folded shape answers
//!   `include_folded`-adjacent — the substrate_read serves folded too (the direct
//!   read returns `Some(shape)` for a folded shape); an absent/invisible one answers
//!   the 200-text posture "Shape not found or not visible to you." — the same FLIPPED
//!   DELTA as get_artifact: the route 404s and the answer becomes `invalid_params`
//!   "shape not found".
//! - *declare_shape* against an authoring home answers the ShapeView; against a home
//!   the caller cannot author (the shape service's authoring gate fires first) refuses
//!   `invalid_params` with the tool's sentence "Not authorized to declare shapes in
//!   this home: authoring authority required."
//!
//! # The wire's own answer surfaces the direct binding hides
//!
//! The declared-delta set, reconciled across the three sites (this header, the tool
//! modules' headers, and the register's beat G4 row — they agree):
//! - *finalize* — the expectation-mismatch Conflict renders the wire 409's
//!   `invalid_params` with the server's sentence (was the direct catch-all's
//!   `internal_error`); pinned at `ingest_finalize_wrong_expected_blocks_flips_at_the_swap`.
//! - *get_artifact / get_shape* — the absent faces flip from the direct 200-text
//!   postures to the flat route's 404 `invalid_params` with the server's sentence;
//!   pinned at `get_artifact_refuses_garbage_and_answers_absent` /
//!   `get_shape_absent_answers_the_not_found_posture`.
//! - *blobs* — the not-found prefixes (`blob_commit: ` / `blob_relate: `) drop for
//!   the server's bare sentence; pinned at `blob_commit_on_an_invisible_home_refuses_as_not_found`.
//! - *blob read* — the result's `content_hash` is now the collected bytes' own hash
//!   (the wire read carries no hash header); pinned at `blob_commit_then_read_round_trips_the_bytes`.
//! - *append* — the occupied-seq face is NOT a delta: both sides refuse
//!   `internal_error` (the route bridges the raise generically); pinned at
//!   `ingest_append_occupied_seq_with_different_bytes_refuses_internal_error`.
//! - A read on an invisible/absent resource (artifact list) answers `[]` on the direct
//!   path — the wire's read route answers the same way, so this face is carried.
//!
//! The register row (RELEASE_REGISTER.md, the beat G4 row) names the same set.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

mod parity {
    use serde::Deserialize;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// The harness principal's email — the one the relay's setup provisions and
    /// standing-approves, and the profile `direct_parts` resolves.
    pub const EMAIL: &str = "e2e@test.example.com";

    /// The L0 kernel cognitive map reserved id (birth migration `20260625000001`) —
    /// every approved profile READS it; nobody holds write without an explicit grant
    /// (`grant_cogmap_write`). Used as this suite's authoring-home-locked-down map.
    pub const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

    /// The default context the harness principal holds (auto-provisioned by
    /// `setup_relay`).
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

    /// The production parts shape for an ARBITRARY approved identity: the claims
    /// extension beside the bearer, exactly as the JWT middleware injects them. The
    /// door forwards on the bearer alone (since teardown nothing in-process reads the
    /// claims), so the claims ride for production fidelity, not need. Identity is
    /// consistent by construction: both halves come from the one `(token, sub, email)`
    /// triple.
    pub fn direct_parts_for(
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

    /// Provision a second approved identity with its own real token — the G3d
    /// idiom. Returns `(token, sub, email)`; the caller builds parts with
    /// `direct_parts_for`.
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

    /// Build a tool input from its WIRE shape, so the deserializer — not a struct
    /// literal — pins the field names an MCP caller actually sends.
    pub fn input<T: for<'de> Deserialize<'de>>(value: serde_json::Value) -> T {
        serde_json::from_value(value).expect("input deserializes from its wire shape")
    }

    /// The one text part of a one-part tool result.
    pub fn one_text(res: &rmcp::model::CallToolResult) -> serde_json::Value {
        let parts = &res.content;
        assert_eq!(parts.len(), 1, "one content part, got {}", parts.len());
        serde_json::from_str(parts[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response")
    }

    pub fn only_text(res: &rmcp::model::CallToolResult) -> String {
        let parts = &res.content;
        assert_eq!(parts.len(), 1, "one content part, got {}", parts.len());
        parts[0].as_text().expect("a text part").text.clone()
    }

    /// `rmcp::ErrorData` codes: -32600 INVALID_REQUEST (the detailed-authority 403s),
    /// -32602 invalid_params, -32603 internal_error.
    pub fn code_of(err: &rmcp::ErrorData) -> i32 {
        err.code.0
    }
}

use common::E2eTestApp;
use parity::{
    code_of, default_context_id, input, one_text, only_text, profile_id_by_email, second_identity,
};

/// The parity harness, once per test: the relay-ready app over this pool, the MCP
/// service (direct mode until the swap), and the harness principal's direct parts.
async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.direct_parts();
    (app, svc, parts)
}

// ── Drivers ────────────────────────────────────────────────────────────────────
//
// `(svc, parts, params)` signatures, byte-stable across the swap: pre-swap the one
// bridging line resolves the profile from parts the way service.rs's dispatch does;
// post-swap the same drivers hand the parts to the relayed tool.

async fn run_reblock(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::reblock::resource_reblock(
        svc,
        parts,
        input::<temper_mcp::tools::reblock::ResourceReblockInput>(params),
    )
    .await
}

async fn run_blob_read(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::blobs::blob_read(
        svc,
        parts,
        input::<temper_mcp::tools::blobs::BlobReadInput>(params),
    )
    .await
}

async fn run_blob_manage(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::blobs::blob_manage(
        svc,
        parts,
        input::<temper_mcp::tools::blobs::BlobManageInput>(params),
    )
    .await
}

async fn run_segmented_ingest(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::ingest::segmented_ingest(
        svc,
        parts,
        input::<temper_mcp::tools::ingest::SegmentedIngestInput>(params),
    )
    .await
}

async fn run_list_artifacts(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::data_artifacts::list_artifacts(
        svc,
        parts,
        input::<temper_mcp::tools::data_artifacts::ListArtifactsInput>(params),
    )
    .await
}

async fn run_get_artifact(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::data_artifacts::get_artifact(
        svc,
        parts,
        input::<temper_mcp::tools::data_artifacts::GetArtifactInput>(params),
    )
    .await
}

async fn run_commit_artifact(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::data_artifacts::commit_artifact(
        svc,
        parts,
        input::<temper_mcp::tools::data_artifacts::CommitArtifactInput>(params),
    )
    .await
}

async fn run_list_shapes(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::data_artifact_shapes::list_shapes(
        svc,
        parts,
        input::<temper_mcp::tools::data_artifact_shapes::ListShapesInput>(params),
    )
    .await
}

async fn run_get_shape(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::data_artifact_shapes::get_shape(
        svc,
        parts,
        input::<temper_mcp::tools::data_artifact_shapes::GetShapeInput>(params),
    )
    .await
}

async fn run_declare_shape(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::data_artifact_shapes::declare_shape(
        svc,
        parts,
        input::<temper_mcp::tools::data_artifact_shapes::DeclareShapeInput>(params),
    )
    .await
}

// ── resource_reblock ──────────────────────────────────────────────────────────

/// The per-arm requirement refusals are tool-door invalid_params — byte-exact.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reblock_requires_the_per_arm_ref(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let err = run_reblock(&svc, &parts, json!({"scope": "resource"}))
        .await
        .expect_err("scope=resource with no `resource` must refuse");
    assert_eq!(code_of(&err), -32602, "invalid_params");
    assert!(
        err.message.contains("scope=resource requires"),
        "the arm names the missing field: {}",
        err.message
    );

    let err = run_reblock(&svc, &parts, json!({"scope": "context"}))
        .await
        .expect_err("scope=context with no `context` must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("scope=context requires"),
        "the arm names the missing field: {}",
        err.message
    );
}

/// Garbage refs refuse at the parse callsite — `invalid_params`, never a 500.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reblock_garbage_refs_refuse_at_the_parse_callsite(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let err = run_reblock(
        &svc,
        &parts,
        json!({"scope": "resource", "resource": "not-a-uuid", "dry_run": true}),
    )
    .await
    .expect_err("a malformed resource ref must refuse");
    assert_eq!(code_of(&err), -32602, "invalid_params: {}", err.message);

    let err = run_reblock(
        &svc,
        &parts,
        json!({"scope": "context", "context": "bare-name-no-owner", "dry_run": true}),
    )
    .await
    .expect_err("a malformed context ref must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("invalid context ref"),
        "the arm names the parse failure: {}",
        err.message
    );
}

/// Every `context_anchor` refusal face, byte-exact, through `scope=context` — the same table
/// the context orientation suite pins (`common::context_anchor_faces`), so the two anchors
/// answer one dialect. Pinned green against the in-process resolver first, then carried through
/// the relay to `GET /api/contexts/resolve` unchanged (teardown).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_context_anchor_face_is_pinned_byte_exact(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    for face in common::context_anchor_faces(&app).await {
        let err = run_reblock(
            &svc,
            &face.parts,
            json!({"scope": "context", "context": face.context_ref, "dry_run": true}),
        )
        .await
        .expect_err(face.label);
        assert_eq!(code_of(&err), -32602, "{}: {err}", face.label);
        assert_eq!(err.message, face.expected, "{}", face.label);
    }
}

/// scope=context over the harness's own default context, dry_run: the answer is a
/// receipt (per-class counts present, cursor carried) — never an error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reblock_context_scope_dry_run_answers_a_receipt(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let value = one_text(
        &run_reblock(
            &svc,
            &parts,
            json!({"scope": "context", "context": "@me/default", "dry_run": true}),
        )
        .await
        .expect("scope=context dry_run answers a receipt"),
    );
    assert!(
        value.get("outcomes").is_some(),
        "the receipt carries the outcome rows: {value}"
    );
    assert!(
        value.get("correlation_id").is_some() && value.get("summary").is_some(),
        "the receipt carries its batch correlation id and per-class summary: {value}"
    );
}

/// scope=all by the harness's approved-but-never-system-admin principal refuses
/// INVALID_REQUEST with the tool's system-administrator sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reblock_all_scope_requires_system_admin(pool: PgPool) {
    let err = {
        let (_app, svc, parts) = harness(pool).await;
        run_reblock(&svc, &parts, json!({"scope": "all", "dry_run": true}))
            .await
            .expect_err("scope=all must require system-admin standing")
    };
    assert_eq!(code_of(&err), -32600, "INVALID_REQUEST: {}", err.message);
    assert!(
        err.message.contains("system-administrator"),
        "the refusal names the gate, in the mapper's own terms: {}",
        err.message
    );
}

// ── data_artifacts: list / get / commit ───────────────────────────────────────

/// A fresh resource answers an empty artifact set — the visibility-gated read
/// posture (never an error for the caller's own resource).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_artifacts_answers_the_caller_gated_set(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = seed_resource(&app).await;

    let value = one_text(
        &run_list_artifacts(&svc, &parts, json!({"resource_id": resource.to_string()}))
            .await
            .expect("list on an owned resource answers"),
    );
    assert_eq!(value, json!([]), "a fresh resource has no artifacts");
}

/// A garbage resource ref refuses invalid_params at the parse callsite.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_artifacts_garbage_resource_ref_refuses(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_list_artifacts(&svc, &parts, json!({"resource_id": "garbage!"}))
        .await
        .expect_err("a malformed resource ref must refuse");
    assert_eq!(code_of(&err), -32602);
}

/// commit → get round-trip: the committed content answers byte-identical through
/// the tool's get, and the act envelope (confidence etc.) rides the request.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn commit_then_get_artifact_round_trips_the_payload(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = seed_resource(&app).await;

    let committed = one_text(
        &run_commit_artifact(
            &svc,
            &parts,
            json!({
                "resource_id": resource.to_string(),
                "kind": "measurement",
                "intent": "current",
                "content": {"reading": 11.5, "unit": "celsius"},
                "confidence": "confident",
                "reasoning": "the sensor is calibrated"
            }),
        )
        .await
        .expect("commit against an owned resource answers the response"),
    );
    let artifact_id = committed["artifact_id"]
        .as_str()
        .expect("the response carries artifact_id")
        .to_string();
    assert_eq!(
        committed["artifact"]["content"],
        json!({"reading": 11.5, "unit": "celsius"}),
        "the committed payload answers in the response"
    );

    let got = one_text(
        &run_get_artifact(&svc, &parts, json!({"artifact_id": artifact_id}))
            .await
            .expect("get of a live artifact answers"),
    );
    assert_eq!(got["content"], json!({"reading": 11.5, "unit": "celsius"}));
    assert_eq!(got["is_folded"], json!(false));
}

/// An unparseable artifact ref refuses at the door; an absent-but-well-formed one
/// answers the 200-text not-found posture — the direct face, flipped at the swap.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_artifact_refuses_garbage_and_answers_absent(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let err = run_get_artifact(&svc, &parts, json!({"artifact_id": "nope!"}))
        .await
        .expect_err("a malformed artifact ref must refuse");
    assert_eq!(code_of(&err), -32602);

    // FLIPPED DELTA (the flat route's 404 posture): an absent artifact refuses
    // `invalid_params` carrying the server's own sentence — the direct binding's
    // 200-text posture is gone with the door.
    let err = run_get_artifact(
        &svc,
        &parts,
        json!({"artifact_id": Uuid::nil().to_string()}),
    )
    .await
    .expect_err("an absent artifact refuses at the door");
    assert_eq!(code_of(&err), -32602, "invalid_params: {}", err.message);
    assert!(
        err.message.contains("artifact not found"),
        "the server's own sentence: {}",
        err.message
    );
}

/// An unparseable confidence value refuses at the envelope-assembly callsite —
/// `invalid_params`, naming the accepted vocabulary.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn commit_artifact_bad_confidence_refuses_at_the_envelope(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = seed_resource(&app).await;
    let err = run_commit_artifact(
        &svc,
        &parts,
        json!({
            "resource_id": resource.to_string(),
            "kind": "measurement",
            "intent": "current",
            "content": {"v": 1},
            "confidence": "maybe-probably"
        }),
    )
    .await
    .expect_err("an unknown confidence must refuse at the door");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("unrecognized confidence"),
        "the arm names the accepted vocabulary: {}",
        err.message
    );
}

/// The read-on-a-resource-the-outsider-cannot-see face: a list scoped to it answers
/// `[]` — the visibility-gated set, never a leak of the row's existence.
///
/// The read-but-not-author commit face (the tool's write-access sentence against a
/// home the caller can READ but not author) needs a shared-team construction —
/// NAMED, not pinned here; the suite header names it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_outsiders_artifact_list_contains_nothing_they_cannot_see(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let (token2, _sub2, _email2) = second_identity(&app, &app.pool, "ro-author").await;
    let second_parts = parity::direct_parts_for(&app, &token2, &_sub2, &_email2);

    // The harness principal seeds a resource in ITS default context — a home the
    // outsider cannot read at all.
    let resource = seed_resource(&app).await;
    let _committed = one_text(
        &run_commit_artifact(
            &svc,
            &parts,
            json!({
                "resource_id": resource.to_string(),
                "kind": "measurement",
                "intent": "current",
                "content": {"v": 1}
            }),
        )
        .await
        .expect("seed commit against an owned resource answers"),
    );

    // The outsider's list on that resource id answers the empty set — visibility is
    // gated on `resources_visible_to`, and the refusal posture is `[]`, not a 404
    // that would disclose the row's existence.
    let value = one_text(
        &run_list_artifacts(
            &svc,
            &second_parts,
            json!({"resource_id": resource.to_string()}),
        )
        .await
        .expect("a visibility-gated list answers"),
    );
    assert_eq!(
        value,
        json!([]),
        "the outsider's view is empty, never a leak"
    );
}

// ── data_artifact_shapes: list / get / declare ────────────────────────────────

/// list_shapes over the harness's own default context answers the caller's gated
/// set — a fresh context has none.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_shapes_answers_the_gated_set(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let value = one_text(
        &run_list_shapes(
            &svc,
            &parts,
            json!({"home_type": "context", "home_id": ctx.to_string()}),
        )
        .await
        .expect("list on an owned home answers"),
    );
    assert_eq!(value, json!([]), "a fresh home declares no shapes");
}

/// A garbage home_type refuses at the vocabulary callsite — `invalid_params`
/// naming the two-kinded grammar.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_shapes_garbage_home_kind_refuses(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_list_shapes(
        &svc,
        &parts,
        json!({"home_type": "kb_teams", "home_id": Uuid::nil().to_string()}),
    )
    .await
    .expect_err("an unrecognized home_type must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("unrecognized home_type"),
        "the arm names the grammar: {}",
        err.message
    );
}

/// declare → get reads back the declared shape, homed on the context the harness
/// owns. The direct face: the act fields are accepted and DROPPED (filed
/// 01a0e2f0-5bdc-7b00-94d2-0fbf9d141df0 — the declare act-drop defect); the
/// witness's assertion stands across the swap (the wire can carry act but both
/// doors drop it today, so parity holds with the defect inherited).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn declare_then_get_shape_round_trips_on_the_owning_home(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    // The kind_owner omission defaults from a resource homed in that anchor — the
    // harness seeds one (the face the refusal names when no seed exists).
    let _ = seed_resource(&app).await;

    let declared = one_text(
        &run_declare_shape(
            &svc,
            &parts,
            json!({
                "home_type": "context",
                "home_id": ctx.to_string(),
                "kind": "measurement",
                "schema": {"type": "object", "properties": {"v": {"type": "number"}}, "required": ["v"]},
                "enforcement": "advisory",
                "confidence": "confident"
            }),
        )
        .await
        .expect("declare on an owned home answers the shape"),
    );
    let shape_id = declared["shape_id"].as_str().expect("shape_id").to_string();
    assert_eq!(declared["home_anchor_table"], json!("kb_contexts"));

    let got = one_text(
        &run_get_shape(&svc, &parts, json!({"shape_id": shape_id}))
            .await
            .expect("get reads the declared shape back"),
    );
    assert_eq!(got["shape_id"].as_str().unwrap(), shape_id);
    assert_eq!(got["artifact_kind"], json!("measurement"));
}

/// An absent shape refuses at the door — FLIPPED DELTA: the direct binding's
/// 200-text posture is gone with the route's 404, which renders `invalid_params`
/// carrying the server's own sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_shape_absent_answers_the_not_found_posture(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let err = run_get_shape(&svc, &parts, json!({"shape_id": Uuid::nil().to_string()}))
        .await
        .expect_err("an absent shape refuses at the door");
    assert_eq!(code_of(&err), -32602, "invalid_params: {}", err.message);
    assert!(
        err.message.contains("shape not found"),
        "the server's own sentence: {}",
        err.message
    );
}

/// Declaring into a home the caller can read but not author refuses with the
/// tool's own requirement sentence — the gate fires in the service, before any
/// write.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn declare_shape_on_a_non_authorable_home_refuses_with_the_sentence(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    // The L0 kernel map: every approved profile READS it; write needs a grant.
    let err = run_declare_shape(
        &svc,
        &parts,
        json!({
            "home_type": "cogmap",
            "home_id": parity::L0_COGMAP.to_string(),
            "kind": "measurement",
            "schema": {"type": "object"},
            "enforcement": "advisory"
        }),
    )
    .await
    .expect_err("declare on a read-only home must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message
            .contains("Not authorized to declare shapes in this home"),
        "the refusal is the tool's requirement sentence: {}",
        err.message
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// The harness principal's own default context, a fresh resource homed in it —
/// the seed every artifact/shapes test writes against. Goes through the API
/// (the real route, the caller's real token) so the seeded row is production-true.
async fn seed_resource(app: &E2eTestApp) -> Uuid {
    let ctx = default_context_id(&app.pool).await;
    let resource = app
        .client
        .resources()
        .create(&temper_workflow::types::resource::ResourceCreateRequest {
            kb_context_id: ctx,
            idempotency_key: Some(Uuid::now_v7()),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/g4-parity".to_string(),
            title: format!("G4 parity seed {}", Uuid::now_v7()),
            act: Default::default(),
        })
        .await
        .expect("resource seed through the API");
    resource.id.into()
}

// ── blob_read / blob_manage ───────────────────────────────────────────────────
//
// These harnesses carry a live in-memory blob store (single_request_max_bytes: 64,
// so the read ceiling is cheap to construct); the app listener carries the shared
// blob config (so the swap's relayed calls land on an app whose own blob routes are
// enabled).

/// The blob harness: the blob-store-enabled app, the blob-carrying MCP service
/// sharing ONE in-memory store with the test (the fixture insert door for the
/// ceiling pin), the harness principal's direct parts.
async fn blob_harness(
    pool: PgPool,
) -> (
    E2eTestApp,
    TemperMcpService,
    axum::http::request::Parts,
    std::sync::Arc<temper_substrate::blob_store::InMemoryBlobStore>,
) {
    let store = std::sync::Arc::new(temper_substrate::blob_store::InMemoryBlobStore::default());
    let app = common::setup_with_blob_store_shared(pool, store.clone(), 64).await;
    let svc = app
        .mcp_relay_service_with_blob(app.pool.clone(), store.clone())
        .await;
    let parts = app.direct_parts();
    (app, svc, parts, store)
}

/// Commit one blob through the tool, answering its id. The harness principal owns
/// the home (its default context), so the gate is a no-reach here.
async fn deploy_blob(
    app: &E2eTestApp,
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    bytes: &[u8],
) -> Uuid {
    let ctx = default_context_id(&app.pool).await;
    let encoded = base64::engine::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
    let value = one_text(
        &run_blob_manage(
            svc,
            parts,
            json!({
                "action": "commit",
                "home_table": "kb_contexts",
                "home_id": ctx.to_string(),
                "content_type": "application/pdf",
                "content": encoded,
            }),
        )
        .await
        .expect("commit a blob against an owned home"),
    );
    value["blob_id"]
        .as_str()
        .expect("the response carries blob_id")
        .parse()
        .expect("a uuid")
}

/// Commit → read round-trip: the bytes come back base64 under their stored media
/// type, `content_hash` is the bare sha256 of the bytes — the
/// blob-bytes-retrievable-whole contract rides the tool too.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn blob_commit_then_read_round_trips_the_bytes(pool: PgPool) {
    let (app, svc, parts, _store) = blob_harness(pool).await;
    let payload: &[u8] = b"hello blob world";
    let blob_id = deploy_blob(&app, &svc, &parts, payload).await;

    let read = one_text(
        &run_blob_read(
            &svc,
            &parts,
            json!({"action": "read", "blob_id": blob_id.to_string()}),
        )
        .await
        .expect("read a blob the caller can see"),
    );
    assert_eq!(read["content_bytes"], json!(16));
    assert_eq!(
        read["content_type"],
        json!("application/pdf"),
        "the STORED media type answers (N2: the first committer's on a dedup hit)"
    );
    let decoded: Vec<u8> = base64::engine::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        read["content_base64"].as_str().expect("base64 payload"),
    )
    .expect("content decodes from base64");
    assert_eq!(decoded, payload, "the bytes round-trip whole");
}

/// The requirement arms are tool-door invalid_params.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn blob_read_requires_blob_id_and_blob_manage_requires_its_fields(pool: PgPool) {
    let (_app, svc, parts, _store) = blob_harness(pool).await;

    let err = run_blob_read(&svc, &parts, json!({"action": "read"}))
        .await
        .expect_err("read without blob_id must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("read requires `blob_id`"),
        "the arm names the missing field: {}",
        err.message
    );

    let err = run_blob_manage(&svc, &parts, json!({"action": "commit"}))
        .await
        .expect_err("commit without home_table must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("commit requires `home_table`"),
        "the arm names the missing field: {}",
        err.message
    );

    let err = run_blob_manage(
        &svc,
        &parts,
        json!({"action": "relate", "blob_id": Uuid::nil().to_string()}),
    )
    .await
    .expect_err("relate without peer_table must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("relate requires `peer_table`"),
        "the arm names the missing field: {}",
        err.message
    );
}

/// The read ceiling refuses over the single-request threshold, naming the bytes,
/// the ceiling, and the streaming doors — byte-stable through the swap (the
/// sentence and the numbers are the tool's own). The over-threshold blob is seeded
/// directly (row + store pathname) because the commit door enforces the same
/// threshold — the ceiling is a READ refusal, not a commit one (ruled in-session).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn blob_read_refuses_over_the_single_request_ceiling(pool: PgPool) {
    let (app, svc, parts, store) = blob_harness(pool).await;
    let bytes: std::collections::VecDeque<u8> =
        (0..200u32).map(|i| b'a' + (i % 26) as u8).collect();
    let bytes: Vec<u8> = bytes.into();
    let pathname = format!("parity/{}", Uuid::new_v4());
    let blob_id: Uuid = {
        let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
            .fetch_one(&app.pool)
            .await
            .expect("the migration-seeded event");
        sqlx::query_scalar(
            "INSERT INTO kb_blobs \
               (id, content_hash, blob_pathname, content_type, content_bytes, \
                home_table, home_id, owner_profile_id, originator_profile_id, \
                asserted_by_event_id, last_event_id) \
             VALUES ($1, $2, $3, 'text/plain', 200, 'kb_contexts', $4, $5, $5, $6, $6) \
             RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(temper_core::hash::sha256_hex(&bytes))
        .bind(&pathname)
        .bind(default_context_id(&app.pool).await)
        .bind(profile_id_by_email(&app.pool, parity::EMAIL).await)
        .bind(ev)
        .fetch_one(&app.pool)
        .await
        .expect("insert the over-threshold fixture blob")
    };
    // The fixture's bytes go in through the store's own put — the read route's
    // stream must match its declared Content-Length or the wire aborts mid-response
    // (which would fail the request before the tool's ceiling check could run).
    {
        use temper_substrate::blob_store::BlobStore as _;
        store
            .put(&pathname, "application/pdf", bytes.clone().into(), 0)
            .await
            .expect("seed the fixture bytes");
    }

    let err = run_blob_read(
        &svc,
        &parts,
        json!({"action": "read", "blob_id": blob_id.to_string()}),
    )
    .await
    .expect_err("a blob over the read ceiling refuses");
    assert_eq!(code_of(&err), -32602, "invalid_params: {}", err.message);
    assert!(
        err.message
            .contains("against a blob_read ceiling of 64 bytes"),
        "the refusal names the ceiling: {}",
        err.message
    );
    assert!(
        err.message.contains("this blob is 200 bytes"),
        "the refusal names the blob's size: {}",
        err.message
    );
    assert!(
        err.message.contains("which stream"),
        "the refusal points at the streaming doors: {}",
        err.message
    );
}

/// The disclosure distinction the mapper's doc comment names: commit/read against
/// a home the caller cannot see answers invalid_params with the service's
/// prefixed not-found sentence, not the authority 403 — the invisible home is the
/// absent home.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn blob_commit_on_an_invisible_home_refuses_as_not_found(pool: PgPool) {
    let (app, svc, _parts, _store) = blob_harness(pool).await;
    let (token2, _sub2, _email2) = second_identity(&app, &app.pool, "blob-outsider").await;
    let outsider_parts = parity::direct_parts_for(&app, &token2, &_sub2, &_email2);

    let encoded = base64::engine::Engine::encode(&base64::engine::general_purpose::STANDARD, b"x");
    let err = run_blob_manage(
        &svc,
        &outsider_parts,
        json!({
            "action": "commit",
            "home_table": "kb_contexts",
            "home_id": default_context_id(&app.pool).await.to_string(),
            "content_type": "application/pdf",
            "content": encoded,
        }),
    )
    .await
    .expect_err("commit against a home the outsider cannot read must refuse");
    assert_eq!(code_of(&err), -32602);
    // FLIPPED DELTA: the direct map's `blob_commit: ` prefix drops — the door's
    // sentence arrives bare (kind and gate identical).
    assert!(
        err.message.contains("home not found"),
        "the server's own sentence, bare: {}",
        err.message
    );
}

/// Relate across a blob and a resource answers the ack; the peer-table vocabulary
/// refusal names the accepted table.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn blob_relate_round_trip_and_peer_vocabulary(pool: PgPool) {
    let (app, svc, parts, _store) = blob_harness(pool).await;
    let blob_id = deploy_blob(&app, &svc, &parts, b"figure bytes").await;
    let resource = seed_resource(&app).await;

    let ack = one_text(
        &run_blob_manage(
            &svc,
            &parts,
            json!({
                "action": "relate",
                "blob_id": blob_id.to_string(),
                "peer_table": "kb_resources",
                "peer_id": resource.to_string(),
                "edge_kind": "express",
                "polarity": "forward",
                "label": "figure_of",
                "weight": 1.0,
                "confidence": "confident",
                "reasoning": "the figure renders this data"
            }),
        )
        .await
        .expect("relate against owned endpoints answers the ack"),
    );
    assert!(
        ack.get("edge_handle").is_some() || ack.get("edge_id").is_some() || ack.get("id").is_some(),
        "the relate ack carries its identity: {ack}"
    );

    let err = run_blob_manage(
        &svc,
        &parts,
        json!({
            "action": "relate",
            "blob_id": blob_id.to_string(),
            "peer_table": "kb_cogmaps",
            "peer_id": parity::L0_COGMAP.to_string(),
            "edge_kind": "express",
            "polarity": "forward",
            "label": "figure_of",
            "weight": 1.0
        }),
    )
    .await
    .expect_err("a peer_table outside the vocabulary must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.starts_with("blob_relate: "),
        "the peer vocabulary refusal carries the action prefix: {}",
        err.message
    );
}

// ── segmented_ingest ──────────────────────────────────────────────────────────

/// Sha256 of a string as bare hex — the per-segment transit check this surface
/// verifies.
fn seg_hash(content: &str) -> String {
    temper_core::hash::sha256_hex(content.as_bytes())
}

/// Begin one segmented ingest against the harness's default context, answering the
/// resource ref for later lifecycle steps. Server-computed embeddings defer to the
/// async drain (TEMPER_ASYNC_EMBED=1 — nextest runs each test in its own process,
/// so the env is test-scoped): the lifecycle faces under test never assert a vector.
async fn begin_ingest(
    app: &E2eTestApp,
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    segment0: &str,
) -> String {
    std::env::set_var("TEMPER_ASYNC_EMBED", "1");
    let ctx = default_context_id(&app.pool).await;
    let value = one_text(
        &run_segmented_ingest(
            svc,
            parts,
            json!({
                "action": "begin",
                "context_ref": ctx.to_string(),
                "doc_type_name": "research",
                "title": format!("G4 segmented parity {}", Uuid::now_v7()),
                "content": segment0,
                "content_hash": seg_hash(segment0),
            }),
        )
        .await
        .expect("begin lands segment 0 and creates the resource"),
    );
    value["resource_id"]
        .as_str()
        .or_else(|| value["id"].as_str())
        .expect("the begin response carries the resource identity")
        .to_string()
}

/// The consolidated tool's required-field arms: begin/append/finalize/blocks each
/// refuse their missing per-action fields at the door with byte-exact sentences.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn segmented_ingest_dispatches_its_required_field_arms(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let err = run_segmented_ingest(&svc, &parts, json!({"action": "begin"}))
        .await
        .expect_err("begin with no create fields must refuse");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("begin requires create fields"),
        "the dispatch arm: {}",
        err.message
    );

    let err = run_segmented_ingest(
        &svc,
        &parts,
        json!({"action": "append", "seq": 1, "content": "x", "content_hash": "y"}),
    )
    .await
    .expect_err("append with no resource must refuse");
    assert!(
        err.message.contains("append requires `resource`"),
        "the dispatch arm: {}",
        err.message
    );

    let err = run_segmented_ingest(
        &svc,
        &parts,
        json!({"action": "finalize", "resource": Uuid::nil().to_string()}),
    )
    .await
    .expect_err("finalize with no expected_blocks must refuse");
    assert!(
        err.message.contains("finalize requires `expected_blocks`"),
        "the dispatch arm: {}",
        err.message
    );

    let err = run_segmented_ingest(&svc, &parts, json!({"action": "blocks"}))
        .await
        .expect_err("blocks with no resource must refuse");
    assert!(
        err.message.contains("blocks requires `resource`"),
        "the dispatch arm: {}",
        err.message
    );
}

/// begin's surface-side integrity check: a content_hash that does not match the
/// segment's content refuses before anything lands.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_begin_refuses_a_mismatched_content_hash(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let err = run_segmented_ingest(
        &svc,
        &parts,
        json!({
            "action": "begin",
            "context_ref": ctx.to_string(),
            "doc_type_name": "research",
            "title": "G4 hash-mismatch",
            "content": "segment zero",
            "content_hash": "0".repeat(64),
        }),
    )
    .await
    .expect_err("a mismatched content_hash refuses at the surface");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("content_hash does not match content"),
        "the surface-side check's sentence: {}",
        err.message
    );
}

/// begin with an empty segment-0 content refuses at the surface.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_begin_refuses_empty_content(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let err = run_segmented_ingest(
        &svc,
        &parts,
        json!({
            "action": "begin",
            "context_ref": ctx.to_string(),
            "doc_type_name": "research",
            "title": "G4 empty-content",
            "content": "",
            "content_hash": seg_hash(""),
        }),
    )
    .await
    .expect_err("empty segment 0 refuses");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("ingest_begin requires content"),
        "the surface-side check: {}",
        err.message
    );
}

/// The full lifecycle through the tool: begin → append → blocks resume read →
/// finalize — the body's last sentence answering the landed state.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn segmented_ingest_full_lifecycle_through_the_tool(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = begin_ingest(&app, &svc, &parts, "segment zero content.").await;

    // append seq 1 with a matching hash.
    let seq1 = "segment one content.";
    let appended = one_text(
        &run_segmented_ingest(
            &svc,
            &parts,
            json!({
                "action": "append",
                "resource": resource,
                "seq": 1,
                "content": seq1,
                "content_hash": seg_hash(seq1),
            }),
        )
        .await
        .expect("append seq 1 lands"),
    );
    assert!(
        appended.get("blocks").is_some() || appended.get("landed").is_some(),
        "the append answers the landed set: {appended}"
    );

    // Idempotent re-append: the same bytes at the same seq is a no-op answer, never
    // a refusal (the wire contract says re-sends converge).
    one_text(
        &run_segmented_ingest(
            &svc,
            &parts,
            json!({
                "action": "append",
                "resource": resource,
                "seq": 1,
                "content": seq1,
                "content_hash": seg_hash(seq1),
            }),
        )
        .await
        .expect("an idempotent re-append is a no-op, not an error"),
    );

    // blocks — the resume read after an assumed interruption.
    let blocks = one_text(
        &run_segmented_ingest(
            &svc,
            &parts,
            json!({"action": "blocks", "resource": resource}),
        )
        .await
        .expect("blocks answers the landed set"),
    );
    assert!(
        blocks["expected_blocks"].as_i64().unwrap_or(0) == 2
            || blocks["blocks"].as_array().map(|b| b.len()).unwrap_or(0) == 2,
        "the resume read answers both landed segments: {blocks}"
    );

    // finalize with the truthful pair — the opaque body_hash echoes back verbatim.
    let body_hash = blocks["body_hash"]
        .as_str()
        .expect("the blocks response carries body_hash to echo")
        .to_string();
    let final_text = only_text(
        &run_segmented_ingest(
            &svc,
            &parts,
            json!({
                "action": "finalize",
                "resource": resource,
                "expected_blocks": 2,
                "expected_body_hash": body_hash,
            }),
        )
        .await
        .expect("finalize answers the success text"),
    );
    assert!(
        final_text.starts_with("Finalized "),
        "finalize's success text: {final_text}"
    );
}

/// Append with a content_hash that does not match its segment refuses with the
/// BACKEND's sentence (the surface's own check guards begin only — the append path
/// verifies at the door-side service): "content_hash mismatch for seq N: declared
/// X, computed Y", invalid_params, bare.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_append_refuses_a_mismatched_content_hash_at_the_backend(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = begin_ingest(&app, &svc, &parts, "segment zero.").await;

    let err = run_segmented_ingest(
        &svc,
        &parts,
        json!({
            "action": "append",
            "resource": resource,
            "seq": 1,
            "content": "segment one.",
            "content_hash": "0".repeat(64),
        }),
    )
    .await
    .expect_err("a mismatched append hash refuses");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("content_hash mismatch for seq 1"),
        "the backend's own sentence rides through: {}",
        err.message
    );
}

/// An occupied seq refusing on re-write with different bytes: the door's face is
/// an INTERNAL ERROR with the scrubbed server sentence — the wire's append route
/// maps the raise through the generic error bridge (no Conflict arm on the route,
/// the direct binding's `api_err` twin), so the class is STABLE across the swap
/// (both sides refuse internal_error) and only the message's shape changes: the
/// raw SQL raise on the direct side, the API's scrubbed 500 body through the door.
/// NOT a declared delta — a stable-class face with a message-shape delta; named
/// here because the blob-upload append's 409 docs could misread this arm. The
/// route-level gap (the raise never typed as Conflict at the append door) is
/// the wire's own pre-existing posture, unchanged by this beat.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_append_occupied_seq_with_different_bytes_refuses_internal_error(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = begin_ingest(&app, &svc, &parts, "segment zero.").await;
    let seq1 = "segment one, first write.";
    let _ = run_segmented_ingest(
        &svc,
        &parts,
        json!({
            "action": "append",
            "resource": resource,
            "seq": 1,
            "content": seq1,
            "content_hash": seg_hash(seq1),
        }),
    )
    .await
    .expect("first write of seq 1 lands");

    let different = "segment one, REWRITTEN (source changed).";
    let err = run_segmented_ingest(
        &svc,
        &parts,
        json!({
            "action": "append",
            "resource": resource,
            "seq": 1,
            "content": different,
            "content_hash": seg_hash(different),
        }),
    )
    .await
    .expect_err("an occupied-seq rewrite refuses");
    // STABLE CLASS through the swap: the direct catch-all's internal_error is the
    // door's internal error too — the wire's append route bridges the raise
    // generically (scrubbed 500 body), never a typed 409. The message shape is the
    // only delta: raw raise direct, scrubbed sentence through the door.
    assert_eq!(code_of(&err), -32603, "internal_error: {}", err.message);
    assert!(
        err.message.contains("ingest_append"),
        "the door's rendering names the tool: {}",
        err.message
    );
}

/// **DECLARED PARITY DELTA** — a wrong expected_blocks at finalize: direct
/// rendering is `internal_error` prefixed `ingest_finalize: Conflict: …`; the wire
/// answers 409 -> `invalid_params` with the server's sentence. Flips at the swap.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_finalize_wrong_expected_blocks_flips_at_the_swap(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = begin_ingest(&app, &svc, &parts, "segment zero.").await;
    let blocks = one_text(
        &run_segmented_ingest(
            &svc,
            &parts,
            json!({"action": "blocks", "resource": resource}),
        )
        .await
        .expect("resume read"),
    );
    let body_hash = blocks["body_hash"].as_str().expect("body_hash").to_string();

    let err = run_segmented_ingest(
        &svc,
        &parts,
        json!({
            "action": "finalize",
            "resource": resource,
            "expected_blocks": 99,
            "expected_body_hash": body_hash,
        }),
    )
    .await
    .expect_err("a wrong expected_blocks refuses");
    // FLIPPED DELTA: the direct catch-all's internal_error renders the door's 409 —
    // invalid_params with the server's own sentence, prefix and label stripped.
    assert_eq!(code_of(&err), -32602, "invalid_params: {}", err.message);
    assert!(
        err.message.contains("has 1 live blocks, expected 99"),
        "the server's own sentence: {}",
        err.message
    );
}
