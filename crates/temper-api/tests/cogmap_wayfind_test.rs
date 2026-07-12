#![cfg(feature = "test-db")]
//! Surface B Half 2 Beat B — `POST /api/search` with `wayfind:true`: the lens-driven region-salience
//! discovery scope as a scope-resolution path on `SearchParams` (spec §4/§5/§6/§7).
//!
//! **T7 made wayfind anchor-agnostic** (spec §3.7). It pools regions from every anchor the principal
//! can read — contexts as well as cogmaps — so it is no longer mutually exclusive with `context_ref` /
//! `cogmap_id`: naming an anchor now *scopes* the pool ("wayfind within this context"). Tests (6)–(9)
//! cover that flip, and (8)/(9) are regressions for two defects it surfaced — the NaN trap and the
//! cross-kind salience crush. See `migrations/20260712000090_anchor_agnostic_wayfind.sql`.
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

/// Ingest a resource homed in a CONTEXT (rather than a cogmap) — the "raw work" half of the
/// composition read T7 exists to deliver.
async fn post_context_ingest(
    app: &common::TestApp,
    token: &str,
    context: Uuid,
    title: &str,
    slug: &str,
    content: &str,
) -> reqwest::Response {
    app.client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": title,
            "origin_uri": format!("test://wayfind-ctx/{}", Uuid::new_v4()),
            "context_ref": context.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": content,
        }))
        .send()
        .await
        .expect("ingest request failed")
}

/// Recover the committed resource id of a context-homed resource by title.
async fn recover_context_resource(pool: &PgPool, context: Uuid, title: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = $1 AND r.title = $2",
    )
    .bind(context)
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("context-homed resource must have committed its home row")
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

/// A 768-dim ZERO vector — the centroid of a region whose members carry no embedding (a bodyless
/// resource ⇒ zero chunks). pgvector's `<=>` against a zero vector is `NaN`; see
/// [`zero_centroid_region_does_not_hijack_the_top_n`].
fn vec768_zero() -> String {
    let mut s = String::with_capacity(768 * 2 + 2);
    s.push('[');
    for i in 0..768 {
        if i > 0 {
            s.push(',');
        }
        s.push('0');
    }
    s.push(']');
    s
}

/// A region row to plant.
///
/// Fixtures MUST write the **anchor pair** — since T7 the wayfind pool is keyed on
/// `(home_anchor_table, home_anchor_id)`, and `kb_cogmap_regions` has **no trigger** deriving that
/// pair from `cogmap_id`. A fixture that sets only `cogmap_id` plants a region the funnel cannot see.
/// The producer dual-writes the vestigial `cogmap_id` alongside the pair (spec §3.6 M1), so this does
/// too — `cogmap_id` is NULL for a context anchor, which its FK to `kb_cogmaps` requires anyway.
struct RegionSpec<'a> {
    anchor_table: &'a str,
    anchor_id: Uuid,
    lens: Uuid,
    event: Uuid,
    salience: f64,
    centroid: &'a str,
    member_count: i32,
}

async fn insert_region_at(pool: &PgPool, spec: RegionSpec<'_>) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience,
            member_count, asserted_by_event_id, last_event_id)
         VALUES ($1, $2, $3, $4, $5::vector, $6, $7, $8, $8)
         RETURNING id",
    )
    .bind((spec.anchor_table == "kb_cogmaps").then_some(spec.anchor_id))
    .bind(spec.anchor_table)
    .bind(spec.anchor_id)
    .bind(spec.lens)
    .bind(spec.centroid)
    .bind(spec.salience)
    .bind(spec.member_count)
    .bind(spec.event)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

/// Insert a region on a COGMAP anchor and return its id (lens = global default; centroid as text).
async fn insert_region(
    pool: &PgPool,
    cogmap: Uuid,
    lens: Uuid,
    event: Uuid,
    salience: f64,
    centroid: &str,
    member_count: i32,
) -> Uuid {
    insert_region_at(
        pool,
        RegionSpec {
            anchor_table: "kb_cogmaps",
            anchor_id: cogmap,
            lens,
            event,
            salience,
            centroid,
            member_count,
        },
    )
    .await
}

