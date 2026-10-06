#![cfg(feature = "test-db")]
//! The sweep off until an operator turns it on (sweep Q52, Q53, Q54), under goal *"Personal data
//! that lands in the corpus by accident is found"* §8's first clause: no deploy and no migration
//! starts a scan or enables a detector.
//!
//! The deployment's half, the door answering without touching the database when
//! `SENSITIVITY_SWEEP_ENABLED` is off, is pass 0 of `sensitivity_sweep_span_test.rs`, which owns the
//! process's subscriber. This file holds the detectors' half (off by default, who provides each,
//! the enabling functions) and the dry run.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use temper_services::services::sensitivity_sweep_service::{sweep, sweep_within, Ended};

const SALT: &[u8] = b"default-off-witness-salt-of-thirty-two-plus";
const SSN: &str = "219-45-6789";
const SSN_B: &str = "536-22-8147";
/// The last migration before default-off: the state every deployment was in when it landed.
const BEFORE_DEFAULT_OFF: i64 = 20261004130000;

const TEMPER_NINE: [&str; 9] = [
    "aba_routing",
    "cloud_saas_key",
    "connection_string_password",
    "jwt",
    "local_path_username",
    "payment_card",
    "private_key_block",
    "us_ssn_contextual",
    "us_ssn_delimited",
];

fn code(e: &sqlx::Error) -> String {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.to_string())
        .unwrap_or_default()
}

