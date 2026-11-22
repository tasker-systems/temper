#![cfg(feature = "test-db")]
//! The guards stand down only for a real replay (migrations 20261203000010, 20261203000020).
//!
//! `temper.replaying` is a custom setting any session can set, so it alone must lower no guard:
//! every guard asks `_replay_mode()`, true only when the setting is `on` AND the database carries
//! the `kb_replay_target` row, which only the replay walk's own target holds. These tests hold:
//!
//! - the catalogue: no function but `_replay_mode` and `replay_posture` reads the setting;
//! - the helper's truth table, the marker writer's refusal, and `replay_posture`'s reasons;
//! - the two guards no other suite pairs: the gating-slug snapshot and the redaction projector's
//!   trail check. The write guard, the ledger-order guard, the public_within shape guard and the
//!   hard-delete triggers are paired where their worlds are built (`resource_erasure_act`,
//!   `context_change_feed`, `public_within_test`): each refuses under the setting alone and
//!   stands down once the database is marked.

use sqlx::{Connection, PgPool};

/// The functions that stand down for replay, each of which must ask `_replay_mode()`.
const GUARDS: &[&str] = &[
    "_resource_write_guard",
    "kb_events_order_recorded",
    "_project_field_redactions",
    "kb_system_settings_gating_before",
    "kb_team_public_within_shape_guard",
    "kb_teams_public_within_cascade",
    "kb_teams_public_within_hard_delete_begin",
    "kb_teams_public_within_hard_delete_transitions",
];

/// The functions, outside the helper and the posture check, that may read or set the setting: a
/// body naming the word `replaying` in any case (so `CURRENT_SETTING('TEMPER.REPLAYING')` and
/// `E'temper.replaying'`), a body reading `pg_settings` at all (whatever name it builds), or a
/// declaration carrying the setting (`SET temper.replaying = …`, in `proconfig`). A name built from
/// fragments and read through `current_setting` is not seen.
const INLINE_READERS: &str = "SELECT p.oid::regprocedure::text FROM pg_proc p \
       JOIN pg_namespace n ON n.oid = p.pronamespace \
      WHERE n.nspname IN ('public', 'sensitivity') \
        AND (p.prosrc ~* '(replaying|pg_settings)' \
             OR array_to_string(p.proconfig, ' ') ~* 'replaying') \
        AND p.proname NOT IN ('_replay_mode', 'replay_posture') \
      ORDER BY 1";

/// FAILS IF: any function reads `temper.replaying` itself (a guard re-issued from a body that
/// predates `_replay_mode`, or a new one written inline), in any spelling, or a listed guard stops
/// asking `_replay_mode()`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn only_replay_mode_reads_the_replay_setting(pool: PgPool) {
    let inline: Vec<String> = sqlx::query_scalar(INLINE_READERS)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        inline.is_empty(),
        "these read temper.replaying instead of asking _replay_mode(): {inline:?}"
    );

    // The catalogue's own bite: a reader spelled in capitals, one through pg_settings, and a
    // function declared with the setting, are seen.
    let mut tx = pool.begin().await.unwrap();
    for ddl in [
        "CREATE FUNCTION public.replay_probe_upper() RETURNS boolean LANGUAGE sql AS \
           $$ SELECT CURRENT_SETTING('TEMPER.REPLAYING', true) = 'on' $$",
        "CREATE FUNCTION public.replay_probe_settings() RETURNS boolean LANGUAGE sql AS \
           $$ SELECT setting = 'on' FROM pg_settings WHERE name = E'temper.' || 'repl' || 'aying' $$",
        "CREATE FUNCTION public.replay_probe_declared() RETURNS boolean LANGUAGE sql \
           SET temper.replaying = 'on' AS $$ SELECT true $$",
    ] {
        sqlx::query(ddl).execute(&mut *tx).await.unwrap();
    }
    let seen: Vec<String> = sqlx::query_scalar(INLINE_READERS)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        seen,
        [
            "replay_probe_declared()",
            "replay_probe_settings()",
            "replay_probe_upper()"
        ],
        "the catalogue sees a capitalised read, a pg_settings read and a declared setting"
    );

    for guard in GUARDS {
        let asks: bool = sqlx::query_scalar(
            "SELECT bool_and(p.prosrc LIKE '%_replay_mode()%') FROM pg_proc p \
               JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE n.nspname = 'public' AND p.proname = $1",
        )
        .bind(guard)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            asks,
            "{guard} stands down for replay through _replay_mode()"
        );
    }
}

