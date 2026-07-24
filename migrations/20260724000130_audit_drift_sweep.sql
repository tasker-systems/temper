-- The auditor's principal-scoped selection sweep (Set 5, Task 5; spec
-- `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §6.2-6.3).
--
-- REUSE, not re-implement (mirrors `steward_drift_sweep`,
-- `migrations/20260705000002_steward_drift_sweep.sql:19-31`): `steward_candidate_cogmaps(p_principal)`
-- IS ALREADY the principal-scoped candidate-cogmap set — the same `anchor_readable_by_profile` gate
-- every other read uses (`20260624000002_canonical_functions.sql:274-287`). Calling it here, rather
-- than restating the predicate, is what spec §6.3 means by "ours must route through the equivalent":
-- the steward's candidate set and the auditor's ARE the equivalent, because both ask "which cogmaps
-- can this principal reach", not two different questions in different clothes.
--
-- THE COGMAP-HOME JOIN IS THE §6.2 BOUNDARY, not a WHERE-clause afterthought.
-- `kb_resource_homes.anchor_table` admits exactly `('kb_contexts', 'kb_cogmaps')`
-- (`20260624000001_canonical_schema.sql:279`), one row per resource. Joining on
-- `h.anchor_table = 'kb_cogmaps' AND h.anchor_id = m.cogmap_id` is what makes a context-homed
-- finding structurally unreachable here — not filtered out downstream, but never joined in. A sweep
-- that instead started from the principal's full readable-resource set
-- (`resources_visible_to`/`resources_readable_by`) and filtered by anchor_table afterward would ALSO
-- admit context-homed findings the principal can read by ownership or grant — exactly the boundary
-- spec §6.2 draws ("the first cut audits cogmap-homed findings only; widening the queue is additive
-- and separate").
--
-- EACH PRODUCER RUNS ONCE PER CANDIDATE ROW (mirrors `resource_standing_shape`'s `components` CTE,
-- `20260724000120_standing_citation_components.sql:316-318`: "the shipped version called
-- `resource_independence_breadth` twice... with five producers that pattern would double every one
-- of them"). The `scored` CTE below calls `resource_citation_magnitude`/`resource_audit_coverage`
-- exactly once each per candidate; repeating them across the WHERE clause and the SELECT list would
-- run each producer up to four times per row instead.
--
-- FILTERS, each named by the failure it prevents:
--   * `r.is_active`                 — a soft-deleted finding must not head the queue forever.
--     Soft-delete only flips this flag; it does not fold blocks or provenance
--     (`_project_resource_deleted`, `20260624000002_canonical_functions.sql:1051-1061`, cited again
--     by `20260724000120...sql:41-49`), so an unfiltered sweep would keep re-surfacing a deleted
--     finding on every tick with no way to ever clear it (spec §3.1's liveness rule, applied to the
--     queue itself, not just the standing components).
--   * `r.ingest_state = 'complete'` — a segmented upload still in progress is not yet a finding to
--     audit: its citation set is incomplete BY CONSTRUCTION while more blocks/sources are still
--     arriving, so auditing it now would spend a verdict against a provenance list that has not
--     finished forming. The rule is CLAUDE.md's own: "`ingest_state = 'complete'` goes exactly where
--     `r.is_active` already goes" (`20260714000001_ingest_state.sql:139`), applied here to the
--     auditor's queue exactly as `20260724000120...sql:45-49` applied it to the standing components.
--   * `magnitude > 0`               — a finding with no live resource-kind citations has nothing for
--     the auditor to weigh. Without this guard, every uncited finding in the corpus would appear at
--     `uncovered = 0` — pure noise ahead of nothing, since `0 - 0 = 0` is not pushed to the tail by
--     the DESC ordering alone.
--   * `coverage < magnitude`        — the actual selection predicate (spec §6.3): incomplete
--     coverage, not low quality. A quality-based predicate would drop a partially-audited finding out
--     of the queue the moment a single citation is weighed — spec §6.3 names and rejects exactly
--     that design ("a quality-based sweep would drop a partially-audited finding out of the queue
--     after a single citation is weighed").
--   * the cogmap-home join          — see above; the §6.2 scope boundary.
--   * `steward_candidate_cogmaps`   — see above; the §6.3 principal-scoping. A sweep with no
--     principal gate is a cross-tenant enumeration oracle (spec §6.3: "a sweep with no principal is
--     a cross-tenant enumeration oracle, which would defeat §7's entire `NotFound` posture").
--   * `resources_visible_to`        — the per-FINDING read gate, and it is not redundant with the
--     candidate-cogmap gate above. The two answer different questions through two INDEPENDENTLY
--     MAINTAINED predicates: `steward_candidate_cogmaps` asks "can this principal reach this cogmap?"
--     via `anchor_readable_by_profile` -> `cogmap_readable_by_profile`
--     (`20260712000010_context_read_predicates.sql:157-166`), while the read this queue feeds —
--     Task 6's `AuditAuthority`, through `readback::is_resource_visible` — asks "can this principal
--     see this resource?" via `resources_visible_to` (`20260712000010...sql:212-263`). Those two
--     sets coincide today only because `resources_visible_to`'s cogmap arms happen to admit every
--     resource homed in a reachable/granted cogmap. Nothing holds them together, and a queue that
--     enumerates finding ids the read gate would refuse is precisely the enumeration oracle the
--     paragraph above says we are avoiding. Filtering through the SAME predicate the gate uses makes
--     the queue a SUBSET of what the gate admits by construction, so the auditor is never handed
--     work that will 404 — the "one spelling" rule `readback::is_resource_visible` exists to enforce.
--
-- KNOWN FIRST-CUT LIMITATION (spec §6.3, documented here rather than fixed): coverage is monotone
-- under the append-only trail — `kb_citation_audits` has no supersession
-- (`20260724000110_citation_audits.sql:12-17`: "there is deliberately NO `is_superseded` column...
-- a later +1.0 never erases an earlier -1.0"). A readable, live, cogmap-homed finding whose
-- remaining uncovered sources the auditor *declines* to verdict — rather than covering them — never
-- regains a fresh selection signal from this predicate: coverage does not fall, and nothing here ages
-- a "still uncovered after N ticks" state. The filters above remove the common causes of permanent
-- unauditability (a remote/deleted source, a half-uploaded resource, an unreachable cogmap), but a
-- citation that is readable, live, and simply never gets audited will re-head this queue every tick
-- with the SAME `uncovered` count forever. The real fix — a terminal "cannot assess" verdict, or a
-- per-finding backoff — is deferred to the future reaper pass spec §6.3 itself describes and defers
-- ("this is deliberately left for a future reaper pass... scoping and building it will likely evolve
-- the auditor persona itself, and it is out of scope here").
--
-- NOT ADDITIVE-ONLY ANY MORE. This file used to say "one new function; nothing altered" — it now also
-- carries the QUEUE-side half of the same principal scoping (see the section below the sweep):
-- an `ALTER TABLE kb_workflow_jobs`, a DROP+CREATE of `workflow_job_claim`, and one new completion
-- primitive. Scoping the sweep without scoping the claim was the whole defect.
CREATE FUNCTION audit_drift_sweep(p_principal uuid, p_limit int)
RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)
LANGUAGE sql STABLE AS $$
    WITH candidates AS (
        SELECT h.anchor_id AS cogmap_id, r.id AS finding_id
          FROM steward_candidate_cogmaps(p_principal) m
          JOIN kb_resource_homes h
            ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = m.cogmap_id
          JOIN kb_resources r ON r.id = h.resource_id
         WHERE r.is_active
           AND r.ingest_state = 'complete'
           -- The same predicate Task 6's gate runs, so the queue cannot offer what the gate refuses.
           AND r.id IN (SELECT resource_id FROM resources_visible_to(p_principal))
    ),
    scored AS (
        SELECT c.cogmap_id, c.finding_id,
               resource_citation_magnitude(c.finding_id) AS magnitude,
               resource_audit_coverage(c.finding_id)     AS coverage
          FROM candidates c
    )
    -- `, s.finding_id` IS THE TIE-BREAKER, and without it this function is not deterministic —
    -- which is what two of its callers' own docs claim it is (`auditor_service::drift_sweep`'s
    -- "deterministic, principal-scoped sweep"; `group_by_cogmap`'s "the cogmaps come out in the
    -- order their first finding appeared", which is stable only relative to a stable input).
    -- `uncovered DESC` alone leaves rows with equal `uncovered` in an unspecified order, so with the
    -- default cap of 50 it is not merely the ORDER that varies run to run but WHICH findings are
    -- enqueued at all when more than 50 tie — the common shape, since most drifting findings sit at
    -- `uncovered = 1`. `finding_id` is total, stable, and free (it is already selected).
    SELECT s.cogmap_id, s.finding_id, (s.magnitude - s.coverage) AS uncovered
      FROM scored s
     WHERE s.magnitude > 0
       AND s.coverage < s.magnitude
     ORDER BY uncovered DESC, s.finding_id
     LIMIT p_limit;
$$;

-- ── AND THE CLAIM MUST BE SCOPED TOO, or the sweep's scoping buys nothing ─────────────────────
-- The tick is reap -> sweep -> enqueue -> claim. Everything above scopes the SWEEP. The CLAIM
-- (`workflow_job_claim`, `20260705000001_workflow_jobs.sql:68-89`, re-created with a correlation
-- argument by `20260710000010_steward_tick_correlation.sql:59-83`) filtered on
-- `(persona, dispatch_type, status)` and NOTHING ELSE — no cogmap, no principal, no reach predicate.
-- So the tick enqueued this principal's work and then claimed ANYONE'S, and `claim_audit` returns
-- each claimed row's payload verbatim: the `cogmap_id` and the FULL finding-id list of cogmaps the
-- caller cannot read. Two failures at once, from an ordinary authenticated account:
--   * cross-tenant enumeration of cogmap and finding ids, and
--   * permanent theft of the audit pipeline — the stolen jobs go `in_progress` under a claimant that
--     never completes them, the real auditor's hourly tick finds nothing claimable, and after
--     `max_attempts` reap cycles the rows go `dead`.
-- This section closes it at the queue, not at one surface, so every future dispatch persona inherits
-- the scoping instead of re-deriving it. The endpoint-level half (the auditor's dispatch tick admits
-- only a registered, unrevoked machine principal) lives in `authz`, and neither half is sufficient
-- alone: the gate stops a human from ticking at all, the scoping stops one machine principal from
-- claiming another tenant's work.
--
-- NOT ADDITIVE-ONLY: this file now also alters `kb_workflow_jobs` and replaces `workflow_job_claim`.
-- The DROP+CREATE follows `20260710000010`'s mechanics exactly — the new parameter is appended LAST
-- with a DEFAULT, so the deployed 5-argument positional call site keeps resolving through the
-- migrate->deploy window (memory: drop_function_non_additive_breaks_deploy_skew). `p_principal IS
-- NULL` reproduces the previous behavior byte-for-byte, which is what keeps the steward's own
-- unscoped claim working unchanged while the auditor's is scoped.

ALTER TABLE kb_workflow_jobs ADD COLUMN claimed_by_profile_id uuid REFERENCES kb_profiles(id);

COMMENT ON COLUMN kb_workflow_jobs.claimed_by_profile_id IS
    'The principal that CLAIMED this job, set at claim alongside correlation_id (NULL when the claimer '
    'passed no principal — the steward''s unscoped claim). Unlike correlation_id, this one IS consulted: '
    'workflow_job_complete_claimed refuses to complete a job the caller did not claim.';

-- ── RECORDED, NOT FIXED: `invocation_open` can inherit the WRONG persona's tick ────────────────
-- `20260710000010_steward_tick_correlation.sql:104-110` resolves an opening invocation's correlation
-- with `WHERE j.cogmap_id = v_orig AND j.status = 'in_progress' ORDER BY j.leased_at DESC LIMIT 1`
-- and NO persona filter. Its own comment concedes the hazard as something a cogmap "could in
-- principle" reach. Set 5 makes it ROUTINE: `Persona::Auditor` exists precisely so an auditor job and
-- a steward job can be in flight over one cogmap at the same time
-- (`auditor_service::tests::auditor_and_steward_jobs_coexist_for_one_cogmap` asserts exactly that),
-- and the auditor's cron trails the steward's by 30 minutes — so whichever was leased more recently
-- wins, and an auditor session's acts can join the steward's tick.
--
-- Not fixed here, deliberately. `invocation_open(p_payload, p_emitter)` takes no persona, so the fix
-- is a behavior change to a function on the STEWARD's live pipeline, outside Set 5's narrative — the
-- same call already made for the steward's own unscoped claim. The shape it should take when someone
-- does it: resolve the emitter's profile and prefer the job that principal actually claimed —
--
--     ORDER BY coalesce(j.claimed_by_profile_id = v_profile, false) DESC, j.leased_at DESC
--
-- `coalesce(..., false)` is not decoration: the steward's claim passes no principal, so its jobs
-- carry a NULL claimant, and a bare `(claimed_by = v_profile) DESC NULLS LAST` would rank the
-- auditor's job (false) ABOVE the steward's own (NULL) and make the regression worse than the bug.
-- With the coalesce, a steward open scores false on both and falls back to today's `leased_at`
-- ordering, while an auditor open matches its own job outright.

-- Claim, scoped. Body verbatim from `20260710000010:64-83` with two additions: the reach constraint
-- and the claimant stamp.
DROP FUNCTION workflow_job_claim(text, text, int, int, uuid);
CREATE FUNCTION workflow_job_claim(
    p_persona text, p_dispatch_type text, p_limit int, p_lease_seconds int,
    p_correlation uuid DEFAULT NULL, p_principal uuid DEFAULT NULL
) RETURNS TABLE(id uuid, cogmap_id uuid, attempts int, payload jsonb)
LANGUAGE sql AS $$
    UPDATE kb_workflow_jobs j
       SET status = 'in_progress',
           leased_at = now(),
           lease_expires_at = now() + make_interval(secs => p_lease_seconds),
           attempts = j.attempts + 1,
           correlation_id = p_correlation,
           claimed_by_profile_id = p_principal
     WHERE j.id IN (
         SELECT c.id
           FROM kb_workflow_jobs c
          WHERE c.persona = p_persona
            AND c.dispatch_type = p_dispatch_type
            AND c.status IN ('pending', 'waiting_for_retry')
            AND c.next_visible_at <= now()
            -- The reach constraint. `steward_candidate_cogmaps` is the SAME predicate the sweep above
            -- gates on, so a principal can only ever claim work over cogmaps its own sweep could have
            -- enqueued. NULL means "unscoped", preserving the pre-Set-5 behavior for callers that
            -- pass no principal.
            AND (p_principal IS NULL
                 OR c.cogmap_id IN (SELECT m.cogmap_id FROM steward_candidate_cogmaps(p_principal) m))
          ORDER BY c.enqueued_at
          LIMIT p_limit
          FOR UPDATE SKIP LOCKED
     )
    RETURNING j.id, j.cogmap_id, j.attempts, j.payload;
$$;

-- Complete, but only the job this principal is actually holding.
--
-- A SEPARATE function rather than a widening of `workflow_job_complete`: that one is the steward's,
-- it rides `steward_advance_watermark`, and its `status IN ('pending','in_progress','waiting_for_retry')`
-- transition is load-bearing there. Changing it would change the steward's pipeline for a fix that is
-- entirely the auditor's.
--
-- Two narrowings against `workflow_job_complete` (`20260705000001:94-104`), each named by what it
-- prevents:
--   * `status = 'in_progress'` — the shipped form also completes a **pending** job, i.e. one that has
--     never been dispatched. A caller polling this endpoint could terminate every job the moment it
--     appeared and suppress a cogmap's auditing indefinitely, with nothing anywhere recording it.
--   * `claimed_by_profile_id = p_principal` — completing someone else's IN-FLIGHT job frees the
--     single-flight slot while their session is still running, so the next tick enqueues and claims a
--     second concurrent audit session over the same finding list, both appending to a trail that by
--     design cannot retract (`20260724000110_citation_audits.sql:12-17`).
-- No match is NULL, not an error: an already-completed job, a reaped lease, or a manual audit outside
-- the dispatch loop are all "nothing to complete", and the session's written verdicts stand either way.
CREATE FUNCTION workflow_job_complete_claimed(
    p_cogmap uuid, p_persona text, p_dispatch_type text, p_principal uuid
) RETURNS uuid LANGUAGE sql AS $$
    UPDATE kb_workflow_jobs
       SET status = 'done', completed_at = now()
     WHERE cogmap_id = p_cogmap
       AND persona = p_persona
       AND dispatch_type = p_dispatch_type
       AND status = 'in_progress'
       AND claimed_by_profile_id = p_principal
    RETURNING id;
$$;
