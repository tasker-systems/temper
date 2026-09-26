#![cfg(feature = "test-db")]
//! The ledger + graph family's parity suite, authored against the DIRECT binding
//! first (beat G3c — the third proof of the G3a-prime pattern).
//!
//! [Beat G3b](temper task: the `search_query_parity_test.rs` header) carried the G3a
//! discipline to search + query; this file applies it to the four ledger/graph tool
//! families — element_trail, facets, relationships, citation_audits — before the
//! migration touches them: a parity suite that never saw the old binding cannot prove
//! parity. At the swap the tools crossed the network door and the driving changed with
//! them — each tool now builds the per-request relay from its request's `Parts`, so the
//! drivers (signatures byte-stable) pass those parts through and identity rides the
//! bearer alone: the harness principal via `relay_parts`, a second identity via its OWN
//! `relay_parts_for(token)`, the API adjudicating each bearer.
//!
//! # The refusal faces, named before they are witnessed (learning 4)
//!
//! **element_trail**
//! - *Unreadable / nonexistent element* — `element_trail_node` /
//!   `element_trail_edge` gate visibility INSIDE the SQL (`resources_visible_to`,
//!   `anchor_readable` + `endpoint_readable`), so an element the caller cannot see
//!   answers a 200 with an EMPTY trail, never an error (leak-safe by design —
//!   `crates/temper-mcp/src/tools/trail.rs:1-7`). Witnessed for a nonexistent node
//!   and for a second identity reading an invisible resource.
//! - *Garbage ref* — `parse_ref` failure renders `invalid_params` at the parse
//!   callsite, prefixed `bad element ref:` (`trail.rs:76-78`), never reaching the
//!   service.
//!
//! **facets**
//! - *`property_key` on target=resource* — the unified set tool's naming refusal:
//!   `invalid_params` with the "property_key applies to target=edge only" sentence
//!   (`crates/temper-mcp/src/tools/facets.rs:304-310`).
//! - *facet_retract target=resource* — the naming refusal: `invalid_params` with the
//!   "applies to target=edge only" sentence
//!   (`facets.rs:448-454`).
//! - *Authority, terse arm* — `set_facet`'s resource arm gates on
//!   `can_modify_resource` (`db_backend.rs:3025` → `check_can_modify_next:730-744`),
//!   whose deny is bare `Forbidden`; the tool renders the terse INVALID_REQUEST
//!   sentence "cannot modify this resource" (`facets.rs:64-68`).
//! - *Authority, detailed arm* — an edge facet's container clause dispatches to
//!   `check_cogmap_authorable`
//!   (`db_backend.rs:1075,1091`), which splits its deny by read standing: a caller
//!   who READS the map but holds no write grant gets `ForbiddenDetail` carrying the
//!   gate's own sentence (`db_backend.rs:107-112`), rendered INVALID_REQUEST
//!   (`facets.rs:59-63`). The scenario this suite constructs: an edge whose source is
//!   kernel-homed in L0, called by a second identity holding a DIRECT write grant on
//!   the source resource (clause 1 passes via the `kb_access_grants` arm of
//!   `can_modify_resource`) plus only team-reach READ on L0 (auto-join watcher, the
//!   `kb_team_cogmaps` binding the L0 migration seeds).
//! - *Re-retract* — the projector's zero-rows outcome types as
//!   `PropertyRetractError` → `NotFound`
//!   (`db_backend.rs:83-84`, message from
//!   `temper-substrate/src/writes.rs:485-491`), rendered `invalid_params`
//!   (`facets.rs:53-55`) — one indistinguishable shape for foreign, missing, and
//!   already-retracted, never an existence oracle over property rows.
//!
//! **relationships**
//! - *Unknown edge handle* — `check_edge_mutable`'s row lookup renders
//!   `NotFound("edge {id} not found")` (`db_backend.rs:985-995`) on retype, reweight,
//!   fold, and the edge-facet write alike → `invalid_params`
//!   (`relationships.rs:104-106`).
//! - *Act correlation, unknown invocation* — `check_act_invocation`'s
//!   absent-or-unreadable arm is a uniform `NotFound` (`db_backend.rs:2004`) →
//!   `invalid_params` naming the id.
//! - *Act correlation, closed invocation* — the non-open arm is `Conflict`
//!   (`db_backend.rs:2005-2011`); the direct binding's `map_err`
//!   (`relationships.rs:102-122`) has NO Conflict arm, so it falls to the
//!   `internal_error` catch-all. **DECLARED PARITY DELTA CANDIDATE**: the door
//!   renders the API's 409 through temper-client, whose conflict body the relayed
//!   call site may map differently — the swap must re-derive this face and name the
//!   outcome either way.
//! - *Authority on assert* — clause 1 `check_can_modify_next(src)`
//!   (`db_backend.rs:1219`) denies with bare `Forbidden` → the terse INVALID_REQUEST
//!   sentence (`relationships.rs:115-119`).
//!
//! **citation_audits**
//! - *The three-cause equivalence class* — unknown block, unreadable finding, and
//!   self-authored render ONE fixed sentence, because `authz::audit_gate`'s
//!   `FINDING_REFUSAL` is one string by construction
//!   (`audit_gate.rs:139`, both denial arms → `NotFound`) and the tool's NotFound arm
//!   deliberately does NOT carry the service message — post-swap it renders its OWN
//!   fixed sentence keyed on the 404 status, never the body. Pinned byte-exact.
//! - *Remote-kind source* — the caller-fault guard at the command boundary
//!   (`db_backend.rs:2909-2916`) → the API's 400, the server's own sentence passed
//!   through bare (`citation_audits.rs`'s 400 arm).
//! - *Out-of-range value* — the range guard
//!   (`db_backend.rs:2896-2901`) → the API's 400, same pass-through.
//! - *Happy* — a gate-admitted auditor (readable finding, did not author) records
//!   the audit; the response is the BARE audit Uuid, no wrapping ack.
//!
//! # The declared parity deltas (flipped at the swap, named here and in the tool
//! files' parity-delta sections)
//!
//! - **Closed-invocation 409** — the DIRECT binding's `map_err` had no `Conflict`
//!   arm, so the act gate's non-open refusal rendered `internal_error`. The wire's
//!   409 is caller-actionable (the run the correlation claim names is closed), so
//!   the door renders it `invalid_params` with the server's own sentence — the
//!   `contexts.rs::map_api_error` Conflict arm's precedent. The suite pinned the
//!   direct face before the swap and pins the door's face now.
//! - **NotFound prefixes** — the DIRECT maps prefixed every not-found with
//!   `{action}: `; the door carries the server's own sentence un-prefixed (the
//!   resources family's precedent: the door does not re-apply prefixes the direct
//!   tool applied — kind and gate identical). The suite's 404 assertions assert by
//!   `contains`, so they carry green across the prefix drop while the delta stays
//!   named. The audit's fixed sentence is exempt — the tool renders its own.
//!
//! # How the tools are driven
//!
//! Through the tool functions over relayed parts — the same hop the deployed
//! relay makes: a real `TemperMcpService` whose relay config points at THIS
//! process's listener, per-request parts carrying each principal's REAL bearer
//! that the API's own auth middleware adjudicates (identity rides the bearer
//! alone; a second identity is its own parts, the G3b swap's idiom).

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

