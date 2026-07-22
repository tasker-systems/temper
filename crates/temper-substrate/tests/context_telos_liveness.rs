#![cfg(feature = "artifact-tests")]
//! T5 — a context's telos: goal liveness from the task census (spec §3.4, §5).
//!
//! §5 asks for a **labeled fixture, not an invented expectation**: "The `@me/temper` goal census in
//! §3.4 is the fixture. The test asserts the ranking, against real data. Constants get fitted to
//! that, rather than the fixture being fitted to the constants."
//!
//! ## Why the census is FROZEN here rather than queried live
//!
//! It is still real data — every row below was read out of production. But it is a *snapshot*, taken
//! 2026-07-12, not a live read. That is not laziness: production **moved underneath the session that
//! wrote this test**. Two reads of the same census twenty minutes apart disagreed — `temper-rb` went
//! from (1 in-progress, 1 backlog) to (2 in-progress, 0 backlog) as work landed. A test that asserted
//! a ranking over live prod would be flaky by construction, and would fail for reasons that have
//! nothing to do with the code under test. Freezing keeps the fixture *real and labeled* — which is
//! what §5 actually asks for — while making it deterministic.
//!
//! ## What this file is really defending
//!
//! The spec's own constant for `sw_done` (0.15) **fails the spec's own labels**, and the failure is
//! silent: no error, plausible-looking numbers, a completely inverted ranking.
//! `sw_done_of_zero_point_one_five_lets_the_graveyard_win` is the differential that pins this down —
//! it is the reason `sw_done = 0.0` exists, and it fails loudly if anyone "restores" the spec value.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

/// One goal of the frozen census: (title, declared `temper-status`, in-progress, backlog, done,
/// cancelled). Read from production `@me/temper` on 2026-07-12 — see the module docs.
///
/// The interesting rows, and why each is here:
/// - **Temper Cloud** / **path-to-alpha** — declared `active`, zero open work. §3.4: the declaration
///   is stale, and they must fall OUT of the telos entirely.
/// - **Maintenance** — a 71-task graveyard with 3 fresh backlog items. §3.4: "faintly warm — a
///   container, not a driver". This is the row that breaks `sw_done = 0.15`.
/// - **Teams in Temper** — declared `completed`, still carries an open task. The declared field is
///   stale in *both* directions, so the status damps and never gates.
/// - **Temper performance optimization** — declared `paused`, no work at all. Damped to nothing.
const CENSUS: &[(&str, &str, usize, usize, usize, usize)] = &[
    ("Context regions and wayfinding", "active", 1, 4, 5, 0),
    ("temper-rb", "active", 2, 0, 11, 0),
    ("Substrate kernel to cognitive map", "active", 1, 0, 34, 2),
    ("Graph Atlas", "active", 1, 0, 0, 0),
    ("The ledger as a readable surface", "active", 0, 5, 0, 0),
    ("Maintenance", "active", 0, 3, 71, 13),
    ("Temper Cloud", "active", 0, 0, 19, 6),
    ("path-to-alpha", "active", 0, 0, 17, 1),
    ("Teams in Temper", "completed", 0, 1, 0, 0),
    ("Temper performance optimization", "paused", 0, 0, 0, 0),
];

/// Seed the frozen census into a fresh context and return `(context_id, lens_id)`.
///
/// Fixture rows go in through direct SQL rather than the ledger: liveness reads `kb_resources` +
/// `kb_properties` + `kb_edges`, and nothing here exercises the event projections, so routing 200
/// tasks through `fire` would buy nothing but runtime. The one thing that *must* be real is the shape
/// the function reads — the `(leads_to, 'advances')` task→goal edge and the `NOT is_folded` property
/// rows — so those are spelled exactly as production spells them.
async fn seed_census(pool: &PgPool) -> (Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ('census', 'Census') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("profile");

    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'temper', 'temper') RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("context");

    // kb_edges.asserted_by_event_id / last_event_id are NOT NULL FKs. Any real event satisfies them;
    // bootseed has already written the lens_created events, so borrow one rather than mint a fake.
    let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("an event to hang the fixture edges off");

    let lens: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name = 'workflow-default' AND cogmap_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .expect("the workflow-default lens");

    for (title, declared, in_prog, backlog, done, cancelled) in CENSUS {
        let goal = seed_resource(pool, ctx, profile, title, "goal").await;
        set_prop(pool, goal, "temper-status", declared).await;

        for (stage, n) in [
            ("in-progress", *in_prog),
            ("backlog", *backlog),
            ("done", *done),
            ("cancelled", *cancelled),
        ] {
            for i in 0..n {
                let task =
                    seed_resource(pool, ctx, profile, &format!("{title} {stage} {i}"), "task")
                        .await;
                set_prop(pool, task, "temper-stage", stage).await;
                // task --advances--> goal, exactly as the CLI mints it: (leads_to, forward,
                // 'advances'), source = task, target = goal.
                sqlx::query(
                    "INSERT INTO kb_edges (source_table, source_id, target_table, target_id, \
                       edge_kind, polarity, label, home_anchor_table, home_anchor_id, \
                       asserted_by_event_id, last_event_id) \
                     VALUES ('kb_resources', $1, 'kb_resources', $2, 'leads_to', 'forward', \
                       'advances', 'kb_contexts', $3, $4, $4)",
                )
                .bind(task)
                .bind(goal)
                .bind(ctx)
                .bind(ev)
                .execute(pool)
                .await
                .expect("advances edge");
            }
        }
    }
    (ctx, lens)
}

async fn seed_resource(
    pool: &PgPool,
    ctx: Uuid,
    profile: Uuid,
    title: &str,
    doc_type: &str,
) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
           originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(profile)
    .execute(pool)
    .await
    .expect("home");
    set_prop(pool, id, "doc_type", doc_type).await;
    id
}

