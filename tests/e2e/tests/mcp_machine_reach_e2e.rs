#![cfg(feature = "test-db")]
//! A machine through MCP reaches content and workflow, and nothing else.
//!
//! MCP holds no authority of its own: every tool relays over HTTP with the caller's bearer, so a
//! machine's reach through MCP is whatever the API admits for that bearer. Two of the registered
//! tools reach acts a machine may not take — `context_manage`'s `share`/`unshare`/`transfer` (a
//! person's act on the context tier) and `resource_reblock`'s deployment-wide `all` (system-admin
//! standing, which a machine never holds). Those are refused here through the relayed 403, each
//! in its own voice. Every other tool family is witnessed admitting the same machine, so the
//! refusals are the line and not a machine that reaches nothing.
//!
//! The machine is provisioned through the real registration door — agent profile, emitters, and a
//! `member` seat on the team that owns the context it works in — then standing-approved, and drives
//! the tools with its own `client_credentials` bearer.

mod common;

use rmcp::model::{CallToolResult, ErrorCode};
use serde::Deserialize;
use serde_json::{json, Value};
use temper_core::context_ref::ContextOwnerRef;
use temper_core::types::team::TeamCreateRequest;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

const MACHINE_CLIENT: &str = "mcp-reach-agent";

/// The API's machine refusal sentence (`temper_services::auth::MACHINE_PRINCIPAL_REFUSAL`).
const MACHINE_REFUSAL: &str = temper_services::auth::MACHINE_PRINCIPAL_REFUSAL;

/// A machine at work: the relay service, the machine's request parts, and what it works on.
struct Machine {
    app: common::E2eTestApp,
    svc: TemperMcpService,
    parts: axum::http::request::Parts,
    profile: Uuid,
    admin: Uuid,
    team: Uuid,
    context: Uuid,
}

/// The harness principal (a person, made system admin) creates a team and a team-owned context,
/// then registers the machine through `POST /api/machine-clients` with a `member` seat on that
/// team. The machine is approved, so it clears the system gate and meets each door's own answer.
async fn machine(pool: sqlx::PgPool) -> Machine {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    machine_on(app, svc).await
}

/// [`machine`] on an app and relay service the caller built — the blob family needs a store.
async fn machine_on(app: common::E2eTestApp, svc: TemperMcpService) -> Machine {
    let admin: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("harness principal");
    common::make_system_admin(&app.pool, admin).await;

    let slug = format!("mcp-reach-{}", &Uuid::now_v7().simple().to_string()[24..]);
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: slug.clone(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("team")
        .id;
    let context = *app
        .client
        .contexts()
        .create("mcp-reach", Some(ContextOwnerRef::Team(slug)))
        .await
        .expect("team context")
        .id;

    let resp = app
        .reqwest_client
        .post(app.url("/api/machine-clients"))
        .bearer_auth(&app.token)
        .json(&json!({
            "client_id": MACHINE_CLIENT,
            "label": "mcp reach",
            "owner_team_id": null,
            "teams": [{ "team_id": team, "role": "member" }],
            "grants": [],
        }))
        .send()
        .await
        .expect("provision request");
    assert_eq!(resp.status(), 200, "the admin registers the machine");
    let client: Value = resp.json().await.expect("machine client");
    let profile: Uuid = client["profile_id"]
        .as_str()
        .expect("profile_id")
        .parse()
        .expect("uuid");
    common::approve(&app.pool, profile).await;

    let parts = app.relay_parts_for(&common::generate_machine_jwt(MACHINE_CLIENT));
    Machine {
        app,
        svc,
        parts,
        profile,
        admin,
        team,
        context,
    }
}

/// Deserialize a tool input from its wire shape.
fn input<T: for<'de> Deserialize<'de>>(value: Value) -> T {
    serde_json::from_value(value).expect("input deserializes from its wire shape")
}

/// The JSON of a one-part tool result.
fn one_text(res: &CallToolResult) -> Value {
    let text = res.content[0].as_text().expect("a text part").text.as_str();
    serde_json::from_str(text).expect("the part is the tool's JSON response")
}

// ── refused ─────────────────────────────────────────────────────────────────────────────────

/// `context_manage`'s share, unshare and transfer take a person. A machine — here a `member` of the
/// team that owns the context — is refused each with the API's machine sentence, carried as the
/// caller's own refusal (`INVALID_REQUEST`), never as an internal fault. No share or transfer lands.
///
/// FAILS IF the detailed-403 arm is dropped from `contexts::map_api_error` (the refusal falls to
/// `INTERNAL_ERROR`), or if any of the three handlers admits a machine.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_is_refused_context_share_unshare_and_transfer(pool: sqlx::PgPool) {
    let m = machine(pool).await;

    let state = |pool: sqlx::PgPool, context: Uuid| async move {
        sqlx::query_as::<_, (i64, String, Uuid)>(
            "SELECT (SELECT count(*) FROM kb_team_contexts WHERE context_id = c.id), \
                    c.owner_table, c.owner_id \
               FROM kb_contexts c WHERE c.id = $1",
        )
        .bind(context)
        .fetch_one(&pool)
        .await
        .expect("context state")
    };
    let before = state(m.app.pool.clone(), m.context).await;

    for action in ["share", "unshare", "transfer"] {
        let err = temper_mcp::tools::contexts::context_manage(
            &m.svc,
            &m.parts,
            input(json!({ "action": action, "context": m.context, "team": m.team })),
        )
        .await
        .expect_err("a machine is refused");
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST, "{action}: {err:?}");
        assert_eq!(
            err.message,
            format!("{action}_context: {MACHINE_REFUSAL}"),
            "{action}"
        );
    }

    assert_eq!(
        state(m.app.pool.clone(), m.context).await,
        before,
        "no share, unshare or transfer landed"
    );
}