mod parity {
    use serde::Deserialize;
    use serde_json::json;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// The harness principal's email — the one `approve_app_principal` provisions and
    /// standing-approves, and the profile the seeded cache resolves.
    pub const EMAIL: &str = "e2e@test.example.com";

    /// The L0 kernel cognitive map reserved id (birth migration `20260625000001`) —
    /// the invocation front door's originating map, and the kernel home this suite
    /// builds its ForbiddenDetail scenario against.
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

    /// A SECOND approved identity, warmed through the real listener (JIT
    /// provisioning with the correct handle, per-surface emitters, and its own
    /// default context) and standing-approved by its own email — the G3a idiom
    /// (`resources_parity_test.rs::second_identity`), returning what the door's
    /// identity swap needs: the bearer its own parts carry.
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

    /// The single text part a one-part tool result carries.
    pub fn one_text(res: &rmcp::model::CallToolResult) -> serde_json::Value {
        let parts = &res.content;
        assert_eq!(parts.len(), 1, "one content part, got {}", parts.len());
        serde_json::from_str(parts[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response")
    }

    /// `rmcp::ErrorData` codes: -32600 request error (INVALID_REQUEST terminal
    /// authority arms), -32602 invalid_params, -32603 internal_error.
    pub fn code_of(err: &rmcp::ErrorData) -> i32 {
        err.code.0
    }

    pub fn facet_body(resource: &str, values: serde_json::Value) -> serde_json::Value {
        json!({ "resource": resource, "values": values })
    }

    pub fn assert_body(source: &str, target: &str, label: &str) -> serde_json::Value {
        json!({
            "source": source,
            "target": target,
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": label,
            "weight": 0.8,
        })
    }

    /// One block of a resource's body — the address half of an `anchored-at` row.
    pub async fn first_block_id(pool: &PgPool, resource: Uuid) -> Uuid {
        sqlx::query_scalar(
            "SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY seq LIMIT 1",
        )
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("the resource's first content block")
    }

    /// A live blob row homed in the harness's own default context — readable by the
    /// harness through its home (`blob_readable_by_profile` = content_type present +
    /// `anchor_readable_by_profile`), the D3 blob-target fixture of the visibility
    /// equivalence suite, minus its grants. The event FKs borrow the migration-seeded
    /// row: no event content is read by anything under test.
    pub async fn insert_blob(app: &super::common::E2eTestApp) -> Uuid {
        let home = default_context_id(&app.pool).await;
        let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
            .fetch_one(&app.pool)
            .await
            .expect("the migration-seeded event");
        sqlx::query_scalar(
            "INSERT INTO kb_blobs \
               (id, content_hash, blob_pathname, content_type, content_bytes, \
                home_table, home_id, owner_profile_id, originator_profile_id, \
                asserted_by_event_id, last_event_id) \
             VALUES ($1, $2, $3, 'application/octet-stream', 4, 'kb_contexts', $4, $5, $5, $6, $6) \
             RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(format!("parity-hash-{}", Uuid::new_v4()))
        .bind(format!("parity/{}", Uuid::new_v4()))
        .bind(home)
        .bind(profile_id_by_email(&app.pool, EMAIL).await)
        .bind(ev)
        .fetch_one(&app.pool)
        .await
        .expect("insert the fixture blob")
    }

    /// Seed a finding (`kb_resources` + `kb_resource_homes`) owned by `owner`, homed
    /// in `ctx`, with one content block that cites one live source resource — the
    /// citation_audit handler test's own fixture
    /// (`crates/temper-api/tests/citation_audit_handler_test.rs:63-126`), so the
    /// audit gate's `citation_is_live` predicate has a real `(block, source)` pair to
    /// read. Returns `(finding_id, block_id, source_id)`.
    pub async fn seed_finding_with_block(
        pool: &PgPool,
        owner: Uuid,
        ctx: Uuid,
        title: &str,
    ) -> (Uuid, Uuid, Uuid) {
        let finding = Uuid::now_v7();
        sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
            .bind(finding)
            .bind(title)
            .bind(format!("test://{finding}"))
            .execute(pool)
            .await
            .expect("insert finding");
        sqlx::query(
            "INSERT INTO kb_resource_homes \
                 (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
              VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(finding)
        .bind(ctx)
        .bind(owner)
        .execute(pool)
        .await
        .expect("home finding");

        let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("seed event");
        let block = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id) \
             VALUES ($1, $2, 0, $3, $3)",
        )
        .bind(block)
        .bind(finding)
        .bind(ev)
        .execute(pool)
        .await
        .expect("insert block");

        let source = Uuid::now_v7();
        sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
            .bind(source)
            .bind(format!("{title} source"))
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
        .bind(ev)
        .execute(pool)
        .await
        .expect("cite the source on the block");

        (finding, block, source)
    }

    /// The harness principal's read-without-write grant on a resource — the fixture
    /// `AuditAuthority` depends on to admit a reader who is not the author.
    pub async fn grant_read_only(pool: &PgPool, finding: Uuid, reader: Uuid, granted_by: Uuid) {
        sqlx::query(
            "INSERT INTO kb_access_grants \
                 (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
                  granted_by_profile_id) \
              VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
        )
        .bind(finding)
        .bind(reader)
        .bind(granted_by)
        .execute(pool)
        .await
        .expect("grant read-only access");
    }
}

use common::E2eTestApp;
use parity::{code_of, input, one_text};

/// The parity harness, once per test: the relay-ready service over this app's real
/// listener (its profile cache IS the direct families' caller path), and the
/// harness principal's parts — byte-stable across the swap, where they become the
/// bearer's vehicle.
async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

/// Ingest a resource through the harness's own client — the G3b setup idiom. With
/// `content`, the body lands as a real content block (an `anchored-at` address
/// half); with `cogmap`, the resource is kernel-homed in that map instead of a
/// context (`ingest.rs:68-70`, the cogmap branch takes precedence).
async fn ingest(
    app: &E2eTestApp,
    title: &str,
    content: Option<&str>,
    cogmap: Option<Uuid>,
) -> Uuid {
    let chunks = content.map(|c| common::chunked(c, 0.1));
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://ledger-graph-parity/{title}"),
        context_ref: "@me/default".to_string(),
        home_cogmap_id: cogmap,
        doc_type_name: "research".to_string(),
        content_hash: content.map(|c| temper_core::hash::sha256_hex(c.as_bytes())),
        content: content.unwrap_or_default().to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: chunks.map(|c| pack_chunks(&c).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest lands")
        .id
        .uuid()
}

/// Drive the `element_trail` tool: the tool function over relayed parts — the
/// swap made the parts the bearer's vehicle; the gate runs at the API.
async fn run_element_trail(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::trail::element_trail(
        svc,
        parts,
        parity::input::<temper_mcp::tools::trail::ElementTrailInput>(params),
    )
    .await
}

async fn run_facet_set(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::facets::facet_set(
        svc,
        parts,
        parity::input::<temper_mcp::tools::facets::FacetSetInput>(params),
    )
    .await
}

async fn run_facet_set_unified(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::facets::facet_set_unified(
        svc,
        parts,
        parity::input::<temper_mcp::tools::facets::FacetSetUnifiedInput>(params),
    )
    .await
}

async fn run_facets_read(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::facets::facets_read(
        svc,
        parts,
        parity::input::<temper_mcp::tools::facets::FacetsReadInput>(params),
    )
    .await
}

async fn run_facet_retract(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::facets::facet_retract(
        svc,
        parts,
        parity::input::<temper_mcp::tools::facets::FacetRetractInput>(params),
    )
    .await
}

async fn run_assert_relationship(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::relationships::assert_relationship(
        svc,
        parts,
        parity::input::<temper_mcp::tools::relationships::AssertRelationshipInput>(params),
    )
    .await
}

async fn run_retype(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::relationships::retype_relationship(
        svc,
        parts,
        parity::input::<temper_mcp::tools::relationships::RetypeRelationshipInput>(params),
    )
    .await
}

async fn run_reweight(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::relationships::reweight_relationship(
        svc,
        parts,
        parity::input::<temper_mcp::tools::relationships::ReweightRelationshipInput>(params),
    )
    .await
}

async fn run_fold(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::relationships::fold_relationship(
        svc,
        parts,
        parity::input::<temper_mcp::tools::relationships::FoldRelationshipInput>(params),
    )
    .await
}

async fn run_record_citation_audit(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::citation_audits::record_citation_audit(
        svc,
        parts,
        parity::input::<temper_mcp::tools::citation_audits::RecordCitationAuditInput>(params),
    )
    .await
}

/// Assert a resource→resource edge through the tool under test and return the
/// handle its ack names — the setup fixture for every edge-addressed face.
async fn assert_edge(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    source: Uuid,
    target: Uuid,
) -> Uuid {
    let res = run_assert_relationship(
        svc,
        parts,
        parity::assert_body(&source.to_string(), &target.to_string(), "parity"),
    )
    .await
    .expect("the edge asserts");
    let v = one_text(&res);
    v["edge_handle"]
        .as_str()
        .expect("the ack carries the edge handle")
        .parse()
        .expect("the handle is a uuid")
}

/// Open a REAL invocation envelope for the harness principal through the standing
/// front door — standing approval (already granted by the harness) plus the L0
/// write grant, then the production client's open. The act gate refuses
/// caller-named ids that address nothing, so every correlated act in this suite
/// must ride an envelope opened this way.
async fn open_invocation_for_harness(app: &E2eTestApp) -> Uuid {
    let principal = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight")
        .id;
    common::enable_invite_only(&app.pool, principal).await;
    common::grant_cogmap_write(&app.pool, parity::L0_COGMAP, principal).await;
    app.client
        .invocations()
        .open(
            &temper_core::types::invocation_requests::OpenInvocationRequest {
                trigger_kind: "e2e".into(),
                originating_cogmap: parity::L0_COGMAP,
                parent_cogmap: None,
            },
        )
        .await
        .expect("open invocation against L0")
        .invocation_id
}

// ── element_trail ───────────────────────────────────────────────────

/// A node's trail answers with the acts that touched it, ledger-ordered: the
/// ingest's `resource_created` leads and the facet act's `property_asserted` rides
/// last — every event category `domain`, ordered by `kb_events.id` (UUIDv7), with
/// the emitter entity's humanized name on each row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_nodes_trail_carries_its_acts_in_ledger_order(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = ingest(&app, "Trail node", Some("A body worth a trail."), None).await;
    run_facet_set(
        &svc,
        &parts,
        parity::facet_body(&resource.to_string(), json!({"status": "open"})),
    )
    .await
    .expect("the facet lands");

    let res = run_element_trail(
        &svc,
        &parts,
        json!({"kind": "node", "element": resource.to_string()}),
    )
    .await
    .expect("the trail answers");
    let v = one_text(&res);
    assert_eq!(v["element_kind"], "node", "{v}");
    assert_eq!(v["element_id"], json!(resource.to_string()), "{v}");
    let events = v["events"].as_array().expect("the events array");
    assert!(
        !events.is_empty(),
        "the resource's acts surface on its trail: {v}"
    );
    assert_eq!(
        events[0]["kind"], "resource_created",
        "the creating act leads: {v}"
    );
    assert_eq!(
        events.last().expect("non-empty")["kind"],
        "property_asserted",
        "the facet act rides the owner arm of the node trail: {v}"
    );
    for e in events {
        assert!(
            e["actor_name"].as_str().is_some_and(|s| !s.is_empty()),
            "every row carries its humanized actor: {v}"
        );
    }
}

/// An edge's trail answers with the `relationship_asserted` act the assert wrote —
/// the edge-addressed arm of the same read.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_edges_trail_carries_its_assertion(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Edge trail source", Some("Source."), None).await;
    let b = ingest(&app, "Edge trail target", None, None).await;
    let handle = assert_edge(&svc, &parts, a, b).await;

