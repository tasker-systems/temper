#![cfg(feature = "test-db")]

//! The gating team is never someone's personal team, whichever path writes it.
//!
//! Person erasure erases every resource in the subject's personal team and retires its contexts.
//! The gating team holds org-wide content, so if the two were one team, erasing that person would
//! take the org's content with them. The settings door refuses the combination with a 400
//! (`admin_settings_test`); these witnesses hold the same rule against hand-written SQL, on both
//! tables the combination is made of.

use sqlx::PgPool;
use uuid::Uuid;

/// A profile, and the personal team genesis made for it.
async fn person_with_personal_team(pool: &PgPool, handle: &str) -> (Uuid, String) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("insert profile");
    let slug: String = sqlx::query_scalar("SELECT slug FROM kb_teams WHERE personal_of = $1")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("genesis made a personal team");
    (profile, slug)
}

async fn gating_slug(pool: &PgPool) -> Option<String> {
    sqlx::query_scalar("SELECT gating_team_slug FROM kb_system_settings WHERE id = 1")
        .fetch_one(pool)
        .await
        .expect("read settings")
}

fn is_check_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| code == "23514")
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_direct_settings_write_naming_a_personal_team_is_refused(pool: PgPool) {
    let (_, personal) = person_with_personal_team(&pool, "dana").await;
    let before = gating_slug(&pool).await;

    let err = sqlx::query("UPDATE kb_system_settings SET gating_team_slug = $1 WHERE id = 1")
        .bind(&personal)
        .execute(&pool)
        .await
        .expect_err("a personal team must be refused as the gating team");
    assert!(
        is_check_violation(&err),
        "refused as a check violation: {err}"
    );
    assert_eq!(gating_slug(&pool).await, before);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_shared_team_is_still_accepted(pool: PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('acme-gate', 'Acme')")
        .execute(&pool)
        .await
        .expect("insert team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'acme-gate' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("a team that is nobody's personal team is accepted");
    assert_eq!(gating_slug(&pool).await.as_deref(), Some("acme-gate"));
}

/// The other side of the combination: the slug already names the gating team, and a team carrying
/// `personal_of` arrives at it. Only hand-written SQL can leave the gating slug naming no team, but
/// that is enough for genesis to create a personal team under it.
#[sqlx::test(migrations = "../../migrations")]
async fn a_personal_team_never_takes_the_gating_slug(pool: PgPool) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'not-yet-a-team' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("a slug naming no team is not this rule's concern");

    let err = sqlx::query(
        "INSERT INTO kb_teams (slug, name, personal_of) VALUES ('not-yet-a-team', 'x', $1)",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a personal team cannot be created under the gating slug");
    assert!(
        is_check_violation(&err),
        "refused as a check violation: {err}"
    );

    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('not-yet-a-team', 'Gate')")
        .execute(&pool)
        .await
        .expect("a shared team takes the slug");
    let err = sqlx::query("UPDATE kb_teams SET personal_of = $1 WHERE slug = 'not-yet-a-team'")
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect_err("the gating team cannot become someone's personal team");
    assert!(
        is_check_violation(&err),
        "refused as a check violation: {err}"
    );
}

/// The `slug` side of the kb_teams trigger: a personal team renamed onto the gating slug.
#[sqlx::test(migrations = "../../migrations")]
async fn a_personal_team_never_renames_onto_the_gating_slug(pool: PgPool) {
    let (_, personal) = person_with_personal_team(&pool, "gus").await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'not-yet-a-team' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("a slug naming no team is not this rule's concern");

    let err = sqlx::query("UPDATE kb_teams SET slug = 'not-yet-a-team' WHERE slug = $1")
        .bind(&personal)
        .execute(&pool)
        .await
        .expect_err("a personal team cannot take the gating slug by rename");
    assert!(
        is_check_violation(&err),
        "refused as a check violation: {err}"
    );
}

/// Sign-up never fails on the rule. The gating slug can name no team and have the
/// `personal-<handle>` shape — erasure renames a former personal team away and frees its handle —
/// and genesis takes the next free slug rather than one the trigger would refuse.
#[sqlx::test(migrations = "../../migrations")]
async fn sign_up_skips_a_gating_slug_that_names_no_team(pool: PgPool) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'personal-erin' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("a slug naming no team is not this rule's concern");

    let (_, personal) = person_with_personal_team(&pool, "erin").await;
    assert_eq!(personal, "personal-erin-2");
    assert_eq!(gating_slug(&pool).await.as_deref(), Some("personal-erin"));
}

/// The same, when a shared team already holds the slug: the operator's root step creates the gating
/// team without the API's reserved-prefix check.
#[sqlx::test(migrations = "../../migrations")]
async fn sign_up_skips_a_shared_gating_team_holding_its_slug(pool: PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('personal-finn', 'Gate')")
        .execute(&pool)
        .await
        .expect("insert team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'personal-finn' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("a shared team is accepted");

    let (_, personal) = person_with_personal_team(&pool, "finn").await;
    assert_eq!(personal, "personal-finn-2");
}

/// A settings write that leaves the slug as it was is not judged, so an unrelated update — the
/// settings door always names the column — never fails on a state this rule did not create.
#[sqlx::test(migrations = "../../migrations")]
async fn an_unchanged_gating_slug_is_not_judged(pool: PgPool) {
    let (_, personal) = person_with_personal_team(&pool, "hal").await;
    // The state the migration refuses to apply over, made by going around both triggers.
    sqlx::raw_sql(&format!(
        "ALTER TABLE kb_system_settings DISABLE TRIGGER trg_gating_team_is_never_personal; \
         UPDATE kb_system_settings SET gating_team_slug = '{personal}' WHERE id = 1; \
         ALTER TABLE kb_system_settings ENABLE TRIGGER trg_gating_team_is_never_personal;"
    ))
    .execute(&pool)
    .await
    .expect("set up the refused state");

    sqlx::query(
        "UPDATE kb_system_settings \
            SET gating_team_slug = COALESCE(NULL, gating_team_slug), instance_name = 'Acme' \
          WHERE id = 1",
    )
    .execute(&pool)
    .await
    .expect("an update that leaves the slug alone succeeds");
}

/// Two writes, one on each table, cannot each miss the other. While one transaction holds an
/// uncommitted gating slug, a write making that team personal waits for it instead of reading
/// around it; `lock_timeout` turns the wait into an error this test can see.
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_writes_on_the_two_tables_are_serialized(pool: PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('race-team', 'Race')")
        .execute(&pool)
        .await
        .expect("insert team");

    let mut a = pool.begin().await.expect("begin a");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'race-team' WHERE id = 1")
        .execute(&mut *a)
        .await
        .expect("a shared team is accepted");

    let mut b = pool.begin().await.expect("begin b");
    sqlx::query("SET LOCAL lock_timeout = '200ms'")
        .execute(&mut *b)
        .await
        .expect("set lock_timeout");
    let err = sqlx::query("UPDATE kb_teams SET personal_of = $1 WHERE slug = 'race-team'")
        .bind(Uuid::now_v7())
        .execute(&mut *b)
        .await
        .expect_err("the personal-team write must wait for the uncommitted gating slug");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("55P03"),
        "it waited on the lock and timed out: {err}"
    );
    b.rollback().await.expect("rollback b");
    a.rollback().await.expect("rollback a");
}

/// Wait until some backend is blocked on an advisory lock, so a race below is staged rather than
/// slept into.
async fn until_an_advisory_lock_is_awaited(pool: &PgPool) {
    for _ in 0..200 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND NOT granted)",
        )
        .fetch_one(pool)
        .await
        .expect("read pg_locks");
        if waiting {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no backend ever waited on an advisory lock");
}

