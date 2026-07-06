#![cfg(feature = "test-db")]
//! Surface B Beat 1D — `--cogmap` create homes a resource in a cognitive map, gated by a
//! producer write seam (`cogmap_authorable_by_profile`). As of D3b (Q-A) authorship is an explicit
//! `can_write` grant, NOT team-cogmap membership: membership still confers READ (`cogmap_readable_by_profile`),
//! but writing into a map requires a `kb_access_grants` row (seeded here via `grant_cogmap_write`).
//!
//! Three invariants, all asserted against real DB state (membership is seeded directly via
//! `kb_team_cogmaps` + `kb_team_members`, the reconcile-test pattern, so `cogmap_readable_by_profile`
//! genuinely returns true/false — a green here is never "the resource simply didn't exist"):
//!
//!   1. A `home_cogmap_id` create writes a `kb_resource_homes` row with `anchor_table='kb_cogmaps'`.
//!   2. A cogmap-homed resource is INVISIBLE to a Surface-A context search of the owner's context.
//!   3. A principal who cannot read the map gets 403 on a `--cogmap` create — AND no home row is
//!      written (auth before writes: the gate denies BEFORE any mutation).
//!
//! NOTE on the happy path: as of Task F the shared `readback::resource_row` LEFT-JOINs both
//! `kb_contexts` AND `kb_cogmaps`, so a cogmap-homed resource reads back cleanly — the create
//! response and `show` both return 200 with the `cogmap_*` fields populated and `context_*` null.
//! Case 1 asserts that readback directly; case 2 still asserts the committed FTS-scoping invariant.

mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

// ── helpers ───────────────────────────────────────────────────────────────────────

/// The seed's system emitter — every cogmap genesis is fired under an entity. Always present.
async fn system_emitter(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE p.handle = 'system' AND e.name = 'system'",
    )
    .fetch_one(pool)
    .await
    .expect("system emitter must exist")
}

/// Birth a fresh cognitive map via the same `cogmap_genesis` SQL function every map uses (content-light
/// empty telos). Returns the new cogmap id.
async fn birth_cogmap(pool: &PgPool, owner: Uuid, name: &str) -> Uuid {
    let cogmap = Uuid::now_v7();
    let telos = Uuid::now_v7();
    let emitter = system_emitter(pool).await;
    sqlx::query("SELECT cogmap_genesis($1, $2, $3)")
        .bind(json!({
            "cogmap_id": cogmap,
            "name": name,
            "owner_profile_id": owner,
            "telos": {
                "resource_id": telos,
                "title": format!("{name} telos"),
                "origin_uri": format!("temper://test/{name}/telos"),
                "blocks": [],
            },
        }))
        .bind(json!({}))
        .bind(emitter)
        .execute(pool)
        .await
        .expect("birth cogmap");
    cogmap
}

/// A fresh team (slug-unique). Returns the team id.
async fn create_team(pool: &PgPool, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(id)
        .bind(slug)
        .execute(pool)
        .await
        .expect("create team");
    id
}

/// Join a cogmap to a team (`kb_team_cogmaps`) — the readability/authorability bridge.
async fn join_cogmap_to_team(pool: &PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("join cogmap to team");
}

/// Add a profile to a team as a real member, so `profile_effective_teams` includes it.
async fn add_member(pool: &PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add team member");
}

/// Grant a profile explicit `can_write` on a cogmap. Post-Q-A (D3b) authorship requires this — a
/// self-anchored `kb_access_grants` row — not team membership. `granted_by` is the grantee itself
/// (a fixture bootstrap, standing in for the creator-seed / backfill / delegated grant a real
/// authorship path would carry).
async fn grant_cogmap_write(pool: &PgPool, cogmap: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("grant cogmap write");
}