async fn enabled(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT id FROM sensitivity.detectors WHERE enabled")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

async fn organization_detector(pool: &PgPool, id: &str, severity: i16) {
    sqlx::query(
        "INSERT INTO sensitivity.detectors (id, category, severity, prefilter, pattern, note) \
         VALUES ($1, 'identifier', $2, 'ACME-', 'ACME-[0-9]{6}', 'an organization''s badge number')",
    )
    .bind(id)
    .bind(severity)
    .execute(pool)
    .await
    .unwrap();
}

/// A resource titled `title`, stamped an hour ago so it sits below a tick's head bound.
/// Resources the migrated database already holds, so a count of what a dry run read is exact.
async fn resources(pool: &PgPool) -> i32 {
    sqlx::query_scalar("SELECT count(*)::int FROM kb_resources")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn titled(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri, updated) \
         VALUES ($1, 'test://default-off', now() - interval '1 hour') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A database no migration has touched, beside the test's own, and its name.
///
/// The test's own database cannot be that. CI copies every `#[sqlx::test]` database from a
/// `template1` that already holds the whole chain (`.github/scripts/sqlx-test-template.sh`), and
/// `migrations = false` stops sqlx from migrating, not Postgres from copying. `template0` is never
/// migrated. The name derives from the test's database, so a copy that a failed run leaked is dropped
/// by the next run.
async fn unmigrated(pool: &PgPool) -> (PgPool, String) {
    // Hashed, not suffixed: sqlx's test database names already fill Postgres's 63-byte identifier
    // limit, so `<name>_unmigrated` truncates back to the test's own database.
    let name: String = sqlx::query_scalar("SELECT 'unmigrated_' || md5(current_database())")
        .fetch_one(pool)
        .await
        .unwrap();
    for ddl in [
        format!(r#"DROP DATABASE IF EXISTS "{name}""#),
        format!(r#"CREATE DATABASE "{name}" TEMPLATE template0"#),
    ] {
        sqlx::query(&ddl).execute(pool).await.unwrap();
    }
    let options = pool.connect_options().as_ref().clone().database(&name);
    (PgPool::connect_with(options).await.unwrap(), name)
}

// ── Detectors: off by default, and who provides each ──────────────────────────────────────────

/// A migration never enables a detector, and changes only the rows temper provides. Run on a
/// database as it stood before default-off, holding a row an organization added and left enabled.
#[sqlx::test(migrations = false)]
async fn default_off_turns_temper_detectors_off_and_leaves_an_organizations_alone(own: PgPool) {
    let (pool, name) = unmigrated(&own).await;
    let all = &temper_services::MIGRATOR;
    let before = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|m| m.version <= BEFORE_DEFAULT_OFF)
                .cloned()
                .collect(),
        ),
        ignore_missing: all.ignore_missing,
        locking: all.locking,
        no_tx: all.no_tx,
    };
    before
        .run(&pool)
        .await
        .expect("the chain before default-off applies");
    organization_detector(&pool, "acme_badge", 2).await;
    assert!(
        enabled(&pool).await.contains("acme_badge"),
        "the organization's row starts enabled, or its survival below proves nothing"
    );

    all.run(&pool).await.expect("the rest of the chain applies");

    let rows: Vec<(String, String, bool)> =
        sqlx::query_as("SELECT id, provided_by, enabled FROM sensitivity.detectors ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let mut expect: Vec<(String, String, bool)> = TEMPER_NINE
        .iter()
        .map(|id| (id.to_string(), "temper".to_string(), false))
        .collect();
    expect.push(("acme_badge".into(), "organization".into(), true));
    expect.sort();
    assert_eq!(rows, expect);

    pool.close().await;
    sqlx::query(&format!(r#"DROP DATABASE "{name}""#))
        .execute(&own)
        .await
        .unwrap();
}

/// A detector an operator inserts without naming either column is the organization's, and off.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_new_detector_is_the_organizations_and_does_not_scan(pool: PgPool) {
    organization_detector(&pool, "acme_badge", 2).await;
    let (provided_by, on): (String, bool) = sqlx::query_as(
        "SELECT provided_by, enabled FROM sensitivity.detectors WHERE id = 'acme_badge'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((provided_by.as_str(), on), ("organization", false));
    let bad = sqlx::query("UPDATE sensitivity.detectors SET provided_by = 'acme' WHERE id = 'jwt'")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&bad), "23514", "{bad}");
}

/// With the deployment opted in and every detector off, the door claims nothing, so it leaves no
/// run and no job, even while another transaction holds the head back (the case in which an idle
/// tick keeps its rows, Q49). Then one detector is enabled and the same content is found, so the
/// silence was the detectors being off, not the plant being unreachable.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_opted_in_door_with_every_detector_off_scans_nothing(pool: PgPool) {
    titled(&pool, &format!("Payroll note for {SSN}")).await;
    assert!(enabled(&pool).await.is_empty(), "a detector ships enabled");
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT 1").execute(&mut *held).await.unwrap();

    let summary = sweep_within(&pool, Some(SALT), Duration::from_secs(60))
        .await
        .expect("sweep runs");
    assert!(summary.ticks.is_empty(), "{summary:?}");
    assert_eq!(
        summary.ended,
        Ended::NoDetectors,
        "nothing enabled must not read as nothing new"
    );
    let left = |pool: PgPool| async move {
        sqlx::query_as::<_, (i64, i64, i64, i64)>(
            "SELECT (SELECT count(*) FROM sensitivity.findings), (SELECT count(*) FROM sensitivity.runs), \
                    (SELECT count(*) FROM sensitivity.cursors), \
                    (SELECT count(*) FROM kb_workflow_jobs WHERE persona = 'sensitivity')",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(left(pool.clone()).await, (0, 0, 0, 0));
    held.rollback().await.unwrap();

    sqlx::query("SELECT sensitivity.enable_detector('us_ssn_delimited', 1)")
        .execute(&pool)
        .await
        .unwrap();
    // A first enabled tick starts the cursors at its bound; the backfill reaches the plant after.
    for _ in 0..3 {
        sweep_within(&pool, Some(SALT), Duration::from_secs(60))
            .await
            .expect("sweep runs");
    }
    let found: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sensitivity.findings WHERE surface = 'kb_resources.title'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(found, 1, "the enabled detector must find the plant");
}

/// Erasure's promise outlives the sweep (Q50): a deployment that is not opted in still gives up the
/// digests of a finding whose place an erasure act emptied, once its 30 days have passed.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_door_that_is_off_still_expires_erased_digests(pool: PgPool) {
    // A finding on a remote source the erasure act deleted, closed and seen so 31 days ago.
    let finding: Uuid = sqlx::query_scalar(
        "INSERT INTO sensitivity.findings (surface, target_table, target_id, content_hash, detector_id, \
             detector_version, category, severity, match_count, fingerprint_state) \
         VALUES ('kb_remote_sources.uri', 'kb_remote_sources', $1, repeat('a', 64), \
                 'us_ssn_delimited', 1, 'national_id', 4, 1, 'complete') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sensitivity.erased_closures (finding_id, closed_seen_at) \
         VALUES ($1, now() - interval '31 days')",
    )
    .bind(finding)
    .execute(&pool)
    .await
    .unwrap();

    let summary = sweep(&pool, None, false).await.expect("sweep answers");
    assert_eq!(summary.fingerprints_expired, 1, "{summary:?}");
    let state: String =
        sqlx::query_scalar("SELECT fingerprint_state FROM sensitivity.findings WHERE id = $1")
            .bind(finding)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "expired");
}

// ── Enabling and disabling ────────────────────────────────────────────────────────────────────

/// An operator enables what they reviewed: a version that is not current refuses, and changes
/// nothing. Each call says whether it changed anything.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn enabling_by_name_refuses_a_version_that_is_not_current(pool: PgPool) {
    let enable = |id: &'static str, version: i32| {
        sqlx::query_scalar::<_, bool>("SELECT sensitivity.enable_detector($1, $2)")
            .bind(id)
            .bind(version)
    };
    // payment_card is at v3 (Q51): v2 was the noisy one.
    let stale = enable("payment_card", 2)
        .fetch_one(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&stale), "22023", "{stale}");
    let unknown = enable("no_such_detector", 1)
        .fetch_one(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&unknown), "P0002", "{unknown}");
    assert!(
        enabled(&pool).await.is_empty(),
        "a refusal enabled something"
    );

    assert!(enable("payment_card", 3).fetch_one(&pool).await.unwrap());
    assert!(!enable("payment_card", 3).fetch_one(&pool).await.unwrap());
    assert_eq!(
        enabled(&pool).await,
        BTreeSet::from(["payment_card".into()])
    );

    let disable =
        || sqlx::query_scalar::<_, bool>("SELECT sensitivity.disable_detector('payment_card')");
    assert!(disable().fetch_one(&pool).await.unwrap());
    assert!(!disable().fetch_one(&pool).await.unwrap());
    assert!(enabled(&pool).await.is_empty());
}

