-- T4a — Ingest-threshold delta → fire steward.
--
-- The team-self-cognition steward (goal `team-self-cognition-steward-agent-eve-mvp`) runs on a
-- cron cadence, but only *acts* when enough new material has landed in the team's contexts since it
-- last ran. This migration adds the two persistence pieces that answer "how much has landed since
-- watermark X":
--
--   (1) A per-cogmap ingest cursor `kb_cogmaps.steward_watermark_event_id` — the last `kb_events.id`
--       a completed steward run observed. `kb_events.id` is uuidv7 (time-ordered), so `e.id > cursor`
--       is the same "touched since" comparison `formation_touched_since` (replay.rs) already relies on.
--       NULL = the steward has never run for this cogmap → the delta counts from the beginning.
--
--   (2) A pure counting function `steward_ingest_delta(cogmap, watermark)` — resolves the cogmap's
--       team(s) via `kb_team_cogmaps`, expands to the team's contexts (team-OWNED via
--       `kb_contexts.owner_table='kb_teams'` ∪ team-SHARED via `kb_team_contexts`), and counts the
--       `kb_events` anchored to those contexts after the watermark. Splits the count into
--       `new_resources` (the `resource_created` events — the ingest signal a threshold gates on) and
--       `new_events` (all activity — context color). Resource creation anchors to its context home
--       (`resource_create` → `_event_append(..., home.table, home.id)`, canonical_functions.sql:752),
--       so a context-homed `resource_created` reliably carries `producing_anchor_table='kb_contexts'`.
--
-- Authorization is NOT enforced here — the function is a pure count. The service layer gates the read
-- on `anchor_readable_by_profile(principal,'kb_cogmaps',cogmap)` before calling it, and the watermark
-- advance on `can(principal,'write','kb_cogmaps',cogmap)`.
--
-- ADDITIVE, additive-only-on-`main`: a new nullable column (every existing row reads NULL = "never
-- run", unchanged behavior) plus a new function. No existing object is altered.
--
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public).
-- STABLE + LANGUAGE sql so `sqlx::query!` callers stay compile-checked.

ALTER TABLE kb_cogmaps
    ADD COLUMN steward_watermark_event_id UUID REFERENCES kb_events(id);

COMMENT ON COLUMN kb_cogmaps.steward_watermark_event_id IS
    'Ingest cursor for the team-self-cognition steward: the last kb_events.id a completed steward run '
    'observed in the team''s contexts. NULL = never run (steward_ingest_delta counts from the '
    'beginning). Advanced by advance_steward_watermark on run completion.';

CREATE FUNCTION steward_ingest_delta(p_cogmap uuid, p_watermark uuid)
RETURNS TABLE(new_resources bigint, new_events bigint)
LANGUAGE sql STABLE AS $$
    WITH team_ctx AS (
        -- Contexts the cogmap's team OWNS.
        SELECT c.id
          FROM kb_team_cogmaps tc
          JOIN kb_contexts c
            ON c.owner_table = 'kb_teams' AND c.owner_id = tc.team_id
         WHERE tc.cogmap_id = p_cogmap
        UNION
        -- Contexts SHARED into the cogmap's team.
        SELECT ktc.context_id
          FROM kb_team_cogmaps tc
          JOIN kb_team_contexts ktc ON ktc.team_id = tc.team_id
         WHERE tc.cogmap_id = p_cogmap
    )
    SELECT
        count(*) FILTER (WHERE et.name = 'resource_created')::bigint AS new_resources,
        count(*)::bigint                                            AS new_events
      FROM kb_events e
      JOIN kb_event_types et ON et.id = e.event_type_id
     WHERE e.producing_anchor_table = 'kb_contexts'
       AND e.producing_anchor_id IN (SELECT id FROM team_ctx)
       AND (p_watermark IS NULL OR e.id > p_watermark);
$$;