/// `resource_reblock`'s deployment-wide arm needs system-admin standing, which a machine never
/// holds: refused with the tool's `all`-scope sentence. The resource arm, which rides ordinary
/// visibility, admits the same machine (`a_machine_reaches_every_tool_family`).
///
/// No source edit makes this fail on its own: `Principal::system_admin` answers a machine `None`
/// without a query, a `SystemAdmin` cannot be minted from a `MachinePrincipal` (the compile-time
/// fixtures pin that), and `is_system_admin` is false for every machine by trigger. What this
/// witnesses is the relayed answer an agent actually sees.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_is_refused_the_deployment_wide_reblock(pool: sqlx::PgPool) {
    let m = machine(pool).await;

    let err = temper_mcp::tools::reblock::resource_reblock(
        &m.svc,
        &m.parts,
        input(json!({ "scope": "all", "dry_run": true })),
    )
    .await
    .expect_err("a machine is refused the all scope");
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST, "{err:?}");
    assert!(
        err.message
            .contains("the deployment-wide `all` scope requires system-administrator standing"),
        "{err:?}"
    );
}

// ── admitted ────────────────────────────────────────────────────────────────────────────────

/// Create one resource in the machine's team context through `create_resource`; its id.
async fn create(m: &Machine, title: &str) -> String {
    let res = temper_mcp::tools::resources::create_resource(
        &m.svc,
        &m.parts,
        input(json!({
            "context_ref": m.context.to_string(),
            "doc_type_name": "session",
            "title": title,
        })),
    )
    .await
    .expect("resources: a machine creates in its team context");
    one_text(&res)["resource"]["id"]
        .as_str()
        .expect("created id")
        .to_string()
}

