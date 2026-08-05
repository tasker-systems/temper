-- Steward candidacy gates on AUTHORSHIP; auditor candidacy stays readability.
--
-- =================================================================================================
-- WHAT THIS FIXES
-- =================================================================================================
--
-- `steward_candidate_cogmaps(p_principal)` selects on `anchor_readable_by_profile`. Two of its four
-- callers feed work whose terminal gate is `cogmap_authorable_by_profile` — an explicit write grant,
-- with no membership arm since `20260701000001`. Readability is strictly broader, so those two hand
-- out maps on which the queued work is then refused, every run, forever:
--
--   * `steward_drift_sweep`              -> steward session -> `advance_steward_watermark` (WRITE)
--   * `steward_service::candidate_cogmaps` -> GET /api/steward/candidates -> the hourly materialize
--                                             cron -> `materialize_on_threshold` (WRITE)
--
-- Observed in production on 2026-08-05: the L0 kernel map (`00000000-…-0005-…0001`) is joined to the
-- root `temper-system` team, so EVERY principal reads it and it lands in every steward's candidate
-- set. Nobody holds write on it by design — `20260701000001`'s backfill deliberately excludes
-- `auto_join_role` teams. Consequences, both measured rather than inferred:
--   * `steward_advance_watermark` refused once per run with zero successes; L0's
--     `steward_watermark_event_id` is still NULL and always would have been.
--   * L0's `shape_materialized_event_id` is still NULL — it has never once materialized.
-- The materialize cron is the louder of the two: it fans out with `Promise.all` and throws on any
-- non-ok, so one un-authorable candidate fails the whole tick.
--
-- =================================================================================================
-- WHY A NEW FUNCTION AND NOT AN EDIT TO THE INCUMBENT
-- =================================================================================================
--
-- The other two callers MUST keep readability, and that is a decided position, not an accident:
--
--   * `audit_drift_sweep`   -> citation audits
--   * `workflow_job_claim`  -> the auditor's claim-scope constraint (the `p_principal` arm)
--
-- `docs/auth/machine-token-contract.md` §C provisions the citation auditor with **zero** cogmap write
-- grants on purpose, so that it is never classified `AuditAuthority::Author` and cannot grade its own
-- work. `crates/temper-services/src/authz/audit_gate.rs` states it outright: "Readability, not
-- writability -- and that is not laziness ... a write gate here would 403 the one caller this endpoint
-- exists for." Tightening the shared function would empty the auditor's candidate set and stop it
-- auditing anything.
--
-- So the two personas get two predicates. The shared one keeps its name, its body, and its callers;
-- the steward's paths move to the narrower one. This is the divergence being made explicit rather
-- than the two uses continuing to share a definition that can only be correct for one of them.
--
-- NOT DONE HERE, deliberately: `AuditorJobAuthority::resolve` inlines
-- `anchor_readable_by_profile(...)` rather than calling `steward_candidate_cogmaps`, asserting
-- equivalence in a comment. That copy stays CORRECT under this change (the auditor wants readability),
-- so touching it would be unrelated churn -- but it is a silent-divergence site that nothing links,
-- and it is named here so the next reader does not have to rediscover it.

