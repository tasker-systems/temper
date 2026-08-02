#![cfg(feature = "test-db")]

//! End-to-end coverage for the resource facet read — goal
//! `019fafd9-a978-7860-ae39-23958b4471b8`.
//!
//! Drives the real `temper-client` → `temper-api` → Postgres path, which is the wire
//! `temper resource facets <ref>` uses. The service layer is covered by
//! `crates/temper-api/tests/resource_facet_read_test.rs`; what only this level can show is that the
//! route, its gate and its refusal compose correctly through the deployed surface — and that the
//! weight and the author survive serialization rather than being dropped somewhere between the SQL
//! and the wire.
//!
//! **Mirrors `edge_facet_e2e_test.rs` in shape deliberately, and not in substance.** The two reads
//! are not interchangeable: the gates differ (`resources_visible_to` vs `edges_visible_to`), and an
//! edge's facets fold with the edge where a resource's do not. Same test shape, different subject —
//! which is what the parity clause asks for and what porting the edge read by analogy would have
//! gotten wrong.

mod common;

use temper_core::types::facet_requests::FacetSetRequest;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_core::types::resource_grant::ResourceGrantBody;
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};
use uuid::Uuid;

/// An MCP service over the same pool. Local to this file, matching `auth_seam_m2m_e2e.rs`'s own
/// copy — the suite's per-file convention, and the reason both files can evolve their harness
/// independently.
async fn build_mcp_service(pool: &sqlx::PgPool) -> temper_mcp::service::TemperMcpService {
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: common::TEST_AUDIENCE.to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
    };
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    temper_mcp::service::TemperMcpService::new(state)
}

/// Synthetic request parts carrying a `client_credentials` claim set — a machine principal as the
/// MCP gate sees one.
fn machine_parts(client_id: &str) -> axum::http::request::Parts {
    axum::http::Request::builder()
        .extension(temper_mcp::middleware::BearerToken("synthetic".to_string()))
        .extension(temper_services::auth::RawJwtClaims {
            sub: format!("{client_id}@clients"),
            email: None,
            email_verified: None,
            azp: Some(client_id.to_string()),
            gty: Some("client-credentials".to_string()),
            exp: 0,
            iat: 0,
        })
        .body(())
        .expect("build request")
        .into_parts()
        .0
}

