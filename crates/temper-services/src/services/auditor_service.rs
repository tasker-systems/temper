//! Read path + grain reconciliation for the citation auditor's dispatch tick (Set 5, Task 13).
//!
//! Spec `temper-artifacts:specs/2026-07-23-set5-adversary-citation-audit-design.md` §6.1-6.3.
//!
//! Service-direct (the read-path convention, mirroring [`crate::services::steward_service`]): the
//! surface passes a resolved principal + optional cap; [`drift_sweep`] gates through
//! `audit_drift_sweep`, which is itself principal-scoped in SQL. The write side (enqueue + claim)
//! routes through `DbBackend`, not here.
//!
//! [`group_by_cogmap`] is the one piece of real logic in this module, and it is deliberately a
//! **pure function** with no pool: it is the fix for the grain mismatch between a finding-grained
//! sweep and a cogmap-grained queue, so it must be provable without a database.

use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::auditor::{AuditCitation, AuditJobPayload, AuditSweepRow};
use temper_core::types::ids::ProfileId;
use temper_core::types::workflow_job::clamp_auditor_cap;

/// One cogmap's worth of audit work: the queue key, and the finding list that rides its payload.
///
/// A named struct rather than a `(Uuid, AuditJobPayload)` tuple because both halves are what the
/// enqueue call takes positionally, and a tuple would let a future edit transpose them into a
/// perfectly-typechecking bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CogmapAuditWork {
    /// The queue's grain — `kb_workflow_jobs.cogmap_id`.
    pub cogmap_id: Uuid,
    /// The auditor's grain — every citation in that cogmap this principal should weigh.
    pub payload: AuditJobPayload,
}

/// Sweep the principal's cogmap-homed findings with incomplete audit coverage or stale citations
/// (spec §6.3). `cap` is a **finding** budget — `audit_drift_sweep`'s `p_limit` — resolved through
/// [`clamp_auditor_cap`], which supplies the default AND bounds the request. The bound is
/// load-bearing, not hygiene: the sweep's `scored` CTE counts every candidate's readable citations
/// and their coverage before `p_limit` applies, and the raw `as i32`
/// this replaced wrapped a large `cap` into a negative `LIMIT` (a 500 from user input).
///
/// Auth: the gate is inside the SQL. `audit_drift_sweep` routes through
/// `steward_candidate_cogmaps(p_principal)` **and** a finding the principal reads by membership
/// (`resources_visible_by_membership`), with each cited source on the reading set
/// (`migrations/20261120000020_public_within_audit_split.sql`) — the same sets the write gate asks —
/// so an unreachable cogmap or a finding the principal may not grade simply never appears. There is no
/// unscoped variant of this call, because a sweep with no principal is a cross-tenant enumeration
/// oracle (spec §6.3).
pub async fn drift_sweep(
    pool: &PgPool,
    principal: ProfileId,
    cap: Option<i64>,
) -> ApiResult<Vec<AuditSweepRow>> {
    let limit = clamp_auditor_cap(cap);
    let rows = sqlx::query!(
        r#"
        SELECT cogmap_id  AS "cogmap_id!: Uuid",
               finding_id AS "finding_id!: Uuid",
               uncovered  AS "uncovered!: i32"
          FROM audit_drift_sweep($1, $2)
        "#,
        *principal,
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| AuditSweepRow {
            cogmap_id: r.cogmap_id,
            finding_id: r.finding_id,
            uncovered: r.uncovered,
        })
        .collect())
}