    let res = run_element_trail(
        &svc,
        &parts,
        json!({"kind": "edge", "element": handle.to_string()}),
    )
    .await
    .expect("the trail answers");
    let v = one_text(&res);
    assert_eq!(v["element_kind"], "edge", "{v}");
    let events = v["events"].as_array().expect("the events array");
    assert!(
        events.iter().any(|e| e["kind"] == "relationship_asserted"),
        "the assert act is on the edge's trail: {v}"
    );
}

/// A node id that addresses nothing answers a 200 with an EMPTY trail — the leak-safe
/// by-design posture (`trail.rs:1-7`): the gate lives in the SQL, so an invisible or
/// absent element is data, never an error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_element_answers_an_empty_trail_not_an_error(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = Uuid::now_v7();

    let res = run_element_trail(
        &svc,
        &parts,
        json!({"kind": "node", "element": ghost.to_string()}),
    )
    .await
    .expect("an absent element is DATA, not an error");
    let v = one_text(&res);
    assert!(
        v["events"].as_array().expect("the events array").is_empty(),
        "the trail is empty, never an error: {v}"
    );
}

/// A SECOND identity reading an invisible resource's trail also answers the empty
/// trail, not an error — the visibility gate renders absence and unreadability
/// identically, so a prober cannot read the resource's existence off the response.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_second_identity_reads_an_empty_trail_not_an_error(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let resource = ingest(&app, "Invisible to others", Some("Hidden body."), None).await;
    let (token, _sub, _email) = parity::second_identity(&app, &app.pool, "trail-other").await;
    // The second identity is its own parts: its real bearer crosses the door,
    // the API adjudicating it exactly like the harness's.
    let other_parts = app.relay_parts_for(&token);

    let res = run_element_trail(
        &svc,
        &other_parts,
        json!({"kind": "node", "element": resource.to_string()}),
    )
    .await
    .expect("an unreadable element is DATA, not an error");
    let v = one_text(&res);
    assert!(
        v["events"].as_array().expect("the events array").is_empty(),
        "no leak through the trail door: {v}"
    );
}

