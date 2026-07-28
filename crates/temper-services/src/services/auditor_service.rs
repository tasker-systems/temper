//! Read path + grain reconciliation for the citation auditor's dispatch tick (Set 5, Task 13).
//!
//! Spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §6.1-6.3.
//!
//! Service-direct (the read-path convention, mirroring [`crate::services::steward_service`]): the
//! surface passes a resolved principal + optional cap; [`drift_sweep`] gates through
//! `audit_drift_sweep`, which is itself principal-scoped in SQL. The write side (enqueue + claim)
//! routes through the `Backend` trait / `DbBackend`, not here.
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
/// load-bearing, not hygiene: the sweep's `scored` CTE calls `resource_citation_magnitude` and
/// `resource_audit_coverage` for **every** candidate before `p_limit` applies, and the raw `as i32`
/// this replaced wrapped a large `cap` into a negative `LIMIT` (a 500 from user input).
///
/// Auth: the gate is inside the SQL. `audit_drift_sweep` routes through
/// `steward_candidate_cogmaps(p_principal)` **and** `resources_visible_to(p_principal)`
/// (`migrations/20260724000130_audit_drift_sweep.sql:89-100`) — the same predicates every other read
/// uses — so an unreachable cogmap or an unreadable finding simply never appears. There is no
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
/// Auth: `resource_auditable_citations` carries the same `resources_visible_to` gate its siblings
/// do, so an unreadable finding yields zero rows here rather than leaking. The sweep has already
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
mod tests {
    use super::*;
    use crate::services::workflow_job_service;
    use temper_core::types::workflow_job::{DispatchType, Persona};
    use temper_workflow::operations::{AuditorDispatchTick, Backend, Surface};

    use crate::backend::DbBackend;

    /// A cogmap-homed finding citing one live, unaudited resource source, for a principal who
    /// reaches the cogmap through team membership.
    ///
    /// Raw inserts rather than the production write path: `writes::create_resource_with` lives in
    /// temper-substrate's test surface, not here, and `steward_service`'s own `#[sqlx::test]` module
    /// already establishes raw fixture inserts as this crate's convention. The shapes are taken from
    /// the DDL — `kb_content_blocks` (`20260624000001_canonical_schema.sql`, `resource_id/seq/
    /// genesis_event_id/last_event_id`) and `kb_block_provenance` (`block_id/source_kind/source_id/
    /// contributed_by_event_id/accretion_seq`) — which together are exactly what
    /// `resource_live_citations` reads (`20260724000120_standing_citation_components.sql:100-107`).
    struct Seeded {
        principal: Uuid,
        cogmap: Uuid,
        /// One event standing in for every block's genesis/last/contribution FK.
        event: Uuid,
    }