/// Insert a region on a CONTEXT anchor — the thing T7 makes wayfind able to see at all.
async fn insert_context_region(
    pool: &PgPool,
    context: Uuid,
    lens: Uuid,
    event: Uuid,
    salience: f64,
    centroid: &str,
    member_count: i32,
) -> Uuid {
    insert_region_at(
        pool,
        RegionSpec {
            anchor_table: "kb_contexts",
            anchor_id: context,
            lens,
            event,
            salience,
            centroid,
            member_count,
        },
    )
    .await
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

    // Issue #360: an empty wayfind must be *diagnosable*. The body contract is unchanged (a bare
    // array), and the scope-stage diagnostics ride the additive `x-temper-search-diagnostics`
    // header — read it BEFORE consuming the body.
    let diag: serde_json::Value = {
        let raw = resp
            .headers()
            .get("x-temper-search-diagnostics")
            .expect("empty wayfind must carry the diagnostics header");
        serde_json::from_slice(raw.as_bytes()).expect("diagnostics header is JSON")
    };

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

    // The header still tells the caller this was a scoped search that matched nothing, rather than
    // handing back an inscrutable `[]`.
    assert_eq!(
        diag["scope"], "wayfind",
        "diagnostics must name the scope; got {diag}"
    );
    assert_eq!(diag["matched"], 0, "no results matched; got {diag}");
    let reason = diag["reason"].as_str().expect("reason string");
    assert!(
        matches!(reason, "no_match" | "out_of_scope"),
        "an empty wayfind is no_match or out_of_scope, never Ok; got {reason}"
    );
    let hint = diag["hint"]
        .as_str()
        .expect("empty wayfind must carry a hint");

    // T7: this assertion is INVERTED. It used to require the hint to point at `--context`, carrying
    // the `WAYFIND_UNREACHABLE` claim that "wayfind only reaches cogmap-distilled content — if what
    // you want is context-homed, it is unreachable here regardless of phrasing." That is now FALSE
    // (wayfind pools context regions too), so the hint is gone and this guards that it stays gone —
    // leaving it in place would actively teach agents to stop asking for the thing that now works.
    assert!(
        !hint.contains("unreachable"),
        "the WAYFIND_UNREACHABLE guidance must be gone — wayfind reaches context-homed content now; \
         got {hint:?}"
    );
}

// ── (4) T7: wayfind COMPOSES with an anchor scope (was: mutual exclusion → 400) ──────

/// T7 INVERTS this test. It used to assert that `wayfind` × `context_ref` and `wayfind` × `cogmap_id`
/// were both `BadRequest` — the three-way mutual exclusion. Since wayfind now pools regions over BOTH
/// anchor kinds (spec §3.7), naming an anchor alongside it is no longer a contradiction: it means
/// *"wayfind within this anchor"*. `temper search --context @me/temper --wayfind` is this task's
/// headline acceptance criterion, and it was a 400 the day before this migration.
///
/// What remains excluded is `context_ref` × `cogmap_id` — two different homes, incoherent together.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_composes_with_an_anchor_scope(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-4-{}@example.com", Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    // wayfind + context_ref → 200. "Wayfind within this context."
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "anything",
            "wayfind": true,
            "context_ref": ctx.to_string(),
            "graph_expand": false,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "wayfind + context_ref must now be allowed — it means 'wayfind within this context'"
    );

    // wayfind + cogmap_id → 200. "Wayfind within this cogmap."
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
        200,
        "wayfind + cogmap_id must now be allowed — an unreadable/absent anchor is deny (zero rows), \
         not an error"
    );

    // context_ref + cogmap_id → still 400. Two homes; the exclusion that survives.
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "anything",
            "context_ref": ctx.to_string(),
            "cogmap_id": Uuid::now_v7(),
            "graph_expand": false,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        400,
        "context_ref + cogmap_id must remain BadRequest"
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

// ── (6) T7: the payoff — one wayfind surfaces BOTH the distilled idea and the raw work ──────

