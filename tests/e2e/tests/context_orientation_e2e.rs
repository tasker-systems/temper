#![cfg(feature = "test-db")]
//! Context orientation reads (spec §3.7, T8) end-to-end: the REAL Axum server, real Postgres, real
//! JWT auth, the real routes, and — for the shape read — the real `temper` binary.
//!
//! This exists because the service-level test (`temper-api/tests/context_orientation_test.rs`) calls
//! `anchor_shape_select` directly and so proves the SQL gate while bypassing HTTP entirely. A route
//! that was never registered, a handler that never resolved its path param, or a client that built
//! the wrong URL would all sail past it. These drive the production caller instead.
//!
//! Proven here:
//!   • `GET /api/contexts/{id}/shape` returns the context's regions to a principal who can read it;
//!   • a stranger gets 200-with-zero-rows, not a 403/404 — deny-as-absence, no existence oracle;
//!   • an explicit context READ grant flips that stranger to seeing the regions (the acceptance
//!     criterion, now at the HTTP layer);
//!   • `temper context shape @me/<slug>` — the real binary, the real `@me` resolution — prints them.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

/// Pre-flight a token (auto-provisions the profile), returning its UUID.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    let body: serde_json::Value = resp.json().await.expect("preflight json parse");
    body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse")
}

/// A region homed in `context`, exactly as the producer writes one: keyed on the anchor pair, with
/// `cogmap_id` NULL (a context region cannot carry one — the column is a FK to kb_cogmaps, which is
/// precisely why the old cogmap-keyed reads could never see it).
///
/// Four real members, homed in the same context and owned by `owner`. They are not decoration: since
/// D5 the count is derived from the members the caller can see, and a region with no visible members
/// is not returned at all — so a member-less region would be a fixture that no longer describes
/// anything the producer can make.
async fn insert_context_region(
    pool: &sqlx::PgPool,
    context: Uuid,
    owner: Uuid,
    label: &str,
) -> Uuid {
    let lens: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_cogmap_lenses WHERE name = 'workflow-default'")
            .fetch_one(pool)
            .await
            .expect("workflow-default lens is seeded by migration");
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("migrations seed at least one event");

    let region = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience, centrality,
            content_cohesion, label, member_count, asserted_by_event_id, last_event_id, is_folded)
         VALUES (NULL, 'kb_contexts', $1, $2,
                 array_fill(0::double precision, ARRAY[768])::vector, 0.75, 1.5, 0.5, $3, 4,
                 $4, $4, false)
         RETURNING id",
    )
    .bind(context)
    .bind(lens)
    .bind(label)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert context region");

    for i in 0..4 {
        let member: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
        )
        .bind(format!("{label}-member-{i}"))
        .fetch_one(pool)
        .await
        .expect("insert member resource");
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(member)
        .bind(context)
        .bind(owner)
        .execute(pool)
        .await
        .expect("home member resource");
        sqlx::query(
            "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
             VALUES ($1, 'kb_resources', $2, $3)",
        )
        .bind(region)
        .bind(member)
        .bind(0.9 - f64::from(i) * 0.1)
        .execute(pool)
        .await
        .expect("add region member");
    }
    region
}

/// The `kb_access_grants` READ row `context_readable_by_profile` (T1) consults.
async fn grant_context_read(pool: &sqlx::PgPool, context: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $2)",
    )
    .bind(context)
    .bind(profile)
    .execute(pool)
    .await
    .expect("grant context read");
}

/// GET the shape route with `token`, asserting 200, and return the parsed rows.
async fn get_shape(app: &common::E2eTestApp, token: &str, context: Uuid) -> Vec<serde_json::Value> {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/contexts/{context}/shape")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("shape request failed");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "the shape read is always 200 — a denied principal gets an empty list, not an error"
    );
    resp.json().await.expect("shape json parse")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_shape_is_reachable_gated_and_grantable_over_http(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let db = app.pool.clone();
    let owner = provision_profile(&app, &app.token).await;

    // The owner's own context, created through the real API.
    let created: serde_json::Value = app
        .reqwest_client
        .post(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "name": "orientation-e2e" }))
        .send()
        .await
        .expect("create context")
        .json()
        .await
        .expect("create context json");
    let context: Uuid = created["id"]
        .as_str()
        .expect("context id")
        .parse()
        .expect("context id parse");
    insert_context_region(&db, context, owner, "e2e-region").await;

    // 1. The owner sees the region — the read the arc exists to deliver.
    let rows = get_shape(&app, &app.token, context).await;
    assert_eq!(rows.len(), 1, "owner sees the context's region: {rows:?}");
    assert_eq!(rows[0]["label"], "e2e-region");
    assert_eq!(
        rows[0]["member_count"], 4,
        "the owner can read all four members, so the count they are handed is all four (D5)"
    );

    // 2. A stranger sees nothing — and gets a 200, not a 403/404 (deny-as-absence).
    let stranger_token =
        common::generate_test_jwt("e2e-t8-stranger", "t8-stranger@test.example.com");
    let stranger = provision_profile(&app, &stranger_token).await;
    let denied = get_shape(&app, &stranger_token, context).await;
    assert!(
        denied.is_empty(),
        "a stranger must not see the context's regions: {denied:?}"
    );

    // 3. THE ACCEPTANCE CRITERION, at the HTTP layer: a context read-grant grants the orientation
    //    read. Same principal, same request — only the grant changed.
    grant_context_read(&db, context, stranger).await;
    let granted = get_shape(&app, &stranger_token, context).await;
    assert_eq!(
        granted.len(),
        1,
        "a context READ grant must grant the orientation read: {granted:?}"
    );
    assert_eq!(granted[0]["label"], "e2e-region");
}

/// The real binary, the real `@me/<slug>` resolution, the real HTTP round-trip.
///
/// The service test could not catch a CLI that resolved `@me` wrongly or built the wrong URL — the
/// `@me` arm is CLI-side (the read resolver deliberately accepts it, unlike `context share`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn temper_context_shape_prints_the_regions(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let db = app.pool.clone();
    let owner = provision_profile(&app, &app.token).await;

    let created: serde_json::Value = app
        .reqwest_client
        .post(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "name": "cli-orientation" }))
        .send()
        .await
        .expect("create context")
        .json()
        .await
        .expect("create context json");
    let context: Uuid = created["id"]
        .as_str()
        .expect("context id")
        .parse()
        .expect("context id parse");

    insert_context_region(&db, context, owner, "cli-visible-region").await;

    let out = common::run_temper_cli(&app, &["context", "shape", "@me/cli-orientation"])
        .await
        .expect("cli run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "`temper context shape @me/cli-orientation` failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("cli-visible-region"),
        "the CLI must print the context's region\nstdout: {stdout}\nstderr: {stderr}"
    );
}