/// One admitted call per relayed tool family, as the same machine the refusals above refuse: the
/// line runs through the families, not around a machine that reaches nothing. Each call is the
/// family's cheapest successful act on the machine's own team context or its own resources. The
/// blob family needs a store and runs on its own app (`a_machine_commits_and_reads_a_blob`).
///
/// The edge-local `describe_schema` is called too, though it never reaches the API: it is open to
/// any verified token by ruling, and is here so every registered family is accounted for.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_reaches_every_tool_family(pool: sqlx::PgPool) {
    std::env::set_var("TEMPER_ASYNC_EMBED", "1");
    let m = machine(pool).await;
    let (svc, parts) = (&m.svc, &m.parts);

    // resources — create and read back; the act ran as the machine.
    let resource = create(&m, "machine reach").await;
    let source = create(&m, "machine reach source").await;
    let owner: Uuid = sqlx::query_scalar(
        "SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id = $1::uuid",
    )
    .bind(&resource)
    .fetch_one(&m.app.pool)
    .await
    .expect("home row");
    assert_eq!(owner, m.profile, "the machine is the resource's owner");
    let got =
        temper_mcp::tools::resources::get_resource(svc, parts, input(json!({ "id": resource })))
            .await
            .expect("resources: get");
    assert_eq!(one_text(&got)["id"], resource.as_str());

    // reblock — the resource arm rides ordinary visibility.
    let receipt = temper_mcp::tools::reblock::resource_reblock(
        svc,
        parts,
        input(json!({ "scope": "resource", "resource": resource, "dry_run": true })),
    )
    .await
    .expect("reblock: the resource scope admits a machine");
    assert!(one_text(&receipt)["summary"].is_object(), "reblock receipt");

    // search and query — the model-free arms.
    temper_mcp::tools::search::search(svc, parts, input(json!({ "query": "anything" })))
        .await
        .expect("search");
    let ran = temper_mcp::tools::query::run_query(
        svc,
        parts,
        input(json!({
            "plan": {
                "stages": [{ "name": "q", "act": "find-exact", "intention": { "query": "zz-probe" } }],
                "outcome": { "returns": [{ "stage": "q" }] },
            }
        })),
    )
    .await
    .expect("run_query");
    assert!(one_text(&ran)["returned"]["q"].is_object(), "query result");

    // trail — the resource's own ledger.
    let trail = temper_mcp::tools::trail::element_trail(
        svc,
        parts,
        input(json!({ "kind": "node", "element": resource })),
    )
    .await
    .expect("element_trail");
    assert_eq!(one_text(&trail)["element_kind"], "node");

    // facets — read, then assert one.
    temper_mcp::tools::facets::facets_read(
        svc,
        parts,
        input(json!({ "target": "resource", "resource": resource })),
    )
    .await
    .expect("facets_read");
    temper_mcp::tools::facets::facet_set_unified(
        svc,
        parts,
        input(json!({ "target": "resource", "resource": resource, "values": { "status": "open" }, "weight": 0.5 })),
    )
    .await
    .expect("facet_set");

    // relationships — an edge between two of the machine's resources.
    temper_mcp::tools::relationships::relationship(
        svc,
        parts,
        input(json!({
            "action": "assert",
            "source": source,
            "target": resource,
            "edge_kind": "express",
            "polarity": "forward",
            "label": "derived_from",
            "weight": 1.0,
        })),
    )
    .await
    .expect("relationship assert");

    // contexts — list, and create (a context of the machine's own is content, not a person's act).
    temper_mcp::tools::contexts::context_read(svc, parts, input(json!({ "view": "list" })))
        .await
        .expect("context_read list");
    temper_mcp::tools::contexts::context_manage(
        svc,
        parts,
        input(json!({ "action": "create", "name": "machine own" })),
    )
    .await
    .expect("context_manage create");

    // cognitive maps — genesis of the machine's own map, then the steward reads its delta.
    let created = temper_mcp::tools::cognitive_maps::cogmap_create(
        svc,
        parts,
        input(json!({ "name": "Machine Map", "telos_title": "Machine telos" })),
    )
    .await
    .expect("cogmap_create");
    let created = one_text(&created);
    let cogmap = created["cogmap_id"]
        .as_str()
        .or_else(|| created["id"].as_str())
        .unwrap_or_else(|| panic!("cogmap id in {created}"))
        .to_string();
    let listed = temper_mcp::tools::cognitive_maps::cogmap_list(svc, parts, input(json!({})))
        .await
        .expect("cogmap_list");
    assert!(listed.content.len() >= 2, "notice then rows");

    // steward — the delta over the machine's own map.
    let delta = temper_mcp::tools::steward::steward_ingest_delta(
        svc,
        parts,
        input(json!({ "cogmap": cogmap })),
    )
    .await
    .expect("steward_ingest_delta");
    assert_eq!(one_text(&delta)["cogmap_id"], cogmap.as_str());

    // invocations — the machine's own list.
    temper_mcp::tools::invocations::invocation_read(svc, parts, input(json!({ "view": "list" })))
        .await
        .expect("invocation_read list");

    // ingest — a segmented ingest begins in the team context.
    let segment = "machine segment zero";
    let begun = temper_mcp::tools::ingest::segmented_ingest(
        svc,
        parts,
        input(json!({
            "action": "begin",
            "context_ref": m.context.to_string(),
            "doc_type_name": "research",
            "title": "machine segmented",
            "content": segment,
            "content_hash": temper_core::hash::sha256_hex(segment.as_bytes()),
        })),
    )
    .await
    .expect("segmented_ingest begin");
    let begun = one_text(&begun);
    assert!(
        begun["resource_id"].is_string() || begun["id"].is_string(),
        "{begun}"
    );

    // data artifacts and shapes — the gated lists.
    temper_mcp::tools::data_artifacts::list_artifacts(
        svc,
        parts,
        input(json!({ "resource_id": resource })),
    )
    .await
    .expect("list_data_artifacts");
    temper_mcp::tools::data_artifact_shapes::list_shapes(
        svc,
        parts,
        input(json!({ "home_type": "context", "home_id": m.context })),
    )
    .await
    .expect("list_data_artifact_shapes");

    // citation audits — a finding the person wrote, read-granted to the machine, audited by it.
    let (block, cited) = seed_finding(&m).await;
    temper_mcp::tools::citation_audits::record_citation_audit(
        svc,
        parts,
        input(json!({
            "block_id": block,
            "source": { "kind": "resource", "value": cited },
            "value": 0.8,
            "reason": "a machine audits a citation",
        })),
    )
    .await
    .expect("record_citation_audit");

    // schema — edge-local.
    temper_mcp::tools::doc_types::describe_schema(
        svc,
        parts,
        input(json!({ "view": "doc_types" })),
    )
    .await
    .expect("describe_schema");
}