/// A garbage ref is refused at the parse callsite as `invalid_params` prefixed
/// `bad element ref:` (`trail.rs:76-78`), before the service is ever reached.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_garbage_ref_refuses_as_invalid_params(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let err = run_element_trail(
        &svc,
        &parts,
        json!({"kind": "node", "element": "not-a-ref"}),
    )
    .await
    .expect_err("garbage is not a ref");
    assert_eq!(code_of(&err), -32602, "a caller error, not a fault: {err}");
    assert!(
        err.message.starts_with("bad element ref:"),
        "the parse callsite's own prefix: {err}"
    );
}

// ── facets ──────────────────────────────────────────────────────────

/// The facet write answers the plural ack — one row per inner key of the asserted
/// object, never a singular id that could under-report a two-mark write.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn facet_set_answers_the_property_ids_ack(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = ingest(&app, "Facet ack", None, None).await;

    let res = run_facet_set(
        &svc,
        &parts,
        parity::facet_body(
            &resource.to_string(),
            json!({"status": "open", "severity": "high"}),
        ),
    )
    .await
    .expect("the facet lands");
    let v = one_text(&res);
    let ids = v["property_ids"].as_array().expect("the ack's ids");
    assert_eq!(ids.len(), 2, "one row per inner key, as written: {v}");
}

/// The facets read is the faithful view: one entry per LIVE row, each with its
/// weight and its author — the anti-collapse read `facets.rs:220-226` exists to be,
/// so a caller can tell a multi-key write from one key and can see every weight.
/// The two asserts here name DIFFERENT keys, because the write path folds the prior
/// row for each key NAMED (`db_backend.rs:2993-2994`) and leaves unnamed marks
/// untouched — which this test also witnesses: both rows stay live side by side.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn facets_read_returns_one_row_per_assert_with_weight_and_author(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let harness_id = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    let resource = ingest(&app, "Facet read", None, None).await;
    let ref_str = resource.to_string();
    run_facet_set(
        &svc,
        &parts,
        json!({
            "resource": ref_str,
            "values": {"status": "open"},
            "weight": 0.5,
        }),
    )
    .await
    .expect("the first assert lands");
    run_facet_set(
        &svc,
        &parts,
        parity::facet_body(&ref_str, json!({"severity": "high"})),
    )
    .await
    .expect("the second assert lands");

    let res = run_facets_read(
        &svc,
        &parts,
        json!({"target": "resource", "resource": ref_str}),
    )
    .await
    .expect("the read lands");
    let v = one_text(&res);
    assert_eq!(v["resource"], json!(ref_str), "{v}");
    let facets = v["facets"].as_array().expect("the facets array");
    assert_eq!(facets.len(), 2, "one row per live assert, NO collapse: {v}");
    for row in facets {
        assert_eq!(row["property_key"], "facet", "the clustering key: {v}");
        assert!(
            row["weight"].as_f64().is_some(),
            "the weight survives the read: {v}"
        );
        assert_eq!(
            row["authored_by_profile_id"],
            json!(harness_id.to_string()),
            "the author rides the row: {v}"
        );
    }
    assert!(
        facets
            .iter()
            .any(|r| r["value"]["status"] == "open" && r["weight"] == json!(0.5)),
        "the first row's value and weight read back: {v}"
    );
    assert!(
        facets
            .iter()
            .any(|r| r["value"]["severity"] == "high" && r["weight"] == json!(1.0)),
        "the unnamed mark stayed live at its own weight: {v}"
    );
}

