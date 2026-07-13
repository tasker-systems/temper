//! HTTP e2e for GET /api/graph/cogmaps/{id}/panorama.
//! A reader sees the cogmap interior (TerritoryOverview); a non-reader gets 404.
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

// Helpers mirror graph_atlas_home_e2e.rs (integration test binaries don't share
// code except via `common`, so these are copied rather than imported).

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .unwrap();
}

async fn create_cogmap(pool: &sqlx::PgPool, name: &str) -> Uuid {
    // kb_cogmaps requires a telos_resource_id; create a throwaway resource for it.
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn join_cogmap(pool: &sqlx::PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_denies_non_reader_as_absence(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let _profile = provision_profile(&app, &app.token).await;

    // A cogmap joined to NO team the caller belongs to → not readable.
    let orphan_map = create_cogmap(&pool, "unreachable-map").await;

    let status = app
        .reqwest_client
        .get(app.url(&format!("/api/graph/cogmaps/{orphan_map}/panorama")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "deny-as-absence, not 403"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_returns_overview_for_reader(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;
    let team = create_team(&pool, "pano-team").await;
    add_member(&pool, team, profile).await;
    let map = create_cogmap(&pool, "readable-map").await;
    join_cogmap(&pool, map, team).await;

    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/graph/cogmaps/{map}/panorama")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let _body: temper_core::types::graph_territory::TerritoryOverview = resp.json().await.unwrap();
    // shape decodes = renderer-compatible
}

/// A resource homed in `cogmap`. Readable by anyone who reaches the cogmap's team.
async fn cogmap_resource(pool: &sqlx::PgPool, cogmap: Uuid, owner: Uuid, title: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(id)
    .bind(cogmap)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// D5 at the ATLAS door. `graph_cogmap_territories` is a second region read that has not yet retired
/// onto `anchor_shape` (D1) — so the member gate has to be proven here too, or the leak simply moves
/// to whichever door was left alone.
///
/// The region stores `member_count = 3`; the caller can read exactly one of its three members. The
/// panorama must say 1.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_counts_only_members_the_caller_can_see(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;
    let team = create_team(&pool, "pano-team").await;
    add_member(&pool, team, profile).await;

    let map = create_cogmap(&pool, "readable-map").await;
    join_cogmap(&pool, map, team).await;
    // A map the caller's team is NOT joined to: what is homed here is unreadable to them.
    let other = create_cogmap(&pool, "unreachable-map").await;

    let lens: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name = 'telos-default' AND cogmap_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let region: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience,
            content_cohesion, label, member_count, asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, 'kb_cogmaps', $1, $2,
                 array_fill(0::double precision, ARRAY[768])::vector, 0.9, 0.5, NULL, 3, $3, $3, false)
         RETURNING id",
    )
    .bind(map)
    .bind(lens)
    .bind(event)
    .fetch_one(&pool)
    .await
    .unwrap();

    // The secrets must be owned by someone ELSE: `resources_visible_to`'s first arm is ownership, so a
    // resource the caller owns is readable no matter where it is homed. Homing it out of reach is not
    // enough — that is the whole gap between "somewhere I can't see" and "not mine".
    let stranger: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('pano-stranger','pano-stranger') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let seen = cogmap_resource(&pool, map, profile, "a resource in my map").await;
    let secret_a = cogmap_resource(&pool, other, stranger, "SECRET one").await;
    let secret_b = cogmap_resource(&pool, other, stranger, "SECRET two").await;
    for (member, affinity) in [(secret_a, 0.99_f64), (secret_b, 0.98), (seen, 0.10)] {
        sqlx::query(
            "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
             VALUES ($1, 'kb_resources', $2, $3)",
        )
        .bind(region)
        .bind(member)
        .bind(affinity)
        .execute(&pool)
        .await
        .unwrap();
    }

    let body: temper_core::types::graph_territory::TerritoryOverview = app
        .reqwest_client
        .get(app.url(&format!("/api/graph/cogmaps/{map}/panorama")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let territory = body
        .territories
        .iter()
        .find(|t| t.id == region)
        .expect("the region surfaces in the panorama");
    assert_eq!(
        territory.member_count, 1,
        "the stored count is 3 and three member rows exist, but this caller can read exactly one"
    );
    assert_eq!(
        territory.label.as_deref(),
        Some("a resource in my map"),
        "and the region is named by the one member they CAN read, never by a secret"
    );
}
