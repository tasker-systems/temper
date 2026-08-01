//! Materialize a cogmap under a candidate formation regime and price what it cost.
//!
//! **This is the only harness in the tree that WRITES.** `region_formation_sweep` answers what
//! regions a lens would *form*; it cannot answer whether they are *reached*, because `salience` and
//! `centroid` are computed by the producer during materialize and a read-only load never recomputes
//! them. That gap is the `examined-and-inexpressible` cell of goal
//! `019fbb77-72a3-72e1-bbbd-13eb6aa64982`, and closing it needs a write. This runs it.
//!
//! # It must never run against production
//!
//! `materialize` folds every live region for the lens, mints a new partition, and fires an
//! append-only `region_materialized` event. `kb_events` does not support deletion — repairs
//! quarantine — so on prod there is no undo, and re-running the incumbent lens afterwards does not
//! restore the prior state: it mints a THIRD generation of ids and leaves both folded generations
//! behind. Region ids turn over, so anything holding a reference to one is dangling.
//!
//! The guard is therefore an **allowlist, not a denylist**: `--expect-host` is required and must
//! match the host in `DATABASE_URL` exactly, so a wrong or stale connection string fails closed
//! rather than open. `main`'s endpoint is refused unconditionally on top of that, because the one
//! database that must never be written is worth naming even when the allowlist already covers it.
//!
//! ```bash
//! DATABASE_URL="$(…branch connection string…)" \
//!   cargo run --release -p temper-substrate --example region_materialize_arms -- \
//!   --expect-host ep-quiet-tooth-amd3dhgy.c-5.us-east-1.aws.neon.tech --arm none
//! ```
//!
//! # Arms
//!
//! Every arm rewrites the GLOBAL `telos-default` row (`home_anchor_table IS NULL` — the row
//! `substrate::load_lens` resolves for a cogmap) and then materializes under the name
//! `telos-default`. Keeping ONE lens id across all arms is load-bearing: `write.rs`'s
//! `fold_live_regions(&mut tx, anchor, s.lens_id, …)` folds only that lens's regions, while
//! `wayfind_region_scores`' candidate CTE selects `FROM kb_cogmap_regions … WHERE NOT r.is_folded`
//! with **no lens predicate at all**. Materializing an arm under a second lens name would leave both
//! partitions live in one candidate pool and one per-kind `percent_rank`, and every reachability
//! number after it would be measuring the union of two regimes.
//!
//! Arms are applied ABSOLUTELY, never as deltas onto whatever the previous arm left behind: the
//! first run snapshots the original formation fields into `_lens_arm_baseline`, and every later arm
//! restores from that snapshot before applying itself. So arms may be run in any order, repeatedly.
//!
//! `workflow-default` copies only the **formation** fields (the affinity kernel's weights, `knn_k`,
//! `cos_floor`, `resolution`). It deliberately leaves `s_telos`/`s_ref`/`s_central` alone, so all
//! arms share one salience blend and the comparison isolates formation — those three are what
//! `wayfind_region_scores` reads for `sal_eff`, and moving them would confound a formation effect
//! with a ranking one.
//!
//! # Why this file authors runtime queries rather than `sqlx::query!`
//!
//! The compile-time macros verify against the schema, which is the repo rule — but the `.sqlx`
//! cache carries no entries for **example** targets, so a `query!` here would fail every offline
//! build until the workspace cache was regenerated, and `cargo make check` gates that artifact.
//! A throwaway measurement harness is not worth restaling a checked artifact for. The library code
//! it drives (`materialize`, `substrate::load`) is entirely macro-verified; only the handful of
//! reporting reads below are runtime.

use anyhow::{bail, Context, Result};
use sqlx::{PgPool, Row};
use temper_core::types::home::HomeAnchor;
use temper_substrate::substrate;
use temper_substrate::write::materialize;

/// `main`'s endpoint on the `crimson-fog-23541670` project. Refused unconditionally.
const PROD_MAIN_HOST_PREFIX: &str = "ep-mute-art-am7rz4e1";