/// Collapse a finding-grained sweep into cogmap-grained queue work — **the grain fix** (spec §6.1).
///
/// `kb_workflow_jobs` enforces single-flight on `(cogmap_id, persona, dispatch_type)`
/// (`migrations/20260705000001_workflow_jobs.sql:43-45`, *"the single-flight guarantee"*), and
/// `workflow_job_enqueue` swallows a collision with `ON CONFLICT DO NOTHING` (`:59-62`). So
/// enqueuing one job per swept row would land the first finding of each cogmap and **silently
/// discard every other one** — no error, no log, N findings become 1. Grouping here is what makes
/// the enqueue call one-per-cogmap by construction.
///
/// Order is preserved on both axes: the cogmaps come out in the order their first citation appeared,
/// and each citation list keeps the order `expand_to_citations` handed over, which is the sweep's.
/// That ordering is rank-interleaved rather than plain `uncovered DESC` since `20260726000010` — it
/// is the only prioritization the auditor has, and re-sorting here would throw it away.
///
/// Takes citations rather than sweep rows since `20260727000050` — a finding that expands to zero
/// auditable citations contributes nothing and does not create an empty job.
pub fn group_by_cogmap(citations: &[(Uuid, AuditCitation)]) -> Vec<CogmapAuditWork> {
    let mut order: Vec<CogmapAuditWork> = Vec::new();
    let mut seen: HashMap<Uuid, usize> = HashMap::new();
    for (cogmap_id, citation) in citations {
        match seen.get(cogmap_id) {
            Some(&idx) => order[idx].payload.citations.push(citation.clone()),
            None => {
                seen.insert(*cogmap_id, order.len());
                order.push(CogmapAuditWork {
                    cogmap_id: *cogmap_id,
                    payload: AuditJobPayload {
                        citations: vec![citation.clone()],
                    },
                });
            }
        }
    }
    order
}