/// Seed a resource via the API client. Mirrors `edge_facet_e2e_test.rs`'s helper — the suite's
/// per-file convention.
async fn seed(client: &temper_client::TemperClient, context: &str, slug: &str) -> Uuid {
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        title: format!("Resource facet e2e {slug}"),
        origin_uri: format!("kb://{context}/research/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        goal: None,
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    let row = client
        .ingest()
        .create(&payload)
        .await
        .expect("seed resource via client");
    Uuid::from(row.id)
}

async fn set_facet(
    client: &temper_client::TemperClient,
    resource: Uuid,
    values: serde_json::Value,
    weight: f64,
) {
    client
        .facets()
        .set(&FacetSetRequest {
            resource,
            values,
            weight,
            act: Default::default(),
        })
        .await
        .expect("set a facet on the resource");
}

/// The whole arc through the deployed surface: empty before, every live mark visible after, each
/// with its weight and its author.
///
/// The empty read is asserted **first and as a success**, because that is the half a collection
/// surface usually gets wrong: `readable but nothing asserted` must be `200 []`, distinguishable
/// from the `404` an unreadable resource gets. The register left that shape question to the build.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_live_mark_is_readable_through_the_deployed_surface(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("resfacet", None)
        .await
        .expect("create resfacet context");

    let resource = seed(&app.client, "resfacet", "marked-up").await;

    // Readable, nothing asserted — a success carrying an empty list, not a refusal.
    let before = app
        .client
        .facets()
        .list_for_resource(resource)
        .await
        .expect("a readable resource with no facets must not refuse");
    assert_eq!(before.resource, resource);
    assert!(
        before.facets.is_empty(),
        "nothing asserted yet: {:?}",
        before.facets
    );

    // Two asserts of one logical facet. Since `019f6d08` the second SUPERSEDES the first per inner
    // key, so what crosses the wire is one live mark per key — not one row per assert.
    set_facet(
        &app.client,
        resource,
        serde_json::json!({"node_label": "concern", "status": "open"}),
        0.85,
    )
    .await;
    set_facet(
        &app.client,
        resource,
        serde_json::json!({"node_label": "concern", "status": "resolved"}),
        1.0,
    )
    .await;

    let read = app
        .client
        .facets()
        .list_for_resource(resource)
        .await
        .expect("read the resource's facets");

    assert_eq!(
        read.facets.len(),
        2,
        "one live row per inner key must cross the wire, not a collapsed object: {:?}",
        read.facets
    );
    let mark = |key: &str| {
        read.facets
            .iter()
            .find(|f| f.value.get(key).is_some())
            .unwrap_or_else(|| panic!("no live mark for {key}: {:?}", read.facets))
    };
    assert_eq!(mark("status").value["status"], "resolved");
    assert_eq!(mark("node_label").value["node_label"], "concern");

    // Weight is what the region producer clusters on, and what `open_meta` drops entirely. If it
    // were lost in serialization the read would answer the wrong question while looking right.
    // Both marks carry the SECOND assert's weight, because it named both keys.
    assert_eq!(mark("status").weight, 1.0);
    assert_eq!(mark("node_label").weight, 1.0);

    let me = app.client.profile().get().await.expect("whoami");
    assert_eq!(
        read.facets[0].authored_by_profile_id,
        Some(me.id),
        "the row names the profile that asserted it"
    );
    assert!(
        read.facets[0].authored_by_handle.is_some(),
        "the author's handle rides along, so a reader needs no second round trip"
    );
    assert_ne!(
        read.facets[0].authored_by_event_id,
        Uuid::nil(),
        "the authoring act is the row's replay-stable identity"
    );

    // The frontmatter door, on the same resource, at the same moment: still one value, still no
    // weight. Not a defect being tolerated — it is the collapsed projection, and this read is the
    // faithful one. Asserted here so the two surfaces' disagreement is pinned at the wire level and
    // any future change to either is a test failure rather than a silent divergence.
    let detail = app
        .client
        .resources()
        .get_meta(resource)
        .await
        .expect("get_meta through the deployed surface");
    let open = detail.open_meta.expect("open_meta present");
    assert_eq!(
        open["facet"]["status"], "resolved",
        "the collapse discloses the newest row only: {open:#?}"
    );
}

/// The refusal face at the HTTP layer: unreadable and never-existed are the same answer.
///
/// The service-level test asserts the two `ApiError`s carry identical messages; this asserts the
/// property that actually reaches a caller — same status, same body. A handler that mapped one arm
/// to `403` would satisfy the service test and still ship an existence oracle, which is why this is
/// checked at the door rather than only underneath it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unreadable_resource_and_a_nonexistent_one_answer_identically(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("resfacetauthz", None)
        .await
        .expect("create context");

    let resource = seed(&app.client, "resfacetauthz", "private").await;
    set_facet(
        &app.client,
        resource,
        serde_json::json!({"node_label": "fact"}),
        1.0,
    )
    .await;

    // A provisioned, approved second profile — a legitimate principal, just not this resource's.
    let stranger_token = common::generate_second_user_jwt();
    common::provision_and_approve_second(&app).await;

    let hidden = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{resource}/facets")))
        .bearer_auth(&stranger_token)
        .send()
        .await
        .expect("stranger read of a private resource's facets");
    let hidden_status = hidden.status();
    let hidden_body = hidden.text().await.expect("hidden body");

    let never_existed = Uuid::now_v7();
    let absent = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{never_existed}/facets")))
        .bearer_auth(&stranger_token)
        .send()
        .await
        .expect("stranger read of an id that never existed");
    let absent_status = absent.status();
    let absent_body = absent.text().await.expect("absent body");

    assert_eq!(
        hidden_status,
        reqwest::StatusCode::NOT_FOUND,
        "an unreadable resource's facets must 404, not 403: {hidden_body}"
    );
    assert_eq!(
        absent_status,
        reqwest::StatusCode::NOT_FOUND,
        "an id that never existed must 404: {absent_body}"
    );
    assert_eq!(
        hidden_body, absent_body,
        "the two refusals must be byte-identical, or the diff between them is an existence oracle"
    );
}

// ── the MCP door, and the equivalence the register flagged as most likely wrong ──────────────────

