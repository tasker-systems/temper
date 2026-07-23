#![cfg(feature = "test-db")]
//! Structural guarantees of the standing tables (spec §10, D2, D7, D9).

use sqlx::PgPool;

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn standing_is_one_row_per_principal(pool: PgPool) {
    let profile: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('t1','T1') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'denied')")
        .bind(profile)
        .execute(&pool)
        .await
        .expect("first insert");

    // D2: ONE authoritative state. A second row for the same principal must be impossible.
    let err =
        sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'approved')")
            .bind(profile)
            .execute(&pool)
            .await
            .expect_err("a second standing row must be refused");

    let db = err.as_database_error().expect("a database error");
    assert_eq!(
        db.code().as_deref(),
        Some("23505"),
        "must fail as a unique violation — standing is one row per principal (D2). Got: {db}"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unknown_state_literal_is_refused_at_write_time(pool: PgPool) {
    let profile: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('t2','T2') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let err =
        sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'admin')")
            .bind(profile)
            .execute(&pool)
            .await
            .expect_err("an unknown state must be refused");

    assert_eq!(
        err.as_database_error().unwrap().code().as_deref(),
        Some("23514"),
        "must fail the CHECK constraint"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_standing_tables_carry_no_team_dimension(pool: PgPool) {
    // D9: "do not carry team_id into the standing tables". Asking to join a TEAM is orthogonal to
    // standing in the SYSTEM; conflating them is what put a team_id on a system-access request.
    for table in [
        "kb_principal_standing",
        "kb_principal_standing_events",
        "kb_principal_governance",
    ] {
        // Assert the table EXISTS before asserting what it lacks. `information_schema.columns`
        // returns zero rows for a table that does not exist, so the D9 assertion below is
        // vacuously true against an unmigrated database — it would stay green if this whole
        // migration were reverted. The existence check is what makes the absence meaningful.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "{table} must exist");

        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.columns
              WHERE table_name = $1 AND column_name LIKE '%team%'",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "{table} must carry no team dimension (D9)");
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_standing_log_is_append_only_in_enforcement_not_only_in_comment(pool: PgPool) {
    // The table's COMMENT says "Append-only. NEVER UPDATE OR DELETE A ROW HERE." A comment is not
    // an enforcement — `SystemAuthorized`'s doc claims a type-state guarantee its public field does
    // not hold, and this is the same failure mode. `kb_events`, the repo's only other append-only
    // log, enforces it with a BEFORE DELETE OR UPDATE trigger; this asserts the same for this log.
    //
    // It matters here specifically: `principal_prior_standing` reads this log to decide what
    // `Reactivate` restores to. A rewritable log means what a reactivation restores can be changed
    // after the fact — Deactivated could be restored as Approved. That is the escalation path this
    // trigger closes, and it is why kb_events is guarded rather than merely documented.
    let p = a_profile_for_log(&pool).await;

    sqlx::query("SELECT principal_standing_apply($1,'provision','denied',NULL,NULL)")
        .bind(p)
        .execute(&pool)
        .await
        .expect("seed one log row");

    let upd = sqlx::query(
        "UPDATE kb_principal_standing_events SET resulting_state='approved' WHERE profile_id=$1",
    )
    .bind(p)
    .execute(&pool)
    .await
    .expect_err("UPDATE on the standing log must be refused");
    assert!(
        upd.to_string().contains("append-only"),
        "UPDATE must raise the append-only guard, got: {upd}"
    );

    let del = sqlx::query("DELETE FROM kb_principal_standing_events WHERE profile_id=$1")
        .bind(p)
        .execute(&pool)
        .await
        .expect_err("DELETE on the standing log must be refused");
    assert!(
        del.to_string().contains("append-only"),
        "DELETE must raise the append-only guard, got: {del}"
    );
}

async fn a_profile_for_log(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('logguard','LogGuard') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn a_profile(pool: &PgPool, handle: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_standing_log_is_append_only_in_shape(pool: PgPool) {
    // The log has no UPDATE path in the design; assert it at least records both endpoints of a
    // transition, so `Reactivate` has something to restore from (spec §5, §11).
    for col in [
        "prior_state",
        "resulting_state",
        "act",
        "actor_profile_id",
        "occurred_at",
    ] {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.columns
              WHERE table_name = 'kb_principal_standing_events' AND column_name = $1",
        )
        .bind(col)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "the standing log must record {col}");
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn both_predicates_are_total(pool: PgPool) {
    // Spec §7: "SQL totality has its own test — has_system_access and is_system_admin return
    // non-NULL for a profile with no standing row, a deactivated one, and an unknown state value."
    let absent = a_profile(&pool, "absent").await;

    let deactivated = a_profile(&pool, "deactivated").await;
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'deactivated')")
        .bind(deactivated)
        .execute(&pool)
        .await
        .unwrap();

    // An unknown state cannot be inserted through the CHECK, so reach past it to simulate the
    // rolling-deploy window this obligation exists for.
    let unknown = a_profile(&pool, "unknown").await;
    sqlx::query(
        "ALTER TABLE kb_principal_standing DROP CONSTRAINT kb_principal_standing_state_check",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'quarantined')")
        .bind(unknown)
        .execute(&pool)
        .await
        .unwrap();

    for (label, id) in [
        ("absent", absent),
        ("deactivated", deactivated),
        ("unknown", unknown),
    ] {
        for f in ["has_system_access", "is_system_admin"] {
            let v: Option<bool> = sqlx::query_scalar(&format!("SELECT {f}($1)"))
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                v,
                Some(false),
                "{f}({label}) must be FALSE, never NULL — a NULL in `IF NOT` falls through, \
                 fail-OPEN, and context_reassign_fns.sql:76 falls open into system admin"
            );
        }
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn only_approved_standing_grants_access(pool: PgPool) {
    for state in ["denied", "requested", "revoked", "deactivated"] {
        let p = a_profile(&pool, &format!("s-{state}")).await;
        sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,$2)")
            .bind(p)
            .bind(state)
            .execute(&pool)
            .await
            .unwrap();
        let v: Option<bool> = sqlx::query_scalar("SELECT has_system_access($1)")
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(v, Some(false), "{state} must not grant access");
    }

    let ok = a_profile(&pool, "s-approved").await;
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'approved')")
        .bind(ok)
        .execute(&pool)
        .await
        .unwrap();
    let v: Option<bool> = sqlx::query_scalar("SELECT has_system_access($1)")
        .bind(ok)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(v, Some(true));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_ness_no_longer_reads_gating_team_ownership(pool: PgPool) {
    // D10: gating-team ownership stops being an authorization fact. Make someone a gating-team
    // OWNER without a governance row and assert they are NOT admin — this is the property that
    // makes the ~20 kb_team_members writers harmless.
    let p = a_profile(&pool, "gating-owner").await;
    let team: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','System')
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'owner')")
        .bind(team)
        .bind(p)
        .execute(&pool)
        .await
        .unwrap();

    let v: Option<bool> = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(p)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        v,
        Some(false),
        "owning the gating team must confer nothing once governance holds its own state (D10)"
    );

    sqlx::query("INSERT INTO kb_principal_governance (profile_id) VALUES ($1)")
        .bind(p)
        .execute(&pool)
        .await
        .unwrap();
    let v: Option<bool> = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(p)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(v, Some(true), "the governance row IS admin-ness now");
}
