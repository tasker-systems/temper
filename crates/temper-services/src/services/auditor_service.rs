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
use temper_core::types::auditor::{AuditJobPayload, AuditSweepRow};
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
    /// The auditor's grain — every swept finding homed in that cogmap.
    pub payload: AuditJobPayload,
}

/// Sweep the principal's cogmap-homed findings with incomplete audit coverage, most-uncovered-first
/// (spec §6.3). `cap` is a **finding** budget — `audit_drift_sweep`'s `p_limit` — resolved through
/// [`clamp_auditor_cap`], which supplies the default AND bounds the request. The bound is
/// load-bearing, not hygiene: the sweep's `scored` CTE calls `resource_citation_magnitude` and
/// `resource_audit_coverage` for **every** candidate before `p_limit` applies, and the raw `as i32`
/// this replaced wrapped a large `cap` into a negative `LIMIT` (a 500 from user input).
///
/// Auth: the gate is inside the SQL. `audit_drift_sweep` routes through
/// `steward_candidate_cogmaps(p_principal)` **and** `resources_visible_to(p_principal)`
/// (`migrations/20260723000030_audit_drift_sweep.sql:89-100`) — the same predicates every other read
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
/// Order is preserved on both axes: the cogmaps come out in the order their first finding appeared,
/// and each finding list keeps the sweep's `uncovered DESC` ordering. The sweep's prioritization is
/// the only prioritization the auditor has, and re-sorting here would throw it away.
pub fn group_by_cogmap(rows: &[AuditSweepRow]) -> Vec<CogmapAuditWork> {
    let mut order: Vec<CogmapAuditWork> = Vec::new();
    let mut seen: HashMap<Uuid, usize> = HashMap::new();
    for row in rows {
        match seen.get(&row.cogmap_id) {
            Some(&idx) => order[idx].payload.findings.push(row.finding_id),
            None => {
                seen.insert(row.cogmap_id, order.len());
                order.push(CogmapAuditWork {
                    cogmap_id: row.cogmap_id,
                    payload: AuditJobPayload {
                        findings: vec![row.finding_id],
                    },
                });
            }
        }
    }
    order
}

#[cfg(test)]
mod grouping_tests {
    use super::*;

    fn row(cogmap: Uuid, finding: Uuid, uncovered: i32) -> AuditSweepRow {
        AuditSweepRow {
            cogmap_id: cogmap,
            finding_id: finding,
            uncovered,
        }
    }

    /// LOAD-BEARING — this is the assertion the whole grain fix exists to satisfy, and it fails
    /// against a per-finding enqueue design: three findings in one cogmap must yield ONE unit of
    /// queue work carrying THREE findings, never three units (of which the queue's single-flight
    /// index would silently keep one) and never one carrying a single finding.
    #[test]
    fn many_findings_in_one_cogmap_become_one_job_carrying_them_all() {
        let cogmap = Uuid::from_u128(1);
        let (f1, f2, f3) = (
            Uuid::from_u128(11),
            Uuid::from_u128(12),
            Uuid::from_u128(13),
        );
        let work = group_by_cogmap(&[row(cogmap, f1, 5), row(cogmap, f2, 3), row(cogmap, f3, 1)]);

        assert_eq!(
            work.len(),
            1,
            "one cogmap is one job — a second would collide on uq_workflow_jobs_in_flight and be \
             dropped by ON CONFLICT DO NOTHING"
        );
        assert_eq!(work[0].cogmap_id, cogmap);
        assert_eq!(
            work[0].payload.findings,
            vec![f1, f2, f3],
            "all three findings ride the one payload, in the sweep's uncovered-DESC order"
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
        // Interleaved, as an uncovered-DESC sweep across two maps genuinely is.
        let work = group_by_cogmap(&[row(a, f1, 9), row(b, f2, 4), row(a, f3, 2)]);

        assert_eq!(
            work.len(),
            2,
            "two cogmaps, two jobs — they never dedup each other"
        );
        assert_eq!(work[0].cogmap_id, a, "a appeared first");
        assert_eq!(work[0].payload.findings, vec![f1, f3]);
        assert_eq!(work[1].cogmap_id, b);
        assert_eq!(work[1].payload.findings, vec![f2]);
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
    /// `resource_live_citations` reads (`20260723000020_standing_citation_components.sql:100-107`).
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

    /// A finding homed in `cogmap` with exactly one live, unaudited resource-kind citation →
    /// `magnitude = 1`, `coverage = 0`, so `audit_drift_sweep` returns it with `uncovered = 1`.
    async fn seed_finding_with_one_citation(pool: &PgPool, s: &Seeded, slug: &str) -> Uuid {
        let source: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id",
        )
        .bind(format!("{slug}-source"))
        .fetch_one(pool)
        .await
        .unwrap();
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
        sqlx::query(
            "INSERT INTO kb_block_provenance \
               (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
             VALUES ($1, 'resource', $2, $3, 0)",
        )
        .bind(block)
        .bind(source)
        .bind(s.event)
        .execute(pool)
        .await
        .unwrap();
        finding
    }

    /// The findings the tick actually enqueued for `cogmap`, read straight off the queue row.
    async fn queued_findings(pool: &PgPool, cogmap: Uuid) -> Vec<Uuid> {
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
            .findings
    }

    /// THE GRAIN TEST (spec §6.1). Three uncovered findings in ONE cogmap must produce exactly one
    /// queue row carrying all three.
    ///
    /// It falsifies the per-finding design directly: against an enqueue-per-swept-row tick, the
    /// first `workflow_job_enqueue` would create the row and the next two would hit
    /// `uq_workflow_jobs_in_flight` and return NULL through `ON CONFLICT DO NOTHING`
    /// (`20260705000001_workflow_jobs.sql:43-45,59-62`). The job count would still be 1 — so a test
    /// that only counted jobs would PASS against the broken design. The load-bearing assertion is
    /// therefore the payload: `findings.len() == 3`, which a per-finding tick could never produce.
    #[sqlx::test(migrations = "../../migrations")]
    async fn three_findings_in_one_cogmap_enqueue_one_job_carrying_three(pool: PgPool) {
        let s = seed(&pool).await;
        let mut expected = vec![
            seed_finding_with_one_citation(&pool, &s, "finding-a").await,
            seed_finding_with_one_citation(&pool, &s, "finding-b").await,
            seed_finding_with_one_citation(&pool, &s, "finding-c").await,
        ];
        expected.sort();

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

        let mut carried = ours[0].findings.clone();
        carried.sort();
        assert_eq!(
            carried, expected,
            "the claimed job carries ALL THREE findings — a per-finding enqueue would have kept one \
             and let ON CONFLICT DO NOTHING discard the other two in silence"
        );

        let mut persisted = queued_findings(&pool, s.cogmap).await;
        persisted.sort();
        assert_eq!(
            persisted, expected,
            "and the list is on the queue row itself, not only in the claim's return value"
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
                findings: vec![Uuid::from_u128(1)],
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
                findings: vec![Uuid::from_u128(2)],
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
            queued_findings(&pool, s.cogmap).await,
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
                && j.findings.len() == 1
                && j.findings[0] == a_finding),
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
