#![cfg(feature = "test-db")]

//! E2E: data artifact commit → show round-trip through the real API stack.
//!
//! Drives the typed `TemperClient` sub-client (`data_artifacts().commit` / `.get`) against
//! the in-process Axum server backed by an isolated `#[sqlx::test]` database — the same
//! harness pattern `resource_crud_test.rs` uses. Asserts byte-identical content round-trip
//! and field-level correctness, then exercises `supersedes` folding.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactListParams, ArtifactPageRequest, ARTIFACT_DRAIN_PAGE_ROWS,
    MAX_ARTIFACT_PAGE_ROWS,
};
use temper_core::types::ids::DataArtifactId;
use temper_workflow::types::resource::ResourceCreateRequest;

/// Commit a data artifact via the API, then get it back and verify:
///
/// - content round-trips byte-identical (structural `serde_json::Value` equality)
/// - `content_hash` and `content_bytes` are stable across the round-trip
/// - all `ArtifactView` fields match the commit request (kind, intent, precedence)
/// - `is_folded` is `false` for a freshly committed artifact with no supersedes
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn data_artifact_commit_show_round_trip(pool: PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-artifact-rt", None)
        .await
        .expect("context create failed");

    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/artifact-rt".to_string(),
            title: "Artifact Round-Trip Test".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let content = json!({
        "measurement": "temperature",
        "value": 42.5,
        "unit": "celsius",
        "metadata": {
            "sensor": "thermocouple-1",
            "calibrated": true,
            "tags": ["lab", "upstairs"]
        }
    });

    let committed = app
        .client
        .data_artifacts()
        .commit(
            resource.id.into(),
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: content.clone(),
                supersedes: Vec::new(),
                act: Default::default(),
            },
        )
        .await
        .expect("artifact commit failed");

    // Commit response fields
    assert_eq!(committed.artifact.artifact_kind, "measurement");
    assert_eq!(committed.artifact.intent, "current");
    assert_eq!(committed.artifact.precedence, 0.0);
    assert!(
        !committed.artifact.is_folded,
        "a freshly committed artifact must not be folded"
    );
    assert_eq!(
        committed.artifact.resource_id, resource.id,
        "the artifact must be owned by the resource it was committed to"
    );

    // Get it back
    let retrieved = app
        .client
        .data_artifacts()
        .get(resource.id.into(), committed.artifact_id.into())
        .await
        .expect("artifact get failed");

    // Byte-identical content: structural equality of the JSON value
    assert_eq!(
        retrieved.content,
        Some(content.clone()),
        "content must round-trip byte-identical through the API"
    );

    // Hash and byte-count stability across the round-trip
    assert_eq!(
        retrieved.content_hash, committed.artifact.content_hash,
        "content_hash must be stable across commit → get"
    );
    assert_eq!(
        retrieved.content_bytes, committed.artifact.content_bytes,
        "content_bytes must be stable across commit → get"
    );

    // All fields match between commit and get responses
    assert_eq!(retrieved.artifact_id, committed.artifact_id);
    assert_eq!(retrieved.resource_id, committed.artifact.resource_id);
    assert_eq!(retrieved.artifact_kind, committed.artifact.artifact_kind);
    assert_eq!(retrieved.intent, committed.artifact.intent);
    assert_eq!(retrieved.precedence, committed.artifact.precedence);
    assert_eq!(retrieved.is_folded, committed.artifact.is_folded);
    assert_eq!(retrieved.shape_state, committed.artifact.shape_state);
}

