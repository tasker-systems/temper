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
--   (1) `steward_team_contexts(cogmap)` — the cogmap's CHANGE-DETECTION scope, as a first-class
--       function so the delta and the advance check below share ONE definition. It is the UNION of
--       every joined team's reachable contexts: the candidate set of contexts where a resource/event
--       change could have landed that this cogmap might need to distill.
--
--       This is deliberately NOT a visibility gate — a crucial distinction. Its only consumers count
--       events and pick the latest event id (the watermark); no resource content is ever read through
--       it. Cross-team leak-safety is enforced DOWNSTREAM, at the resource grain, by the
--       cogmap-principal distillation read (`resources_accessible_to_cogmap` — the producer-
--       INTERSECTION the access model mandates: canonical_schema.sql, "Cognitive maps reach workflow
--       content ONLY through producer-intersection over their joined teams"). The steward distills AS
--       the cogmap, so that intersection binds every resource it reads; the team-binding of the M2M
--       credential is deliberately not the axis (multi-team creds are a known future case).
--
--       Union, not intersection, is correct HERE precisely because this is a trigger, not a gate.
--       Over-approximation is safe: at worst a wasted tick that distills nothing (the downstream
--       intersection drops it). A narrower intersection would UNDER-trigger — miss a change to a
--       resource that IS distillable (e.g. one visible to the cogmap via a resource-grant whose home
--       context is not shared to every joined team) — and silently stale the map. A dropped tick is
--       invisible; a wasted one is cheap.
--
--       A team reaches a context it OWNS directly (ownership does not inherit down the team DAG) or
--       one SHARED to it or any of its ANCESTORS (shares inherit down via `team_ancestors`, the same
--       up-traversal vis_team / anchor_readable_by_profile use for context reach).
--   (2) `steward_ingest_delta` gains `max_event_id uuid` — the newest `kb_events.id` in the delta
--       window (NULL when the window is empty). `kb_events.id` is uuidv7, whose byte order IS time
--       order, so `ORDER BY id DESC LIMIT 1` is the latest event — exactly the cursor `advance`
--       should move to. (`ORDER BY … LIMIT 1`, not `max(id)`: there is no `max(uuid)` aggregate.)
--   (3) `steward_event_in_ingest_window(cogmap, event)` — a boolean the write path uses for watermark
--       HYGIENE: an advance target must be a real `kb_events.id` within the cogmap's change-detection
--       scope (anchored to a context some joined team can reach), not any global event id — so the
--       cursor can only move to an event this tick could have observed, never a resource id or an
--       unrelated event. It composes (1), so it tracks the same union scope. Not a content gate;
--       content leak-safety lives downstream (see (1)).
--
-- Changing a function's RETURNS TABLE signature requires DROP + CREATE (CREATE OR REPLACE cannot
-- change the return type). `steward_ingest_delta` is a `LANGUAGE sql` function with a quoted body, so
-- its callers (`steward_drift_sweep`) hold no recorded dependency on it — the DROP is safe and the
-- re-CREATE re-binds by name. `steward_drift_sweep` composes `steward_ingest_delta`, so the
-- change-detection scope reaches the drift sweep too, with no change there.
--
-- ADDITIVE, additive-only-on-`main`: one column added to a function's output plus two new functions;
-- no table altered, no column dropped. Namespace-free (resolves against the connection's search_path
-- = public); STABLE + LANGUAGE sql so `sqlx::query!` callers stay compile-checked.

CREATE FUNCTION steward_team_contexts(p_cogmap uuid)
RETURNS TABLE(context_id uuid)
LANGUAGE sql STABLE AS $$
    -- The UNION of every joined team's reachable contexts: the candidate set of contexts where a
    -- resource/event change could have landed that this cogmap might need to distill. This is a
    -- CHANGE-DETECTION scope, NOT a visibility gate — it only feeds event counts and the latest-event
    -- watermark; no resource content is read through it. Cross-team leak-safety is enforced DOWNSTREAM
    -- at the resource grain by the cogmap-principal distillation read (resources_accessible_to_cogmap,
    -- the producer-intersection). Union is deliberate: as a "did anything I might need to look at
    -- change?" trigger, over-approximation is safe (a wasted tick), while an intersection would
    -- UNDER-trigger — miss a distillable-but-grant-visible change whose home context isn't shared to
    -- every team — and silently stale the map.
    --
    -- A team reaches a context it OWNS directly (ownership does not inherit down the team DAG)...
    SELECT c.id AS context_id
      FROM kb_team_cogmaps tc
      JOIN kb_contexts c ON c.owner_table = 'kb_teams' AND c.owner_id = tc.team_id
     WHERE tc.cogmap_id = p_cogmap
    UNION
    -- ...or one SHARED to it or any of its ANCESTORS (shares inherit down the team DAG via
    -- team_ancestors, the same up-traversal vis_team / anchor_readable_by_profile use for context reach).
    SELECT ktc.context_id
      FROM kb_team_cogmaps tc
      CROSS JOIN LATERAL team_ancestors(tc.team_id) a
      JOIN kb_team_contexts ktc ON ktc.team_id = a.team_id
     WHERE tc.cogmap_id = p_cogmap;
$$;

COMMENT ON FUNCTION steward_team_contexts(uuid) IS
    'A steward cogmap''s CHANGE-DETECTION scope: the UNION of its joined teams'' reachable contexts (a '
    'team reaches a context it OWNS, or one SHARED to it or an ANCESTOR via team_ancestors). Feeds '
    'steward_ingest_delta''s counts + max_event_id (the watermark) and steward_event_in_ingest_window''s '
    'advance check — it is NOT a visibility gate and reads no resource content. Cross-team leak-safety '
    'is enforced downstream at the resource grain by the cogmap-principal distillation read '
    '(resources_accessible_to_cogmap, the producer-intersection). Union not intersection on purpose: '
    'over-approximate change detection is safe, a narrower scope would under-trigger and stale the map.';

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
    -- True iff the event is within this cogmap's change-detection scope: a real kb_events row anchored
    -- to a context some joined team can reach (steward_team_contexts). Watermark HYGIENE, not a content
    -- gate — it keeps the cursor from jumping to a resource id or an unrelated event. Position-
    -- independent (membership in the scope, not rank vs the cursor); max_event_id is always in scope.
    SELECT EXISTS (
        SELECT 1
          FROM kb_events e
         WHERE e.id = p_event
           AND e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id IN (SELECT context_id FROM steward_team_contexts(p_cogmap))
    );
$$;

COMMENT ON FUNCTION steward_event_in_ingest_window(uuid, uuid) IS
    'True iff p_event is within p_cogmap''s change-detection scope (anchored to a context some joined '
    'team can reach, steward_team_contexts). Watermark hygiene for advance_steward_watermark — keeps '
    'the cursor from moving to an unrelated event or a resource id. Not a content gate.';
