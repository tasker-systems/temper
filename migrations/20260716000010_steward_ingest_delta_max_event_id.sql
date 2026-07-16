-- Steward watermark can advance: expose `max_event_id` from the ingest delta (issue #459).
--
-- The team-self-cognition steward reads `steward_ingest_delta(cogmap, watermark)` at the top of a
-- tick, distills the delta, then calls `steward_advance_watermark(cogmap, event_id)` to mark the
-- delta ingested. But `advance` needs a real `kb_events.id` and the delta returned only counts
-- (`new_resources`, `new_events`) — no event identifier. With no deterministic way to obtain the
-- value `advance` requires, the watermark never moved, so the drift sweep re-selected and re-
-- distilled the same corpus on every tick. The map-stewardship guidance already documents advancing
-- to `delta.max_event_id`; this migration makes the substrate actually return it.
--
-- Three changes, all additive in behavior (existing callers keep working — `steward_drift_sweep`'s
-- LATERAL join and the service query both reference the counting columns by name, and a new output
-- column is invisible to them):
--
--   (1) `steward_team_contexts(cogmap)` — extract the "contexts this cogmap's team ingests from"
--       set (team-OWNED ∪ team-SHARED) that was inline in `steward_ingest_delta`, so both the delta
--       and the new window check below share ONE definition of the ingest scope.
--   (2) `steward_ingest_delta` gains `max_event_id uuid` — the newest `kb_events.id` in the delta
--       window (NULL when the window is empty). `kb_events.id` is uuidv7, whose byte order IS time
--       order, so `ORDER BY id DESC LIMIT 1` is the latest event — exactly the cursor `advance`
--       should move to. (`ORDER BY … LIMIT 1`, not `max(id)`: there is no `max(uuid)` aggregate.)
--   (3) `steward_event_in_ingest_window(cogmap, event)` — a boolean the write path uses to make the
--       advance server-verified: an advance target must be an event the cogmap actually ingests
--       (anchored to one of its team contexts), not any global `kb_events.id`. This guards the
--       inverse failure — a blocked/empty tick moving the watermark past content it never processed.
--
-- Changing a function's RETURNS TABLE signature requires DROP + CREATE (CREATE OR REPLACE cannot
-- change the return type). `steward_ingest_delta` is a `LANGUAGE sql` function with a quoted body, so
-- its callers (`steward_drift_sweep`) hold no recorded dependency on it — the DROP is safe and the
-- re-CREATE re-binds by name.
--
-- ADDITIVE, additive-only-on-`main`: one column added to a function's output, two new functions; no
-- table altered, no existing column dropped. Namespace-free (resolves against the connection's
-- search_path = public); STABLE + LANGUAGE sql so `sqlx::query!` callers stay compile-checked.

CREATE FUNCTION steward_team_contexts(p_cogmap uuid)
RETURNS TABLE(context_id uuid)
LANGUAGE sql STABLE AS $$
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
     WHERE tc.cogmap_id = p_cogmap;
$$;

COMMENT ON FUNCTION steward_team_contexts(uuid) IS
    'The contexts a cogmap''s team ingests from (team-OWNED via kb_contexts.owner_table=''kb_teams'' '
    'UNION team-SHARED via kb_team_contexts). The single definition of a steward cogmap''s ingest '
    'scope, shared by steward_ingest_delta and steward_event_in_ingest_window.';

DROP FUNCTION steward_ingest_delta(uuid, uuid);

CREATE FUNCTION steward_ingest_delta(p_cogmap uuid, p_watermark uuid)
RETURNS TABLE(new_resources bigint, new_events bigint, max_event_id uuid)
LANGUAGE sql STABLE AS $$
    -- One window definition (the CTE), then the counts and the latest id off it.
    WITH win AS (
        SELECT e.id, et.name AS type_name
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
         WHERE e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id IN (SELECT context_id FROM steward_team_contexts(p_cogmap))
           AND (p_watermark IS NULL OR e.id > p_watermark)
    )
    SELECT
        count(*) FILTER (WHERE type_name = 'resource_created')::bigint AS new_resources,
        count(*)::bigint                                              AS new_events,
        -- uuidv7 byte order is time order, so the DESC-first id is the newest event in the window;
        -- NULL when the window is empty (no events since the watermark) — the "nothing to advance
        -- to" signal. ORDER BY … LIMIT 1 rather than max(id): there is no max(uuid) aggregate.
        (SELECT id FROM win ORDER BY id DESC LIMIT 1)               AS max_event_id
      FROM win;
$$;

CREATE FUNCTION steward_event_in_ingest_window(p_cogmap uuid, p_event uuid)
RETURNS boolean
LANGUAGE sql STABLE AS $$
    -- True iff the event is one this cogmap actually ingests: a real kb_events row anchored to one of
    -- the cogmap's team contexts. Watermark-independent (the write path checks membership in the
    -- ingest scope, not position relative to the current cursor) — max_event_id is always in scope.
    SELECT EXISTS (
        SELECT 1
          FROM kb_events e
         WHERE e.id = p_event
           AND e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id IN (SELECT context_id FROM steward_team_contexts(p_cogmap))
    );
$$;

COMMENT ON FUNCTION steward_event_in_ingest_window(uuid, uuid) IS
    'True iff p_event is an event p_cogmap ingests (anchored to one of its team contexts). Gates '
    'advance_steward_watermark so the watermark can only move to an event the tick actually observed.';
