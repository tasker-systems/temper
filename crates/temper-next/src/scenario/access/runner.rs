//! Access-scenario runner: loads the world, then evaluates each `AccessCheck` against one kernel gate
//! function. Each check is a boolean (or count) compared to its declared expectation, with a failure
//! message naming the referents — a declarative echo of `schema-artifact/04_scenarios.sql`'s S1-S5.

use crate::scenario::access::loader::{self, LoadedAccess};
use crate::scenario::access::model::*;
use anyhow::{bail, Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn run_access_scenario(pool: &PgPool, doc: &AccessScenario) -> Result<()> {
    let loaded = loader::load(pool, &doc.world).await?;
    for (i, c) in doc.checks.iter().enumerate() {
        eval_access_check(pool, &loaded, c)
            .await
            .with_context(|| format!("check {i} failed"))?;
    }
    Ok(())
}

async fn eval_access_check(pool: &PgPool, loaded: &LoadedAccess, c: &AccessCheck) -> Result<()> {
    match c {
        AccessCheck::VisibleTo {
            profile,
            resource,
            expect,
        } => {
            let p = profile_id(loaded, profile)?;
            let r = resource_id(loaded, resource)?;
            let got = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2)",
                p,
                r,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);
            if got != *expect {
                bail!("visible_to: profile {profile} / resource {resource} = {got}, expected {expect}");
            }
        }
        AccessCheck::ProducerReach {
            cogmap,
            resource,
            expect,
        } => {
            let m = cogmap_id(loaded, cogmap)?;
            let r = resource_id(loaded, resource)?;
            let got = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM resources_accessible_to_cogmap($1) a WHERE a.resource_id=$2)",
                m,
                r,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);
            if got != *expect {
                bail!("producer_reach: cogmap {cogmap} / resource {resource} = {got}, expected {expect}");
            }
        }
        AccessCheck::EdgeVisibleTo {
            profile,
            edge,
            expect,
        } => {
            let p = profile_id(loaded, profile)?;
            let eid = sqlx::query_scalar!(
                "SELECT id FROM kb_edges WHERE label=$1 AND NOT is_folded",
                edge,
            )
            .fetch_optional(pool)
            .await?
            .with_context(|| format!("edge_visible_to: no edge labelled {edge:?}"))?;
            let got = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM edges_visible_to($1) e WHERE e.edge_id=$2)",
                p,
                eid,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);
            if got != *expect {
                bail!(
                    "edge_visible_to: profile {profile} / edge {edge} = {got}, expected {expect}"
                );
            }
        }
        AccessCheck::CogmapsShareTeam { a, b, expect } => {
            let ca = cogmap_id(loaded, a)?;
            let cb = cogmap_id(loaded, b)?;
            let got = sqlx::query_scalar!("SELECT cogmaps_share_a_team($1,$2)", ca, cb)
                .fetch_one(pool)
                .await?
                .unwrap_or(false);
            if got != *expect {
                bail!("cogmaps_share_team: {a} & {b} = {got}, expected {expect}");
            }
        }
        AccessCheck::CharterBlocksVisible {
            cogmap,
            profile,
            expect_count,
        } => {
            let m = cogmap_id(loaded, cogmap)?;
            let p = profile_id(loaded, profile)?;
            let n = sqlx::query_scalar!(
                "SELECT count(*) FROM resource_blocks(cogmap_telos($1), 'profile', $2, NULL)",
                m,
                p,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0);
            if n != *expect_count {
                bail!("charter_blocks_visible: cogmap {cogmap} / profile {profile} = {n} blocks, expected {expect_count}");
            }
        }
        AccessCheck::CanModify {
            profile,
            resource,
            expect,
        } => {
            let p = profile_id(loaded, profile)?;
            let r = resource_id(loaded, resource)?;
            let got = sqlx::query_scalar!("SELECT can_modify_resource($1, $2)", p, r)
                .fetch_one(pool)
                .await?
                .unwrap_or(false);
            if got != *expect {
                bail!("can_modify: profile {profile} / resource {resource} = {got}, expected {expect}");
            }
        }
    }
    Ok(())
}

fn profile_id(l: &LoadedAccess, h: &str) -> Result<Uuid> {
    l.profiles
        .get(h)
        .copied()
        .with_context(|| format!("unknown profile handle {h}"))
}
fn resource_id(l: &LoadedAccess, k: &str) -> Result<Uuid> {
    l.resources
        .get(k)
        .copied()
        .with_context(|| format!("unknown resource key {k}"))
}
fn cogmap_id(l: &LoadedAccess, n: &str) -> Result<Uuid> {
    l.cogmaps
        .get(n)
        .copied()
        .with_context(|| format!("unknown cogmap name {n}"))
}