/// Commit two artifacts of the same family with intent `current`; the second declares
/// `supersedes: [first]`. Assert:
///
/// - the first artifact becomes `is_folded: true`
/// - the second artifact is `is_folded: false`
/// - the default list (no `include_folded`) returns only the live one
/// - the list with `include_folded: true` returns both
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn data_artifact_supersedes_folding(pool: PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-artifold", None)
        .await
        .expect("context create failed");

    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/artifold".to_string(),
            title: "Artifact Fold Test".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    // First artifact
    let first = app
        .client
        .data_artifacts()
        .commit(
            resource.id.into(),
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: json!({ "value": 1 }),
                supersedes: Vec::new(),
                act: Default::default(),
            },
        )
        .await
        .expect("first artifact commit failed");

    assert!(
        !first.artifact.is_folded,
        "first artifact must start unfolded"
    );

    // Second artifact that supersedes the first
    let second = app
        .client
        .data_artifacts()
        .commit(
            resource.id.into(),
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: json!({ "value": 2 }),
                supersedes: vec![first.artifact_id],
                act: Default::default(),
            },
        )
        .await
        .expect("second artifact commit failed");

    assert!(
        !second.artifact.is_folded,
        "the superseding artifact must not be folded"
    );

    // Get the first back — it must now be folded
    let first_after = app
        .client
        .data_artifacts()
        .get(resource.id.into(), first.artifact_id.into())
        .await
        .expect("get first artifact after supersession");

    assert!(
        first_after.is_folded,
        "the superseded artifact must be folded after a declared supersession"
    );

    // Default list (include_folded = false) → only the live (second) artifact
    let live_artifacts = app
        .client
        .data_artifacts()
        .list_all(
            resource.id.into(),
            &ArtifactListParams {
                kind: Some("measurement".to_string()),
                include_folded: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("list live artifacts failed");

    assert_eq!(
        live_artifacts.len(),
        1,
        "default list must return only the live (non-folded) artifact"
    );
    assert_eq!(
        live_artifacts[0].artifact_id, second.artifact_id,
        "the live artifact must be the superseding one"
    );

    // List with include_folded = true → both artifacts
    let all_artifacts = app
        .client
        .data_artifacts()
        .list_all(
            resource.id.into(),
            &ArtifactListParams {
                kind: Some("measurement".to_string()),
                include_folded: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("list all artifacts failed");

    assert_eq!(
        all_artifacts.len(),
        2,
        "list with include_folded must return both the folded and live artifacts"
    );
}

// ── The bounded per-resource list ─────────────────────────────────────────────

/// A resource carrying `count` live artifacts of one family, committed one after another so
/// `(created, id)` order is commit order. Returns the resource and the ids in that order.
async fn resource_with_artifacts(
    app: &common::E2eTestApp,
    slug: &str,
    count: usize,
) -> (uuid::Uuid, Vec<DataArtifactId>) {
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    let context = app
        .client
        .contexts()
        .create(slug, None)
        .await
        .expect("context create failed");
    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: format!("test://e2e/{slug}"),
            title: format!("Bounded list {slug}"),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let mut ids = Vec::with_capacity(count);
    for n in 0..count {
        let committed = app
            .client
            .data_artifacts()
            .commit(
                resource.id.into(),
                &ArtifactCommitRequest {
                    kind: "measurement".to_string(),
                    kind_owner: None,
                    intent: "member".to_string(),
                    precedence: 0.0,
                    content: json!({ "n": n }),
                    supersedes: Vec::new(),
                    act: Default::default(),
                },
            )
            .await
            .expect("artifact commit failed");
        ids.push(committed.artifact_id);
    }
    (resource.id.into(), ids)
}

/// **The C2 witness, through the real door.** Five artifacts, `limit = 2`: the pages hold 2, 2
/// and 1, `next` is `null` on the last page and only there, the union is exactly the five
/// committed artifacts in order, and none repeats.
///
/// Witnesses the cursor contract over HTTP. It does not witness the SQL bound (the readback
/// truncates in Rust; see `the_sql_arity_returns_at_most_p_limit_rows` in temper-substrate). It
/// fails if the keyset predicate is dropped (every page then restarts and the union repeats).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn bounded_list_pages_two_two_one_with_an_exact_union(pool: PgPool) {
    let app = common::setup(pool).await;
    let (resource, committed) = resource_with_artifacts(&app, "e2e-page-witness", 5).await;

    let mut after = None;
    let mut sizes = Vec::new();
    let mut nexts = Vec::new();
    let mut seen = Vec::new();
    loop {
        let page = app
            .client
            .data_artifacts()
            .list_page(
                resource,
                &ArtifactListParams::default(),
                ArtifactPageRequest { limit: 2, after },
            )
            .await
            .expect("list_page failed");
        sizes.push(page.artifacts.len());
        nexts.push(page.next.is_some());
        seen.extend(page.artifacts.iter().map(|a| a.artifact_id));
        match page.next {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }

    assert_eq!(sizes, vec![2, 2, 1], "pages of two, two and one");
    assert_eq!(
        nexts,
        vec![true, true, false],
        "`next` is null on the last page and only there"
    );
    assert_eq!(
        seen, committed,
        "the union is exactly the committed artifacts, in order, with no repeats"
    );
}

/// With `limit` the wire answers the `{ artifacts, next }` envelope, and `next` is a legible
/// `(created, artifact_id)` object rather than an opaque token.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_list_with_limit_answers_the_envelope_with_a_legible_cursor(pool: PgPool) {
    let app = common::setup(pool).await;
    let (resource, committed) = resource_with_artifacts(&app, "e2e-envelope", 3).await;

    let answer = app
        .client
        .data_artifacts()
        .list(
            resource,
            &ArtifactListParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .expect("bounded list failed");

    assert_eq!(answer["artifacts"].as_array().map(Vec::len), Some(2));
    assert!(
        answer["next"]["created"].is_string(),
        "next.created, got {answer}"
    );
    assert_eq!(
        answer["next"]["artifact_id"],
        json!(committed[1]),
        "the cursor names the last artifact on the page"
    );
}

/// A call with no `limit` still answers the bare array of every artifact, exactly as the route
/// did before it could be bounded (ruling P11).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_list_without_limit_still_answers_the_bare_array(pool: PgPool) {
    let app = common::setup(pool).await;
    let (resource, committed) = resource_with_artifacts(&app, "e2e-bare-array", 3).await;

    let answer = app
        .client
        .data_artifacts()
        .list(resource, &ArtifactListParams::default())
        .await
        .expect("unbounded list failed");

    let ids: Vec<_> = answer
        .as_array()
        .unwrap_or_else(|| panic!("a no-limit list is a bare array, got {answer}"))
        .iter()
        .map(|a| a["artifact_id"].clone())
        .collect();
    assert_eq!(
        ids,
        committed.iter().map(|id| json!(id)).collect::<Vec<_>>()
    );
}

/// A `limit` of zero and one above the ceiling are refused, each naming the parameter at fault;
/// neither is clamped into a page.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_limit_of_zero_or_above_the_ceiling_is_refused_by_name(pool: PgPool) {
    let app = common::setup(pool).await;
    let (resource, _) = resource_with_artifacts(&app, "e2e-limit-refusal", 1).await;

    for (limit, named) in [
        (0, "`limit`"),
        (MAX_ARTIFACT_PAGE_ROWS + 1, "MAX_ARTIFACT_PAGE_ROWS"),
    ] {
        let refused = app
            .client
            .data_artifacts()
            .list(
                resource,
                &ArtifactListParams {
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await
            .expect_err("an out-of-range limit must be refused, not clamped");
        let said = refused.to_string();
        assert!(said.contains("400"), "a 400-class refusal, got: {said}");
        assert!(
            said.contains(named),
            "the refusal names {named}, got: {said}"
        );
    }
}

/// A continuation position sent without `limit` is refused, not ignored: ignoring it would
/// answer the whole list again to a caller that thinks it is paging.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_continuation_position_without_limit_is_refused(pool: PgPool) {
    let app = common::setup(pool).await;
    let (resource, committed) = resource_with_artifacts(&app, "e2e-after-refusal", 2).await;

    let refused = app
        .client
        .data_artifacts()
        .list(
            resource,
            &ArtifactListParams {
                after_created: Some(chrono::Utc::now()),
                after_id: Some(committed[0].uuid()),
                ..Default::default()
            },
        )
        .await
        .expect_err("`after_*` without `limit` must be refused");
    assert!(
        refused.to_string().contains("require `limit`"),
        "got: {refused}"
    );
}

/// **The C3 drain.** A caller whose contract is "everything" gets everything over a fixture
/// larger than one drain page: one more artifact than `ARTIFACT_DRAIN_PAGE_ROWS` comes back whole,
/// in order, none repeated.
///
/// Fails if the drain stops after the first page (it would return `ARTIFACT_DRAIN_PAGE_ROWS`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn draining_the_list_returns_everything_across_pages(pool: PgPool) {
    let app = common::setup(pool).await;
    let count = ARTIFACT_DRAIN_PAGE_ROWS as usize + 1;
    let (resource, committed) = resource_with_artifacts(&app, "e2e-drain", count).await;

    let drained = app
        .client
        .data_artifacts()
        .list_all(resource, &ArtifactListParams::default())
        .await
        .expect("drain failed");

    assert_eq!(
        drained.iter().map(|a| a.artifact_id).collect::<Vec<_>>(),
        committed,
        "every artifact, in order, none repeated"
    );
}

// ── The flat artifact read (route-first for beat G4) ───────────────────────────

/// A `TemperClient` bound to an arbitrary principal's token — the
/// `team_invitations_test.rs` idiom, for a second identity's gates.
fn client_for(app: &common::E2eTestApp, token: &str) -> temper_client::TemperClient {
    use temper_client::auth::{MemoryTokenStore, Provider, StoredAuth};

    let stored_auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: token.to_string().into(),
        refresh_token: None,
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-test-device".to_string()),
    };
    let store: std::sync::Arc<dyn temper_client::auth::TokenStore> =
        std::sync::Arc::new(MemoryTokenStore::with_auth(stored_auth));
    temper_client::config::build_client_from(
        &app.config,
        store,
        temper_workflow::operations::Surface::Sdk,
    )
    .expect("second client builds")
}

