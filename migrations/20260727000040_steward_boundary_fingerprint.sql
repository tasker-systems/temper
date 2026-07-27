-- The steward's ingest trigger counts events INSIDE a boundary that itself moves uncounted.
--
-- `steward_ingest_delta` counts kb_events anchored inside `steward_team_contexts(cogmap)`. That set
-- is the window frame, and the frame moves with NO event: share/unshare are bare INSERT/DELETE on
-- kb_team_contexts (product decision 5 — contexts are infrastructure), ownership moves by a single
-- `context_reassigned` of a type the gate does not count, and team re-parenting moves it through
-- `team_ancestors`. A newly-shared context's resources carry `resource_created` events BELOW the
-- watermark, so the count over the widened frame is 0 and the steward never ticks.
--
-- Fix: store a digest of the frame per completed run and compare. Events stay the "what landed
-- inside" signal; the fingerprint is the "did the frame move" signal.
--
-- NULL = never snapshotted = MOVED, which is why there is NO BACKFILL: every pre-existing cogmap
-- fires once and settles. Backfilling would swallow every boundary move that has already happened
-- and never been distilled — the entire population this exists for.
--
-- ADDITIVE: one nullable column, one new function, two functions gaining output columns.
--
-- Rationale, the traps below, and the two hot loops found while building this:
-- docs/superpowers/specs/2026-07-27-steward-boundary-fingerprint-design.md

ALTER TABLE kb_cogmaps
    ADD COLUMN steward_boundary_fingerprint TEXT;

COMMENT ON COLUMN kb_cogmaps.steward_boundary_fingerprint IS
    'Boundary cursor for the team-self-cognition steward, companion to steward_watermark_event_id: '
    'the steward_boundary_fingerprint(cogmap) digest of steward_team_contexts(cogmap) as a completed '
    'run observed it. NULL = never snapshotted, which steward_ingest_delta renders as '
    'boundary_moved = true — so no backfill is needed or wanted. Exists because the frame moves with '
    'no event to count (share/unshare are bare INSERT/DELETE on kb_team_contexts).';

CREATE FUNCTION steward_boundary_fingerprint(p_cogmap uuid)
RETURNS text
LANGUAGE sql STABLE AS $$
    -- Membership only, ordered so the digest depends on the SET and not on the plan.
    -- coalesce INSIDE the hash: a zero-row aggregate is NULL, and a NULL digest is indistinguishable
    -- from the column's "never snapshotted" NULL, so an empty scope would re-fire forever. Empty
    -- scope is a real state and gets sha256(''). Same form as `20260715000030:57-58`.
    SELECT encode(sha256(convert_to(
             coalesce(string_agg(t.context_id::text, ',' ORDER BY t.context_id), ''), 'UTF8')), 'hex')
      FROM steward_team_contexts(p_cogmap) t;
$$;

