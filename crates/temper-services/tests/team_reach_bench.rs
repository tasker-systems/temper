#![cfg(feature = "test-db")]

//! Measurement, not a test: the cost of team reach before and after migration `20261129000010`
//! (team reach in one walk), on one corpus and one build.
//!
//! Ignored by default. Run it on a release build:
//!
//! ```text
//! cargo nextest run --release -p temper-services --features test-db \
//!     --test team_reach_bench --run-ignored only --no-capture
//! ```
//!
//! "Before" installs the pre-migration bodies of `team_ancestors` (`20260703000001`) and
//! `profile_reachable_teams` (`20260804000010`) verbatim in the bench's own database; "after" is
//! the migrated schema. Both run against the same rows, so the difference is the walk alone.
//!
//! The headline case is the locked single-resource gate a conditional write runs:
//! `lock_resource_for_write` (`FOR NO KEY UPDATE`) and then `readback::is_resource_visible` on the
//! same transaction. Only the visibility call is timed. 60 runs per case, 5 discarded first; p50
//! and p95 in milliseconds.

use sqlx::PgPool;
use std::time::Instant;
use temper_core::types::ids::{ProfileId, ResourceId};
use uuid::Uuid;

const RUNS: usize = 60;
const WARMUP: usize = 5;

const BEFORE: &str = r#"
CREATE OR REPLACE FUNCTION team_ancestors(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE up AS (
        SELECT t.id AS team_id
        FROM kb_teams t
        WHERE t.id = p_team AND t.is_active
        UNION
        SELECT tp.parent_id
        FROM kb_teams_parents tp
        JOIN up ON tp.child_id = up.team_id
        JOIN kb_teams pt ON pt.id = tp.parent_id AND pt.is_active
    )
    SELECT team_id FROM up;
$$;
CREATE OR REPLACE FUNCTION profile_reachable_teams(p_profile uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT a.team_id
    FROM profile_effective_teams(p_profile) e
    CROSS JOIN LATERAL team_ancestors(e.team_id) a;
$$;
"#;

/// One corpus: an org, ten divisions, `teams` teams under them, `contexts` contexts per team and
/// `per_context` resources per context. Readers are members of the first 1, 50 and 300 teams.
/// Returns (readers by team count, a resource every reader sees, a resource only the 300 reader
/// sees).
async fn corpus(
    pool: &PgPool,
    prefix: &str,
    teams: i32,
    contexts: i32,
    per_context: i32,
) -> (Vec<(i32, Uuid)>, Uuid, Uuid) {
    let sql = format!(
        r#"
        INSERT INTO kb_profiles (handle, display_name)
            SELECT '{p}-' || h, '{p}-' || h FROM unnest(ARRAY['owner','r1','r50','r300']) h;
        INSERT INTO kb_teams (slug, name) VALUES ('{p}-org', '{p}-org');
        INSERT INTO kb_teams (slug, name) SELECT '{p}-div' || i, '{p}-div' || i FROM generate_series(1, 10) i;
        INSERT INTO kb_teams_parents (child_id, parent_id)
            SELECT d.id, o.id FROM kb_teams d, kb_teams o
             WHERE d.slug LIKE '{p}-div%' AND o.slug = '{p}-org';
        INSERT INTO kb_teams (slug, name)
            SELECT '{p}-team' || i, '{p}-team' || i FROM generate_series(1, {teams}) i;
        INSERT INTO kb_teams_parents (child_id, parent_id)
            SELECT t.id, d.id FROM generate_series(1, {teams}) i
              JOIN kb_teams t ON t.slug = '{p}-team' || i
              JOIN kb_teams d ON d.slug = '{p}-div' || (1 + i % 10);
        INSERT INTO kb_team_members (team_id, profile_id, role)
            SELECT t.id, p.id, 'member'
              FROM generate_series(1, {teams}) i
              JOIN kb_teams t ON t.slug = '{p}-team' || i
              JOIN kb_profiles p ON p.handle IN ('{p}-r1', '{p}-r50', '{p}-r300')
             WHERE (p.handle = '{p}-r1' AND i <= 1) OR (p.handle = '{p}-r50' AND i <= 50)
                OR p.handle = '{p}-r300';
        INSERT INTO kb_contexts (owner_table, owner_id, slug, name)
            SELECT 'kb_teams', t.id, t.slug || '-c' || j, t.slug || '-c' || j
              FROM kb_teams t, generate_series(1, {contexts}) j
             WHERE t.slug LIKE '{p}-team%';
        WITH r AS (
            INSERT INTO kb_resources (title, origin_uri)
            SELECT c.slug || '-r' || k, c.id::text
              FROM kb_contexts c, generate_series(1, {per_context}) k
             WHERE c.slug LIKE '{p}-team%'
            RETURNING id, origin_uri
        )
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            SELECT r.id, 'kb_contexts', r.origin_uri::uuid, o.id, o.id
              FROM r, kb_profiles o WHERE o.handle = '{p}-owner';
        "#,
        p = prefix,
    );
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .expect("seed corpus");

    let mut readers = Vec::new();
    for n in [1, 50, 300] {
        let id: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle = $1")
            .bind(format!("{prefix}-r{n}"))
            .fetch_one(pool)
            .await
            .expect("reader");
        readers.push((n, id));
    }
    let first = resource_titled(pool, &format!("{prefix}-team1-c1-r1")).await;
    let last = resource_titled(pool, &format!("{prefix}-team{teams}-c1-r1")).await;
    (readers, first, last)
}

async fn resource_titled(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_resources WHERE title = $1")
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("resource")
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx]
}

/// p50/p95 (ms) of the locked gate, and the verdict.
async fn locked_gate(pool: &PgPool, reader: Uuid, resource: Uuid) -> (f64, f64, bool) {
    let mut samples = Vec::with_capacity(RUNS);
    let mut verdict = false;
    for i in 0..RUNS + WARMUP {
        let mut tx = pool.begin().await.expect("begin");
        temper_substrate::writes::lock_resource_for_write(&mut tx, ResourceId::from(resource))
            .await
            .expect("lock");
        let t0 = Instant::now();
        verdict = temper_substrate::readback::is_resource_visible(
            &mut *tx,
            ProfileId::from(reader),
            ResourceId::from(resource),
        )
        .await
        .expect("gate");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        tx.rollback().await.expect("rollback");
        if i >= WARMUP {
            samples.push(ms);
        }
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    (
        percentile(&samples, 0.5),
        percentile(&samples, 0.95),
        verdict,
    )
}

/// p50/p95 (ms) of one scalar SQL statement taking two uuids.
async fn statement(pool: &PgPool, sql: &str, a: Uuid, b: Uuid) -> (f64, f64) {
    let mut conn = pool.acquire().await.expect("conn");
    let mut samples = Vec::with_capacity(RUNS);
    for i in 0..RUNS + WARMUP {
        let t0 = Instant::now();
        sqlx::query(sql)
            .bind(a)
            .bind(b)
            .execute(&mut *conn)
            .await
            .expect("statement");
        if i >= WARMUP {
            samples.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    (percentile(&samples, 0.5), percentile(&samples, 0.95))
}

struct Case {
    name: String,
    reader: Uuid,
    resource: Uuid,
}

#[derive(Default)]
struct Collateral {
    sql: Vec<(String, String, Uuid, Uuid)>,
}

async fn run_cases(pool: &PgPool, cases: &[Case], collateral: &Collateral) -> Vec<String> {
    let mut rows = Vec::new();
    for c in cases {
        let (p50, p95, v) = locked_gate(pool, c.reader, c.resource).await;
        rows.push(format!("{:<44} {p50:>8.3} {p95:>8.3}  {v}", c.name));
    }
    for (name, sql, a, b) in &collateral.sql {
        let (p50, p95) = statement(pool, sql, *a, *b).await;
        rows.push(format!("{name:<44} {p50:>8.3} {p95:>8.3}"));
    }
    rows
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
#[ignore = "measurement; run on a release build with --run-ignored only --no-capture"]
async fn team_reach_before_and_after(pool: PgPool) {
    // Base corpus: 300 teams x 3 contexts x 20 resources.
    let (base, b_first, b_last) = corpus(&pool, "b", 300, 3, 20).await;
    // Resource-heavy corpus: 300 teams x 3 contexts x 500 resources.
    let (heavy, h_first, h_last) = corpus(&pool, "h", 300, 3, 500).await;

    // Grant corpus: a resource homed in a context no reader reaches by membership, granted to the
    // base org, which every base reader reaches through its division.
    let outsider_team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('g-private', 'g-private') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let granted: Uuid = sqlx::query_scalar(
        "WITH c AS (INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
                    VALUES ('kb_teams', $1, 'g-ctx', 'g-ctx') RETURNING id), \
              r AS (INSERT INTO kb_resources (title, origin_uri) VALUES ('g-r', '') RETURNING id), \
              h AS (INSERT INTO kb_resource_homes \
                      (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
                    SELECT r.id, 'kb_contexts', c.id, o.id, o.id FROM r, c, kb_profiles o \
                     WHERE o.handle = 'b-owner' RETURNING resource_id) \
         SELECT resource_id FROM h",
    )
    .bind(outsider_team)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         SELECT 'kb_resources', $1, 'kb_teams', t.id, true, o.id \
           FROM kb_teams t, kb_profiles o WHERE t.slug = 'b-org' AND o.handle = 'b-owner'",
    )
    .bind(granted)
    .execute(&pool)
    .await
    .unwrap();

    // public_within corpus: team b-pub under b-div2 opened to b-div2's subtree; the 1-team reader
    // (b-team1 sits under b-div2) and the 300-team reader become persons.
    let pub_resource: Uuid = sqlx::query_scalar(
        "WITH t AS (INSERT INTO kb_teams (slug, name) VALUES ('b-pub', 'b-pub') RETURNING id), \
              l AS (INSERT INTO kb_teams_parents (child_id, parent_id) \
                    SELECT t.id, d.id FROM t, kb_teams d WHERE d.slug = 'b-div2' RETURNING child_id), \
              c AS (INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
                    SELECT 'kb_teams', l.child_id, 'b-pub-ctx', 'b-pub-ctx' FROM l RETURNING id), \
              r AS (INSERT INTO kb_resources (title, origin_uri) VALUES ('b-pub-r', '') RETURNING id), \
              h AS (INSERT INTO kb_resource_homes \
                      (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
                    SELECT r.id, 'kb_contexts', c.id, o.id, o.id FROM r, c, kb_profiles o \
                     WHERE o.handle = 'b-owner' RETURNING resource_id) \
         SELECT resource_id FROM h",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by) \
         SELECT t.id, d.id, o.id FROM kb_teams t, kb_teams d, kb_profiles o \
          WHERE t.slug = 'b-pub' AND d.slug = 'b-div2' AND o.handle = 'b-owner'",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (_, reader) in &base {
        sqlx::query(
            "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
             VALUES (gen_random_uuid(), $1, 'auth0', $1::text)",
        )
        .bind(reader)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::raw_sql("ANALYZE").execute(&pool).await.unwrap();

    let mut cases = Vec::new();
    for (n, r) in &base {
        cases.push(Case {
            name: format!("base {n:>3} teams, visible R"),
            reader: *r,
            resource: b_first,
        });
        cases.push(Case {
            name: format!("base {n:>3} teams, last team's R"),
            reader: *r,
            resource: b_last,
        });
        cases.push(Case {
            name: format!("grant {n:>3} teams (org grant)"),
            reader: *r,
            resource: granted,
        });
        cases.push(Case {
            name: format!("public_within {n:>3} teams"),
            reader: *r,
            resource: pub_resource,
        });
    }
    for (n, r) in &heavy {
        cases.push(Case {
            name: format!("heavy {n:>3} teams, visible R"),
            reader: *r,
            resource: h_first,
        });
        cases.push(Case {
            name: format!("heavy {n:>3} teams, last team's R"),
            reader: *r,
            resource: h_last,
        });
    }

    let r300 = base[2].1;
    let team1: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'b-team1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let pub_team: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'b-pub'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let div2: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'b-div2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let collateral = Collateral {
        sql: vec![
            (
                "team_ancestors(leaf team)".into(),
                "SELECT count(*) FROM team_ancestors($1) WHERE $2 IS NOT NULL".into(),
                team1,
                team1,
            ),
            (
                "vis_team(leaf team)".into(),
                "SELECT count(*) FROM vis_team($1) WHERE $2 IS NOT NULL".into(),
                team1,
                team1,
            ),
            (
                "public_within_row_effective(pub, div2)".into(),
                "SELECT public_within_row_effective($1, $2, false)".into(),
                pub_team,
                div2,
            ),
            (
                "list: count(resources_visible_to(r300))".into(),
                "SELECT count(*) FROM resources_visible_to($1) WHERE $2 IS NOT NULL".into(),
                r300,
                r300,
            ),
            (
                "profile_reachable_teams(r300)".into(),
                "SELECT count(*) FROM profile_reachable_teams($1) WHERE $2 IS NOT NULL".into(),
                r300,
                r300,
            ),
        ],
    };

    let after = run_cases(&pool, &cases, &collateral).await;
    sqlx::raw_sql(BEFORE)
        .execute(&pool)
        .await
        .expect("install the before bodies");
    let before = run_cases(&pool, &cases, &collateral).await;

    println!(
        "\n{:<44} {:>8} {:>8}  verdict",
        "BEFORE (pre-20261129000010)", "p50 ms", "p95 ms"
    );
    for row in &before {
        println!("{row}");
    }
    println!(
        "\n{:<44} {:>8} {:>8}  verdict",
        "AFTER (one walk)", "p50 ms", "p95 ms"
    );
    for row in &after {
        println!("{row}");
    }
}
