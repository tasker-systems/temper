#![cfg(feature = "test-db")]
//! The transition committer: row + log + ledger event, atomically (spec §10, D4).

use sqlx::PgPool;

async fn a_profile(pool: &PgPool, handle: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn apply_writes_row_log_and_event_together(pool: PgPool) {
    let p = a_profile(&pool, "applies").await;

    let state: String =
        sqlx::query_scalar("SELECT principal_standing_apply($1,'provision','denied',NULL,NULL)")
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "denied");

    let row: String =
        sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, "denied", "the projection row must exist");

    let log: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_principal_standing_events WHERE profile_id=$1 AND act='provision'",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(log, 1, "the log entry must exist");

    let ev: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE t.name = 'principal_standing_changed' AND e.payload->>'subject_id' = $1::text",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        ev, 1,
        "the ledger event must exist — D4 makes the trio atomic"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_log_records_the_prior_state_so_reactivate_can_restore(pool: PgPool) {
    let p = a_profile(&pool, "restores").await;
    let admin = a_profile(&pool, "restores-admin").await;

    for (act, resulting) in [
        ("provision", "denied"),
        ("request", "requested"),
        ("approve", "approved"),
        ("deactivate", "deactivated"),
    ] {
        sqlx::query_scalar::<_, String>("SELECT principal_standing_apply($1,$2,$3,$4,NULL)")
            .bind(p)
            .bind(act)
            .bind(resulting)
            .bind(admin)
            .fetch_one(&pool)
            .await
            .unwrap();
    }

    // Spec §5: "Prior standing is recoverable from the log, so reactivation restores rather than
    // guesses."
    let prior: Option<String> = sqlx::query_scalar("SELECT principal_prior_standing($1)")
        .bind(p)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        prior.as_deref(),
        Some("approved"),
        "the state immediately before deactivation must be recoverable"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn prior_standing_is_null_when_there_is_nothing_to_restore(pool: PgPool) {
    let p = a_profile(&pool, "no-prior").await;
    let prior: Option<String> = sqlx::query_scalar("SELECT principal_prior_standing($1)")
        .bind(p)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        prior.is_none(),
        "must be NULL, so the Rust machine refuses rather than guesses"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_standing_change_without_its_audit_record_is_not_representable(pool: PgPool) {
    // D4's whole point. Assert the counts move together across a sequence.
    let p = a_profile(&pool, "atomic").await;
    let admin = a_profile(&pool, "atomic-admin").await;

    for (act, resulting) in [
        ("provision", "denied"),
        ("approve", "approved"),
        ("revoke", "revoked"),
    ] {
        sqlx::query_scalar::<_, String>("SELECT principal_standing_apply($1,$2,$3,$4,'because')")
            .bind(p)
            .bind(act)
            .bind(resulting)
            .bind(admin)
            .fetch_one(&pool)
            .await
            .unwrap();
    }

    let logs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_principal_standing_events WHERE profile_id=$1")
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap();
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='principal_standing_changed' AND e.payload->>'subject_id' = $1::text",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(logs, 3);
    assert_eq!(events, 3, "one ledger event per transition, always");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_events_are_unanchored(pool: PgPool) {
    // kb_events_admin_is_unanchored: admin category implies a NULL producing anchor. An admission
    // act is an authority act with no cognition home; anchoring it would put it in front of every
    // region producer.
    let p = a_profile(&pool, "unanchored").await;
    sqlx::query_scalar::<_, String>(
        "SELECT principal_standing_apply($1,'provision','denied',NULL,NULL)",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();

    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='principal_standing_changed'
            AND (e.producing_anchor_table IS NOT NULL OR e.producing_anchor_id IS NOT NULL)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(anchored, 0);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn governance_set_is_idempotent_and_emits_only_on_change(pool: PgPool) {
    let p = a_profile(&pool, "gov").await;
    let admin = a_profile(&pool, "gov-admin").await;

    let first: bool = sqlx::query_scalar("SELECT principal_governance_set($1,true,$2,NULL)")
        .bind(p)
        .bind(admin)
        .fetch_one(&pool)
        .await
        .unwrap();
    let second: bool = sqlx::query_scalar("SELECT principal_governance_set($1,true,$2,NULL)")
        .bind(p)
        .bind(admin)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert!(first, "the first grant changes something");
    assert!(!second, "the second is a no-op");

    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='principal_governance_changed' AND e.payload->>'subject_id' = $1::text",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(events, 1, "a no-op is not an admin act; the ledger is append-only and a spurious row can never be corrected, only quarantined");
}