/// The composition read (spec §3.7). An unscoped wayfind pools regions from **every visible anchor**,
/// so a single pass returns a cogmap-distilled resource *and* the context-homed raw work — which
/// before T7 required a separate traversal (`graph_region_composition_edges`), because wayfind pooled
/// over `cogmap_visible_maps` and nothing else.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_pools_both_anchor_kinds(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-6-{}@example.com", Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "wayfind-6-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-6-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await;

    // The distilled half: a cogmap-homed resource in a cogmap region.
    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "zwayword6 distilled node",
        "zwayword6-distilled",
        "zwayword6 the distilled idea, homed in a cognitive map.",
    )
    .await;
    let distilled = recover_cogmap_resource(&app.pool, cogmap, "zwayword6 distilled node").await;

    // The raw half: a context-homed resource in a CONTEXT region — invisible to wayfind before T7.
    post_context_ingest(
        &app,
        &token,
        ctx,
        "zwayword6 raw work",
        "zwayword6-raw",
        "zwayword6 the raw work it came from, homed in a context.",
    )
    .await;
    let raw = recover_context_resource(&app.pool, ctx, "zwayword6 raw work").await;

    let lens = global_lens(&app.pool).await;
    let event = any_event(&app.pool).await;
    let cog_region = insert_region(&app.pool, cogmap, lens, event, 1.0, &vec768_axis(0), 1).await;
    add_region_member(&app.pool, cog_region, distilled).await;
    let ctx_region =
        insert_context_region(&app.pool, ctx, lens, event, 1.0, &vec768_axis(0), 1).await;
    add_region_member(&app.pool, ctx_region, raw).await;

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zwayword6",
            "embedding": embedding_axis0_json(),
            "wayfind": true,
            "regions": 10,
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
        ids.contains(&distilled.to_string()),
        "unscoped wayfind must still reach cogmap-distilled content; got {ids:?}"
    );
    assert!(
        ids.contains(&raw.to_string()),
        "unscoped wayfind must NOW reach context-homed raw work — this is T7's whole payoff, and the \
         `WAYFIND_UNREACHABLE` hint used to tell agents it was impossible; got {ids:?}"
    );
}

// ── (7) T7: a named anchor scopes the region pool to itself ──────────────────────────

/// `--context X --wayfind` means "wayfind *within* this context": the pool is restricted to X's
/// regions, so the cogmap-distilled resource that an unscoped pass would surface is excluded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wayfind_scoped_to_a_context_excludes_other_anchors(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-7-{}@example.com", Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "wayfind-7-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-7-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await;

    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "zwayword7 distilled node",
        "zwayword7-distilled",
        "zwayword7 distilled, homed in a cognitive map.",
    )
    .await;
    let distilled = recover_cogmap_resource(&app.pool, cogmap, "zwayword7 distilled node").await;

    post_context_ingest(
        &app,
        &token,
        ctx,
        "zwayword7 raw work",
        "zwayword7-raw",
        "zwayword7 raw, homed in a context.",
    )
    .await;
    let raw = recover_context_resource(&app.pool, ctx, "zwayword7 raw work").await;

    let lens = global_lens(&app.pool).await;
    let event = any_event(&app.pool).await;
    let cog_region = insert_region(&app.pool, cogmap, lens, event, 1.0, &vec768_axis(0), 1).await;
    add_region_member(&app.pool, cog_region, distilled).await;
    let ctx_region =
        insert_context_region(&app.pool, ctx, lens, event, 1.0, &vec768_axis(0), 1).await;
    add_region_member(&app.pool, ctx_region, raw).await;

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zwayword7",
            "embedding": embedding_axis0_json(),
            "wayfind": true,
            "context_ref": ctx.to_string(),
            "regions": 10,
            "graph_expand": false,
            "limit": 50,
        }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "scoped wayfind must return 200"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str().map(String::from))
        .collect();
    assert!(
        ids.contains(&raw.to_string()),
        "wayfind scoped to a context must return that context's region members; got {ids:?}"
    );
    assert!(
        !ids.contains(&distilled.to_string()),
        "wayfind scoped to a context must NOT pool the cogmap's regions; got {ids:?}"
    );
}