/// The MCP surface, read by a **machine principal holding read-only reach** — two things at once,
/// because they are the same call.
///
/// The register's closure asserts *"human and machine principals are interchangeable for this read —
/// both resolve through `resources_visible_to`"*, marks it **most likely to be wrong**, and names a
/// read-only-reach machine as where to attack it. The structural argument is that every authenticated
/// principal reaches a handler as a resolved `Profile` (`AuthenticatedProfile::profile`) and the gate
/// consumes a profile id, so there is no principal-kind branch to get wrong. That argument is worth
/// exactly as much as an empirical probe of the live predicate, which is to say less — so this is the
/// probe: a registered machine, approved, granted `can_read` and nothing else, reading facets it did
/// not write, through the MCP tool rather than through the service it wraps.
///
/// The second half is clause `reading-a-facet-never-becomes-a-licence-to-assert-one` on the surface
/// where it is easiest to get wrong: an MCP tool list is a capability menu, and a machine that can
/// see `resource_facets` must not thereby reach `facet_set`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_read_only_machine_principal_reads_facets_on_mcp_and_cannot_assert_one(
    pool: sqlx::PgPool,
) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("resfacetmachine", None)
        .await
        .expect("create context");

    // The human owns the resource and writes the facet. The machine never authored it — which is the
    // point: the read has to disclose someone else's mark, attributed to them.
    let resource = seed(&app.client, "resfacetmachine", "steward-readable").await;
    set_facet(
        &app.client,
        resource,
        serde_json::json!({"node_label": "concern", "status": "open", "severity": "high"}),
        0.9,
    )
    .await;
    let human = app.client.profile().get().await.expect("whoami");

    // A registered machine principal, born Denied and approved — the `auth_seam_m2m_e2e` shape.
    let machine_profile = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-facet-reader', 'agent-facet-reader', NULL, '{}')",
    )
    .bind(machine_profile)
    .execute(&pool)
    .await
    .expect("seed machine profile");
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ('facet-reader-client', 'test', $1, $1)",
    )
    .bind(machine_profile)
    .execute(&pool)
    .await
    .expect("seed machine registration");
    common::approve(&pool, machine_profile).await;

    let svc = build_mcp_service(&pool).await;
    svc.ensure_profile_from_parts(&machine_parts("facet-reader-client"))
        .await
        .expect("the mcp gate admits a registered, approved machine");

    // ── the negative control, and it is not optional ──────────────────────────────────────────────
    // Without this, the success below proves nothing: a read that would have succeeded anyway —
    // because the resource turned out to be reachable by any admitted principal — is a probe that
    // cannot fail. Admitted-but-ungranted must refuse first, so the grant is demonstrably what
    // changes the answer.
    let before_grant = temper_mcp::tools::facets::resource_facets(
        &svc,
        temper_mcp::tools::facets::ResourceFacetsInput {
            resource: resource.to_string(),
        },
    )
    .await;
    assert!(
        before_grant.is_err(),
        "an admitted machine with no reach on this resource must be refused: {before_grant:?}"
    );

    // Read-only reach, granted by the resource's owner through the real door. `can_write: false` is
    // the whole experiment — the auditor precedent is that a *write* grant here would make the
    // principal `can_modify_resource` and change what the second half of this test measures.
    app.client
        .resources()
        .grant(
            resource,
            &ResourceGrantBody {
                principal_table: "kb_profiles".to_string(),
                principal_id: machine_profile,
                can_read: true,
                can_write: false,
                can_delete: false,
                can_grant: false,
            },
        )
        .await
        .expect("owner grants the machine read-only reach");

    let read = temper_mcp::tools::facets::resource_facets(
        &svc,
        temper_mcp::tools::facets::ResourceFacetsInput {
            resource: resource.to_string(),
        },
    )
    .await
    .expect("a read-only machine principal may read facets through MCP");

    // The MCP tool returns its payload as text content; parse it back rather than trusting a shape.
    let text = read
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<String>();
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("tool output is JSON");
    let facets = parsed["facets"].as_array().expect("facets array");

    // The human asserted ONE facet with three inner keys, which is three rows at the stored grain
    // (`019f6d08`). The machine sees every one of them — a read-only principal is not shown a
    // summarized subset.
    assert_eq!(facets.len(), 3, "the human's three marks: {parsed:#}");
    let severity = facets
        .iter()
        .find(|f| f["value"].get("severity").is_some())
        .unwrap_or_else(|| panic!("no severity mark: {parsed:#}"));
    assert_eq!(severity["weight"], 0.9, "weight survives the MCP door too");
    assert_eq!(
        severity["value"]["severity"], "high",
        "the value arrives whole, not summarized"
    );
    assert_eq!(
        facets[0]["authored_by_profile_id"],
        serde_json::json!(human.id),
        "attributed to the human who wrote it, not the machine that read it"
    );

    // And the read is not a licence. Same principal, same session, the write arm of the same tool
    // family — refused.
    let denied = temper_mcp::tools::facets::facet_set(
        &svc,
        serde_json::from_value(serde_json::json!({
            "resource": resource.to_string(),
            "values": {"node_label": "planted-by-a-reader"},
            "weight": 1.0,
        }))
        .expect("build facet_set input"),
    )
    .await;
    // Pinned to the *modification* refusal, not merely to "an error". `facet_set`'s error mapping
    // sends a ref-parse failure and an authorization denial to the same `invalid_params` code, so an
    // `is_err()` assertion would pass if this test's ref were malformed — the failure would look like
    // a working authorization gate.
    let denial = denied.expect_err("read-only reach must not authorize a facet assert on MCP");
    assert!(
        denial.message.contains("cannot modify"),
        "the refusal must be the modification gate, not a parse error: {denial:?}"
    );

    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = $1
            AND property_key = 'facet' AND NOT is_folded",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .expect("count live facets");
    assert_eq!(
        live, 3,
        "the denied assert wrote nothing — the human's three marks are all that remain, and \
         auth-before-writes holds on the MCP surface too"
    );
}

