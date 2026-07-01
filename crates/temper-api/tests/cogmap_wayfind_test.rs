#![cfg(feature = "test-db")]
//! Surface B Half 2 Beat B — `POST /api/search` with `wayfind:true`: the lens-driven region-salience
//! discovery scope as a THIRD scope-resolution path on `SearchParams`, mutually exclusive with
//! `context_ref` and `cogmap_id` (spec §4/§5/§6/§7).
//!
//! These are surface/access-semantics tests over the real Axum `/api/search` handler. The query
//! embedding is passed explicitly in the POST body (a 768-vec along axis 0), so no ONNX is needed;
//! region rows are inserted directly with hand-chosen centroids/salience for determinism.
//!
//! Access-semantics reality (open mode): every approved profile is auto-joined to the `temper-system`
//! root team, and the region-less L0 kernel cogmap is therefore always in a principal's wayfind scope
//! via the cold-start direct path (its public telos is always returned). So "a principal with zero
//! visible maps" is unreachable — the deny test (3) proves a non-member does not see a PRIVATE peer
//! map's content (the always-in-scope L0 telos does not carry the private term).

mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// ── helpers (modelled on cogmap_home_test.rs) ───────────────────────────────────────

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

/// Birth a fresh cognitive map via the `cogmap_genesis` SQL function. Returns the new cogmap id.
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

/// Grant a profile explicit `can_write` on a cogmap. Post-Q-A (D3b), authoring into a map requires
/// this explicit `kb_access_grants` row, not team membership.
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
            "origin_uri": format!("test://wayfind-home/{}", Uuid::new_v4()),
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

/// Recover the committed resource id of a cogmap-homed resource by title.
async fn recover_cogmap_resource(pool: &PgPool, cogmap: Uuid, title: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1 AND r.title = $2",
    )
    .bind(cogmap)
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("cogmap-homed resource must have committed its home row")
}

/// The global default lens (`telos-default`, `cogmap_id IS NULL`) — the lens region rows are stored
/// against in test fixtures.
async fn global_lens(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name='telos-default' AND cogmap_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .expect("global telos-default lens")
}

/// Any event id, for the NOT NULL `asserted_by_event_id`/`last_event_id` FKs on a region row.
async fn any_event(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("any event for FK")
}

/// Build a 768-dim pgvector text literal with `[index]=1.0` along the given axis; all others zero.
fn vec768_axis(axis: usize) -> String {
    let mut v = vec![0.0_f64; 768];
    v[axis] = 1.0;
    let mut s = String::with_capacity(768 * 4 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&x.to_string());
    }
    s.push(']');
    s
}

/// The query embedding as a JSON array pointing along axis 0 (cosine 1 to an axis-0 centroid).
fn embedding_axis0_json() -> serde_json::Value {
    let mut v = vec![0.0_f64; 768];
    v[0] = 1.0;
    serde_json::Value::from(v)
}

/// Insert a region into a cogmap and return its id (lens = global default; centroid given as text).
async fn insert_region(
    pool: &PgPool,
    cogmap: Uuid,
    lens: Uuid,
    event: Uuid,
    salience: f64,
    centroid: &str,
    member_count: i32,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, lens_id, centroid, salience, member_count, asserted_by_event_id, last_event_id)
         VALUES ($1, $2, $3::vector, $4, $5, $6, $6)
         RETURNING id",
    )
    .bind(cogmap)
    .bind(lens)
    .bind(centroid)
    .bind(salience)
    .bind(member_count)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

/// Attach a resource as a member of a region.
async fn add_region_member(pool: &PgPool, region: Uuid, resource: Uuid) {
    sqlx::query(
        "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id)
         VALUES ($1, 'kb_resources', $2)",
    )
    .bind(region)
    .bind(resource)
    .execute(pool)
    .await
    .expect("add region member");
}

// ── (1) wayfind scopes into the principal's visible maps' regions ───────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_scopes_into_regions(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-1-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "wayfind-1-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-1-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await; // Q-A: authorship needs an explicit grant

    // A cogmap-homed resource with a distinctive FTS term.
    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "zwayword1 region resource",
        "zwayword1-region",
        "zwayword1 unique term homed in a wayfind region.",
    )
    .await;
    let resource_id = recover_cogmap_resource(&app.pool, cogmap, "zwayword1 region resource").await;

    // A region (centroid toward the query axis, high salience) containing the resource.
    let lens = global_lens(&app.pool).await;
    let event = any_event(&app.pool).await;
    let region = insert_region(&app.pool, cogmap, lens, event, 1.0, &vec768_axis(0), 1).await;
    add_region_member(&app.pool, region, resource_id).await;

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zwayword1",
            "embedding": embedding_axis0_json(),
            "wayfind": true,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "wayfind search must return 200"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();
    assert!(
        ids.contains(&resource_id.to_string()),
        "wayfind must scope into the region containing the resource; got {ids:?}"
    );
}

// ── (2) cold-start: region-less owned map degrades to its direct homed scope ─────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_cold_start_returns_whole_map(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-2-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    // A region-LESS map (no region rows seeded) → cold-start direct scope.
    let cogmap = birth_cogmap(&app.pool, profile, "wayfind-2-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-2-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await; // Q-A: authorship needs an explicit grant

    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "zwayword2 cold-start resource",
        "zwayword2-coldstart",
        "zwayword2 unique term homed in a region-less map.",
    )
    .await;
    let resource_id =
        recover_cogmap_resource(&app.pool, cogmap, "zwayword2 cold-start resource").await;

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zwayword2",
            "embedding": embedding_axis0_json(),
            "wayfind": true,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "wayfind cold-start must return 200, never error"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();
    assert!(
        ids.contains(&resource_id.to_string()),
        "a region-less map degrades to its direct homed scope; got {ids:?}"
    );
}