/// The race in the other order: a personal-team write is in flight, and a gating slug change naming
/// that team waits for it.
#[sqlx::test(migrations = "../../migrations")]
async fn a_gating_slug_change_waits_for_an_inflight_personal_team_write(pool: PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('race-team', 'Race')")
        .execute(&pool)
        .await
        .expect("insert team");

    let mut b = pool.begin().await.expect("begin b");
    sqlx::query("UPDATE kb_teams SET personal_of = $1 WHERE slug = 'race-team'")
        .bind(Uuid::now_v7())
        .execute(&mut *b)
        .await
        .expect("no gating slug names it yet");

    let mut a = pool.begin().await.expect("begin a");
    sqlx::query("SET LOCAL lock_timeout = '200ms'")
        .execute(&mut *a)
        .await
        .expect("set lock_timeout");
    let err =
        sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'race-team' WHERE id = 1")
            .execute(&mut *a)
            .await
            .expect_err("the gating slug change must wait for the uncommitted personal team");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("55P03"),
        "it waited on the lock and timed out: {err}"
    );
    a.rollback().await.expect("rollback a");
    b.rollback().await.expect("rollback b");
}

/// Personal-team writes never wait on one another: one sign-up in flight does not hold up the next.
#[sqlx::test(migrations = "../../migrations")]
async fn sign_ups_do_not_wait_on_one_another(pool: PgPool) {
    let mut first = pool.begin().await.expect("begin first");
    sqlx::query("INSERT INTO kb_profiles (handle, display_name) VALUES ('jo', 'jo')")
        .execute(&mut *first)
        .await
        .expect("first sign-up");

    let mut second = pool.begin().await.expect("begin second");
    sqlx::query("SET LOCAL lock_timeout = '200ms'")
        .execute(&mut *second)
        .await
        .expect("set lock_timeout");
    sqlx::query("INSERT INTO kb_profiles (handle, display_name) VALUES ('kai', 'kai')")
        .execute(&mut *second)
        .await
        .expect("a second sign-up proceeds while the first is uncommitted");
    second.rollback().await.expect("rollback second");
    first.rollback().await.expect("rollback first");
}

/// Genesis reads the gating slug under the lock. A sign-up arriving while a change to its own
/// `personal-<handle>` is uncommitted waits, then skips the slug, rather than reading the old value
/// and being refused by the trigger.
#[sqlx::test(migrations = "../../migrations")]
async fn sign_up_waits_for_an_inflight_gating_slug_change_then_skips_it(pool: PgPool) {
    let mut a = pool.begin().await.expect("begin a");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'personal-ivy' WHERE id = 1")
        .execute(&mut *a)
        .await
        .expect("a slug naming no team is not this rule's concern");

    let signup = tokio::spawn({
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO kb_profiles (handle, display_name) VALUES ('ivy', 'ivy') RETURNING id",
            )
            .fetch_one(&pool)
            .await
        }
    });
    until_an_advisory_lock_is_awaited(&pool).await;
    a.commit().await.expect("commit a");

    let profile = signup
        .await
        .expect("join")
        .expect("the sign-up succeeds once the slug change commits");
    let personal: String = sqlx::query_scalar("SELECT slug FROM kb_teams WHERE personal_of = $1")
        .bind(profile)
        .fetch_one(&pool)
        .await
        .expect("genesis made a personal team");
    assert_eq!(personal, "personal-ivy-2");
}