-- ── The steward's candidate set: readable AND authorable ────────────────────────────────────────
-- Deliberately built on `steward_candidate_cogmaps` rather than re-deriving the join, so the two can
-- never disagree about what "a candidate at all" means; this only ever narrows it. The added
-- conjunct is the exact predicate the queued work's own gate calls, which is what keeps "dispatched"
-- and "permitted" from drifting apart again.
CREATE FUNCTION steward_authorable_cogmaps(p_principal uuid)
RETURNS TABLE(cogmap_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.cogmap_id
      FROM steward_candidate_cogmaps(p_principal) c
     WHERE cogmap_authorable_by_profile(p_principal, c.cogmap_id);
$$;

-- ── Repoint the steward's drift sweep ───────────────────────────────────────────────────────────
-- ONE token changes: the candidate source. Signature, return shape, both drift axes, the
-- three-argument `steward_ingest_delta` call, and the ORDER BY are verbatim from `20260727000040`, so
-- this is CREATE OR REPLACE and no Rust call site moves. `boundary_moved` is still READ from `d` and
-- never recomputed here -- `20260727000040` makes `steward_ingest_delta` the one place that decides
-- what "the boundary moved" means, and that stays true.
CREATE OR REPLACE FUNCTION steward_drift_sweep(p_principal uuid, p_threshold bigint)
RETURNS TABLE(cogmap_id uuid, watermark uuid, new_resources bigint, new_events bigint,
              boundary_fingerprint text, boundary_moved boolean)
LANGUAGE sql STABLE AS $$
    SELECT m.cogmap_id,
           cm.steward_watermark_event_id AS watermark,
           d.new_resources,
           d.new_events,
           d.boundary_fingerprint,
           d.boundary_moved
      FROM steward_authorable_cogmaps(p_principal) m
      JOIN kb_cogmaps cm ON cm.id = m.cogmap_id
      -- Both stored cursors ride in from the same row: two cursors of one completed run.
      CROSS JOIN LATERAL steward_ingest_delta(m.cogmap_id,
                                              cm.steward_watermark_event_id,
                                              cm.steward_boundary_fingerprint) d
     -- Either axis admits the map. `d.boundary_moved` is READ, never recomputed.
     WHERE d.new_resources >= p_threshold
        OR d.boundary_moved
     -- Boundary-moved leads because that class is SELF-CLEARING and its count is typically 0 — the
     -- minimum of the second key — so it would otherwise sort behind every map with one new
     -- resource, while count-drift recurs every tick anyway. Presentation, not admission: no LIMIT,
     -- and steward_dispatch_tick enqueues every row before the downstream cap. cogmap_id makes the
     -- sort total.
     ORDER BY d.boundary_moved DESC, d.new_resources DESC, m.cogmap_id;
$$;

COMMENT ON FUNCTION steward_drift_sweep(uuid, bigint) IS
$c$Team-joined cogmaps this principal can AUTHOR that the steward should look at, most-drifted-first.

Repointed by `20260805000010` from `steward_candidate_cogmaps` (readable) to
`steward_authorable_cogmaps` (readable AND `cogmap_authorable_by_profile`). The work this sweep
queues terminates in `advance_steward_watermark`, whose gate is an explicit write grant, so a
readable-but-unauthorable map was dispatched work that could only ever be refused -- observed on the
L0 kernel map, which every principal reads via the root temper-system team and nobody may author.

Everything else is unchanged from `20260727000040`: both drift axes
(`new_resources >= p_threshold OR boundary_moved`), `boundary_moved` READ from steward_ingest_delta
and never recomputed here, and the ORDER BY boundary_moved DESC, new_resources DESC, cogmap_id.$c$;

SELECT declare_migration(
    20260805000010,
    'additive',
    'Steward candidacy gates on authorship; auditor candidacy stays readability. Adds steward_authorable_cogmaps (candidates INTERSECT cogmap_authorable_by_profile) and repoints steward_drift_sweep at it; the Rust materialize fan-out (steward_service::candidate_cogmaps) moves in the same PR. steward_candidate_cogmaps itself is UNCHANGED because audit_drift_sweep and workflow_job_claim must keep readability -- the citation auditor is provisioned with zero cogmap write grants by design (machine-token-contract.md SecC), so tightening the shared function would empty its candidate set. Additive: one new function, and one CREATE OR REPLACE whose signature and return shape are unchanged, so no deployed binary can disagree with this schema across the apply. Fixes the L0 kernel appearing in every steward candidate set: readable by all via the root temper-system team, authorable by none by design, so advance_steward_watermark refused once per run with zero successes and materialize never ran at all.'
);