/// A finding the harness person authored in their own context, with one block citing one live
/// source, and a read-only grant to the machine — the audit gate admits a reader who neither
/// authored nor may modify the finding. Returns `(block_id, source_id)`.
async fn seed_finding(m: &Machine) -> (Uuid, Uuid) {
    let pool = &m.app.pool;
    let own_context: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_contexts WHERE owner_table = 'kb_profiles' AND owner_id = $1 LIMIT 1",
    )
    .bind(m.admin)
    .fetch_one(pool)
    .await
    .expect("the person's own context");
    let finding = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, 'finding', $2)")
        .bind(finding)
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
    .bind(own_context)
    .bind(m.admin)
    .execute(pool)
    .await
    .expect("home finding");
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
              granted_by_profile_id) \
          VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
    )
    .bind(finding)
    .bind(m.profile)
    .bind(m.admin)
    .execute(pool)
    .await
    .expect("read-only grant to the machine");
    let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("an event");
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
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, 'cited', $2)")
        .bind(source)
        .bind(format!("test://{source}"))
        .execute(pool)
        .await
        .expect("insert source");
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
    .expect("cite the source");
    (block, source)
}

/// The blob family needs a store, so it runs on its own app: the machine commits a blob homed in
/// its team context and reads the bytes back.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_commits_and_reads_a_blob(pool: sqlx::PgPool) {
    const SINGLE_REQUEST_MAX_BYTES: usize = 64;
    let store = std::sync::Arc::new(temper_substrate::blob_store::InMemoryBlobStore::default());
    let app = common::setup_with_blob_store_shared(pool, store, SINGLE_REQUEST_MAX_BYTES).await;
    let svc = app
        .mcp_relay_service_with_blob(SINGLE_REQUEST_MAX_BYTES)
        .await;
    let m = machine_on(app, svc).await;

    let payload: &[u8] = b"machine blob";
    let committed = temper_mcp::tools::blobs::blob_manage(
        &m.svc,
        &m.parts,
        input(json!({
            "action": "commit",
            "home_table": "kb_contexts",
            "home_id": m.context.to_string(),
            "content_type": "application/pdf",
            "content": base64::engine::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                payload,
            ),
        })),
    )
    .await
    .expect("blob_manage commit");
    let blob_id = one_text(&committed)["blob_id"]
        .as_str()
        .expect("blob_id")
        .to_string();

    let read = temper_mcp::tools::blobs::blob_read(
        &m.svc,
        &m.parts,
        input(json!({ "action": "read", "blob_id": blob_id })),
    )
    .await
    .expect("blob_read");
    assert_eq!(one_text(&read)["content_bytes"], json!(payload.len()));
}