/// A bump is a pattern nobody reviewed, so it turns the detector off, whoever provides it; the
/// operator enables the new version by name.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_version_bump_turns_the_detector_off(pool: PgPool) {
    sqlx::query("SELECT sensitivity.enable_detector('jwt', 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sensitivity.detectors \
            SET version = 2, pattern = 'eyJ[A-Za-z0-9_-]{12,}\\.[A-Za-z0-9_-]{12,}\\.[A-Za-z0-9_-]{12,}' \
          WHERE id = 'jwt'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        enabled(&pool).await.is_empty(),
        "the bump left the new version scanning"
    );
    // The same statement without a bump leaves `enabled` alone.
    sqlx::query("SELECT sensitivity.enable_detector('jwt', 2)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE sensitivity.detectors SET note = 'reworded' WHERE id = 'jwt'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(enabled(&pool).await, BTreeSet::from(["jwt".into()]));
}

/// Severity is a threshold: enable at least this serious, disable at most this serious, either
/// narrowed to one provider. Each returns exactly what it changed.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn severity_thresholds_select_by_provider(pool: PgPool) {
    organization_detector(&pool, "acme_badge", 4).await;
    organization_detector(&pool, "acme_minor", 3).await;
    let changed = |sql: &'static str, severity: i32, provider: Option<&'static str>| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(sql)
                .bind(severity)
                .bind(provider)
                .fetch_all(&pool)
                .await
                .map(|ids| ids.into_iter().collect::<BTreeSet<_>>())
        }
    };
    const ENABLE: &str = "SELECT detector_id FROM sensitivity.enable_detectors($1, $2)";
    const DISABLE: &str = "SELECT detector_id FROM sensitivity.disable_detectors($1, $2)";
    let set = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();

    assert_eq!(
        changed(ENABLE, 4, Some("temper")).await.unwrap(),
        set(&[
            "cloud_saas_key",
            "connection_string_password",
            "payment_card",
            "private_key_block",
            "us_ssn_contextual",
            "us_ssn_delimited",
        ]),
    );
    // At or above 3, any provider: the temper threes and both organization rows. Already enabled
    // rows are not changed again, and the severity-1 detector stays off.
    assert_eq!(
        changed(ENABLE, 3, None).await.unwrap(),
        set(&["aba_routing", "jwt", "acme_badge", "acme_minor"]),
    );
    assert!(!enabled(&pool).await.contains("local_path_username"));
    // At or below 3, temper only: the organization's three stays on.
    assert_eq!(
        changed(DISABLE, 3, Some("temper")).await.unwrap(),
        set(&["aba_routing", "jwt"]),
    );
    assert!(enabled(&pool).await.contains("acme_minor"));

    for (severity, provider) in [(0, None), (5, None), (3, Some("acme"))] {
        let e = changed(ENABLE, severity, provider).await.unwrap_err();
        assert_eq!(code(&e), "22023", "{severity} {provider:?}: {e}");
    }
}

