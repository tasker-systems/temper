#![cfg(all(feature = "test-db", feature = "test-embed"))]
//! Org-bootstrap capstone (org-provisioning Chunk 7): the CI "harness operator" arm of the
//! `docs/guides/org-bootstrap.md` SoP gate. Against a blank database it drives the EXACT runbook
//! command sequence through the real `temper` binary — the irreducible SQL root step, then the
//! surfaced, idempotent admin/team/cogmap commands — and asserts the result is a usable org: a
//! team-visible resource becomes reachable through the org-identity map once it is bound.
//!
//! `embed`-gated because `cogmap create` / `cogmap reconcile` embed the charter + landmarks
//! CLIENT-SIDE (ONNX) before the request, exactly as an operator's `embed`-capable binary does.
//! Runs in the Embed CI job / `cargo make test-e2e-embed`. The committed manifests under
//! `schema-artifact/manifests/` are the ones exercised, so the runbook cannot drift from reality
//! without breaking this test.

mod common;

use std::path::PathBuf;
use uuid::Uuid;

use reqwest::StatusCode;

/// Absolute path to a committed manifest, resolved from this crate dir (`tests/e2e`) so it is
/// robust to the spawned CLI's working directory.
fn manifest(rel: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p.push(rel);
    p.canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize {rel}: {e}"))
        .to_string_lossy()
        .into_owned()
}

/// Pre-flight a token (auto-provisions the profile), returning its UUID.
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// The irreducible 2-UPDATE operator root step (runbook §0): configure gating + mint first admin.
/// Mirrors `root_bootstrap_first_admin` in `admin_surface_e2e.rs`.
async fn root_bootstrap_first_admin(pool: &sqlx::PgPool, admin_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(pool)
    .await
    .expect("gating team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(pool)
        .await
        .expect("gating");
    sqlx::query("UPDATE kb_profiles SET system_access='admin' WHERE id=$1")
        .bind(admin_id)
        .execute(pool)
        .await
        .expect("promote first admin"); // trigger mints owner of temper-system
                                        // D11: is_system_admin reads governance, has_system_access reads standing; the column + gating
                                        // ownership above confer neither. Grant both so the bootstrapped admin can actually act.
    common::approved_admin(pool, admin_id).await;
}

/// Run a `temper` CLI step, asserting success and returning parsed JSON stdout.
async fn cli_json(app: &common::E2eTestApp, args: &[&str]) -> serde_json::Value {
    let out = common::run_temper_cli(app, args)
        .await
        .expect("spawn temper");
    assert!(
        out.status.success(),
        "`temper {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "`temper {}` stdout not JSON ({e}): {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

async fn team_id_by_slug(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM kb_teams WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("team {slug} not found: {e}"))
}

/// `true` when `resources_accessible_to_cogmap(cogmap)` includes `resource_id`.
async fn resource_accessible(pool: &sqlx::PgPool, cogmap_id: Uuid, resource_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM resources_accessible_to_cogmap($1) r WHERE r.resource_id = $2)",
    )
    .bind(cogmap_id)
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("resources_accessible_to_cogmap")
}

/// Insert a resource granted-readable to `team_id` (the shape a team-shared resource has).
async fn grant_team_resource(pool: &sqlx::PgPool, team_id: Uuid, granted_by: Uuid) -> Uuid {
    let resource_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, 'Org note', $2)")
        .bind(resource_id)
        .bind(format!("test://{resource_id}"))
        .execute(pool)
        .await
        .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_access_grants \
            (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_teams', $2, true, $3)",
    )
    .bind(resource_id)
    .bind(team_id)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("grant team read");
    resource_id
}

/// The full SoP, end to end: blank DB → usable org. One test so the steps compose in order.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn org_bootstrap_sop_end_to_end(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;

    // §0 — irreducible SQL root step.
    root_bootstrap_first_admin(&pool, admin_id).await;

    // §1 — instance settings (surfaced, admin-gated).
    let settings = cli_json(
        &app,
        &[
            "admin",
            "settings",
            "--instance-name",
            "Acme Temper",
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(settings["instance_name"], "Acme Temper");
    assert_eq!(settings["gating_team_slug"], "temper-system");

    // §2 — the everyone auto-join team (admin-gated --auto-join-role).
    cli_json(
        &app,
        &[
            "team",
            "create",
            "everyone",
            "--name",
            "Everyone",
            "--auto-join-role",
            "watcher",
            "--format",
            "json",
        ],
    )
    .await;
    let everyone_id = team_id_by_slug(&pool, "everyone").await;
    let auto_join: Option<String> =
        sqlx::query_scalar("SELECT auto_join_role::text FROM kb_teams WHERE slug = 'everyone'")
            .fetch_one(&pool)
            .await
            .expect("auto_join_role");
    assert_eq!(
        auto_join.as_deref(),
        Some("watcher"),
        "everyone-team is auto-join watcher"
    );

    // §3 — genesis the org-identity cogmap (charter embedded client-side).
    let genesis = cli_json(
        &app,
        &[
            "cogmap",
            "create",
            "--manifest",
            &manifest("schema-artifact/manifests/org-identity.yaml"),
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(genesis["created"], true, "first genesis creates the map");
    let cogmap_id: Uuid = genesis["cogmap_id"]
        .as_str()
        .expect("cogmap_id")
        .parse()
        .expect("uuid");

    // A team-visible resource exists, but an unbound map reaches nothing through the team.
    let resource_id = grant_team_resource(&pool, everyone_id, admin_id).await;
    assert!(
        !resource_accessible(&pool, cogmap_id, resource_id).await,
        "before binding, the map must not reach the team's resource"
    );

    // §4 — deliver landmark content; reconcile is idempotent on re-run.
    let landmarks = manifest("schema-artifact/manifests/org-identity-landmarks.yaml");
    let r1 = cli_json(
        &app,
        &[
            "cogmap",
            "reconcile",
            &cogmap_id.to_string(),
            "--manifest",
            &landmarks,
            "--format",
            "json",
        ],
    )
    .await;
    assert!(
        r1["created"].as_u64().unwrap_or(0) >= 1,
        "first reconcile delivers landmarks: {r1}"
    );
    let r2 = cli_json(
        &app,
        &[
            "cogmap",
            "reconcile",
            &cogmap_id.to_string(),
            "--manifest",
            &landmarks,
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(
        r2["created"], 0,
        "re-reconcile creates nothing (idempotent)"
    );
    assert_eq!(
        r2["updated"], 0,
        "re-reconcile updates nothing (idempotent)"
    );

    // §5 — bind the map to the everyone-team; the team's resource is now reachable.
    cli_json(
        &app,
        &[
            "cogmap",
            "bind",
            &cogmap_id.to_string(),
            "+everyone",
            "--format",
            "json",
        ],
    )
    .await;
    let bound: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_team_cogmaps WHERE cogmap_id = $1 AND team_id = $2)",
    )
    .bind(cogmap_id)
    .bind(everyone_id)
    .fetch_one(&pool)
    .await
    .expect("binding exists");
    assert!(bound, "the bind wrote a kb_team_cogmaps row");
    assert!(
        resource_accessible(&pool, cogmap_id, resource_id).await,
        "after binding, the map reaches the team's shared resource — the org is usable"
    );
}
