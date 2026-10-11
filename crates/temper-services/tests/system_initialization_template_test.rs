#![cfg(feature = "test-db")]

//! The operator root step, `migrations/templates/system_initialization.sql`, runs against the
//! migrated schema and leaves a first admin who is both admitted and governing.
//!
//! `scripts/bootstrap/system-bootstrap.sh --run-root` executes the template as-is, and the
//! bootstrap playbook points operators at it. It once wrote columns that no longer existed and
//! described gating-team ownership as the way to become admin, and nothing failed, because nothing
//! ran it.
//! This test runs it, so a schema change that breaks the template fails here instead of in a
//! self-hoster's terminal.

use sqlx::PgPool;
use uuid::Uuid;

const TEMPLATE: &str = include_str!("../../../migrations/templates/system_initialization.sql");

/// Substitute the template's psql variables the way `psql --set=name=value` does for `:'name'`:
/// as a quoted literal. Fails if any `:'` survives, so a variable added to the template without
/// being handled here cannot run as the literal text.
fn with_variables(gating: &str, admin: Uuid) -> String {
    let sql = TEMPLATE
        .replace(":'gating'", &format!("'{gating}'"))
        .replace(":'admin'", &format!("'{admin}'"));
    assert!(
        !sql.contains(":'"),
        "the template uses a psql variable this test does not substitute"
    );
    sql
}

/// Run the template for a new first admin under `gating`, and assert the outcome.
async fn bootstraps_a_first_admin(pool: &PgPool, gating: &str) {
    let admin: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('first-admin', 'First Admin') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("the first admin has signed in once");

    sqlx::raw_sql(&with_variables(gating, admin))
        .execute(pool)
        .await
        .expect("the template runs against the migrated schema");

    let (admitted, governs, slug, team): (bool, bool, Option<String>, bool) = sqlx::query_as(
        "SELECT has_system_access($1), is_system_admin($1), \
                (SELECT gating_team_slug FROM kb_system_settings WHERE id = 1), \
                EXISTS (SELECT 1 FROM kb_teams WHERE slug = $2)",
    )
    .bind(admin)
    .bind(gating)
    .fetch_one(pool)
    .await
    .expect("read the outcome");
    assert!(admitted, "the first admin is admitted");
    assert!(governs, "the first admin governs");
    assert_eq!(slug.as_deref(), Some(gating));
    assert!(team, "the gating team exists");
}

/// The conventional slug, which the migrations already seed: the team insert is a no-op.
#[sqlx::test(migrations = "../../migrations")]
async fn the_root_step_template_bootstraps_a_first_admin(pool: PgPool) {
    bootstraps_a_first_admin(&pool, "temper-system").await;
}

/// A self-hoster's own slug, which no migration seeds: the template creates the team.
#[sqlx::test(migrations = "../../migrations")]
async fn the_root_step_template_creates_a_custom_gating_team(pool: PgPool) {
    bootstraps_a_first_admin(&pool, "acme-gate").await;
}