COMMENT ON FUNCTION steward_boundary_fingerprint(uuid) IS
    'sha256 hex digest of a steward cogmap''s change-detection frame — steward_team_contexts(cogmap) '
    'ordered by context_id. Compared against kb_cogmaps.steward_boundary_fingerprint to detect a '
    'frame that moved with no event to count. Membership only; what landed inside the frame is '
    'steward_ingest_delta''s event count. An EMPTY scope digests to sha256('''') and never to NULL — '
    'NULL is reserved for "never snapshotted", and conflating the two re-fires that cogmap forever.';

-- `CREATE OR REPLACE` cannot change a return type and both functions gain columns, so DROP+CREATE.
-- Dropped explicitly rather than left as a 2-arg overload, which would let a stale caller keep
-- resolving to the blind version.
DROP FUNCTION steward_ingest_delta(uuid, uuid);

CREATE FUNCTION steward_ingest_delta(p_cogmap uuid, p_watermark uuid, p_fingerprint text)
RETURNS TABLE(new_resources bigint, new_events bigint, max_event_id uuid,
              boundary_fingerprint text, boundary_moved boolean)
LANGUAGE sql STABLE AS $$
    -- Counting logic UNCHANGED from `20260716000010`; this adds two columns beside it.
    WITH win AS (
        SELECT e.id, et.name AS type_name
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
         WHERE e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id IN (SELECT context_id FROM steward_team_contexts(p_cogmap))
           AND (p_watermark IS NULL OR e.id > p_watermark)
    ),
    -- MATERIALIZED makes one-call-per-invocation the floor, not a planner behaviour.
    fp AS MATERIALIZED (
        SELECT steward_boundary_fingerprint(p_cogmap) AS digest
    )
    SELECT
        count(*) FILTER (WHERE type_name = 'resource_created')::bigint AS new_resources,
        count(*)::bigint                                              AS new_events,
        -- uuidv7 byte order is time order, so DESC-first is newest. NULL when the window is empty —
        -- the "nothing to advance to" signal. No max(uuid) aggregate exists.
        (SELECT id FROM win ORDER BY id DESC LIMIT 1)                 AS max_event_id,
        (SELECT digest FROM fp)                                       AS boundary_fingerprint,
        -- IS DISTINCT FROM, not <>: p_fingerprint is NULL for a never-snapshotted cogmap, and
        -- `NULL <> digest` is NULL, which a WHERE reads as "not moved" — inverting the default and
        -- rendering this inert.
        (p_fingerprint IS DISTINCT FROM (SELECT digest FROM fp))      AS boundary_moved
      -- Scalar subqueries over `fp`, NOT `FROM win CROSS JOIN fp`: projecting the digest as a column
      -- would need a GROUP BY, and a grouped query over an empty `win` returns ZERO rows where
      -- callers (fetch_one) require exactly one.
      FROM win;
$$;

COMMENT ON FUNCTION steward_ingest_delta(uuid, uuid, text) IS
$c$How much has changed for a steward cogmap since its last completed run, along BOTH axes.

INSIDE the frame: new_resources / new_events / max_event_id, counted over kb_events anchored to
steward_team_contexts(cogmap) after p_watermark. Unchanged from `20260716000010`.

THE FRAME ITSELF: boundary_fingerprint is the current steward_boundary_fingerprint(cogmap) — the
value a completed run stores back — and boundary_moved is p_fingerprint IS DISTINCT FROM it. Added
by `20260727000040` because the frame moves with no event to count.

boundary_moved is computed HERE and only here; steward_drift_sweep and the Rust read surface consume
it without restating the comparison (`20260705000002:3-4`, one source of truth for what counts as
drift).

Authorization is NOT enforced here — a pure read. The service layer gates on
anchor_readable_by_profile before calling it.$c$;

DROP FUNCTION steward_drift_sweep(uuid, bigint);

CREATE FUNCTION steward_drift_sweep(p_principal uuid, p_threshold bigint)
RETURNS TABLE(cogmap_id uuid, watermark uuid, new_resources bigint, new_events bigint,
              boundary_fingerprint text, boundary_moved boolean)
LANGUAGE sql STABLE AS $$
    SELECT m.cogmap_id,
           cm.steward_watermark_event_id AS watermark,
           d.new_resources,
           d.new_events,
           d.boundary_fingerprint,
           d.boundary_moved
      FROM steward_candidate_cogmaps(p_principal) m
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
$c$Team-joined cogmaps this principal can read that the steward should look at, most-drifted-first.

Re-created by `20260727000040` to admit a SECOND drift axis. The gate is now
`new_resources >= p_threshold OR boundary_moved`: a cogmap whose change-detection frame moved
qualifies even at a zero count, because the frame moves with no event to count and a newly-shared
context's resources carry resource_created events BELOW the watermark. boundary_moved is READ from
steward_ingest_delta, never recomputed here.

ORDER BY boundary_moved DESC, new_resources DESC, cogmap_id — the boundary-moved class is
self-clearing and its count is typically 0, so it leads. Presentation, not admission: there is no
LIMIT and steward_dispatch_tick enqueues every returned row before the downstream cap.

The candidate set and its read gate are unchanged: steward_candidate_cogmaps, scoped through
anchor_readable_by_profile — the steward app-principal's broad read comes from grants /
access_mode=open, NOT a bypass.$c$;