/// The edge facet write answers the ack, and the `property_key` keyed mode writes
/// ONE row under the key: an `anchored-at` span qualification whose value is the
/// exact `{"endpoint", "address"}` payload, read back on the edge's facet rows.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edge_facet_set_answers_the_ack_including_the_keyed_anchored_at_row(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Anchored source", Some("The anchored body."), None).await;
    let b = ingest(&app, "Anchored target", None, None).await;
    let handle = assert_edge(&svc, &parts, a, b).await;
    let block = parity::first_block_id(&app.pool, a).await;

    let res = temper_mcp::tools::facets::edge_facet_set(
        &svc,
        &parts,
        input(json!({
            "edge_handle": handle.to_string(),
            "values": {"note": "witnesses clause one"},
        })),
    )
    .await
    .expect("the ordinary edge facet lands");
    let v = one_text(&res);
    assert_eq!(
        v["property_ids"].as_array().expect("ids").len(),
        1,
        "one row for the ordinary facet: {v}"
    );

    let res = temper_mcp::tools::facets::edge_facet_set(
        &svc,
        &parts,
        input(json!({
            "edge_handle": handle.to_string(),
            "property_key": "anchored-at",
            "values": {
                "endpoint": "source",
                "address": format!("{a}#{block}"),
            },
        })),
    )
    .await
    .expect("the keyed row lands");
    let v = one_text(&res);
    let keyed = v["property_ids"]
        .as_array()
        .expect("ids")
        .first()
        .expect("exactly one keyed row")
        .clone();

    let res = run_facets_read(
        &svc,
        &parts,
        json!({"target": "edge", "edge_handle": handle.to_string()}),
    )
    .await
    .expect("the edge read lands");
    let v = one_text(&res);
    let facets = v["facets"].as_array().expect("the facets array");
    assert_eq!(facets.len(), 2, "both rows are live: {v}");
    let anchored = facets
        .iter()
        .find(|r| r["property_key"] == "anchored-at")
        .expect("the keyed row reads back under its key");
    assert_eq!(anchored["property_id"], keyed, "{v}");
    assert_eq!(anchored["value"]["endpoint"], "source", "{v}");
    assert_eq!(
        anchored["value"]["address"],
        json!(format!("{a}#{block}")),
        "{v}"
    );
}

/// The retraction answers the echo ack; the SAME id a second time is the projector's
/// zero-rows outcome — `invalid_params` with the one indistinguishable
/// not-a-live-facet sentence (foreign, missing, and already-retracted are one shape).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn facet_retract_answers_the_ack_and_a_second_retract_refuses(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Retract source", Some("The cited body."), None).await;
    let b = ingest(&app, "Retract target", None, None).await;
    let handle = assert_edge(&svc, &parts, a, b).await;
    let block = parity::first_block_id(&app.pool, a).await;
    let res = temper_mcp::tools::facets::edge_facet_set(
        &svc,
        &parts,
        input(json!({
            "edge_handle": handle.to_string(),
            "property_key": "anchored-at",
            "values": {"endpoint": "source", "address": format!("{a}#{block}")},
        })),
    )
    .await
    .expect("the keyed row lands");
    let property_id: Uuid = one_text(&res)["property_ids"][0]
        .as_str()
        .expect("the row id")
        .parse()
        .expect("a uuid");

    let res = run_facet_retract(
        &svc,
        &parts,
        json!({
            "target": "edge",
            "edge_handle": handle.to_string(),
            "property_id": property_id.to_string(),
        }),
    )
    .await
    .expect("the retraction lands");
    let v = one_text(&res);
    assert_eq!(
        v["property_id"],
        json!(property_id.to_string()),
        "the echo ack: {v}"
    );

    let err = run_facet_retract(
        &svc,
        &parts,
        json!({
            "target": "edge",
            "edge_handle": handle.to_string(),
            "property_id": property_id.to_string(),
        }),
    )
    .await
    .expect_err("an already-retracted row refuses");
    assert_eq!(
        code_of(&err),
        -32602,
        "the 404 arm renders invalid_params, never a fault: {err}"
    );
    assert!(
        err.message.contains("is not a live facet of edge"),
        "the one indistinguishable sentence: {err}"
    );
}

/// The unified set tool refuses a `property_key` beside target=resource with its
/// MCP-local naming refusal (`facets.rs:304-310`) — a keyed row qualifies a
/// relationship, and the refusal is the handler's, not a schema error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn facet_set_unified_refuses_property_key_on_the_resource_target(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = ingest(&app, "Keyed refusal", None, None).await;

    let err = run_facet_set_unified(
        &svc,
        &parts,
        json!({
            "target": "resource",
            "resource": resource.to_string(),
            "property_key": "anchored-at",
            "values": {"endpoint": "source", "address": format!("{resource}#{resource}")},
        }),
    )
    .await
    .expect_err("the keyed mode is edge-only");
    assert_eq!(code_of(&err), -32602, "{err}");
    assert!(
        err.message
            .starts_with("property_key applies to target=edge only"),
        "the naming refusal: {err}"
    );
}

/// The retract tool refuses target=resource with its MCP-local naming refusal
/// (`facets.rs:448-454`): a resource's facet rows are projector-minted surrogates
/// with no retract-by-id; overwrite via facet_set instead.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn facet_retract_refuses_the_resource_target_by_name(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = ingest(&app, "Retract target refusal", None, None).await;

    let err = run_facet_retract(
        &svc,
        &parts,
        json!({
            "target": "resource",
            "resource": resource.to_string(),
            "property_id": Uuid::now_v7().to_string(),
        }),
    )
    .await
    .expect_err("the resource target is refused by name");
    assert_eq!(code_of(&err), -32602, "{err}");
    assert!(
        err.message
            .starts_with("facet_retract applies to target=edge only"),
        "the naming refusal: {err}"
    );
    assert!(
        err.message.contains("Use facet_set to overwrite"),
        "the refusal names the remediation: {err}"
    );
}