/// Expand a finding-grained sweep into the citations this principal should actually weigh.
///
/// ONE round trip, not one per finding: the swept finding ids go down as an array and
/// `resource_auditable_citations` is applied per element with `CROSS JOIN LATERAL`. A per-finding
/// loop would be N+1 queries against a function that is already `STABLE` and gated internally.
///
/// `WITH ORDINALITY` preserves the sweep's order across the array round trip — without it the
/// planner is free to return the unnested rows in any order, which would silently discard the
/// prioritization that is the auditor's only sense of what matters most. That prioritization is
/// `audit_drift_sweep`'s rank-interleaved ordering, not a plain `uncovered DESC`, since
/// `20260726000010` — see `expansion_keeps_the_sweeps_priority_order`, which asserts it survives.
///
/// Auth: `resource_auditable_citations` carries the same gate its siblings do (the finding on
/// membership, each cited source on reading), so a finding the principal may not grade yields zero
/// rows here rather than leaking. The sweep has already
/// applied that gate too; this is the defense-in-depth the repo asks for, not a redundant filter.
pub async fn expand_to_citations(
    pool: &PgPool,
    principal: ProfileId,
    rows: &[AuditSweepRow],
) -> ApiResult<Vec<(Uuid, AuditCitation)>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let cogmaps: Vec<Uuid> = rows.iter().map(|r| r.cogmap_id).collect();
    let findings: Vec<Uuid> = rows.iter().map(|r| r.finding_id).collect();

    let expanded = sqlx::query!(
        r#"
        SELECT f.cogmap_id   AS "cogmap_id!: Uuid",
               f.finding_id  AS "finding_id!: Uuid",
               ac.block_id   AS "block_id!: Uuid",
               ac.source_id  AS "source_id!: Uuid"
          FROM unnest($1::uuid[], $2::uuid[]) WITH ORDINALITY AS f(cogmap_id, finding_id, ord)
          CROSS JOIN LATERAL resource_auditable_citations(f.finding_id, $3) ac
         ORDER BY f.ord, ac.block_id, ac.source_id
        "#,
        &cogmaps,
        &findings,
        *principal,
    )
    .fetch_all(pool)
    .await?;

    Ok(expanded
        .into_iter()
        .map(|r| {
            (
                r.cogmap_id,
                AuditCitation {
                    finding_id: r.finding_id,
                    block_id: r.block_id,
                    source_id: r.source_id,
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod grouping_tests {
    use super::*;

    /// One expanded citation as `expand_to_citations` hands it over: the cogmap it groups under,
    /// paired with the `(finding, block, source)` triple the session will act on.
    fn cite(cogmap: Uuid, finding: Uuid, block: u128, source: u128) -> (Uuid, AuditCitation) {
        (
            cogmap,
            AuditCitation {
                finding_id: finding,
                block_id: Uuid::from_u128(block),
                source_id: Uuid::from_u128(source),
            },
        )
    }

    /// LOAD-BEARING — this is the assertion the whole grouping exists to satisfy, and it fails
    /// against a per-finding enqueue design: the citations of three findings in one cogmap must
    /// yield ONE unit of queue work carrying ALL of them, never three units (of which the queue's
    /// single-flight index would silently keep one).
    ///
    /// `f1` deliberately carries TWO citations. That is what makes this a *citation*-grain
    /// assertion rather than a renamed finding-grain one: four entries from three findings cannot be
    /// produced by anything still counting findings, and the payload's length no longer equals the
    /// swept-row count.
    #[test]
    fn many_findings_in_one_cogmap_become_one_job_carrying_every_citation() {
        let cogmap = Uuid::from_u128(1);
        let (f1, f2, f3) = (
            Uuid::from_u128(11),
            Uuid::from_u128(12),
            Uuid::from_u128(13),
        );
        let work = group_by_cogmap(&[
            cite(cogmap, f1, 101, 201),
            cite(cogmap, f1, 101, 202),
            cite(cogmap, f2, 102, 203),
            cite(cogmap, f3, 103, 204),
        ]);

        assert_eq!(
            work.len(),
            1,
            "one cogmap is one job — a second would collide on uq_workflow_jobs_in_flight and be \
             dropped by ON CONFLICT DO NOTHING"
        );
        assert_eq!(work[0].cogmap_id, cogmap);
        assert_eq!(
            work[0].payload.citations.len(),
            4,
            "four citations, not three findings — the payload is measured in the auditor's unit"
        );
        assert_eq!(
            work[0]
                .payload
                .citations
                .iter()
                .map(|c| c.finding_id)
                .collect::<Vec<_>>(),
            vec![f1, f1, f2, f3],
            "every finding's citations ride the one payload, in the order the sweep produced them"
        );
    }

    #[test]
    fn distinct_cogmaps_get_distinct_jobs_in_first_appearance_order() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let (f1, f2, f3) = (
            Uuid::from_u128(11),
            Uuid::from_u128(12),
            Uuid::from_u128(13),
        );
        // Interleaved, as a sweep spanning two maps genuinely is.
        let work = group_by_cogmap(&[
            cite(a, f1, 101, 201),
            cite(b, f2, 102, 202),
            cite(a, f3, 103, 203),
        ]);

        assert_eq!(
            work.len(),
            2,
            "two cogmaps, two jobs — they never dedup each other"
        );
        assert_eq!(work[0].cogmap_id, a, "a appeared first");
        assert_eq!(
            work[0]
                .payload
                .citations
                .iter()
                .map(|c| c.finding_id)
                .collect::<Vec<_>>(),
            vec![f1, f3]
        );
        assert_eq!(work[1].cogmap_id, b);
        assert_eq!(
            work[1]
                .payload
                .citations
                .iter()
                .map(|c| c.finding_id)
                .collect::<Vec<_>>(),
            vec![f2]
        );
    }

    /// Two citations of ONE finding that differ only in their block are two units of work, not one.
    /// Grouping keys on the cogmap and nothing else — deduping by finding here would re-create the
    /// grain bug inside the very function that fixed it.
    #[test]
    fn two_blocks_of_one_finding_citing_one_source_stay_two_citations() {
        let cogmap = Uuid::from_u128(1);
        let finding = Uuid::from_u128(11);
        let work = group_by_cogmap(&[
            cite(cogmap, finding, 101, 201),
            cite(cogmap, finding, 102, 201),
        ]);

        assert_eq!(work.len(), 1);
        assert_eq!(
            work[0]
                .payload
                .citations
                .iter()
                .map(|c| c.block_id)
                .collect::<Vec<_>>(),
            vec![Uuid::from_u128(101), Uuid::from_u128(102)],
            "same finding, same source, different citing blocks — each is its own verdict"
        );
    }

    #[test]
    fn an_empty_sweep_is_no_work() {
        assert!(
            group_by_cogmap(&[]).is_empty(),
            "no drift, no jobs, no tick side effects"
        );
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