/// The flat read answers by artifact id alone, including FOLDED artifacts, and 404s
/// a caller who cannot see the owning resource — the direct MCP tool's posture, now
/// carried by `GET /api/data-artifacts/{artifact_id}` (route-first for beat G4).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn flat_artifact_get_by_id_answers_folded_rows_and_gates_invisible_callers(pool: PgPool) {
    let app = common::setup(pool).await;
    let context = app
        .client
        .contexts()
        .create("e2e-flat-get", None)
        .await
        .expect("context create failed");
    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/flat-get".to_string(),
            title: "Flat Get".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let commit = |content: serde_json::Value| ArtifactCommitRequest {
        kind: "measurement".to_string(),
        kind_owner: None,
        intent: "current".to_string(),
        precedence: 0.0,
        content,
        supersedes: Vec::new(),
        act: Default::default(),
    };
    let first = app
        .client
        .data_artifacts()
        .commit(resource.id.into(), &commit(json!({"v": 1})))
        .await
        .expect("first commit failed");
    let second = app
        .client
        .data_artifacts()
        .commit(
            resource.id.into(),
            &ArtifactCommitRequest {
                supersedes: vec![first.artifact_id],
                ..commit(json!({"v": 2}))
            },
        )
        .await
        .expect("superseding commit failed");
    assert_ne!(first.artifact_id, second.artifact_id);

    // The flat read answers the FOLDED artifact — the posture the nested route's
    // REST-parent path shares but the list+filter composition cannot.
    let folded = app
        .client
        .data_artifacts()
        .get_by_id(first.artifact_id.into())
        .await
        .expect("flat get of the folded artifact failed");
    assert!(folded.is_folded, "the superseded artifact reads folded");
    assert_eq!(folded.artifact_id, first.artifact_id);

    // A caller who cannot see the owning resource is refused with the uniform 404.
    let outsider_token = common::generate_second_user_jwt();
    let _ = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&outsider_token)
        .send()
        .await
        .expect("provision the outsider");
    let outsider_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("second@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("the outsider profile");
    common::approve(&app.pool, outsider_id).await;

    let outsider = client_for(&app, &outsider_token);
    let refused = outsider
        .data_artifacts()
        .get_by_id(first.artifact_id.into())
        .await
        .expect_err("the outsider's flat get must refuse");
    assert!(
        matches!(refused, temper_client::error::ClientError::NotFound { .. }),
        "an invisible artifact is not-found, never a leak: {refused}"
    );
}