async fn mode(conn: &mut sqlx::PgConnection) -> bool {
    sqlx::query_scalar("SELECT _replay_mode()")
        .fetch_one(conn)
        .await
        .unwrap()
}

async fn posture(conn: &mut sqlx::PgConnection) -> Vec<String> {
    sqlx::query_scalar("SELECT replay_posture()")
        .fetch_one(conn)
        .await
        .unwrap()
}

/// FAILS IF: `_replay_mode()` is true for the setting without the marker, or for the marker
/// without the setting.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn replay_mode_needs_the_setting_and_the_marker(pool: PgPool) {
    let mut tx = pool.begin().await.unwrap();
    assert!(!mode(&mut tx).await, "neither");
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(!mode(&mut tx).await, "the setting alone");
    temper_substrate::test_schema::mark_replay_target(&mut tx).await;
    assert!(mode(&mut tx).await, "the setting in a marked database");
    sqlx::query("SELECT set_config('temper.replaying', 'off', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(!mode(&mut tx).await, "the marker alone");
    tx.rollback().await.unwrap();
}

/// FAILS IF: the marker can be written, by `_replay_target_mark()` or by a plain INSERT, in a
/// database whose ledger holds events, or cannot be in an empty one; or the fixture's DDL bypass
/// leaves the trigger disabled.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_marker_writer_marks_only_an_empty_ledger(pool: PgPool) {
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        events > 0,
        "a migrated database's ledger holds its seed events"
    );
    let err = sqlx::query("SELECT _replay_target_mark()")
        .execute(&pool)
        .await
        .expect_err("a populated ledger is never a replay target");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514"),
        "{err}"
    );
    let err = sqlx::query("INSERT INTO kb_replay_target (id) VALUES (true)")
        .execute(&pool)
        .await
        .expect_err("a plain INSERT is held to the same rule: the trigger binds every writer");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514"),
        "{err}"
    );
    let marked: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_replay_target")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(marked, 0, "the refusals wrote no marker");

    // The fixture's bypass is DDL inside one transaction, and leaves the trigger as it found it.
    let mut conn = pool.acquire().await.unwrap();
    temper_substrate::test_schema::mark_replay_target(&mut conn).await;
    let enabled: String = sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_trigger \
          WHERE tgname = 'kb_replay_target_requires_an_empty_ledger'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        enabled, "O",
        "the trigger is enabled again after the fixture"
    );
    sqlx::query("DELETE FROM kb_replay_target")
        .execute(&mut *conn)
        .await
        .unwrap();
    drop(conn);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("TRUNCATE kb_events CASCADE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT _replay_target_mark()")
        .execute(&mut *tx)
        .await
        .expect("an empty ledger is marked");
    sqlx::query("SELECT _replay_target_mark()")
        .execute(&mut *tx)
        .await
        .expect("marking is idempotent");
    let marked: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_replay_target")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(marked, 1);
    tx.rollback().await.unwrap();
}

/// FAILS IF: `replay_posture()` reports a fresh database, or misses the marker, a database
/// default for the setting, or a session carrying it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn replay_posture_names_each_unsafe_state(pool: PgPool) {
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        posture(&mut conn).await,
        Vec::<String>::new(),
        "a fresh database is safe"
    );

    // A session-level setting: the connection, outside any transaction, carries it.
    sqlx::query("SET temper.replaying = 'on'")
        .execute(&mut *conn)
        .await
        .unwrap();
    assert_eq!(
        posture(&mut conn).await,
        ["this session carries temper.replaying = on"]
    );
    sqlx::query("RESET temper.replaying")
        .execute(&mut *conn)
        .await
        .unwrap();

    // A database default, which every new session would inherit. Scoped to this test's own
    // database, never the role, which every parallel test shares.
    sqlx::query(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET temper.replaying = %L', \
         current_database(), 'on'); END $$",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    let with_default = posture(&mut conn).await;
    sqlx::query(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I RESET temper.replaying', \
         current_database()); END $$",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        with_default,
        ["a database or role default sets temper.replaying"]
    );

    // The marker.
    temper_substrate::test_schema::mark_replay_target(&mut conn).await;
    assert_eq!(
        posture(&mut conn).await,
        ["kb_replay_target carries the replay-target marker"]
    );
    conn.close().await.unwrap();
    assert_eq!(
        temper_services::services::replay_posture_service::check(&pool).await,
        ["kb_replay_target carries the replay-target marker"],
        "the API's startup check reads the same reasons"
    );
}