// ── (8) T7 REGRESSION: the NaN trap ─────────────────────────────────────────────────

/// A region whose members carry no embedding (a bodyless resource ⇒ zero chunks) has a **zero-vector
/// centroid**, and pgvector's `<=>` against a zero vector is **`NaN`**. Postgres sorts `NaN` **above
/// every real value** under `ORDER BY … DESC`, and the funnel's `NULLS LAST` does **not** guard it —
/// so un-guarded, the top-N `LIMIT` hands back those contentless regions for **every query**,
/// deterministically. Ten such regions (3.7% of context regions) existed in prod when this was found.
///
/// The bug was latent in the *shipped* function; it stayed dormain only because no COGMAP region has a
/// zero centroid. Turning contexts on is what fires it — which is why the guard is bundled here.
///
/// `regions = 1` admits exactly one region, so this asserts the ordering head-on: the real,
/// query-aligned cogmap region must win the single slot; the zero-centroid region must lose it.
/// Without the `COALESCE(NULLIF(…, 'NaN'), 0.0)` guard this test fails — the zero-centroid region
/// takes the slot and the genuinely relevant resource is never returned.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn zero_centroid_region_does_not_hijack_the_top_n(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-8-{}@example.com", Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "wayfind-8-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-8-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await;

    // The resource that SHOULD win: relevant, and in a region whose centroid points at the query.
    post_cogmap_ingest(
        &app,
        &token,
        cogmap,
        "zwayword8 relevant node",
        "zwayword8-relevant",
        "zwayword8 the genuinely relevant content.",
    )
    .await;
    let relevant = recover_cogmap_resource(&app.pool, cogmap, "zwayword8 relevant node").await;

    // The resource that must NOT win: it sits in a ZERO-CENTROID region (the bodyless-resource case).
    post_context_ingest(
        &app,
        &token,
        ctx,
        "zwayword8 contentless node",
        "zwayword8-contentless",
        "zwayword8 sits in a region with no semantic direction at all.",
    )
    .await;
    let contentless = recover_context_resource(&app.pool, ctx, "zwayword8 contentless node").await;

    let lens = global_lens(&app.pool).await;
    let event = any_event(&app.pool).await;

    // Salience deliberately FAVOURS the zero-centroid region, so the only thing that can keep it out
    // of the single slot is the NaN guard — not a lucky salience ordering.
    let good = insert_region(&app.pool, cogmap, lens, event, 1.0, &vec768_axis(0), 1).await;
    add_region_member(&app.pool, good, relevant).await;
    let degenerate =
        insert_context_region(&app.pool, ctx, lens, event, 99.0, &vec768_zero(), 1).await;
    add_region_member(&app.pool, degenerate, contentless).await;

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zwayword8",
            "embedding": embedding_axis0_json(),
            "wayfind": true,
            "regions": 1,
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
        ids.contains(&relevant.to_string()),
        "the query-aligned region must win the single top-N slot; got {ids:?}"
    );
    assert!(
        !ids.contains(&contentless.to_string()),
        "a ZERO-CENTROID region must not hijack the top-N: `1 - (centroid <=> emb)` is NaN, and \
         Postgres sorts NaN ABOVE every real value on DESC (NULLS LAST does not guard it). Un-guarded, \
         this region wins every query. got {ids:?}"
    );
}

// ── (9) T7 REGRESSION: salience is normalized PER ANCHOR KIND ────────────────────────