/// The formation fields an arm may rewrite. `s_telos`/`s_ref`/`s_central` are deliberately absent —
/// see the module header.
const FORMATION_FIELDS: [&str; 9] = [
    "w_express",
    "w_contains",
    "w_leads_to",
    "w_near",
    "w_prop",
    "w_cos",
    "knn_k",
    "cos_floor",
    "resolution",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    /// No lens change — materialize under the incumbent regime. Validates the environment.
    None,
    /// Declared-only granularity: `resolution` 0.5 → 0.2, `w_cos` still 0.
    Resolution02,
    /// Evidence kind: `w_cos` 0 → 1.0, `resolution` still 0.5.
    WCos10,
    /// The whole context formation regime, copied from `workflow-default`.
    WorkflowDefault,
}

impl Arm {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "none" => Arm::None,
            "resolution-0.2" => Arm::Resolution02,
            "w-cos-1.0" => Arm::WCos10,
            "workflow-default" => Arm::WorkflowDefault,
            other => bail!(
                "unknown --arm {other:?} (none | resolution-0.2 | w-cos-1.0 | workflow-default)"
            ),
        })
    }

    fn label(self) -> &'static str {
        match self {
            Arm::None => "none (incumbent telos-default)",
            Arm::Resolution02 => "resolution 0.2 (declared-only)",
            Arm::WCos10 => "telos-default + w_cos 1.0",
            Arm::WorkflowDefault => "workflow-default formation fields",
        }
    }
}

/// Parse the host out of a Postgres URL without ever surfacing the credential.
fn host_of(url: &str) -> Result<String> {
    let after_at = url.rsplit_once('@').context("no '@' in DATABASE_URL")?.1;
    let host = after_at
        .split(['/', '?'])
        .next()
        .context("no host in DATABASE_URL")?;
    Ok(host.to_owned())
}

/// The allowlist. Returns the connection string only once it has proved which database it names.
fn guarded_url(expect_host: &str) -> Result<String> {
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let host = host_of(&url)?;
    if host.starts_with(PROD_MAIN_HOST_PREFIX) {
        bail!("REFUSING: {host} is production main. This harness writes and prod has no undo.");
    }
    if host != expect_host {
        bail!(
            "REFUSING: DATABASE_URL names {host}, but --expect-host is {expect_host}. \
             Failing closed rather than writing to a database you did not name."
        );
    }
    Ok(url)
}