/// FAILS IF: the gating-slug snapshot (`kb_system_settings_gating_before`) stands down under the
/// setting alone. A probe trigger, ordered between the BEFORE snapshot and the AFTER consumer,
/// reads the snapshot each row write left.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_gating_snapshot_is_taken_under_the_setting_alone(pool: PgPool) {
    let mut conn = pool.acquire().await.unwrap();
    let mut tx = conn.begin().await.unwrap();
    for ddl in [
        "CREATE TEMP TABLE gating_seen (n serial, v text)",
        "CREATE FUNCTION pg_temp.gating_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
           INSERT INTO gating_seen (v) VALUES \
             (current_setting('temper.public_within_gating_slug', true)); \
           RETURN NULL; END $$",
        // AFTER triggers fire by name: `…_gating_a_probe` runs before `…_gating_after` clears it.
        "CREATE TRIGGER kb_system_settings_gating_a_probe AFTER UPDATE OF gating_team_slug \
           ON kb_system_settings FOR EACH ROW EXECUTE FUNCTION pg_temp.gating_probe()",
        "SELECT set_config('temper.replaying', 'on', true)",
        "UPDATE kb_system_settings SET gating_team_slug = 'replay-probe-unmarked'",
    ] {
        sqlx::query(ddl).execute(&mut *tx).await.unwrap();
    }
    temper_substrate::test_schema::mark_replay_target(&mut tx).await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'replay-probe-marked'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let seen: Vec<Option<String>> = sqlx::query_scalar("SELECT v FROM gating_seen ORDER BY n")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(
        seen[0].as_deref().is_some_and(|v| !v.is_empty()),
        "unmarked, the setting alone still snapshots the gating write: {seen:?}"
    );
    assert_eq!(
        seen[1].as_deref(),
        Some(""),
        "control: in a marked target the snapshot stands down"
    );
    tx.rollback().await.unwrap();
}

/// FAILS IF: the redaction projector's trail check (`_project_field_redactions`) stands down under
/// the setting alone. A record naming an event outside its subject's trail is refused for the
/// trail; in a marked target the check stands down and the allowlist check is what refuses.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_trail_check_holds_under_the_setting_alone(pool: PgPool) {
    const PROJECT: &str = "SELECT _project_field_redactions(gen_random_uuid(), \
        jsonb_build_object('subject_id', gen_random_uuid(), 'redacted_fields', \
            jsonb_build_array(jsonb_build_object( \
                'event', (SELECT id FROM kb_events ORDER BY id LIMIT 1), \
                'paths', jsonb_build_array('title')))), 'erasure')";
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    let unmarked = {
        let mut sp = tx.begin().await.unwrap();
        let err = sqlx::query(PROJECT)
            .execute(&mut *sp)
            .await
            .expect_err("a record naming an event outside its trail")
            .to_string();
        sp.rollback().await.unwrap();
        err
    };
    assert!(
        unmarked.contains("not in its subject's trail"),
        "unmarked, the setting alone keeps the trail check: {unmarked}"
    );
    temper_substrate::test_schema::mark_replay_target(&mut tx).await;
    let marked = sqlx::query(PROJECT)
        .execute(&mut *tx)
        .await
        .expect_err("the allowlist still refuses the path")
        .to_string();
    assert!(
        !marked.contains("not in its subject's trail") && marked.contains("allowlist"),
        "control: in a marked target the trail check stands down: {marked}"
    );
    tx.rollback().await.unwrap();
}