/// `facet_set` on a resource the caller cannot modify speaks the terse authority
/// arm: INVALID_REQUEST with the "cannot modify this resource" sentence
/// (`facets.rs:64-68`, deny from `check_can_modify_next`, `db_backend.rs:730-744`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn facet_set_on_a_foreign_resource_speaks_the_forbidden_arm(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let resource = ingest(&app, "Foreign facet target", None, None).await;
    let (token, _sub, _email) = parity::second_identity(&app, &app.pool, "facet-other").await;
    // The second identity is its own parts: its real bearer crosses the door,
    // the API adjudicating it exactly like the harness's.
    let other_parts = app.relay_parts_for(&token);

    let err = run_facet_set(
        &svc,
        &other_parts,
        parity::facet_body(&resource.to_string(), json!({"status": "hijacked"})),
    )
    .await
    .expect_err("a non-writer is refused");
    assert_eq!(
        code_of(&err),
        -32600,
        "the INVALID_REQUEST authority arm: {err}"
    );
    assert_eq!(
        err.message, "facet_set: cannot modify this resource",
        "the terse arm's exact sentence: {err}"
    );
}

/// An edge facet in a map the caller READS but cannot AUTHOR speaks the detailed
/// authority arm: the container clause's ForbiddenDetail carries the gate's own
/// sentence (`db_backend.rs:107-112`), rendered INVALID_REQUEST
/// (`facets.rs:59-63`). The scenario splits the two map predicates deliberately:
/// clause 1 passes on a DIRECT write grant on the source resource while L0
/// authorship stays withheld (team-reach read only), because
/// `cogmap_authorable_by_profile` is write-grant-only — read is never authorship.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_edge_facet_in_a_readable_unauthorable_map_speaks_the_forbidden_detail_arm(
    pool: PgPool,
) {
    let (app, svc, parts) = harness(pool).await;
    let harness_id = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    // Kernel-homed source: the edge's home is L0, so the edge facet's container
    // clause dispatches to the cogmap authority. The HARNESS holds the L0 write
    // grant both the kernel ingest (create-into-cogmap, F1) and the assert's
    // container clause require.
    common::grant_cogmap_write(&app.pool, parity::L0_COGMAP, harness_id).await;
    let kernel_src = ingest(
        &app,
        "Kernel source",
        Some("Kernel body."),
        Some(parity::L0_COGMAP),
    )
    .await;
    let kernel_tgt = ingest(&app, "Kernel target", None, None).await;
    let handle = assert_edge(&svc, &parts, kernel_src, kernel_tgt).await;

    // The second identity reads L0 by team reach (auto-join watcher on the
    // temper-system team the L0 migration binds the map to) and gets clause 1
    // through a DIRECT write grant on the source resource alone.
    let (token, _sub, email) = parity::second_identity(&app, &app.pool, "detail-other").await;
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
              granted_by_profile_id) \
          VALUES ('kb_resources', $1, 'kb_profiles', $2, true, true, $3)",
    )
    .bind(kernel_src)
    .bind(parity::profile_id_by_email(&app.pool, &email).await)
    .bind(harness_id)
    .execute(&app.pool)
    .await
    .expect("the direct resource write grant lands");
    // The second identity is its own parts: its real bearer crosses the door.
    let other_parts = app.relay_parts_for(&token);

    let err = temper_mcp::tools::facets::edge_facet_set(
        &svc,
        &other_parts,
        input(json!({
            "edge_handle": handle.to_string(),
            "values": {"note": "planted claim"},
        })),
    )
    .await
    .expect_err("a reader who is not an author is refused");
    assert_eq!(
        code_of(&err),
        -32600,
        "the INVALID_REQUEST authority arm: {err}"
    );
    assert!(
        err.message
            .starts_with("edge_facet_set: cannot author cognitive map"),
        "the gate's own sentence names the capability and the subject: {err}"
    );
    assert!(
        err.message.ends_with("reading confers no authorship."),
        "the readable arm, not the terse one: {err}"
    );
}

// ── relationships ───────────────────────────────────────────────────

/// The assert answers the handle ack for both target tables: the incumbent
/// resource→resource edge, and the D3 `target_table: "blob"` pointing act — an edge
/// at a blob the caller can read, homed in the source's home.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_answers_the_edge_handle_ack_including_a_blob_target(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Assert source", Some("The pointing body."), None).await;
    let b = ingest(&app, "Assert target", None, None).await;

    let res = run_assert_relationship(
        &svc,
        &parts,
        parity::assert_body(&a.to_string(), &b.to_string(), "derived_from"),
    )
    .await
    .expect("the edge asserts");
    let v = one_text(&res);
    let handle: Uuid = v["edge_handle"]
        .as_str()
        .expect("the ack carries the handle")
        .parse()
        .expect("a uuid");

    let blob = parity::insert_blob(&app).await;
    let res = run_assert_relationship(
        &svc,
        &parts,
        json!({
            "source": a.to_string(),
            "target": blob.to_string(),
            "target_table": "blob",
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": "source_doc",
            "weight": 1.0,
        }),
    )
    .await
    .expect("the blob-pointing edge asserts");
    let v = one_text(&res);
    let blob_handle: Uuid = v["edge_handle"]
        .as_str()
        .expect("the ack carries the handle")
        .parse()
        .expect("a uuid");
    assert_ne!(handle, blob_handle, "two asserts, two edges: {v}");
}

/// Retype, reweight, and fold each answer the same handle ack — the three mutation
/// verbs on one shape.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retype_reweight_and_fold_each_answer_the_ack(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Verb source", None, None).await;
    let b = ingest(&app, "Verb target", None, None).await;
    let handle = assert_edge(&svc, &parts, a, b).await;
    let handle_str = handle.to_string();

    let v = one_text(
        &run_retype(
            &svc,
            &parts,
            json!({
                "edge_handle": handle_str,
                "edge_kind": "near",
                "polarity": "inverse",
            }),
        )
        .await
        .expect("the retype lands"),
    );
    assert_eq!(v["edge_handle"], json!(handle_str), "{v}");

    let v = one_text(
        &run_reweight(
            &svc,
            &parts,
            json!({"edge_handle": handle_str, "weight": 0.25}),
        )
        .await
        .expect("the reweight lands"),
    );
    assert_eq!(v["edge_handle"], json!(handle_str), "{v}");

    let v = one_text(
        &run_fold(
            &svc,
            &parts,
            json!({"edge_handle": handle_str, "reason": "superseded"}),
        )
        .await
        .expect("the fold lands"),
    );
    assert_eq!(v["edge_handle"], json!(handle_str), "{v}");
}