/// POST a resource to `/api/ingest` homed in a cognitive map. Returns the raw response.
async fn post_cogmap_ingest(
    app: &common::TestApp,
    token: &str,
    cogmap: Uuid,
    title: &str,
    slug: &str,
    content: &str,
) -> reqwest::Response {
    app.client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": title,
            "origin_uri": format!("test://cogmap-home/{}", Uuid::new_v4()),
            "context_ref": "",
            "home_cogmap_id": cogmap.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": content,
        }))
        .send()
        .await
        .expect("ingest request failed")
}

/// POST a resource to `/api/ingest` homed in a context ref.
async fn post_context_ingest(
    app: &common::TestApp,
    token: &str,
    context_ref: &str,
    title: &str,
    slug: &str,
    content: &str,
) -> reqwest::Response {
    app.client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": title,
            "origin_uri": format!("test://ctx-home/{}", Uuid::new_v4()),
            "context_ref": context_ref,
            "doc_type_name": "research",
            "slug": slug,
            "content": content,
        }))
        .send()
        .await
        .expect("ingest request failed")
}

async fn homes_in_cogmap(pool: &PgPool, cogmap: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_homes WHERE anchor_table = 'kb_cogmaps' AND anchor_id = $1",
    )
    .bind(cogmap)
    .fetch_one(pool)
    .await
    .expect("count homes")
}