// ── The cogmap-home shapes pair (route-first for beat G4) ──────────────────────

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`) —
/// every approved profile READS it; nobody holds write without an explicit grant.
const L0_COGMAP: uuid::Uuid = uuid::Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// A shape declares on a cognitive-map home through `POST /api/cognitive-maps/{id}/shapes`,
/// reads back through the list and the by-id get, and the authoring gate refuses a
/// reader without an explicit write grant with 403 — the context pair's gate train,
/// now carried by the cogmap-home twin (route-first for beat G4).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_home_shapes_declare_list_get_round_trip_and_authority_gate(pool: PgPool) {
    use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};

    let app = common::setup(pool).await;

    let owner_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("the harness profile");
    common::grant_cogmap_write(&app.pool, L0_COGMAP, owner_id).await;

    let request = ShapeDeclareRequest {
        kind: "measurement".to_string(),
        kind_owner: None,
        schema: serde_json::json!({
            "type": "object",
            "properties": { "value": { "type": "number" } },
            "required": ["value"]
        }),
        enforcement: EnforcementMode::Advisory,
        act: Default::default(),
    };

    let declared = app
        .client
        .data_artifacts()
        .declare_cogmap_shape(L0_COGMAP, &request)
        .await
        .expect("declare on a granted cogmap home failed");
    assert_eq!(
        declared.home_anchor_table, "kb_cogmaps",
        "the shape is homed on the cognitive map, not defaulted to a context"
    );

    let listed = app
        .client
        .data_artifacts()
        .list_cogmap_shapes(L0_COGMAP)
        .await
        .expect("cogmap-home list failed");
    assert!(
        listed.iter().any(|s| s.shape_id == declared.shape_id),
        "the declared shape reads back through the cogmap-home list"
    );

    let fetched = app
        .client
        .data_artifacts()
        .get_shape(declared.shape_id.uuid())
        .await
        .expect("shape get failed");
    assert_eq!(fetched.shape_id, declared.shape_id);

    // A reader of the map with NO write grant is refused with 403 — the same
    // authoring-authority gate the context declare route runs.
    let reader_token = common::generate_second_user_jwt();
    let _ = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&reader_token)
        .send()
        .await
        .expect("provision the reader");
    let reader_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("second@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("the reader profile");
    common::approve(&app.pool, reader_id).await;

    let refused = client_for(&app, &reader_token)
        .data_artifacts()
        .declare_cogmap_shape(L0_COGMAP, &request)
        .await
        .expect_err("a reader without a grant must not declare");
    assert!(
        matches!(refused, temper_client::error::ClientError::Forbidden),
        "the authoring gate refuses with Forbidden: {refused}"
    );
}