// ── the CLI door, driving the real binary ────────────────────────────────────────────────────────

/// `temper resource facets <ref>` through the **actual `temper` binary**, not through the client
/// library it wraps.
///
/// The HTTP test above covers the CLI's transport (the CLI *is* a `temper-client` caller) and
/// `commands::facet`'s unit tests pin its argument surface — but neither executes the binary, so
/// neither would notice a rendering path that printed nothing, dropped `weight` on its way through
/// `format::render`, or exited non-zero while looking fine. That is the whole gap this closes: the
/// door a human or an agent actually types into is the one least covered by testing the layer beneath
/// it.
///
/// **Local runs need a fresh bin.** The helper spawns `target/<profile>/temper`, and building the
/// e2e crate does not rebuild temper-cli's separate bin target — so a new subcommand fails here as
/// `unrecognized subcommand` until `cargo build -p temper-cli --bin temper` runs. CI builds it first.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_cli_prints_every_live_facet_with_its_weight(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("resfacetcli", None)
        .await
        .expect("create context");

    let resource = seed(&app.client, "resfacetcli", "cli-readable").await;
    set_facet(
        &app.client,
        resource,
        serde_json::json!({"node_label": "concern", "status": "open"}),
        0.85,
    )
    .await;
    set_facet(
        &app.client,
        resource,
        serde_json::json!({"node_label": "concern", "status": "resolved"}),
        1.0,
    )
    .await;

    let out = common::run_temper_cli(
        &app,
        &[
            "resource",
            "facets",
            &resource.to_string(),
            "--format",
            "json",
        ],
    )
    .await
    .expect("spawn the temper binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the CLI must exit 0.\nstdout: {stdout}\nstderr: {stderr}"
    );

    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("CLI stdout is not JSON ({e}).\nstdout: {stdout}\nstderr: {stderr}")
    });
    let facets = parsed["facets"]
        .as_array()
        .unwrap_or_else(|| panic!("no facets array in CLI output: {parsed:#}"));

    assert_eq!(
        facets.len(),
        2,
        "the CLI must print every live mark, not a collapsed object: {parsed:#}"
    );
    let cli_mark = |key: &str| {
        facets
            .iter()
            .find(|f| f["value"].get(key).is_some())
            .unwrap_or_else(|| panic!("no live mark for {key}: {parsed:#}"))
    };
    assert_eq!(cli_mark("status")["value"]["status"], "resolved");
    assert_eq!(cli_mark("node_label")["value"]["node_label"], "concern");
    assert_eq!(
        cli_mark("status")["weight"],
        1.0,
        "weight must survive the CLI's own rendering path: {parsed:#}"
    );
    assert!(
        facets[0]["created"].is_string(),
        "`created` is what makes the double-assert legible to a human reading the output: {parsed:#}"
    );
}