/// Snapshot the incumbent formation fields once, so every arm can be applied absolutely.
async fn ensure_baseline_snapshot(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _lens_arm_baseline AS \
         SELECT id, w_express, w_contains, w_leads_to, w_near, w_prop, w_cos, knn_k, cos_floor, \
                resolution \
         FROM kb_cogmap_lenses WHERE name='telos-default' AND home_anchor_table IS NULL",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Restore `telos-default` to the snapshot, then apply the arm. Absolute, never cumulative.
async fn apply_arm(pool: &PgPool, arm: Arm) -> Result<()> {
    let assignments = FORMATION_FIELDS
        .iter()
        .map(|f| format!("{f} = b.{f}"))
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!(
        "UPDATE kb_cogmap_lenses l SET {assignments} \
         FROM _lens_arm_baseline b WHERE l.id = b.id"
    ))
    .execute(pool)
    .await?;

    match arm {
        Arm::None => {}
        Arm::Resolution02 => {
            sqlx::query(
                "UPDATE kb_cogmap_lenses SET resolution = 0.2 \
                 WHERE name='telos-default' AND home_anchor_table IS NULL",
            )
            .execute(pool)
            .await?;
        }
        Arm::WCos10 => {
            sqlx::query(
                "UPDATE kb_cogmap_lenses SET w_cos = 1.0 \
                 WHERE name='telos-default' AND home_anchor_table IS NULL",
            )
            .execute(pool)
            .await?;
        }
        Arm::WorkflowDefault => {
            let assignments = FORMATION_FIELDS
                .iter()
                .map(|f| format!("{f} = src.{f}"))
                .collect::<Vec<_>>()
                .join(", ");
            sqlx::query(&format!(
                "UPDATE kb_cogmap_lenses l SET {assignments} \
                 FROM kb_cogmap_lenses src \
                 WHERE src.name='workflow-default' AND src.home_anchor_table IS NULL \
                   AND l.name='telos-default' AND l.home_anchor_table IS NULL"
            ))
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn print_lens(pool: &PgPool, when: &str) -> Result<()> {
    let r = sqlx::query(
        "SELECT w_express, w_contains, w_leads_to, w_near, w_prop, w_cos, knn_k, cos_floor, \
                resolution \
         FROM kb_cogmap_lenses WHERE name='telos-default' AND home_anchor_table IS NULL",
    )
    .fetch_one(pool)
    .await?;
    let f = |c: &str| -> f64 { r.get(c) };
    println!(
        "  lens {when:<7} express {} contains {} leads_to {} near {} prop {} cos {} \
         knn_k {} cos_floor {} resolution {}",
        f("w_express"),
        f("w_contains"),
        f("w_leads_to"),
        f("w_near"),
        f("w_prop"),
        f("w_cos"),
        r.get::<i32, _>("knn_k"),
        f("cos_floor"),
        f("resolution")
    );
    Ok(())
}

async fn count_events(pool: &PgPool) -> Result<i64> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_events")
            .fetch_one(pool)
            .await?,
    )
}

/// Region shape and event volume, read back after the act — the cost half of the measurement.
async fn report_cost(pool: &PgPool, anchor: HomeAnchor, events_before: i64) -> Result<()> {
    let shape = sqlx::query(
        "SELECT count(*) AS live, \
                count(*) FILTER (WHERE member_count <= 1) AS singletons, \
                coalesce(max(member_count), 0) AS largest, \
                coalesce(avg(member_count), 0)::float8 AS mean \
           FROM kb_cogmap_regions \
          WHERE home_anchor_table=$1 AND home_anchor_id=$2 AND NOT is_folded",
    )
    .bind(anchor.table())
    .bind(anchor.uuid())
    .fetch_one(pool)
    .await?;
    let events_after = count_events(pool).await?;
    println!(
        "  shape          live {} | singletons {} | largest {} | mean {:.2}",
        shape.get::<i64, _>("live"),
        shape.get::<i64, _>("singletons"),
        shape.get::<i32, _>("largest"),
        shape.get::<f64, _>("mean")
    );
    println!("  events written {}", events_after - events_before);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    let expect_host = flag("--expect-host").context(
        "--expect-host is REQUIRED: this harness writes, and names its target explicitly",
    )?;
    let arm = Arm::parse(&flag("--arm").unwrap_or_else(|| "none".to_owned()))?;
    let map_name = flag("--cogmap").unwrap_or_else(|| "Temper — self-cognition".to_owned());

    let url = guarded_url(&expect_host)?;
    println!("target host   {expect_host}  (allowlisted, not prod main)");
    println!("arm           {}", arm.label());
    println!("cogmap        {map_name:?}");

    let pool = PgPool::connect(&url).await?;
    let cogmap_id = substrate::cogmap_by_name(&pool, &map_name).await?;
    let anchor = HomeAnchor::Cogmap(cogmap_id);
    let emitter = substrate::cogmap_genesis_emitter(&pool, cogmap_id).await?;

    ensure_baseline_snapshot(&pool).await?;
    print_lens(&pool, "before").await?;
    apply_arm(&pool, arm).await?;
    print_lens(&pool, "after").await?;

    let events_before = count_events(&pool).await?;

    let started = std::time::Instant::now();
    let outcome = materialize(&pool, anchor, "telos-default", emitter).await?;
    let elapsed = started.elapsed();

    println!("  materialize    {:.1}s", elapsed.as_secs_f64());
    println!(
        "  regions        {} (reused {} | minted {})  \u{2190} id turnover",
        outcome.regions, outcome.regions_reused, outcome.regions_minted
    );
    // The fingerprint is the whole partition's member ids concatenated — tens of kilobytes. Print a
    // prefix and its length: enough to compare two arms, short enough to read.
    let fp = &outcome.membership_fingerprint;
    println!(
        "  fingerprint    {}… ({} bytes)",
        &fp[..fp.len().min(32)],
        fp.len()
    );
    report_cost(&pool, anchor, events_before).await?;
    Ok(())
}