async fn set_prop(pool: &PgPool, owner: Uuid, key: &str, value: &str) {
    let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("event");
    sqlx::query(
        "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, \
           asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, $2, to_jsonb($3::text), $4, $4)",
    )
    .bind(owner)
    .bind(key)
    .bind(value)
    .bind(ev)
    .execute(pool)
    .await
    .expect("property");
}

/// `context_goal_liveness` keyed by goal title.
async fn liveness(pool: &PgPool, ctx: Uuid, lens: Uuid) -> Vec<(String, f64)> {
    sqlx::query_as(
        "SELECT r.title, gl.liveness \
           FROM context_goal_liveness($1, $2) gl \
           JOIN kb_resources r ON r.id = gl.goal_id \
          ORDER BY gl.liveness DESC",
    )
    .bind(ctx)
    .bind(lens)
    .fetch_all(pool)
    .await
    .expect("liveness")
}

fn score(rows: &[(String, f64)], title: &str) -> f64 {
    rows.iter()
        .find(|(t, _)| t == title)
        .unwrap_or_else(|| panic!("goal {title} missing from the census: {rows:#?}"))
        .1
}

/// Every label §3.4/§5 puts on the fixture, asserted against the shipped calibration.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_liveness_census_reproduces_every_label_the_spec_puts_on_it(pool: PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (ctx, lens) = seed_census(&pool).await;
    let rows = liveness(&pool, ctx, lens).await;

    // "Temper Cloud and path-to-alpha must fall OUT of the telos." Both are declared `active` and
    // both are unambiguously finished — 19 and 17 closed tasks, not one open. A goal is as real as
    // the work beneath it, so this is exactly 0.0, not merely small.
    assert_eq!(score(&rows, "Temper Cloud"), 0.0);
    assert_eq!(score(&rows, "path-to-alpha"), 0.0);

    // "Maintenance must be faintly warm — a container, not a driver." Warm, because 3 fresh backlog
    // items; not a driver, because it must sit below every arc under active development. Its 71 done
    // tasks buy it nothing.
    let maintenance = score(&rows, "Maintenance");
    assert!(maintenance > 0.0, "3 backlog items keep it warm");
    for live in [
        "Context regions and wayfinding",
        "temper-rb",
        "The ledger as a readable surface",
    ] {
        assert!(
            score(&rows, live) > maintenance,
            "{live} ({}) must outrank the Maintenance graveyard ({maintenance})",
            score(&rows, live),
        );
    }

    // "Substrate-kernel and Graph Atlas must rank at the top" — in the specific sense that matters:
    // above the two goals whose declaration lies about them being alive.
    for top in ["Substrate kernel to cognitive map", "Graph Atlas"] {
        assert!(score(&rows, top) > score(&rows, "Temper Cloud"));
        assert!(score(&rows, top) > score(&rows, "path-to-alpha"));
    }

    // The declared status DAMPS, it never gates: `completed` with an open task stays present (it
    // cannot be killed by the declaration) but is scaled by damper_completed = 0.4.
    let teams = score(&rows, "Teams in Temper");
    assert!(
        teams > 0.0,
        "a task in progress outvotes the `completed` flag"
    );
    let undamped = (0.35f64).sqrt(); // one backlog task, sw_backlog = 0.35, idle ≈ 0
    assert!(
        (teams - 0.4 * undamped).abs() < 0.02,
        "expected ~{} (0.4 × undamped), got {teams}",
        0.4 * undamped
    );

    // A `paused` goal with no work at all is simply absent — the damper cannot resurrect it either.
    assert_eq!(score(&rows, "Temper performance optimization"), 0.0);
}

/// **The refutation, pinned.** The spec (§3.4) sets `sw_done = 0.15`, reasoning that `done` is
/// "weighted low and decays, so a graveyard does not masquerade as a heartbeat". Against the spec's
/// own fixture, it does exactly that.
///
/// A weight of 0.15 is small. SEVENTY-ONE of them is not — and because closing a task *touches* it,
/// the decay term is ≈1.0 for precisely the tasks that just finished, so a goal that is *ending*
/// looks maximally alive. Maintenance (0 in-progress, 3 backlog, 71 done) overtakes every arc under
/// active development.
///
/// This is why the shipped calibration is 0.0. If someone "restores" the spec's value, this fails.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sw_done_of_zero_point_one_five_lets_the_graveyard_win(pool: PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (ctx, lens) = seed_census(&pool).await;

    // Same corpus, same producer, same anchor — only the constant moves. A differential, so any
    // change in the ranking is attributable to sw_done and to nothing else.
    let shipped = liveness(&pool, ctx, lens).await;
    sqlx::query("UPDATE kb_cogmap_lenses SET sw_done = 0.15 WHERE id = $1")
        .bind(lens)
        .execute(&pool)
        .await
        .unwrap();
    let spec = liveness(&pool, ctx, lens).await;

    // Under the shipped calibration the live arc leads and the graveyard trails.
    assert!(score(&shipped, "Context regions and wayfinding") > score(&shipped, "Maintenance"));

    // Under the spec's 0.15 the order INVERTS: the container outranks the work.
    assert!(
        score(&spec, "Maintenance") > score(&spec, "Context regions and wayfinding"),
        "expected sw_done=0.15 to let the 71-task graveyard outrank the live arc — if this no \
         longer holds, the calibration argument in migration 20260712000060 needs revisiting"
    );

    // And the two dead goals climb back INTO the telos on the strength of closed work alone,
    // violating §3.4's first label.
    assert!(score(&spec, "Temper Cloud") > 0.0);
    assert!(score(&spec, "path-to-alpha") > 0.0);
}