// ── (3) deny: a non-member does NOT see a private peer map's content via wayfind ─────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_excludes_private_peer_map_content(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    // Profile A homes a resource in A's PRIVATE map (a team B is NOT in).
    let a_email = format!("wayfind-3-a-{}@example.com", Uuid::new_v4());
    let (a, _ac) = common::fixtures::create_test_profile_with_context(&app.pool, &a_email).await;
    let a_token = common::generate_test_jwt(&format!("test|{a}"), &a_email);

    let cogmap = birth_cogmap(&app.pool, a, "wayfind-3-private-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-3-team-{}", &a.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, a).await;
    grant_cogmap_write(&app.pool, cogmap, a).await; // Q-A: authorship needs an explicit grant

    post_cogmap_ingest(
        &app,
        &a_token,
        cogmap,
        "zprivateA cogmap resource",
        "zprivatea-cogmap",
        "zprivateA unique term only on A's private map.",
    )
    .await;
    let private_id = recover_cogmap_resource(&app.pool, cogmap, "zprivateA cogmap resource").await;

    // Profile B is NOT a member of A's private map's team. Its wayfind scope is bounded to the
    // public L0 kernel (cold-start), which does not carry "zprivateA".
    let b_email = format!("wayfind-3-b-{}@example.com", Uuid::new_v4());
    let (b, _bc) = common::fixtures::create_test_profile_with_context(&app.pool, &b_email).await;
    let b_token = common::generate_test_jwt(&format!("test|{b}"), &b_email);

    // Sanity: B genuinely cannot read A's private map (a real deny, not absence).
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(b)
        .bind(cogmap)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!readable, "B must NOT be able to read A's private map");

    // FTS-only (no embedding): the L0 telos never FTS-matches "zprivateA", so B's results are empty.
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {b_token}"))
        .json(&json!({
            "query": "zprivateA",
            "wayfind": true,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "wayfind deny must return 200 (deny-as-empty), never error"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();
    assert!(
        !ids.contains(&private_id.to_string()),
        "B must NOT see A's private-map resource via wayfind; got {ids:?}"
    );
    assert!(
        rows.is_empty(),
        "B's wayfind for A's private term yields zero results; got {rows:?}"
    );
}

// ── (4) mutual exclusion: wayfind with context_ref or cogmap_id → 400 ────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_with_context_or_cogmap_is_bad_request(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-4-{}@example.com", Uuid::new_v4());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    // wayfind + context_ref → 400.
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "anything",
            "wayfind": true,
            "context_ref": "@me/temper",
            "graph_expand": false,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        400,
        "wayfind + context_ref must be BadRequest"
    );

    // wayfind + cogmap_id → 400.
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "anything",
            "wayfind": true,
            "cogmap_id": Uuid::now_v7(),
            "graph_expand": false,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        400,
        "wayfind + cogmap_id must be BadRequest"
    );
}

// ── (5) multi-author (post-A0): a peer's resource on a shared map is returned ────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_includes_peer_resource_on_shared_map(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    // Author homes a resource in a shared map's region; the searcher is a co-member (not the owner),
    // so the resource is visible to them ONLY through the A0 cogmap-membership read clause — proving
    // that clause flows through the wayfind region member dereference.
    let author_email = format!("wayfind-5-author-{}@example.com", Uuid::new_v4());
    let (author, _ac) =
        common::fixtures::create_test_profile_with_context(&app.pool, &author_email).await;
    let author_token = common::generate_test_jwt(&format!("test|{author}"), &author_email);

    let cogmap = birth_cogmap(&app.pool, author, "wayfind-5-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-5-team-{}", &author.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, author).await;
    grant_cogmap_write(&app.pool, cogmap, author).await; // Q-A: authorship needs an explicit grant

    post_cogmap_ingest(
        &app,
        &author_token,
        cogmap,
        "zwayword5 peer-owned resource",
        "zwayword5-peer",
        "zwayword5 unique term authored by a peer, not the searcher.",
    )
    .await;
    let peer_resource_id =
        recover_cogmap_resource(&app.pool, cogmap, "zwayword5 peer-owned resource").await;

    let lens = global_lens(&app.pool).await;
    let event = any_event(&app.pool).await;
    let region = insert_region(&app.pool, cogmap, lens, event, 1.0, &vec768_axis(0), 1).await;
    add_region_member(&app.pool, region, peer_resource_id).await;

    // The co-member searches via wayfind — sees the peer's resource through A0 read RBAC.
    let comember_email = format!("wayfind-5-comember-{}@example.com", Uuid::new_v4());
    let (comember, _cc) =
        common::fixtures::create_test_profile_with_context(&app.pool, &comember_email).await;
    let comember_token = common::generate_test_jwt(&format!("test|{comember}"), &comember_email);
    add_member(&app.pool, team, comember).await;

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {comember_token}"))
        .json(&json!({
            "query": "zwayword5",
            "embedding": embedding_axis0_json(),
            "wayfind": true,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "co-member wayfind search must return 200"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();
    assert!(
        ids.contains(&peer_resource_id.to_string()),
        "multi-author read RBAC must flow through wayfind: a co-member sees the peer's resource; \
         got {ids:?}"
    );
}