    async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Register `profile` in the `kb_machine_clients` allowlist — the conjunct the dispatch tick now
    /// requires (`authz::require_machine_principal`). The auditor IS a machine principal by spec
    /// §5.2; before the fix wave any authenticated principal could run the tick.
    async fn register_machine(pool: &PgPool, profile: Uuid, client_id: &str) {
        sqlx::query(
            "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
             VALUES ($1, $1, $2, $2)",
        )
        .bind(client_id)
        .bind(profile)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed(pool: &PgPool) -> Seeded {
        let principal = insert_profile(pool, "auditor").await;
        register_machine(pool, principal, "auditor@clients").await;

        let team: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('aud-team', 'Aud Team') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(team)
        .bind(principal)
        .execute(pool)
        .await
        .unwrap();

        let telos: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let cogmap: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('aud-map', $1) RETURNING id",
        )
        .bind(telos)
        .fetch_one(pool)
        .await
        .unwrap();
        // Direct membership in a team joined to the cogmap is the ONLY path
        // `cogmap_readable_by_profile` recognizes, and therefore what `steward_candidate_cogmaps`
        // (and so `audit_drift_sweep`) admits.
        sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
            .bind(cogmap)
            .bind(team)
            .execute(pool)
            .await
            .unwrap();

        let entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'e') RETURNING id",
        )
        .bind(principal)
        .fetch_one(pool)
        .await
        .unwrap();
        // One event stands in for every block's genesis/last/contribution FK — the sweep reads none
        // of its content, only the provenance row that references it.
        let event: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = 'block_mutated'), $1) RETURNING id",
        )
        .bind(entity)
        .fetch_one(pool)
        .await
        .unwrap();

        Seeded {
            principal,
            cogmap,
            event,
        }
    }

    /// A finding homed in `cogmap` citing `n` live, unaudited resource-kind sources from ONE block
    /// → `magnitude = n`, `coverage = 0`, so `audit_drift_sweep` returns it with `uncovered = n`.
    ///
    /// One block with `n` provenance rows rather than `n` blocks: `resource_live_citations` returns
    /// `DISTINCT (block_id, source_id)`, so the citation count is the pair count either way, and the
    /// single-block shape keeps the fixture honest about what varies.
    ///
    /// Returns the finding and its `(block, source)` pairs in insertion order.
    async fn seed_finding_with_citations(
        pool: &PgPool,
        s: &Seeded,
        slug: &str,
        n: usize,
    ) -> (Uuid, Vec<(Uuid, Uuid)>) {
        let finding: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id",
        )
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
        )
        .bind(finding)
        .bind(s.cogmap)
        .bind(s.principal)
        .execute(pool)
        .await
        .unwrap();
        let block: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id) \
             VALUES ($1, 0, $2, $2) RETURNING id",
        )
        .bind(finding)
        .bind(s.event)
        .fetch_one(pool)
        .await
        .unwrap();

        let mut citations = Vec::with_capacity(n);
        for i in 0..n {
            let source: Uuid = sqlx::query_scalar(
                "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id",
            )
            .bind(format!("{slug}-source-{i}"))
            .fetch_one(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO kb_block_provenance \
                   (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
                 VALUES ($1, 'resource', $2, $3, $4)",
            )
            .bind(block)
            .bind(source)
            .bind(s.event)
            .bind(i as i32)
            .execute(pool)
            .await
            .unwrap();
            citations.push((block, source));
        }
        (finding, citations)
    }

    /// The single-citation case, which is what most tests here want.
    async fn seed_finding_with_one_citation(pool: &PgPool, s: &Seeded, slug: &str) -> Uuid {
        seed_finding_with_citations(pool, s, slug, 1).await.0
    }

    /// Record an audit of `(block, source)` attributed to `profile`.
    ///
    /// A raw insert, matching this module's fixture convention — it stamps `audited_by_profile_id`
    /// directly rather than going through the projector, which in production fills it from the
    /// owning event's emitter. What matters to these tests is only that the column names the
    /// principal, since that is what `resource_auditable_citations` filters on.
    ///
    /// The event is minted because it is genuinely required, not for symmetry:
    /// `audited_by_event_id` is `NOT NULL` with an FK to `kb_events`, and `resource_stale_citations`
    /// compares audit timestamps against block events. Its emitter entity is owned by `profile` so
    /// the two attributions cannot disagree if a later test does route through the projector.
    async fn record_audit(pool: &PgPool, profile: Uuid, block: Uuid, source: Uuid, value: f64) {
        let entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'auditor-entity') RETURNING id",
        )
        .bind(profile)
        .fetch_one(pool)
        .await
        .unwrap();
        let event: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = 'citation_audited'), $1) \
             RETURNING id",
        )
        .bind(entity)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_citation_audits \
               (block_id, source_kind, source_id, value, audited_by_event_id, \
                audited_by_profile_id) \
             VALUES ($1, 'resource', $2, $3, $4, $5)",
        )
        .bind(block)
        .bind(source)
        .bind(value)
        .bind(event)
        .bind(profile)
        .execute(pool)
        .await
        .unwrap();
    }

    /// The citations the tick actually enqueued for `cogmap`, read straight off the queue row.
    async fn queued_citations(pool: &PgPool, cogmap: Uuid) -> Vec<AuditCitation> {
        let payload: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM kb_workflow_jobs \
              WHERE cogmap_id = $1 AND persona = 'auditor' AND dispatch_type = 'citation-audit'",
        )
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap();
        serde_json::from_value::<AuditJobPayload>(payload)
            .unwrap()
            .citations
    }

    /// The distinct findings represented in a citation list, sorted — the finding-grain projection
    /// of a citation-grain payload.
    fn findings_of(citations: &[AuditCitation]) -> Vec<Uuid> {
        let mut out: Vec<Uuid> = citations.iter().map(|c| c.finding_id).collect();
        out.sort();
        out.dedup();
        out
    }

    /// THE GRAIN TEST (spec §6.1), now at citation grain. Three uncovered findings in ONE cogmap
    /// must produce exactly one queue row carrying every auditable citation of all three.
    ///
    /// It falsifies the per-finding design directly: against an enqueue-per-swept-row tick, the
    /// first `workflow_job_enqueue` would create the row and the next two would hit
    /// `uq_workflow_jobs_in_flight` and return NULL through `ON CONFLICT DO NOTHING`
    /// (`20260705000001_workflow_jobs.sql:43-45,59-62`). The job count would still be 1 — so a test
    /// that only counted jobs would PASS against the broken design. The load-bearing assertion is
    /// therefore the payload's contents, which a per-finding tick could never produce.
    ///
    /// `finding-a` carries THREE citations while its siblings carry one each, so the payload holds
    /// five entries drawn from three findings. That spread is what keeps this a citation-grain
    /// assertion after `20260727000050` rather than a renamed finding-grain one: five-from-three
    /// distinguishes the new payload from both a per-finding enqueue (which would carry one
    /// finding) and a list that merely renamed `findings` to `citations` (which would carry three).
    ///
    /// Verified to bite: bypassing `group_by_cogmap` with a per-citation enqueue fails this test.
    /// The pure `grouping_tests` above stay green under that same bypass, because it routes around
    /// the function rather than breaking it — which is why this DB-level test has to exist.
    #[sqlx::test(migrations = "../../migrations")]
    async fn three_findings_in_one_cogmap_enqueue_one_job_carrying_every_citation(pool: PgPool) {
        let s = seed(&pool).await;
        let (a, a_cites) = seed_finding_with_citations(&pool, &s, "finding-a", 3).await;
        let (b, b_cites) = seed_finding_with_citations(&pool, &s, "finding-b", 1).await;
        let (c, c_cites) = seed_finding_with_citations(&pool, &s, "finding-c", 1).await;
        let mut expected_findings = vec![a, b, c];
        expected_findings.sort();
        let mut expected_pairs: Vec<(Uuid, Uuid)> =
            a_cites.into_iter().chain(b_cites).chain(c_cites).collect();
        expected_pairs.sort();

        let backend = DbBackend::new(pool.clone(), s.principal.into());
        let claimed = backend
            .auditor_dispatch_tick(AuditorDispatchTick {
                cap: None,
                correlation: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap()
            .value;

        // Scoped to OUR cogmap: the L0 kernel map is seeded into every test database by the
        // migrations, so an assertion over all claimed rows would be coupled to background seed data.
        let ours: Vec<_> = claimed.iter().filter(|j| j.cogmap_id == s.cogmap).collect();
        assert_eq!(
            ours.len(),
            1,
            "one cogmap is one job, however many findings drifted"
        );

        assert_eq!(
            findings_of(&ours[0].citations),
            expected_findings,
            "the claimed job represents ALL THREE findings — a per-finding enqueue would have kept \
             one and let ON CONFLICT DO NOTHING discard the other two in silence"
        );
        let mut carried: Vec<(Uuid, Uuid)> = ours[0]
            .citations
            .iter()
            .map(|c| (c.block_id, c.source_id))
            .collect();
        carried.sort();
        assert_eq!(
            carried, expected_pairs,
            "and it carries all FIVE citations, not one per finding — the payload is the auditor's \
             unit of work, so `finding-a`'s three citations are three entries"
        );

        let persisted = queued_citations(&pool, s.cogmap).await;
        assert_eq!(
            findings_of(&persisted),
            expected_findings,
            "and the list is on the queue row itself, not only in the claim's return value"
        );
        let mut persisted_pairs: Vec<(Uuid, Uuid)> = persisted
            .iter()
            .map(|c| (c.block_id, c.source_id))
            .collect();
        persisted_pairs.sort();
        assert_eq!(persisted_pairs, expected_pairs);
    }

    /// **The wiring witness, and the task's own outcome asserted end to end through the tick.**
    ///
    /// `resource_auditable_citations` has its own witnesses in temper-substrate, and the grouping
    /// has its own above — but nothing tested that the tick actually *routes* the sweep through the
    /// producer. That gap is exactly where the defect lived before `20260727000050`: the sweep was
    /// correct, the queue was correct, and the payload still carried work the principal had already
    /// done.
    ///
    /// Verified to bite: against an expansion reverted to `resource_live_citations` (every live
    /// citation, unfiltered) the payload comes back with all three and the remainder assertion
    /// fails. Note it is the REMAINDER assertion that catches it — the two above it still pass,
    /// because an unfiltered payload does dispatch the finding and does represent it. A test that
    /// stopped at "the finding is dispatched" would have been green against the defect.
    ///
    /// Both halves in one test on purpose, mirroring the substrate witness: a tick that dropped the
    /// whole finding once any citation was weighed would satisfy "does not re-offer" and silently
    /// starve the remainder, and would look from the queue's side exactly like draining correctly.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_partially_weighed_finding_is_dispatched_with_only_its_remainder(pool: PgPool) {
        let s = seed(&pool).await;
        let (finding, cites) = seed_finding_with_citations(&pool, &s, "finding-a", 3).await;
        let (block, weighed) = cites[0];
        record_audit(&pool, s.principal, block, weighed, 0.5).await;

        let claimed = DbBackend::new(pool.clone(), s.principal.into())
            .auditor_dispatch_tick(AuditorDispatchTick {
                cap: None,
                correlation: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap()
            .value;

        let ours: Vec<_> = claimed.iter().filter(|j| j.cogmap_id == s.cogmap).collect();
        assert_eq!(ours.len(), 1, "the finding is still dispatched");
        assert_eq!(
            findings_of(&ours[0].citations),
            vec![finding],
            "the finding is NOT skipped because one of its citations is weighed"
        );

        let mut carried: Vec<(Uuid, Uuid)> = ours[0]
            .citations
            .iter()
            .map(|c| (c.block_id, c.source_id))
            .collect();
        carried.sort();
        let mut remainder = vec![cites[1], cites[2]];
        remainder.sort();
        assert_eq!(
            carried, remainder,
            "exactly the two unweighed citations ride the payload — the weighed one is withheld, so \
             the session is given no opportunity to re-weigh it and needs no instruction not to"
        );
        assert!(
            !carried.contains(&(block, weighed)),
            "stated as absence too, since this is the defect itself: source {weighed} was weighed \
             minutes ago and must not come back"
        );
    }

    /// The expansion preserves the sweep's ordering across the array round trip — the claim
    /// `WITH ORDINALITY` exists to make true, and the auditor's only sense of what matters most.
    ///
    /// `audit_drift_sweep` ranks the uncovered class by `(magnitude - coverage) DESC` before
    /// interleaving (`20260726000010`), so the four-citation finding outranks the one-citation one.
    /// Without `ORDINALITY` the planner is free to return the unnested rows in any order and this
    /// prioritization is silently discarded — with no error and no log, which is why it is asserted
    /// rather than assumed.
    #[sqlx::test(migrations = "../../migrations")]
    async fn expansion_keeps_the_sweeps_priority_order(pool: PgPool) {
        let s = seed(&pool).await;
        // Seeded least-urgent first, so insertion order cannot be mistaken for sweep order.
        let (minor, _) = seed_finding_with_citations(&pool, &s, "finding-minor", 1).await;
        let (major, _) = seed_finding_with_citations(&pool, &s, "finding-major", 4).await;

        let swept = drift_sweep(&pool, s.principal.into(), None).await.unwrap();
        let ours: Vec<_> = swept
            .iter()
            .filter(|r| r.cogmap_id == s.cogmap)
            .cloned()
            .collect();
        assert_eq!(
            ours.iter().map(|r| r.finding_id).collect::<Vec<_>>(),
            vec![major, minor],
            "precondition: the sweep really does rank the four-citation finding first, so the \
             assertion below is about the expansion and not about the sweep"
        );

        let expanded = expand_to_citations(&pool, s.principal.into(), &ours)
            .await
            .unwrap();
        assert_eq!(
            expanded
                .iter()
                .map(|(_, c)| c.finding_id)
                .collect::<Vec<_>>(),
            vec![major, major, major, major, minor],
            "the sweep's order survives the array round trip — all four of the urgent finding's \
             citations precede the minor one's"
        );
    }

    /// The mechanism the grouping protects against, asserted head-on rather than inferred: a second
    /// enqueue for the same `(cogmap, 'auditor', 'citation-audit')` is a silent no-op, so a
    /// per-finding tick loses every finding after the first.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_second_enqueue_for_the_same_cogmap_is_silently_dropped(pool: PgPool) {
        let s = seed(&pool).await;
        let first = workflow_job_service::enqueue_with_payload(
            &pool,
            s.cogmap,
            Persona::Auditor.as_str(),
            DispatchType::CitationAudit.as_str(),
            serde_json::to_value(AuditJobPayload {
                citations: vec![AuditCitation {
                    finding_id: Uuid::from_u128(1),
                    block_id: Uuid::from_u128(101),
                    source_id: Uuid::from_u128(201),
                }],
            })
            .unwrap(),
        )
        .await
        .unwrap();
        let second = workflow_job_service::enqueue_with_payload(
            &pool,
            s.cogmap,
            Persona::Auditor.as_str(),
            DispatchType::CitationAudit.as_str(),
            serde_json::to_value(AuditJobPayload {
                citations: vec![AuditCitation {
                    finding_id: Uuid::from_u128(2),
                    block_id: Uuid::from_u128(102),
                    source_id: Uuid::from_u128(202),
                }],
            })
            .unwrap(),
        )
        .await
        .unwrap();

        assert!(first.is_some(), "the first enqueue creates the row");
        assert!(
            second.is_none(),
            "the second returns NULL — no error is raised, which is exactly why a per-finding \
             enqueue loop would look successful while dropping work"
        );
        assert_eq!(
            findings_of(&queued_citations(&pool, s.cogmap).await),
            vec![Uuid::from_u128(1)],
            "the surviving row still carries the FIRST payload — the second finding is simply gone"
        );
    }

    /// The auditor persona does not contend with the steward's queue slot: both can be in flight
    /// over the same cogmap, because single-flight is keyed on `(cogmap_id, persona, dispatch_type)`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn auditor_and_steward_jobs_coexist_for_one_cogmap(pool: PgPool) {
        let s = seed(&pool).await;
        seed_finding_with_one_citation(&pool, &s, "finding-a").await;
        workflow_job_service::enqueue(
            &pool,
            s.cogmap,
            Persona::Steward.as_str(),
            DispatchType::Steward.as_str(),
        )
        .await
        .unwrap()
        .expect("steward job enqueues");

        let backend = DbBackend::new(pool.clone(), s.principal.into());
        let claimed = backend
            .auditor_dispatch_tick(AuditorDispatchTick {
                cap: None,
                correlation: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap()
            .value;

        assert!(
            claimed.iter().any(|j| j.cogmap_id == s.cogmap),
            "the auditor claims its own job despite an in-flight steward job on the same cogmap"
        );
        let steward_still_pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_workflow_jobs \
              WHERE cogmap_id = $1 AND persona = 'steward' AND status = 'pending'",
        )
        .bind(s.cogmap)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            steward_still_pending, 1,
            "and the auditor's claim does not steal the steward's job"
        );
    }

    /// **The endpoint half of the Critical fix, at the tick.** An ordinary authenticated principal
    /// — reaching cogmap and finding, so nothing else refuses it — cannot run the tick at all.
    /// Before the fix this returned claimed jobs; the assertion is the refusal AND its class.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unregistered_principal_cannot_run_the_tick(pool: PgPool) {
        let s = seed(&pool).await;
        seed_finding_with_one_citation(&pool, &s, "finding-a").await;
        // A second profile in the SAME team — full reach, no `kb_machine_clients` row.
        let human = insert_profile(&pool, "human").await;
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             SELECT team_id, $1, 'member' FROM kb_team_cogmaps WHERE cogmap_id = $2",
        )
        .bind(human)
        .bind(s.cogmap)
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            !drift_sweep(&pool, human.into(), None)
                .await
                .unwrap()
                .is_empty(),
            "precondition: it reaches the work — reach is not why the tick refuses"
        );

        let err = DbBackend::new(pool.clone(), human.into())
            .auditor_dispatch_tick(AuditorDispatchTick {
                cap: None,
                correlation: None,
                origin: Surface::ApiHttp,
            })
            .await
            .expect_err("an unregistered principal must not run the auditor's tick");
        assert!(
            matches!(err, temper_core::error::TemperError::Forbidden),
            "expected Forbidden — the tick names no subject whose existence a 404 could hide; \
             got {err:?}"
        );
    }

    /// **The claim half of the Critical fix, end to end through the tick.** Two tenants, each with
    /// its own team, cogmap and drifted finding, and neither reaching the other. Tenant A's tick
    /// must come back with A's job only — against the shipped unscoped claim it comes back with
    /// both, disclosing B's `cogmap_id` and B's finding ids and taking B's job `in_progress` under
    /// A's lease, where it dies at `max_attempts` without B ever auditing anything.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_tick_never_claims_another_tenants_job(pool: PgPool) {
        let a = seed(&pool).await;
        let a_finding = seed_finding_with_one_citation(&pool, &a, "finding-a").await;

        // Tenant B: a second principal, team, cogmap and finding, disjoint from A's.
        let b_principal = insert_profile(&pool, "auditor-b").await;
        register_machine(&pool, b_principal, "auditor-b@clients").await;
        let b_team: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('b-team', 'B Team') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(b_team)
        .bind(b_principal)
        .execute(&pool)
        .await
        .unwrap();
        let b_telos: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('b-telos', '') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let b_cogmap: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('b-map', $1) RETURNING id",
        )
        .bind(b_telos)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
            .bind(b_cogmap)
            .bind(b_team)
            .execute(&pool)
            .await
            .unwrap();
        let b = Seeded {
            principal: b_principal,
            cogmap: b_cogmap,
            event: a.event,
        };
        let b_finding = seed_finding_with_one_citation(&pool, &b, "finding-b").await;

        // B's own tick enqueues and claims B's work — the job A must not be able to take.
        DbBackend::new(pool.clone(), b_principal.into())
            .auditor_dispatch_tick(AuditorDispatchTick {
                cap: None,
                correlation: None,
                origin: Surface::ApiHttp,
            })
            .await
            .expect("B ticks its own tenant");

        let claimed = DbBackend::new(pool.clone(), a.principal.into())
            .auditor_dispatch_tick(AuditorDispatchTick {
                cap: None,
                correlation: None,
                origin: Surface::ApiHttp,
            })
            .await
            .expect("A ticks")
            .value;

        assert!(
            claimed.iter().any(|j| j.cogmap_id == a.cogmap
                && j.citations.len() == 1
                && j.citations[0].finding_id == a_finding),
            "precondition: A really did claim its OWN work, so an empty result cannot be why the \
             assertion below holds"
        );
        assert!(
            !claimed.iter().any(|j| j.cogmap_id == b.cogmap),
            "A must never receive B's job — its payload is B's finding-id list ({b_finding}) and \
             its slot is B's pipeline"
        );
        let b_claimant: Option<Uuid> = sqlx::query_scalar(
            "SELECT claimed_by_profile_id FROM kb_workflow_jobs \
              WHERE cogmap_id = $1 AND persona = 'auditor'",
        )
        .bind(b.cogmap)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            b_claimant,
            Some(b_principal),
            "and B's job is still leased to B, not re-stamped by A's claim"
        );
    }

    /// Principal scoping, asserted as absence: a profile with no route to the cogmap sweeps nothing,
    /// so the tick enqueues nothing for it (spec §6.3 — a sweep with no principal gate is a
    /// cross-tenant enumeration oracle).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unreaching_principal_sweeps_the_finding_away(pool: PgPool) {
        let s = seed(&pool).await;
        let finding = seed_finding_with_one_citation(&pool, &s, "finding-a").await;
        let outsider = insert_profile(&pool, "outsider").await;

        let mine = drift_sweep(&pool, s.principal.into(), None).await.unwrap();
        assert!(
            mine.iter().any(|r| r.finding_id == finding),
            "the reaching principal sees its own cogmap's uncovered finding"
        );
        let theirs = drift_sweep(&pool, outsider.into(), None).await.unwrap();
        assert!(
            !theirs.iter().any(|r| r.finding_id == finding),
            "the outsider never sees it"
        );
    }
}