// ── Dry runs ──────────────────────────────────────────────────────────────────────────────────

type Surface = (String, String, i32, i32, i64, i32, bool, serde_json::Value);

async fn dry_run(
    pool: &PgPool,
    detector: &str,
    version: i32,
    bound: &str,
    bound_id: Option<Uuid>,
    surface: Option<&str>,
    max_units: i32,
) -> Result<Vec<Surface>, sqlx::Error> {
    sqlx::query_as(
        "SELECT surface, shape, rows_examined, places_matched, matches, skipped_oversize, complete, \
                hotspots \
           FROM sensitivity.dry_run($1, $2, $3, $4, $5, $6)",
    )
    .bind(detector)
    .bind(version)
    .bind(bound)
    .bind(bound_id)
    .bind(surface)
    .bind(max_units)
    .fetch_all(pool)
    .await
}

fn hot_targets(row: &Surface) -> BTreeSet<Uuid> {
    row.7
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["target_id"].as_str().unwrap().parse().unwrap())
        .collect()
}

/// Every row the sweep or its queue could hold, counted per table.
async fn sweep_state(pool: &PgPool) -> Vec<i64> {
    let mut counts = Vec::new();
    for table in [
        "sensitivity.findings",
        "sensitivity.finding_fingerprints",
        "sensitivity.dispositions",
        "sensitivity.cursors",
        "sensitivity.runs",
        "sensitivity.memo",
        "sensitivity.place_observations",
        "sensitivity.unscanned_places",
        "sensitivity.erased_closures",
        "sensitivity.detectors",
        "sensitivity.detector_versions",
    ] {
        counts.push(
            sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    counts.push(
        sqlx::query_scalar("SELECT count(*) FROM kb_workflow_jobs WHERE persona = 'sensitivity'")
            .fetch_one(pool)
            .await
            .unwrap(),
    );
    counts
}

/// A dry run of a disabled detector counts matches per surface and points at the places with the
/// most, never quoting a match, and leaves every table the sweep reads as it was.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dry_run_counts_and_points_and_writes_nothing(pool: PgPool) {
    let two = titled(&pool, &format!("Two numbers: {SSN} and {SSN_B}")).await;
    let one = titled(&pool, &format!("One number: {SSN}")).await;
    titled(&pool, "Nothing to see").await;
    let titles = resources(&pool).await;
    let before = sweep_state(&pool).await;

    let rows = dry_run(&pool, "us_ssn_delimited", 1, "latest", None, None, 5000)
        .await
        .unwrap();
    assert_eq!(
        sweep_state(&pool).await,
        before,
        "the dry run wrote something"
    );

    let title = rows
        .iter()
        .find(|r| r.0 == "kb_resources.title")
        .expect("the title surface is reported");
    assert_eq!(title.1, "text");
    assert_eq!(
        (title.2, title.3, title.4, title.5, title.6),
        (titles, 2, 3, 0, true)
    );
    let hot = title.7.as_array().unwrap();
    assert_eq!(hot[0]["target_id"].as_str().unwrap(), two.to_string());
    assert_eq!(hot[0]["matches"], 2);
    assert_eq!(hot[1]["target_id"].as_str().unwrap(), one.to_string());
    assert!(
        rows.iter().any(|r| r.1 == "jsonb"),
        "every surface with a source is reported, the jsonb ones too"
    );

    let answer = format!("{rows:?}");
    for needle in [SSN, SSN_B, "219456789", "536228147"] {
        assert!(!answer.contains(needle), "the dry run quoted `{needle}`");
    }

    let unknown = dry_run(&pool, "us_ssn_delimited", 2, "latest", None, None, 10)
        .await
        .unwrap_err();
    assert_eq!(code(&unknown), "P0002", "{unknown}");
}

/// Each bound reads the resources homed in it, by structure, and nothing else.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dry_run_bound_reads_only_what_is_homed_there(pool: PgPool) {
    let profile = |handle: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
            )
            .bind(handle)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let alice = profile("alice-dryrun").await;
    let bob = profile("bob-dryrun").await;
    let team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('dryrun', 'Dry run') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let context = |owner_table: &'static str, owner: Uuid, slug: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
                 VALUES ($1, $2, $3, $3) RETURNING id",
            )
            .bind(owner_table)
            .bind(owner)
            .bind(slug)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let alices = context("kb_profiles", alice, "alices").await;
    let teams = context("kb_teams", team, "teams").await;
    let charter = titled(&pool, "Charter").await;
    let map: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('map', $1) RETURNING id",
    )
    .bind(charter)
    .fetch_one(&pool)
    .await
    .unwrap();
    let home = |resource: Uuid, table: &'static str, anchor: Uuid, owner: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO kb_resource_homes \
                     (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
                 VALUES ($1, $2, $3, $4, $4)",
            )
            .bind(resource)
            .bind(table)
            .bind(anchor)
            .bind(owner)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    let in_alices = titled(&pool, &format!("alice's {SSN}")).await;
    home(in_alices, "kb_contexts", alices, alice).await;
    let in_teams = titled(&pool, &format!("team's {SSN}")).await;
    home(in_teams, "kb_contexts", teams, bob).await;
    let in_map = titled(&pool, &format!("map's {SSN}")).await;
    home(in_map, "kb_cogmaps", map, bob).await;
    let homeless = titled(&pool, &format!("nobody's {SSN}")).await;

    let found = |bound: &'static str, id: Option<Uuid>| {
        let pool = pool.clone();
        async move {
            let rows = dry_run(
                &pool,
                "us_ssn_delimited",
                1,
                bound,
                id,
                Some("kb_resources.title"),
                5000,
            )
            .await
            .unwrap();
            assert_eq!(rows.len(), 1, "one surface asked for, one reported");
            hot_targets(&rows[0])
        }
    };
    assert_eq!(
        found("context", Some(alices)).await,
        BTreeSet::from([in_alices])
    );
    assert_eq!(found("team", Some(team)).await, BTreeSet::from([in_teams]));
    assert_eq!(found("cogmap", Some(map)).await, BTreeSet::from([in_map]));
    assert_eq!(
        found("profile", Some(bob)).await,
        BTreeSet::from([in_teams, in_map])
    );
    assert_eq!(
        found("latest", None).await,
        BTreeSet::from([in_alices, in_teams, in_map, homeless])
    );

    for (bound, id, want) in [
        ("context", Some(Uuid::now_v7()), "P0002"),
        ("context", None, "P0002"),
        ("everyone", None, "22023"),
    ] {
        let e = dry_run(&pool, "us_ssn_delimited", 1, bound, id, None, 10)
            .await
            .unwrap_err();
        assert_eq!(code(&e), want, "{bound}: {e}");
    }
}