// ── (1) create --cogmap homes the resource in the map ───────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_cogmap_homed_resource_writes_cogmap_home(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("cogmap-home-1-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    // Fresh cogmap joined to a team the author IS a member of → readable → authorable.
    let cogmap = birth_cogmap(&app.pool, profile, "home-1-map").await;
    let team = create_team(
        &app.pool,
        &format!("home-1-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;

    // Q-A (D3b): membership alone NO LONGER confers authoring — the gate is explicit-grant only.
    let member_only: bool = sqlx::query_scalar("SELECT cogmap_authorable_by_profile($1, $2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(
        !member_only,
        "post-Q-A, team membership alone must NOT make the map authorable"
    );

    // An explicit can_write grant DOES confer authoring — the seam the ingest gate now checks.
    grant_cogmap_write(&app.pool, cogmap, profile).await;
    let authorable: bool = sqlx::query_scalar("SELECT cogmap_authorable_by_profile($1, $2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(
        authorable,
        "an explicit can_write grant makes the map authorable"
    );

    // The genesis already homed the map's telos here; capture the baseline so we assert the DELTA.
    let before = homes_in_cogmap(&app.pool, cogmap).await;

    let resp = post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "cogmap homed resource",
        "cogmap-homed-resource",
        "Body homed in a cognitive map.",
    )
    .await;
    // Task F: the create response now reads the cogmap-homed resource back cleanly (200) — the
    // readback LEFT-JOINs kb_contexts AND kb_cogmaps, so a cogmap home no longer errors.
    let status = resp.status().as_u16();
    assert_eq!(
        status, 200,
        "an authorable cogmap create must return a clean 200, got {status}"
    );

    // The returned ResourceRow carries the cogmap home (cogmap_* populated, context_* null).
    let created: serde_json::Value = resp.json().await.expect("create response JSON");
    assert_eq!(
        created["cogmap_id"].as_str(),
        Some(cogmap.to_string().as_str()),
        "the create response must carry the home cogmap id; got {created}"
    );
    assert_eq!(
        created["cogmap_name"].as_str(),
        Some("home-1-map"),
        "the create response must carry the home cogmap name; got {created}"
    );
    assert!(
        created["context_name"].is_null(),
        "a cogmap-homed resource has no context_name; got {created}"
    );
    let created_id = created["id"].as_str().expect("created id").to_string();

    // The feature: exactly one NEW cogmap-homed resource row, table = 'kb_cogmaps'.
    let after = homes_in_cogmap(&app.pool, cogmap).await;
    assert_eq!(
        after - before,
        1,
        "the create must write exactly one new kb_resource_homes row anchored on the cogmap"
    );

    // `show` of the cogmap-homed resource returns it with the same cogmap_* populated.
    let show = app
        .client
        .get(app.url(&format!("/api/resources/{created_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("show request failed");
    assert_eq!(
        show.status().as_u16(),
        200,
        "show of a cogmap-homed resource must be 200"
    );
    let shown: serde_json::Value = show.json().await.expect("show response JSON");
    assert_eq!(
        shown["cogmap_id"].as_str(),
        Some(cogmap.to_string().as_str()),
        "show must carry the home cogmap id; got {shown}"
    );
    assert_eq!(
        shown["cogmap_name"].as_str(),
        Some("home-1-map"),
        "show must carry the home cogmap name; got {shown}"
    );
    assert!(
        shown["context_name"].is_null(),
        "a cogmap-homed resource shown back has no context_name; got {shown}"
    );

    // The created resource (isolated by its unique title — the map's telos shares the anchor) is
    // homed on 'kb_cogmaps' / this cogmap and owned by the author.
    let (anchor_table, anchor_id, owner_id): (String, Uuid, Uuid) = sqlx::query_as(
        "SELECT h.anchor_table, h.anchor_id, h.owner_profile_id \
           FROM kb_resource_homes h JOIN kb_resources r ON r.id = h.resource_id \
          WHERE r.title = 'cogmap homed resource'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("the created resource must have a home row");
    assert_eq!(anchor_table, "kb_cogmaps", "homed on a cognitive map");
    assert_eq!(anchor_id, cogmap, "homed in the requested cogmap");
    assert_eq!(owner_id, profile, "owned by the author");
}

// ── (2) a cogmap-homed resource is invisible to context search ──────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_homed_resource_invisible_to_context_search(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("cogmap-home-2-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "home-2-map").await;
    let team = create_team(
        &app.pool,
        &format!("home-2-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await; // Q-A: authorship needs an explicit grant

    // Two resources sharing one distinctive FTS term — one homed in the owner's context, one in the
    // cogmap. The context filter must isolate the context-homed one.
    let ctx_resp = post_context_ingest(
        &app,
        &token,
        "@me/temper",
        "ztmpcogword context resource",
        "ztmpcogword-context",
        "ztmpcogword body in the context.",
    )
    .await;
    assert!(
        ctx_resp.status().is_success(),
        "context-homed create must succeed, got {}",
        ctx_resp.status()
    );
    let ctx_body: serde_json::Value = ctx_resp.json().await.expect("ctx create JSON");
    let ctx_id = ctx_body["id"]
        .as_str()
        .expect("context resource id")
        .to_string();

    // The cogmap-homed sibling (its create-response readback is a later beat; the row commits).
    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "ztmpcogword cogmap resource",
        "ztmpcogword-cogmap",
        "ztmpcogword body in the cogmap.",
    )
    .await;
    // Recover the cogmap-homed resource id directly from its committed home row.
    let cogmap_resource_id: Uuid = sqlx::query_scalar(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1 \
            AND r.title = 'ztmpcogword cogmap resource'",
    )
    .bind(cogmap)
    .fetch_one(&app.pool)
    .await
    .expect("cogmap-homed resource must have committed its home row");

    // Surface-A context search scoped to the owner's context.
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "ztmpcogword",
            "context_ref": "@me/temper",
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "context search must return 200"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str())
        .collect();

    assert!(
        ids.contains(&ctx_id.as_str()),
        "the context-homed resource must appear in its own context search; got {ids:?}"
    );
    assert!(
        !ids.contains(&cogmap_resource_id.to_string().as_str()),
        "the cogmap-homed resource must NOT appear in a context search; got {ids:?}"
    );
}

// ── (3) a principal who cannot read the map gets 403, with NO write ──────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_into_unreadable_cogmap_is_forbidden(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    // Author (member) births + owns the map; a second profile is NOT a member of its team.
    let owner_email = format!("cogmap-home-3-owner-{}@example.com", Uuid::new_v4());
    let (owner, _oc) =
        common::fixtures::create_test_profile_with_context(&app.pool, &owner_email).await;
    let cogmap = birth_cogmap(&app.pool, owner, "home-3-map").await;
    let team = create_team(
        &app.pool,
        &format!("home-3-team-{}", &owner.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, owner).await;

    // The outsider: a fully-provisioned profile NOT in the map's team.
    let intruder_email = format!("cogmap-home-3-intruder-{}@example.com", Uuid::new_v4());
    let (intruder, _ic) =
        common::fixtures::create_test_profile_with_context(&app.pool, &intruder_email).await;
    let intruder_token = common::generate_test_jwt(&format!("test|{intruder}"), &intruder_email);

    // Sanity: the map is genuinely NOT readable/authorable by the intruder (a real deny, not absence).
    let authorable: bool = sqlx::query_scalar("SELECT cogmap_authorable_by_profile($1, $2)")
        .bind(intruder)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(
        !authorable,
        "the intruder must NOT be able to author in the map"
    );

    let before = homes_in_cogmap(&app.pool, cogmap).await;

    let resp = post_cogmap_ingest(
        &app,
        &intruder_token,
        cogmap,
        "forbidden cogmap resource",
        "forbidden-cogmap-resource",
        "This write must be refused before it happens.",
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "a create into an unreadable cogmap must be Forbidden"
    );

    // Auth before writes: the deny ran BEFORE any home-row write — nothing was created.
    let after = homes_in_cogmap(&app.pool, cogmap).await;
    assert_eq!(
        after, before,
        "a forbidden create must write NO kb_resource_homes row (auth before writes)"
    );
    let by_intruder: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_homes \
          WHERE anchor_table = 'kb_cogmaps' AND anchor_id = $1 AND owner_profile_id = $2",
    )
    .bind(cogmap)
    .bind(intruder)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        by_intruder, 0,
        "no home row may be owned by the refused intruder"
    );
}

// ── (4) search --cogmap returns the map's homed resource ────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_search_scopes_to_map(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("cogmap-search-4-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "search-4-map").await;
    let team = create_team(
        &app.pool,
        &format!("search-4-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await; // Q-A: authorship needs an explicit grant

    // Sanity: the membership is real and readable by the member.
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(readable, "seeded membership must make the map readable");

    // Create a cogmap-homed resource with a distinctive FTS term. The create-response
    // readback returns 500 (known issue — deferred beat), so tolerate non-200 but verify
    // the row committed by reading from kb_resource_homes directly.
    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "zmapword cogmap search resource",
        "zmapword-cogmap-search",
        "zmapword unique term for cogmap scoped search.",
    )
    .await;
    // Recover the committed resource id from its home row.
    let resource_id: Uuid = sqlx::query_scalar(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1 \
            AND r.title = 'zmapword cogmap search resource'",
    )
    .bind(cogmap)
    .fetch_one(&app.pool)
    .await
    .expect("cogmap-homed resource must have committed its home row");

    // A context-homed sibling owned by the SAME searcher, sharing the SAME FTS term ("zmapword").
    // Under a `--cogmap` (cogmap_id) scope it must NOT appear — without this sibling the positive
    // assertion alone cannot tell a scoped result from an unscoped one (the searcher owns both and
    // there is no context filter). Its exclusion proves the API surface genuinely narrows to the
    // map's homed set.
    let ctx_resp = post_context_ingest(
        &app,
        &token,
        "@me/temper",
        "zmapword context sibling",
        "zmapword-context-sibling",
        "zmapword body homed in the searcher's own context, not the map.",
    )
    .await;
    assert!(
        ctx_resp.status().is_success(),
        "context sibling create must succeed, got {}",
        ctx_resp.status()
    );
    let ctx_sibling: serde_json::Value = ctx_resp.json().await.expect("ctx sibling JSON");
    let ctx_sibling_id = ctx_sibling["id"]
        .as_str()
        .expect("ctx sibling id")
        .to_string();

    // Member searches with cogmap_id scope — must find the resource.
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zmapword",
            "cogmap_id": cogmap,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(resp.status().as_u16(), 200, "cogmap search must return 200");

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();

    assert!(
        ids.contains(&resource_id.to_string()),
        "the cogmap-homed resource must appear in a cogmap-scoped search; got {ids:?}"
    );
    assert!(
        !ids.contains(&ctx_sibling_id),
        "a context-homed resource sharing the FTS term must NOT appear under a --cogmap scope \
         (proves the scope narrows, not just includes); got {ids:?}"
    );
}

// ── (5) non-member searching the same cogmap gets 200 with zero results ─────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_search_denied_for_non_member_returns_zero(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    // Owner/member sets up the cogmap and homes a resource in it.
    let owner_email = format!("cogmap-search-5-owner-{}@example.com", Uuid::new_v4());
    let (owner, _oc) =
        common::fixtures::create_test_profile_with_context(&app.pool, &owner_email).await;
    let owner_token = common::generate_test_jwt(&format!("test|{owner}"), &owner_email);

    let cogmap = birth_cogmap(&app.pool, owner, "search-5-map").await;
    let team = create_team(
        &app.pool,
        &format!("search-5-team-{}", &owner.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, owner).await;
    grant_cogmap_write(&app.pool, cogmap, owner).await; // Q-A: authorship needs an explicit grant

    post_cogmap_ingest(
        &app,
        &owner_token,
        cogmap,
        "zmapword5 non-member cogmap resource",
        "zmapword5-nonmember",
        "zmapword5 unique term for non-member denial test.",
    )
    .await;

    // A second profile who is NOT a member of the map's team.
    let outsider_email = format!("cogmap-search-5-outsider-{}@example.com", Uuid::new_v4());
    let (outsider, _oc2) =
        common::fixtures::create_test_profile_with_context(&app.pool, &outsider_email).await;
    let outsider_token = common::generate_test_jwt(&format!("test|{outsider}"), &outsider_email);

    // Sanity: the map is genuinely NOT readable by the outsider.
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(outsider)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!readable, "outsider must NOT be able to read the map");

    // Non-member searches the same cogmap — must get 200 with empty results (deny-as-empty,
    // not an error: cogmap_scope_ids returns zero rows for non-members).
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {outsider_token}"))
        .json(&json!({
            "query": "zmapword5",
            "cogmap_id": cogmap,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "non-member cogmap search must return 200 (deny-as-empty)"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    assert!(
        rows.is_empty(),
        "non-member cogmap search must return zero results; got {rows:?}"
    );
}

// ── (6) ex-member who STILL OWNS a homed resource gets zero — readability gate is load-bearing ──

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_search_ex_member_who_owns_resource_gets_zero(pool: PgPool) {
    // Unlike test (5)'s outsider — who never owned the resource, so `resources_visible_to` alone
    // empties the set — this searcher AUTHORS (owns) a cogmap-homed resource and is only THEN
    // removed from the map's team. Ownership keeps the row in `resources_visible_to`, so the empty
    // result here is carried solely by the `cogmap_readable_by_profile` clause inside
    // `cogmap_scope_ids`. Deleting that clause would surface the owned row and FAIL this test —
    // which test (5) would not catch. This is the ex-member-still-owns regression the gate guards.
    let app = common::setup_test_app(pool).await;

    let email = format!("cogmap-search-6-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "search-6-map").await;
    let team = create_team(
        &app.pool,
        &format!("search-6-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await; // Q-A: authorship needs an explicit grant

    // As a member, the author homes a resource in the map (so they OWN it).
    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "zmapword6 ex-member owned resource",
        "zmapword6-exmember",
        "zmapword6 unique term authored while a member.",
    )
    .await;
    let resource_id: Uuid = sqlx::query_scalar(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1 \
            AND r.title = 'zmapword6 ex-member owned resource'",
    )
    .bind(cogmap)
    .fetch_one(&app.pool)
    .await
    .expect("authored resource must have committed its home row");

    // Now REMOVE the author from the map's team AND revoke their authoring grant. They still OWN the
    // resource, but can no longer READ the map (the write grant carried read by coherence, so both the
    // membership and the grant must go for the ex-member to lose read).
    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(profile)
        .execute(&app.pool)
        .await
        .expect("remove member");
    sqlx::query(
        "DELETE FROM kb_access_grants WHERE subject_table = 'kb_cogmaps' AND subject_id = $1 \
           AND principal_table = 'kb_profiles' AND principal_id = $2",
    )
    .bind(cogmap)
    .bind(profile)
    .execute(&app.pool)
    .await
    .expect("revoke authoring grant");

    // Sanity: the map is genuinely no longer readable by the ex-member (a real deny, not absence).
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(
        !readable,
        "ex-member must no longer be able to read the map"
    );

    // A `--cogmap` search by the ex-member returns ZERO rows — the readability clause empties the
    // scope set even though the searcher still owns a resource homed in the map.
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zmapword6",
            "cogmap_id": cogmap,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "ex-member cogmap search must return 200 (deny-as-empty)"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();
    assert!(
        !ids.contains(&resource_id.to_string()),
        "an ex-member who still owns the resource must get zero cogmap-scoped hits; got {ids:?}"
    );
    assert!(
        rows.is_empty(),
        "ex-member cogmap search must be empty; got {rows:?}"
    );
}

// ── (7) co-member DOES see a peer's homed resource on a shared map (multi-author read RBAC) ──

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_search_includes_peer_resource_on_shared_map(pool: PgPool) {
    // MULTI-AUTHOR READ RBAC (Surface B Half 2 Beat A0): `resources_visible_to` now has a
    // cogmap-membership clause (the resource-grain mirror of `cogmap_readable_by_profile`,
    // membership-flat), so a `--cogmap` search by a co-member of the map's team SEES a peer's
    // resource homed in the map — map-read and resource-read agree by construction. A principal
    // who is NOT a member of any team joined to the map still sees nothing (additive false-negative
    // fix, never a leak — the deny path is asserted at the end of this test).
    let app = common::setup_test_app(pool).await;

    // Author/owner births the map, joins it to a team, and homes a resource in it.
    let author_email = format!("cogmap-search-7-author-{}@example.com", Uuid::new_v4());
    let (author, _ac) =
        common::fixtures::create_test_profile_with_context(&app.pool, &author_email).await;
    let author_token = common::generate_test_jwt(&format!("test|{author}"), &author_email);

    let cogmap = birth_cogmap(&app.pool, author, "search-7-map").await;
    let team = create_team(
        &app.pool,
        &format!("search-7-team-{}", &author.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, author).await;
    grant_cogmap_write(&app.pool, cogmap, author).await; // Q-A: authorship needs an explicit grant

    post_cogmap_ingest(
        &app,
        &author_token,
        cogmap,
        "zmapword7 peer-owned cogmap resource",
        "zmapword7-peer",
        "zmapword7 unique term authored by a peer, not the searcher.",
    )
    .await;
    let peer_resource_id: Uuid = sqlx::query_scalar(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1 \
            AND r.title = 'zmapword7 peer-owned cogmap resource'",
    )
    .bind(cogmap)
    .fetch_one(&app.pool)
    .await
    .expect("peer resource must have committed its home row");

    // A co-member of the SAME team/map — can READ the map, but does NOT own the peer's resource.
    let comember_email = format!("cogmap-search-7-comember-{}@example.com", Uuid::new_v4());
    let (comember, _cc) =
        common::fixtures::create_test_profile_with_context(&app.pool, &comember_email).await;
    let comember_token = common::generate_test_jwt(&format!("test|{comember}"), &comember_email);
    add_member(&app.pool, team, comember).await;

    // Sanity: the co-member genuinely CAN read the map (so a zero result is the ownership-scoped
    // visibility boundary, not a readability deny).
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(comember)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(readable, "co-member must be able to read the map");

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {comember_token}"))
        .json(&json!({
            "query": "zmapword7",
            "cogmap_id": cogmap,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "co-member cogmap search must return 200"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();
    // MULTI-AUTHOR READ RBAC: the co-member (a member of the map's team) now SEES the peer's
    // cogmap-homed resource — the cogmap-membership clause on `resources_visible_to` flows through
    // the `--cogmap` scope set.
    assert!(
        ids.contains(&peer_resource_id.to_string()),
        "multi-author read RBAC: a co-member must SEE a peer's cogmap-homed resource; got {ids:?}"
    );

    // DENY PATH (additive fix, never a leak): a principal who is NOT a member of any team joined to
    // the map sees nothing — the readability gate empties the scope set, so zero rows.
    let outsider_email = format!("cogmap-search-7-outsider-{}@example.com", Uuid::new_v4());
    let (outsider, _oc) =
        common::fixtures::create_test_profile_with_context(&app.pool, &outsider_email).await;
    let outsider_token = common::generate_test_jwt(&format!("test|{outsider}"), &outsider_email);

    // Sanity: the outsider genuinely CANNOT read the map (so a zero result is the deny gate).
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(outsider)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!readable, "outsider must NOT be able to read the map");

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {outsider_token}"))
        .json(&json!({
            "query": "zmapword7",
            "cogmap_id": cogmap,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "outsider cogmap search must return 200 (deny-as-empty)"
    );
    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    assert!(
        rows.is_empty(),
        "non-member cogmap search must be empty; got {rows:?}"
    );
}

// ── (F1) the create-into-cogmap gate lives on the BACKEND, not only the surfaces ─────

/// F1 — `DbBackend::create_resource` denies a non-granted principal on a `Cogmap` home DIRECTLY, not
/// only via the surface pre-checks. This is the belt-and-suspenders the surfaces (mcp create tool, api
/// ingest) also enforce: the shared write path must not trust callers to pre-check (one new caller away
/// from a silent bypass — the SAML `is_active` failure mode). A granted principal succeeds; the denial
/// writes no home row (auth before writes).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_into_cogmap_denied_at_backend_for_nongranted(pool: PgPool) {
    let email = format!("f1-backend-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let cogmap = birth_cogmap(&pool, profile, "f1-map").await;

    fn cmd(cogmap: Uuid, slug: &str) -> CreateResource {
        CreateResource {
            slug: slug.to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Cogmap(CogmapId::from(cogmap)),
            title: format!("F1 backend {slug}"),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: Some(format!("test://f1-{slug}")),
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: Surface::ApiHttp,
        }
    }

    // Baseline: genesis already homes the map's telos resource, so measure the delta, not an absolute.
    let baseline = homes_in_cogmap(&pool, cogmap).await;

    // No write grant on the map → the backend command itself denies (not just the surface).
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let denied = backend.create_resource(cmd(cogmap, "denied")).await;
    assert!(
        matches!(denied, Err(temper_core::error::TemperError::Forbidden)),
        "backend create into a cogmap without a write grant must be Forbidden: {denied:?}"
    );
    assert_eq!(
        homes_in_cogmap(&pool, cogmap).await,
        baseline,
        "denied create must write no home row (auth before writes)"
    );

    // Grant explicit cogmap write → the same backend create now succeeds and homes the resource.
    grant_cogmap_write(&pool, cogmap, profile).await;
    backend
        .create_resource(cmd(cogmap, "granted"))
        .await
        .expect("backend create into a cogmap WITH a write grant must succeed");
    assert_eq!(
        homes_in_cogmap(&pool, cogmap).await,
        baseline + 1,
        "granted create writes exactly one new cogmap home row"
    );
}