/// Raw salience is **not comparable across anchor kinds**. A context's salience is driven by
/// `centrality`, an unbounded degree count — measured on prod: max 276 in a context vs 21.5 in a
/// cogmap, giving max salience 69.55 vs 9.53. The shipped funnel min-max normalized salience over the
/// **pooled** candidate set, so admitting contexts collapsed every cogmap region's `sal_norm` to
/// ≤ 0.137: the α term shrank from a [0, 0.4] range to [0, 0.055], annihilating the distilled salience
/// signal. That is the very drowning §3.7 says the anchor prior exists to prevent — and an *additive*
/// prior cannot repair a *multiplicative* range crush. Hence `percent_rank` PARTITIONed by anchor kind.
///
/// The fixture isolates the normalizer as the *only* discriminator: all four regions share one
/// centroid, so `query_cos` is identical across them and cannot decide the ordering. Salience alone
/// separates them, and `regions = 1` forces a single winner.
///
///   context regions:  A salience 69.0   B salience 0.5
///   cogmap  regions:  C salience  9.0   D salience 0.3
///
/// * **Per-kind `percent_rank` (correct):** A and C each top their own kind ⇒ `sal_norm` 1.0 each, so
///   κ breaks the tie in the cogmap's favour ⇒ **C wins**.
/// * **Pooled min-max (the bug):** A ⇒ 1.0 but C ⇒ (9.0−0.3)/68.7 = **0.127** ⇒ **A wins**, and the
///   cogmap's salience signal is gone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn salience_is_normalized_per_anchor_kind(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("wayfind-9-{}@example.com", Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let cogmap = birth_cogmap(&app.pool, profile, "wayfind-9-map").await;
    let team = create_team(
        &app.pool,
        &format!("wayfind-9-team-{}", &profile.simple().to_string()[..8]),
    )
    .await;
    join_cogmap_to_team(&app.pool, cogmap, team).await;
    add_member(&app.pool, team, profile).await;
    grant_cogmap_write(&app.pool, cogmap, profile).await;

    // Four resources, one per region. All match the query term, so FTS cannot decide the outcome
    // either — only which region the funnel admits can.
    for (title, slug) in [
        ("zwayword9 cogmap top", "zwayword9-cog-top"),
        ("zwayword9 cogmap tail", "zwayword9-cog-tail"),
    ] {
        post_cogmap_ingest(
            &app,
            &token,
            cogmap,
            title,
            slug,
            "zwayword9 distilled content.",
        )
        .await;
    }
    for (title, slug) in [
        ("zwayword9 context top", "zwayword9-ctx-top"),
        ("zwayword9 context tail", "zwayword9-ctx-tail"),
    ] {
        post_context_ingest(&app, &token, ctx, title, slug, "zwayword9 raw content.").await;
    }
    let cog_top = recover_cogmap_resource(&app.pool, cogmap, "zwayword9 cogmap top").await;
    let cog_tail = recover_cogmap_resource(&app.pool, cogmap, "zwayword9 cogmap tail").await;
    let ctx_top = recover_context_resource(&app.pool, ctx, "zwayword9 context top").await;
    let ctx_tail = recover_context_resource(&app.pool, ctx, "zwayword9 context tail").await;

    let lens = global_lens(&app.pool).await;
    let event = any_event(&app.pool).await;
    let axis0 = vec768_axis(0);

    // The context's runaway salience (69.0) is what crushed the cogmap under pooled min-max.
    let a = insert_context_region(&app.pool, ctx, lens, event, 69.0, &axis0, 1).await;
    add_region_member(&app.pool, a, ctx_top).await;
    let b = insert_context_region(&app.pool, ctx, lens, event, 0.5, &axis0, 1).await;
    add_region_member(&app.pool, b, ctx_tail).await;
    let c = insert_region(&app.pool, cogmap, lens, event, 9.0, &axis0, 1).await;
    add_region_member(&app.pool, c, cog_top).await;
    let d = insert_region(&app.pool, cogmap, lens, event, 0.3, &axis0, 1).await;
    add_region_member(&app.pool, d, cog_tail).await;

    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "query": "zwayword9",
            "embedding": embedding_axis0_json(),
            "wayfind": true,
            "regions": 1,
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
        ids.contains(&cog_top.to_string()),
        "the cogmap region that tops ITS OWN kind must win the single slot — its salience (9.0) is \
         only 'small' against a context scale it should never have been normalized against; got {ids:?}"
    );
    assert!(
        !ids.contains(&ctx_top.to_string()),
        "the runaway-salience context region must NOT take the slot: under pooled min-max it would \
         (sal_norm 1.0 vs the cogmap's 0.127), which is exactly the drowning per-kind normalization \
         fixes; got {ids:?}"
    );
}