/// A candidate pattern is tried by inserting it and rolling back: the insert is off by default, the
/// dry run writes nothing, and the rollback leaves no trace of the trial.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_candidate_pattern_is_tried_inside_a_rolled_back_transaction(pool: PgPool) {
    let badge = titled(&pool, "Visitor ACME-123456 signed in").await;
    let before = sweep_state(&pool).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO sensitivity.detectors (id, category, severity, prefilter, pattern, note) \
         VALUES ('acme_badge', 'identifier', 2, 'ACME-', 'ACME-[0-9]{6}', 'candidate')",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    let row = sqlx::query(
        "SELECT places_matched, hotspots FROM sensitivity.dry_run('acme_badge', 1, 'latest', \
                p_surface => 'kb_resources.title')",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(row.get::<i32, _>(0), 1);
    let hot: serde_json::Value = row.get(1);
    assert_eq!(hot[0]["target_id"].as_str().unwrap(), badge.to_string());
    tx.rollback().await.unwrap();

    assert_eq!(sweep_state(&pool).await, before, "the trial left a trace");
}

/// A dry run reads at most its cap per surface, and says when the bound was not read to its end.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dry_run_stops_at_its_cap_and_says_so(pool: PgPool) {
    for n in 0..3 {
        titled(&pool, &format!("record {n}: {SSN}")).await;
    }
    let titles = resources(&pool).await;
    let capped = dry_run(
        &pool,
        "us_ssn_delimited",
        1,
        "latest",
        None,
        Some("kb_resources.title"),
        2,
    )
    .await
    .unwrap();
    assert_eq!((capped[0].2, capped[0].6), (2, false));
    let whole = dry_run(
        &pool,
        "us_ssn_delimited",
        1,
        "latest",
        None,
        Some("kb_resources.title"),
        titles,
    )
    .await
    .unwrap();
    assert_eq!((whole[0].2, whole[0].6), (titles, true));
}
