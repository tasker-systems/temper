-- Attribute every citation audit to the profile that emitted it (Set 5 per-auditor collapse, Beat 1;
-- spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §3.1, §5.2).
--
-- WHAT IS MISSING TODAY. `kb_citation_audits` records WHAT was judged and HOW
-- (`20260724000110_citation_audits.sql:23-39`) and never names the JUDGE. The auditor is *reachable*
-- — `audited_by_event_id → kb_events.emitter_entity_id → kb_entities.profile_id`, the exact chain
-- spec §5.2 declares identity travels on (`:430`: *"One credential means one emitter entity"*), and
-- the same chain `citation_contributed_by_profile` already walks for the self-audit denial arm
-- (`20260724000110_citation_audits.sql:151-165`) — but reachable only through a two-join hop that no
-- standing producer performs. So no read can presently tell "three auditors agreed" from "one
-- auditor said it three times."
--
-- WHY THAT MATTERS, IN THE SPEC'S OWN ARGUMENT. `resource_citation_quality` is deliberately
-- TWO-stage — collapse within a source, THEN mean across sources — because a naive flat join
-- *"gives a five-block source five votes in the outer mean — the echo/actor-count fallacy
-- re-entering through block multiplicity, in the very function written to exclude it"*
-- (spec §3.1 `:113-119`; the shipped implementation and its header are
-- `20260724000120_standing_citation_components.sql:154-224`). The identical fallacy re-enters one
-- axis over: a single auditor re-auditing one citation N times contributes N weighted terms to that
-- source's stage-1 aggregate today, so persistence alone buys influence. The repair is a THIRD
-- collapse — within auditor, then within source, then across sources — and it cannot be written at
-- all until the auditor is a column the aggregation can `GROUP BY`. That collapse is the NEXT
-- migration's job. This one only makes it expressible; it changes no standing number.
--
-- WHY A COLUMN RATHER THAN THE JOIN. The denormalization CANNOT DRIFT, which is the only thing that
-- makes it legitimate rather than a second source of truth:
--   * `kb_events` is append-only — the `kb_events_append_only` trigger
--     (`20260624000001_canonical_schema.sql:498-506`) rejects every UPDATE — so an event's
--     `emitter_entity_id` is fixed the moment it is appended.
--   * NOTHING reassigns `kb_entities.profile_id`. `grep -rn "UPDATE kb_entities" migrations/ crates/
--     api/ packages/` yields exactly one hit, `20260709000040_kb_entities_unique_profile_name.sql:96`,
--     and it rewrites `name` only. There is no runtime writer at all.
-- The stored value is therefore a materialization of an immutable fact, not a cache of a mutable one.
--
-- WHY IT IS NOT NULLABLE, AND WHY THE BACKFILL CANNOT STRAND A ROW. Both links of the chain are
-- `NOT NULL` FKs to a primary key — `kb_events.emitter_entity_id` (`canonical_schema.sql:468`) and
-- `kb_entities.profile_id` (`:146`) — so the join is exactly 1:1: it can neither drop an audit nor
-- duplicate one. Verified against the dev database rather than asserted:
--     SELECT count(*) AS events, count(en.profile_id) AS resolved
--       FROM kb_events e JOIN kb_entities en ON en.id = e.emitter_entity_id;
--      events | resolved
--      -------+----------
--           5 |        5
-- A nullable column would therefore encode a state the schema forbids, and every future reader would
-- have to write a `COALESCE` arm for it.

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 1. The column — added NULLABLE, backfilled, then constrained. In that order, in one migration.
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- Three statements rather than one `ADD COLUMN ... NOT NULL DEFAULT ...` because there IS no honest
-- constant default: the value is per-row and derived. A literal default would silently attribute
-- every pre-existing audit to one wrong profile — and attribution that is wrong is worse than
-- attribution that is absent, since the per-auditor collapse this enables would then read those rows
-- as a single auditor agreeing with itself.
--
-- The sequence is correct whether the table holds zero rows or many: on an empty table the UPDATE is
-- a no-op and the `SET NOT NULL` validates trivially; on a populated one the UPDATE resolves every
-- row by the 1:1 chain above, so the `SET NOT NULL` scan finds nothing to reject. Not a defensive
-- ordering — the *only* ordering that works for both, which is what this repo's fresh-database CI and
-- its already-populated targets respectively exercise.
--
-- No `ON DELETE` clause, matching every other authorship-shaped profile reference in the schema
-- (`kb_resources.originator_profile_id`/`owner_profile_id` `:281-282`,
-- `kb_access_grants.granted_by_profile_id` `:305`, `kb_invitations.invited_by_profile_id` `:381`).
-- Default `NO ACTION` is the correct reading: who audited is a permanent historical fact, and a
-- profile deletion that would orphan it should be refused, not cascade the ledger away. The
-- `ON DELETE CASCADE` siblings (`:193`, `:333`) are membership rows, which are not history.

