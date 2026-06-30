#![cfg(feature = "test-db")]
//! Surface B Beat 1D — `--cogmap` create homes a resource in a cognitive map, gated by a
//! producer write seam (`cogmap_authorable_by_profile` → team-cogmap membership).
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

    // Sanity: the membership is REAL — the gate predicate genuinely passes for this principal.
    let authorable: bool = sqlx::query_scalar("SELECT cogmap_authorable_by_profile($1, $2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(authorable, "seeded membership must make the map authorable");

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
