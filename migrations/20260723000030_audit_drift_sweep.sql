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
-- `20260723000020_standing_citation_components.sql:316-318`: "the shipped version called
-- `resource_independence_breadth` twice... with five producers that pattern would double every one
-- of them"). The `scored` CTE below calls `resource_citation_magnitude`/`resource_audit_coverage`
-- exactly once each per candidate; repeating them across the WHERE clause and the SELECT list would
-- run each producer up to four times per row instead.
--
-- FILTERS, each named by the failure it prevents:
--   * `r.is_active`                 — a soft-deleted finding must not head the queue forever.
--     Soft-delete only flips this flag; it does not fold blocks or provenance
--     (`_project_resource_deleted`, `20260624000002_canonical_functions.sql:1051-1061`, cited again
--     by `20260723000020...sql:41-49`), so an unfiltered sweep would keep re-surfacing a deleted
--     finding on every tick with no way to ever clear it (spec §3.1's liveness rule, applied to the
--     queue itself, not just the standing components).
--   * `r.ingest_state = 'complete'` — a segmented upload still in progress is not yet a finding to
--     audit: its citation set is incomplete BY CONSTRUCTION while more blocks/sources are still
--     arriving, so auditing it now would spend a verdict against a provenance list that has not
--     finished forming. The rule is CLAUDE.md's own: "`ingest_state = 'complete'` goes exactly where
--     `r.is_active` already goes" (`20260714000001_ingest_state.sql:139`), applied here to the
--     auditor's queue exactly as `20260723000020...sql:45-49` applied it to the standing components.
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
-- (`20260723000010_citation_audits.sql:12-17`: "there is deliberately NO `is_superseded` column...
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
-- ADDITIVE: one new function; nothing altered.

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
    SELECT s.cogmap_id, s.finding_id, (s.magnitude - s.coverage) AS uncovered
      FROM scored s
     WHERE s.magnitude > 0
       AND s.coverage < s.magnitude
     ORDER BY uncovered DESC
     LIMIT p_limit;
$$;