ALTER TABLE kb_citation_audits
    ADD COLUMN audited_by_profile_id UUID REFERENCES kb_profiles(id);

UPDATE kb_citation_audits a
   SET audited_by_profile_id = en.profile_id
  FROM kb_events   e
  JOIN kb_entities en ON en.id = e.emitter_entity_id
 WHERE e.id = a.audited_by_event_id;

ALTER TABLE kb_citation_audits
    ALTER COLUMN audited_by_profile_id SET NOT NULL;

COMMENT ON COLUMN kb_citation_audits.audited_by_profile_id IS
  'The profile that EMITTED this audit — the auditor — resolved from the owning event via '
  'kb_events.emitter_entity_id -> kb_entities.profile_id (spec §5.2''s identity chain). NOT the '
  'citer: `citation_contributed_by_profile` answers who contributed the citation, and the self-audit '
  'rule is precisely that the two must differ. This column does NOT enforce that rule — the '
  'AuditAuthority gate in temper-services runs before the write, and a row here is already past it. '
  'Immutable by construction: kb_events is append-only and nothing reassigns kb_entities.profile_id.';

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 2. Indexing — deliberately unchanged, and this is a decision, not an omission
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- `idx_kb_citation_audits_citation(block_id, source_kind, source_id)`
-- (`20260724000110_citation_audits.sql:39`) is left exactly as it is. The next migration's
-- `GROUP BY source_id, audited_by_profile_id` makes `audited_by_profile_id` a GROUPING key, not a LOOKUP
-- key, and the two want different things from an index:
--   * The row set being grouped is already selected by the join predicate
--     `a.block_id = c.block_id AND a.source_kind = 'resource' AND a.source_id = c.source_id`
--     (`20260724000120_standing_citation_components.sql:213-216`) — a full three-column match on the
--     incumbent index. Appending a fourth column moves nothing: it is not in the predicate, and the
--     grouping happens over the already-fetched rows.
--   * Nor would it buy an index-only scan, the other reason to widen. The same query also reads
--     `a.value` and `a.created` (`:205-206`), so every candidate row is a heap fetch regardless; a
--     covering index would have to carry all four extra columns to change that, for a table whose
--     per-citation row count is a handful.
-- Add an index when a measured plan asks for one, on the shape it actually asks for. Widening this
-- one now would be speculative cost — every audit write pays the maintenance — bought with no read.

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 3. The projector — DROP + CREATE, because a shipped migration is immutable
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- `20260724000110_citation_audits.sql` has applied; editing it in place would leave every already-
-- migrated target on the old body forever. The extension is a NEW migration that DROPs and CREATEs.
-- DROP+CREATE rather than CREATE OR REPLACE only for symmetry with the sibling
-- (`20260724000120_standing_citation_components.sql:81-86`); the signature is unchanged here, so
-- REPLACE would also work — but a reader who sees REPLACE has to check the signature to know that,
-- and DROP+CREATE states it.
--
-- ⚠️ NON-ADDITIVE FOR THE LENGTH OF THIS TRANSACTION ONLY. `DROP FUNCTION` breaks migrate-ahead-of-
-- deploy in general (a binary calling the dropped name errors), but here the CREATE follows in the
-- same migration with the SAME name and SAME signature, so no window exists outside the transaction.
-- The genuinely non-additive change in this set is the next migration's, not this one's.
--
-- ── THE PROFILE COMES FROM THE OWNING EVENT, NEVER FROM AN AMBIENT/CURRENT PRINCIPAL ──
-- The sibling rule for `created` is already stated at `20260724000110_citation_audits.sql:68-78`
-- (*"Projected timestamps come from the owning event's occurred_at, never now() — replay-stable by
-- construction"*, `20260624000002_canonical_functions.sql:614,791`). The identical reasoning governs
-- the profile, and for a sharper reason. THIS FUNCTION IS THE REPLAY PATH: `replay` walks the ledger
-- back through these same `_project_*` halves, so anything it reads from outside the event is
-- rewritten by the replay's own circumstances. A `p_profile` parameter fed by the current principal
-- would make a replay attribute EVERY historical audit to whoever ran the replay — collapsing a trail
-- written by three auditors into one, which is exactly the signal the per-auditor collapse is being
-- built to read. Derived from `p_event`, replay reproduces the identical row, and the projector needs
-- no new parameter at all: its two callers (`citation_audit` and replay's dispatch) keep their
-- existing shape.
--
-- ONE ROW READ, NOT TWO. `created` and `audited_by_profile_id` are both facts of the same event, so they
-- are fetched by one `SELECT ... INTO` over the join rather than by two scalar subqueries. This is
-- behavior-preserving, not merely equivalent-looking: the added `JOIN kb_entities` cannot drop the
-- row that the incumbent's bare `SELECT occurred_at FROM kb_events` would have found, because
-- `emitter_entity_id` is a `NOT NULL` FK to `kb_entities(id)` (`canonical_schema.sql:468`). So
-- `v_occurred` resolves to exactly what it resolved to before, for every event that exists.
--
-- EVERYTHING ELSE IS CARRIED OVER VERBATIM and must stay that way:
--   * `ON CONFLICT (audited_by_event_id) DO NOTHING` — the only uniqueness on the table, and what
--     makes replay idempotent rather than duplicating (`:32-34`).
--   * The `RETURNING ... IF v_audit IS NULL THEN SELECT` fallback — on the replay path the INSERT
--     is a no-op and `RETURNING` yields NO row, so without the fallback the function returns NULL
--     for an audit that exists (`:64-66`).
--   * `created` from `v_occurred`, never the column DEFAULT (`:68-78`).
DROP FUNCTION _project_citation_audited(uuid, jsonb);

CREATE FUNCTION _project_citation_audited(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_audit uuid;
        v_occurred timestamptz;
        v_profile uuid;
BEGIN
    -- Both event-derived facts, from the one row. See the header: this is the replay path, so every
    -- projected value must come off the ledger and nothing may come off the ambient session.
    SELECT e.occurred_at, en.profile_id
      INTO v_occurred, v_profile
      FROM kb_events   e
      JOIN kb_entities en ON en.id = e.emitter_entity_id
     WHERE e.id = p_event;

    INSERT INTO kb_citation_audits (block_id, source_kind, source_id, value, reason,
                                    audited_by_event_id, audited_by_profile_id, created)
    VALUES (
        (p_payload->>'block_id')::uuid,
        (p_payload #>> '{source,kind}')::provenance_source_kind,
        (p_payload #>> '{source,value}')::uuid,
        (p_payload->>'value')::double precision,
        p_payload->>'reason',
        p_event,
        v_profile,
        v_occurred
    )
    ON CONFLICT (audited_by_event_id) DO NOTHING
    RETURNING id INTO v_audit;

    IF v_audit IS NULL THEN
        SELECT id INTO v_audit FROM kb_citation_audits WHERE audited_by_event_id = p_event;
    END IF;

    RETURN v_audit;
END;
$$;

COMMENT ON FUNCTION _project_citation_audited(uuid, jsonb) IS
  'Projects a citation_audited event into kb_citation_audits. Every projected value is derived from '
  'the OWNING EVENT — `created` from occurred_at, `audited_by_profile_id` from emitter_entity_id -> '
  'kb_entities.profile_id — never from now() and never from an ambient/current principal, because '
  'this function is also the replay path: anything read from outside the event is rewritten by the '
  'replay''s own circumstances. Idempotent on audited_by_event_id, with a RETURNING fallback so the '
  'replay no-op still yields the existing audit id.';