/// An unknown edge handle refuses each verb with the `check_edge_mutable` lookup's
/// not-found shape (`db_backend.rs:985-995`) rendered `invalid_params`
/// (`relationships.rs:104-106`) — the same refusal a folded handle gets, one
/// indistinguishable shape.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_edge_handle_refuses_every_verb_as_invalid_params(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = Uuid::now_v7().to_string();

    for (name, params) in [
        (
            "retype",
            json!({"edge_handle": ghost, "edge_kind": "near", "polarity": "forward"}),
        ),
        ("reweight", json!({"edge_handle": ghost, "weight": 0.5})),
        (
            "fold",
            json!({"edge_handle": ghost, "reason": "no such edge"}),
        ),
    ] {
        let err = match name {
            "retype" => run_retype(&svc, &parts, params).await,
            "reweight" => run_reweight(&svc, &parts, params).await,
            _ => run_fold(&svc, &parts, params).await,
        }
        .expect_err("an unknown handle does not answer");
        assert_eq!(
            code_of(&err),
            -32602,
            "{name}: the not-found shape is a caller error: {err}"
        );
        assert!(
            err.message.contains(&format!("edge {ghost} not found")),
            "{name}: the lookup's own sentence: {err}"
        );
    }
}

/// A caller-supplied `invocation_id` that names nothing refuses with the act gate's
/// uniform not-found arm (`db_backend.rs:2004`) rendered `invalid_params` — the
/// correlation claim is guarded, additive to the edge's own mutability gate, so the
/// edge itself is real and the invocation is the ONLY thing wrong.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_invocation_id_that_names_nothing_refuses_with_the_not_found_arm(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Ghost run source", None, None).await;
    let b = ingest(&app, "Ghost run target", None, None).await;
    let handle = assert_edge(&svc, &parts, a, b).await;
    let ghost = Uuid::now_v7();

    let err = run_reweight(
        &svc,
        &parts,
        json!({
            "edge_handle": handle.to_string(),
            "weight": 0.5,
            "invocation_id": ghost.to_string(),
        }),
    )
    .await
    .expect_err("a correlation claim to nowhere is refused");
    assert_eq!(
        code_of(&err),
        -32602,
        "the 404 arm is a caller error: {err}"
    );
    assert!(
        err.message
            .contains(&format!("invocation {ghost} not found")),
        "the uniform not-found sentence: {err}"
    );
}

/// A CLOSED invocation refuses the correlation claim with the act gate's non-open
/// arm (`db_backend.rs:2005-2011`) — THE DECLARED PARITY DELTA: the direct
/// binding's `map_err` had no Conflict arm and rendered this face
/// `internal_error` (pinned there by this suite before the swap); the wire's 409
/// is caller-actionable, so the door renders it `invalid_params` with the
/// server's own sentence — the `contexts.rs::map_api_error` Conflict arm's
/// precedent.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_closed_invocation_refuses_with_the_conflict_arm(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Closed run source", None, None).await;
    let b = ingest(&app, "Closed run target", None, None).await;
    let handle = assert_edge(&svc, &parts, a, b).await;

    let invocation = open_invocation_for_harness(&app).await;
    app.client
        .invocations()
        .close(
            invocation,
            &temper_core::types::invocation_requests::CloseInvocationRequest {
                disposition: temper_core::types::invocation::Disposition::Completed,
                outcome: serde_json::json!({}),
            },
        )
        .await
        .expect("the envelope closes");

    let err = run_reweight(
        &svc,
        &parts,
        json!({
            "edge_handle": handle.to_string(),
            "weight": 0.5,
            "invocation_id": invocation.to_string(),
        }),
    )
    .await
    .expect_err("a terminal envelope takes no new acts");
    assert_eq!(
        code_of(&err),
        -32602,
        "a caller-actionable 409, not a fault — the declared delta: {err}"
    );
    assert!(
        err.message
            .contains("cannot stamp an act onto a non-open run"),
        "the server's own sentence, carried through: {err}"
    );
}

/// An authored assert under a REAL OPEN invocation lands — the bite that keeps the
/// not-found arm honest: the correlation claim addresses a live envelope, the gate
/// PASSES, and the `relationship_asserted` act carries the invocation's id on the
/// ledger. Without this witness a 404-masking regression (the gate refusing
/// everything) would leave the unknown-invocation test green.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_authored_assert_under_a_real_open_invocation_lands(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let a = ingest(&app, "Authored source", None, None).await;
    let b = ingest(&app, "Authored target", None, None).await;
    let invocation = open_invocation_for_harness(&app).await;

    let handle: Uuid = {
        let mut params = parity::assert_body(&a.to_string(), &b.to_string(), "correlated");
        params["invocation_id"] = json!(invocation.to_string());
        let v = one_text(
            &run_assert_relationship(&svc, &parts, params)
                .await
                .expect("the correlated assert lands"),
        );
        v["edge_handle"]
            .as_str()
            .expect("the ack carries the handle")
            .parse()
            .expect("a uuid")
    };

    let stamped: Option<Uuid> = sqlx::query_scalar(
        "SELECT e.invocation_id FROM kb_events e \
         JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'relationship_asserted' \
           AND (e.payload ->> 'edge_id')::uuid = $1",
    )
    .bind(handle)
    .fetch_one(&app.pool)
    .await
    .expect("the assert act is on the ledger");
    assert_eq!(
        stamped,
        Some(invocation),
        "the act correlates to the invocation it claimed"
    );
}

