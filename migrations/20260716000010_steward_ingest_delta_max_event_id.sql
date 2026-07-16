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
-- Three changes:
--
--   (1) `steward_team_contexts(cogmap)` — the cogmap's ingest scope, as a first-class function so
--       the delta and the window check below share ONE definition. This corrects a pre-existing leak
--       inherited from the original `steward_ingest_delta` (migration 20260701000005): that function
--       scoped ingest to the *union* of every joined team's contexts. Because `kb_team_cogmaps` is
--       many-to-many (PK `(cogmap_id, team_id)`), a cogmap joined to a high- and a low-privilege team
--       would ingest BOTH teams' context activity — and since any joined team can read the map, the
--       high-privilege team's context content leaks down through the shared map. The access model's
--       stated invariant is the opposite (canonical_schema.sql: "Cognitive maps reach workflow
--       content ONLY through producer-INTERSECTION over their joined teams"), and the canonical
--       cogmap-principal reach `resources_accessible_to_cogmap` (canonical_functions.sql) implements
--       exactly that — "the only bound that closes the cross-team leak." This function is the
--       context-grain sibling of that construction: a context is in scope only when EVERY joined team
--       reaches it (`HAVING count(DISTINCT team_id) = |joined teams|`), default-closed on the empty
--       join (⋂ over ∅ would be the universe — the leak, backwards). Single-team cogmaps (the common
--       case) are unaffected: for one team, intersection == union.
--   (2) `steward_ingest_delta` gains `max_event_id uuid` — the newest `kb_events.id` in the delta
--       window (NULL when the window is empty). `kb_events.id` is uuidv7, whose byte order IS time
--       order, so `ORDER BY id DESC LIMIT 1` is the latest event — exactly the cursor `advance`
--       should move to. (`ORDER BY … LIMIT 1`, not `max(id)`: there is no `max(uuid)` aggregate.)
--   (3) `steward_event_in_ingest_window(cogmap, event)` — a boolean the write path uses to make the
--       advance server-verified: an advance target must be an event the cogmap actually ingests
--       (anchored to a context in its producer-intersection), not any global `kb_events.id`. This
--       guards the inverse failure — a blocked/empty tick moving the watermark past content it never
--       processed — and, by composing (1), inherits the leak fix: it can never green-light an advance
--       to an event from a context outside the intersection.
--
-- Changing a function's RETURNS TABLE signature requires DROP + CREATE (CREATE OR REPLACE cannot
-- change the return type). `steward_ingest_delta` is a `LANGUAGE sql` function with a quoted body, so
-- its callers (`steward_drift_sweep`) hold no recorded dependency on it — the DROP is safe and the
-- re-CREATE re-binds by name. `steward_drift_sweep` composes `steward_ingest_delta`, so the
-- intersection fix reaches the drift sweep too, with no change there.
--
-- ADDITIVE, additive-only-on-`main`: one column added to a function's output, two new functions, and
-- a narrowing (leak-closing) correction to how ingest scope is computed; no table altered, no column
-- dropped. Namespace-free (resolves against the connection's search_path = public); STABLE +
-- LANGUAGE sql so `sqlx::query!` callers stay compile-checked.

CREATE FUNCTION steward_team_contexts(p_cogmap uuid)
RETURNS TABLE(context_id uuid)
LANGUAGE sql STABLE AS $$
    -- Producer-INTERSECTION over the cogmap's joined teams (the context-grain sibling of
    -- resources_accessible_to_cogmap). A (team, context) row is a context that team can reach —
    -- team-OWNED (owner_table='kb_teams') or team-SHARED (kb_team_contexts). A context is in the
    -- cogmap's ingest scope only when it appears for EVERY joined team, so a context one team cannot
    -- reach can never be ingested (and thus never distilled into a map its members read). Conservative
    -- by design: excluding a context is safe (less ingested); including a cross-team one is the leak.
    WITH joined AS (
        SELECT team_id FROM kb_team_cogmaps WHERE cogmap_id = p_cogmap
    ),
    per_team AS (
        SELECT j.team_id, c.id AS context_id
          FROM joined j
          JOIN kb_contexts c ON c.owner_table = 'kb_teams' AND c.owner_id = j.team_id
        UNION
        SELECT j.team_id, ktc.context_id
          FROM joined j
          JOIN kb_team_contexts ktc ON ktc.team_id = j.team_id
    )
    SELECT pt.context_id
      FROM per_team pt
     GROUP BY pt.context_id
    HAVING count(DISTINCT pt.team_id) = (SELECT count(*) FROM joined)
       AND (SELECT count(*) FROM joined) > 0;
$$;

COMMENT ON FUNCTION steward_team_contexts(uuid) IS
    'A steward cogmap''s ingest scope: the producer-INTERSECTION of its joined teams'' reachable '
    'contexts (team-OWNED or team-SHARED), so a context is ingested only when EVERY joined team can '
    'reach it. Context-grain sibling of resources_accessible_to_cogmap; closes the cross-team leak a '
    'union would open on a multi-team cogmap. Default-closed on the empty join. The single definition '
    'of ingest scope, shared by steward_ingest_delta and steward_event_in_ingest_window.';

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
    -- True iff the event is one this cogmap actually ingests: a real kb_events row anchored to a
    -- context in the cogmap's producer-intersection (steward_team_contexts). Watermark-independent
    -- (the write path checks membership in the ingest scope, not position relative to the current
    -- cursor) — max_event_id is always in scope; a cross-team context's event never is.
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
