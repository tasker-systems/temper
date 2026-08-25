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

/// The banned-key scan for the two standing types, put where the events are ACTUALLY WRITTEN.
///
/// `element_trail_node` / `element_trail_edge` match payloads on key SHAPE with no event-type
/// filter, gated only by `resources_visible_to` — so an admin payload spelling one of those keys
/// leaks an authority record into an ordinary cognition read (spec 2026-07-16 §5).
///
/// **Why here and not in `admin_ledger_test.rs`.** That suite's corpus scan binds the admin
/// vocabulary, and its own doc warns that naming a type there buys nothing on its own: the scan
/// inspects rows that EXIST, and that file's writer hardcodes `grant_created`. For the two standing
/// types it would match no rows and pass VACUOUSLY — a green result proving only that the corpus was
/// empty. This file drives `principal_standing_apply` and `principal_governance_set` themselves, so
/// the payloads under the scan are the ones production writes. The row-count assertions below are
/// what make that claim checkable rather than asserted.
///
/// Mirrors `slack_disconnect_service`'s `the_disconnect_payload_spells_no_trail_matched_key`, which
/// is the same move for the same reason on the type that established the pattern.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_standing_writers_payloads_spell_no_trail_matched_key(pool: PgPool) {
    /// Keys the `element_trail_*` functions match on by shape.
    /// Mirrors `BANNED_ADMIN_PAYLOAD_KEYS` in `tests/admin_ledger_test.rs`.
    const BANNED: &[&str] = &["resource_id", "block_id", "edge_id", "owner"];

    let p = a_profile(&pool, "trail-keys").await;
    let admin = a_profile(&pool, "trail-keys-admin").await;

    // Every payload-shaping branch the two writers have: an actor-less act (`jsonb_strip_nulls`
    // drops `actor`), an act with an actor, one carrying a `reason`, and both governance
    // directions — so the scan covers each shape rather than one representative of them.
    sqlx::query_scalar::<_, String>(
        "SELECT principal_standing_apply($1,'provision','denied',NULL,NULL)",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query_scalar::<_, String>(
        "SELECT principal_standing_apply($1,'request','requested',$2,NULL)",
    )
    .bind(p)
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query_scalar::<_, String>(
        "SELECT principal_standing_apply($1,'approve','approved',$2,'admin approval')",
    )
    .bind(p)
    .bind(admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query_scalar::<_, bool>("SELECT principal_governance_set($1,true,$2,'promotion')")
        .bind(p)
        .bind(admin)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query_scalar::<_, bool>("SELECT principal_governance_set($1,false,$2,'demotion')")
        .bind(p)
        .bind(admin)
        .fetch_one(&pool)
        .await
        .unwrap();

    // Non-vacuity, asserted rather than hoped for: without rows the scan below passes on an empty
    // corpus and proves nothing at all.
    let (standing, governance): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE t.name = 'principal_standing_changed'),
                count(*) FILTER (WHERE t.name = 'principal_governance_changed')
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE e.payload->>'subject_id' = $1::text",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(standing, 3, "the scan is vacuous without standing rows");
    assert_eq!(governance, 2, "the scan is vacuous without governance rows");

    let offenders: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT t.name, k.key
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
             CROSS JOIN LATERAL jsonb_object_keys(e.payload) AS k(key)
            WHERE t.name = ANY($1) AND k.key = ANY($2)"#,
    )
    .bind(["principal_standing_changed", "principal_governance_changed"].as_slice())
    .bind(BANNED)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        offenders.is_empty(),
        "a standing payload spelling one of these keys is matched by element_trail_* on shape \
         alone, which would surface an authority record in an ordinary cognition read. The subject \
         is carried as subject_table/subject_id plus a `references` entry, never as resource_id. \
         Offenders: {offenders:?}"
    );
}