/// An assert FROM a resource the caller cannot modify speaks the terse authority
/// arm — clause 1 leads the gate train (`db_backend.rs:1219`), bare `Forbidden` →
/// the "cannot modify this resource" sentence (`relationships.rs:115-119`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_assert_from_a_foreign_resource_speaks_the_forbidden_arm(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let a = ingest(&app, "Assert authority source", None, None).await;
    let b = ingest(&app, "Assert authority target", None, None).await;
    let (token, _sub, _email) = parity::second_identity(&app, &app.pool, "assert-other").await;
    // The second identity is its own parts: its real bearer crosses the door,
    // the API adjudicating it exactly like the harness's.
    let other_parts = app.relay_parts_for(&token);

    let err = run_assert_relationship(
        &svc,
        &other_parts,
        parity::assert_body(&a.to_string(), &b.to_string(), "hijack"),
    )
    .await
    .expect_err("a non-writer is refused");
    assert_eq!(
        code_of(&err),
        -32600,
        "the INVALID_REQUEST authority arm: {err}"
    );
    assert_eq!(
        err.message, "assert_relationship: cannot modify this resource",
        "the terse arm's exact sentence: {err}"
    );
}

// ── citation_audits ─────────────────────────────────────────────────

/// The finding-shaped refusal is ONE sentence for every cause: an unknown block and
/// the author auditing its own citation both render
/// "record_citation_audit: finding not found, unreadable, or self-authored" —
/// byte-exact, because `FINDING_REFUSAL` is one string by construction and the
/// tool's NotFound arm deliberately re-states the equivalence locally
/// (`citation_audits.rs:60-72`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_finding_shaped_refusals_are_one_indistinguishable_sentence(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let harness_id = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    let ctx = parity::default_context_id(&app.pool).await;
    let (finding, block, source) =
        parity::seed_finding_with_block(&app.pool, harness_id, ctx, "Own finding").await;
    let _ = finding;

    let unknown = run_record_citation_audit(
        &svc,
        &parts,
        json!({
            "block_id": Uuid::now_v7().to_string(),
            "source": {"kind": "resource", "value": source.to_string()},
            "value": 0.8,
        }),
    )
    .await
    .expect_err("an unknown block refuses");
    let self_audit = run_record_citation_audit(
        &svc,
        &parts,
        json!({
            "block_id": block.to_string(),
            "source": {"kind": "resource", "value": source.to_string()},
            "value": 0.8,
        }),
    )
    .await
    .expect_err("the author may not grade its own citation");

    const EXPECTED: &str = "record_citation_audit: finding not found, unreadable, or self-authored";
    assert_eq!(code_of(&unknown), -32602, "{unknown}");
    assert_eq!(code_of(&self_audit), -32602, "{self_audit}");
    assert_eq!(unknown.message, EXPECTED, "byte-exact: {unknown}");
    assert_eq!(self_audit.message, EXPECTED, "byte-exact: {self_audit}");
}

/// A DIFFERENT approved principal, granted read on a finding it did not author,
/// records the audit — and the response is the BARE audit Uuid (no wrapping ack,
/// `citation_audits.rs:123-129`), naming a real `kb_citation_audits` row on the
/// audited block.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_approved_reader_who_did_not_author_the_citation_records_an_audit(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;
    let harness_id = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    let ctx = parity::default_context_id(&app.pool).await;
    let (_finding, block, source) =
        parity::seed_finding_with_block(&app.pool, harness_id, ctx, "Audited finding").await;

    let (token, _sub, email) = parity::second_identity(&app, &app.pool, "auditor").await;
    parity::grant_read_only(
        &app.pool,
        _finding,
        parity::profile_id_by_email(&app.pool, &email).await,
        harness_id,
    )
    .await;
    // The second identity is its own parts: its real bearer crosses the door.
    let other_parts = app.relay_parts_for(&token);

    let res = run_record_citation_audit(
        &svc,
        &other_parts,
        json!({
            "block_id": block.to_string(),
            "source": {"kind": "resource", "value": source.to_string()},
            "value": 0.8,
            "reason": "independently verified",
        }),
    )
    .await
    .expect("the gate admits the reader");
    let v = one_text(&res);
    let audit_id: Uuid = v
        .as_str()
        .expect("the BARE audit id, no wrapping ack: {v}")
        .parse()
        .expect("a uuid");

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_citation_audits WHERE id = $1 AND block_id = $2)",
    )
    .bind(audit_id)
    .bind(block)
    .fetch_one(&app.pool)
    .await
    .expect("check the audit row");
    assert!(exists, "the verdict lands on the audited block");
}

/// A Remote-kind citation refuses at the command boundary's source-kind guard
/// (`db_backend.rs:2909-2916`) with the server's own sentence passed through
/// (`citation_audits.rs:73`) — only resource-kind citations are auditable.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_remote_kind_source_refuses_with_the_server_sentence(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let harness_id = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    let ctx = parity::default_context_id(&app.pool).await;
    let (_finding, block, _source) =
        parity::seed_finding_with_block(&app.pool, harness_id, ctx, "Remote refusal").await;

    let err = run_record_citation_audit(
        &svc,
        &parts,
        json!({
            "block_id": block.to_string(),
            "source": {"kind": "remote", "value": "https://example.com/doc"},
            "value": 0.5,
        }),
    )
    .await
    .expect_err("a remote citation is not auditable");
    assert_eq!(
        code_of(&err),
        -32602,
        "a caller fault, not a fault of the server: {err}"
    );
    assert_eq!(
        err.message, "only resource-kind citations are auditable",
        "the server's own sentence: {err}"
    );
}

/// An out-of-range verdict refuses at the range guard
/// (`db_backend.rs:2896-2901`) with the server's own sentence — a caller fault that
/// would otherwise surface as a 500 off the ledger column's CHECK.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_out_of_range_value_refuses_with_the_server_sentence(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let harness_id = parity::profile_id_by_email(&app.pool, parity::EMAIL).await;
    let ctx = parity::default_context_id(&app.pool).await;
    let (_finding, block, source) =
        parity::seed_finding_with_block(&app.pool, harness_id, ctx, "Range refusal").await;

    let err = run_record_citation_audit(
        &svc,
        &parts,
        json!({
            "block_id": block.to_string(),
            "source": {"kind": "resource", "value": source.to_string()},
            "value": 1.5,
        }),
    )
    .await
    .expect_err("a verdict outside [-1.0, 1.0] is refused");
    assert_eq!(code_of(&err), -32602, "{err}");
    assert_eq!(
        err.message, "citation audit value 1.5 is outside [-1.0, 1.0]",
        "the server's own sentence: {err}"
    );
}
