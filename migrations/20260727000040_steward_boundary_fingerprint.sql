-- The steward's ingest trigger counts events INSIDE a boundary that itself moves uncounted.
--
-- THE DEFECT. `steward_ingest_delta` answers "did anything change?" by counting `kb_events` whose
-- `producing_anchor_id` falls inside `steward_team_contexts(p_cogmap)` — team-OWNED contexts ∪
-- contexts SHARED to a joined team or its ancestors (`20260716000010:58-84`). That set is the
-- window frame, and the window frame moves with no event of its own:
--
--   * SHARED, the sharp case. `context_service::share` is a bare
--     `INSERT INTO kb_team_contexts … ON CONFLICT DO NOTHING`
--     (`crates/temper-services/src/services/context_service.rs:421`) — no event is emitted, by
--     an explicit product decision the module states in its own header: "Context creation is a plain
--     INSERT (no event emission — product decision 5: contexts are infrastructure)"
--     (`context_service.rs:10-11`). The resources already living in the newly-shared context carry
--     `resource_created` events BELOW the cogmap's watermark, so the count over the widened frame is
--     `new_resources = 0` and the steward never ticks. An entire corpus becomes distillable and the
--     map never learns it exists.
--   * UNSHARED, the same hole in reverse. `unshare` is a bare `DELETE` (`context_service.rs:456`).
--     The frame SHRINKS with no event either — material the map has distilled is now out of scope.
--   * OWNED. Ownership moves by `context_reassigned`, which is ONE event for N resources, and not
--     even of the type the gate counts: the gate filters `type_name = 'resource_created'`
--     (`20260716000010:110`) against a default threshold of 5. One event of the wrong type clears
--     nothing.
--   * ANCESTRY. `steward_team_contexts` reaches shares through `team_ancestors`, so re-parenting a
--     team moves the frame without touching `kb_team_contexts` or `kb_contexts` at all.
--
-- THE FIX IS A STATE COMPARISON, NOT A NEW EVENT. Product decision 5 is not overturned here — this
-- migration stops DEPENDING on an event that decision guarantees will never exist. Each completed
-- steward run stores a digest of its boundary; the next tick recomputes the digest and compares.
-- Events remain the "what landed inside the frame" signal; the fingerprint is the "did the frame
-- itself move" signal. They are different questions and now have different mechanisms.
--
-- NULL IS MEANINGFUL, AND IT IS WHY THERE IS NO BACKFILL. `steward_boundary_fingerprint` is
-- nullable and NULL means "never snapshotted", which `IS DISTINCT FROM` renders as MOVED. Every
-- pre-existing cogmap therefore fires exactly once and then settles on its stored digest. Backfilling
-- a digest at migration time would be the strictly worse choice: it would silently swallow every
-- boundary move that has ALREADY happened and never been distilled, which is the entire population
-- this fix exists for. Over-trigger is the safe default here for the same reason
-- `steward_team_contexts` is a UNION and not an intersection (`20260716000010:27-32`) — this is a
-- trigger, not a gate, so a wasted tick is cheap and a dropped tick is invisible.
--
-- THE EMPTY-SET TRAP, and why the zero-row digest must not be NULL. An aggregate over zero rows
-- returns NULL, so a naive `sha256(string_agg(...))` over a cogmap with no contexts in scope yields
-- NULL — indistinguishable from "never snapshotted", so that cogmap would report `boundary_moved`
-- forever and re-fire on every tick for the rest of its life. "No contexts in scope" is a real,
-- stable state and gets a real, stable digest. The `coalesce(string_agg(…), '')` INSIDE the hash is
-- this repo's existing answer to exactly this trap — `20260715000030:57-58`,
-- `20260726000030:58`, `canonical_functions.sql:601` all hash the coalesced-to-empty aggregate
-- rather than coalescing the hash afterwards — so the empty scope digests to sha256('')
-- (`e3b0c442…`), a genuine member of the digest space that no non-empty scope can collide with
-- (any non-empty aggregate is at least one 36-character uuid).
--
-- THE CARDINALITY TRAP in `steward_ingest_delta`. The fingerprint is read through a scalar subquery
-- over a CTE, NOT joined in as `FROM win CROSS JOIN fp`. The join shape looks tidier and is wrong:
-- projecting `fp.digest` alongside the aggregates would demand a `GROUP BY`, and a grouped query over
-- an EMPTY `win` returns ZERO rows instead of one. Every caller — `steward_service::ingest_delta`
-- uses `fetch_one` — depends on this function returning exactly one row for a cogmap with no new
-- events. The aggregate-only form over a bare `FROM win` keeps that guarantee, and the scalar
-- subqueries ride along without changing the query's cardinality.
--
-- ONE DEFINITION OF "MOVED", read by both surfaces. `boundary_moved` is computed once, inside
-- `steward_ingest_delta`, and returned. Neither `steward_drift_sweep` nor the Rust read surface
-- restates the comparison — the same "REUSE, not re-implement … one source of truth for 'what counts
-- as drift'" rule `20260705000002:3-4` states for the sweep's use of the delta. The threshold gate
-- already has two implementations (SQL `20260705000002:29`, and Rust's `exceeds_threshold` in
-- `steward_service::ingest_delta`) and that pair is precisely the drift this rule exists to prevent;
-- a second copy is not added here. The Rust surface ORs in the SQL-computed `boundary_moved` rather
-- than recomputing the digest comparison.
--
-- WHY THE SWEEP IS THE LOAD-BEARING HALF. `DbBackend::steward_dispatch_tick` →
-- `steward_service::drift_sweep` → `steward_drift_sweep` is the path the cron actually runs and the
-- only one that enqueues work. The Rust `exceeds_threshold` on the single-cogmap read is an
-- informational surface. A fix landing only there would fix nothing that runs.
--
-- ORDER BY: `boundary_moved DESC, new_resources DESC, cogmap_id`. A boundary-moved row typically has
-- `new_resources = 0` — the MINIMUM of the incumbent sort key — so under the old ordering it would
-- sort behind every map with a single new resource. The classes are not symmetric and that is what
-- decides it: a boundary move is SELF-CLEARING (the run that consumes it stores the digest and the
-- row does not come back), while count-drift RECURS every tick until it is distilled. Putting the
-- self-clearing class first bounds its latency to one tick and costs the recurring class nothing.
-- Note this sweep has no LIMIT and `steward_dispatch_tick` enqueues EVERY returned row before the
-- cap is applied downstream by `workflow_job_service::claim`, so the order here is not an admission
-- gate — unlike `audit_drift_sweep`, where the cap sits on the ORDER BY and starvation is structural
-- (`20260727000010:68-72`). The trailing `m.cogmap_id` makes the sort total; `20260705000002` calls
-- itself a "Deterministic drift sweep" while sorting on a single non-unique key, and
-- `20260727000010:89` ends its ordering with `k.finding_id` for the same reason.
--
-- DROP + CREATE, NOT `CREATE OR REPLACE`. `20260727000010:41-44` argues for replacing in place —
-- correct there, where the return columns were unchanged. Both functions here GAIN return columns,
-- and `CREATE OR REPLACE FUNCTION` cannot change a function's return type, so DROP is not a
-- preference. It is the same constraint `20260716000010:48-52` hit and the same remedy: both are
-- `LANGUAGE sql` with quoted bodies, so callers hold no recorded dependency and the re-CREATE
-- re-binds by name. `steward_ingest_delta` is dropped explicitly rather than left as a 2-argument
-- overload — an overload would let a stale caller keep resolving to the blind version.
--
-- NOT FIXED HERE — THE STORE-BACK IS UNREACHABLE IN EXACTLY THE CASE THIS EXISTS FOR. The digest is
-- written by `DbBackend::advance_steward_watermark`, which sets `steward_watermark_event_id` and
-- `steward_boundary_fingerprint` in one UPDATE. But that path takes a NON-optional
-- `AdvanceStewardWatermark::event_id` and gates it on `steward_event_in_ingest_window` before the
-- write. A boundary-move-only tick has `new_resources = 0`, `new_events = 0` and therefore
-- `max_event_id = NULL` — the run has no event id to pass, and any substitute is 404'd by the
-- hygiene gate. So the canonical case (a context shared whose resources all predate the watermark)
-- distills, cannot record its digest, and re-fires on the NEXT tick, and every tick after, until
-- some unrelated event happens to land in the frame and supplies a `max_event_id`. That is a
-- distillation hot loop, not a missed tick. Storing the fingerprint must be possible with no event
-- to advance to — either an optional `event_id` on the advance command whose NULL skips the window
-- check and the watermark half of the UPDATE, or a separate boundary-only store-back. The bound is
-- real but this migration cannot close it: the write lives in Rust, not in a SQL function, so there
-- is no in-schema seam to extend.
--
-- ADDITIVE, additive-only-on-`main`: one new nullable column (existing rows read NULL = "never
-- snapshotted"), one new function, and two functions gaining output columns. No column dropped, no
-- data rewritten. Namespace-free (resolves against the connection's search_path = public); STABLE +
-- LANGUAGE sql so `sqlx::query!` callers stay compile-checked.

ALTER TABLE kb_cogmaps
    ADD COLUMN steward_boundary_fingerprint TEXT;

COMMENT ON COLUMN kb_cogmaps.steward_boundary_fingerprint IS
    'Boundary cursor for the team-self-cognition steward, the companion to '
    'steward_watermark_event_id: the steward_boundary_fingerprint(cogmap) digest of '
    'steward_team_contexts(cogmap) as a completed steward run observed it. NULL = never snapshotted, '
    'which steward_ingest_delta renders as boundary_moved = true (over-trigger by default — a '
    'wasted tick is cheap, a dropped one is invisible), so no backfill is needed or wanted. Exists '
    'because the change-detection frame moves with NO event: share/unshare are bare '
    'INSERT/DELETE on kb_team_contexts (contexts are infrastructure, product decision 5) and team '
    're-parenting moves the frame through team_ancestors without touching a context at all.';

-- ---------------------------------------------------------------------------------------------
-- The digest of the change-detection frame.
-- ---------------------------------------------------------------------------------------------

CREATE FUNCTION steward_boundary_fingerprint(p_cogmap uuid)
RETURNS text
LANGUAGE sql STABLE AS $$
    -- A deterministic digest of steward_team_contexts(p_cogmap)'s output, ordered by context_id so
    -- the value depends on the SET and not on the plan that produced it. Membership is all that is
    -- hashed: this is the frame's identity, not its contents — what landed INSIDE the frame is
    -- steward_ingest_delta's event count, a separate question with a separate mechanism.
    --
    -- coalesce INSIDE the hash, never outside (`20260715000030:57-58`, `20260726000030:58`): an
    -- aggregate over zero rows is NULL, and a NULL digest would be indistinguishable from the
    -- column's "never snapshotted" NULL, so an empty-scope cogmap would report moved forever. Empty
    -- scope is a real, stable state and gets sha256('') — a real digest, and one no non-empty scope
    -- can produce (the smallest non-empty aggregate is a 36-character uuid).
    --
    -- The ',' delimiter is belt-and-braces: uuid::text is fixed-width, so no two distinct sets can
    -- concatenate to the same string even undelimited. It is kept because it makes the hashed value
    -- legible as a list when this is debugged by hand.
    SELECT encode(sha256(convert_to(
             coalesce(string_agg(t.context_id::text, ',' ORDER BY t.context_id), ''), 'UTF8')), 'hex')
      FROM steward_team_contexts(p_cogmap) t;
$$;

COMMENT ON FUNCTION steward_boundary_fingerprint(uuid) IS
    'sha256 hex digest of a steward cogmap''s CHANGE-DETECTION frame — steward_team_contexts(cogmap) '
    'ordered by context_id. Compared against kb_cogmaps.steward_boundary_fingerprint to detect a '
    'frame that moved with no event to count (share/unshare are bare INSERT/DELETE on '
    'kb_team_contexts; team re-parenting moves the frame via team_ancestors). Membership only — what '
    'landed inside the frame is steward_ingest_delta''s event count. An EMPTY scope digests to '
    'sha256('''') and never to NULL: NULL is reserved for "never snapshotted", and conflating the two '
    'would re-fire an empty-scope cogmap forever.';

-- ---------------------------------------------------------------------------------------------
-- The delta, now answering both questions.
-- ---------------------------------------------------------------------------------------------

DROP FUNCTION steward_ingest_delta(uuid, uuid);

CREATE FUNCTION steward_ingest_delta(p_cogmap uuid, p_watermark uuid, p_fingerprint text)
RETURNS TABLE(new_resources bigint, new_events bigint, max_event_id uuid,
              boundary_fingerprint text, boundary_moved boolean)
LANGUAGE sql STABLE AS $$
    -- One window definition (the CTE), then the counts and the latest id off it. The counting logic
    -- is UNCHANGED from `20260716000010`; this adds two columns beside it, it does not alter a count.
    WITH win AS (
        SELECT e.id, et.name AS type_name
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
         WHERE e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id IN (SELECT context_id FROM steward_team_contexts(p_cogmap))
           AND (p_watermark IS NULL OR e.id > p_watermark)
    ),
    -- MATERIALIZED so the digest is computed once regardless of how many times it is read below.
    -- Two references already defeat inlining, but declaring it makes one-call-per-invocation the
    -- floor rather than a planner behaviour — the lesson `20260727000010:60-61` paid for.
    fp AS MATERIALIZED (
        SELECT steward_boundary_fingerprint(p_cogmap) AS digest
    )
    SELECT
        count(*) FILTER (WHERE type_name = 'resource_created')::bigint AS new_resources,
        count(*)::bigint                                              AS new_events,
        -- uuidv7 byte order is time order, so the DESC-first id is the newest event in the window;
        -- NULL when the window is empty (no events since the watermark) — the "nothing to advance
        -- to" signal. ORDER BY … LIMIT 1 rather than max(id): there is no max(uuid) aggregate.
        (SELECT id FROM win ORDER BY id DESC LIMIT 1)                 AS max_event_id,
        -- The frame as it stands NOW — the value a completed run should store back.
        (SELECT digest FROM fp)                                       AS boundary_fingerprint,
        -- IS DISTINCT FROM, not <>: p_fingerprint is NULL for a cogmap that has never been
        -- snapshotted, and `NULL <> digest` is NULL, which a WHERE clause reads as "not moved" —
        -- exactly inverting the intended default. IS DISTINCT FROM makes never-snapshotted MOVED.
        (p_fingerprint IS DISTINCT FROM (SELECT digest FROM fp))      AS boundary_moved
      -- Scalar subqueries over `fp`, NOT `FROM win CROSS JOIN fp`. Projecting fp.digest as a column
      -- would require a GROUP BY, and a grouped query over an empty `win` returns ZERO rows where
      -- callers (steward_service::ingest_delta, fetch_one) require exactly one. Aggregate-only over
      -- a bare `FROM win` always yields one row.
      FROM win;
$$;

COMMENT ON FUNCTION steward_ingest_delta(uuid, uuid, text) IS
$c$How much has changed for a steward cogmap since its last completed run — along BOTH axes.

INSIDE the frame: new_resources / new_events / max_event_id, counted over kb_events anchored to
steward_team_contexts(cogmap) after p_watermark. Unchanged from `20260716000010`.

THE FRAME ITSELF: boundary_fingerprint is the current steward_boundary_fingerprint(cogmap) — the
value a completed run stores back — and boundary_moved is p_fingerprint IS DISTINCT FROM it. Added
by `20260727000040` because the frame moves with no event to count: share/unshare are bare
INSERT/DELETE on kb_team_contexts (contexts are infrastructure, product decision 5), context
ownership moves by a single context_reassigned of a type the gate does not count, and team
re-parenting moves the frame through team_ancestors. A newly-shared context's resources carry
resource_created events BELOW the watermark, so the count over the widened frame is 0 and the
steward never ticks.

boundary_moved is computed HERE and only here. steward_drift_sweep and the Rust read surface consume
it; neither restates the comparison (`20260705000002:3-4`, one source of truth for what counts as
drift).

Authorization is NOT enforced here — this is a pure read. The service layer gates on
anchor_readable_by_profile before calling it.$c$;

-- ---------------------------------------------------------------------------------------------
-- The sweep the cron actually runs.
-- ---------------------------------------------------------------------------------------------

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
      -- The stored fingerprint rides in beside the stored watermark, the same way and from the same
      -- row: two cursors of one completed run, read together.
      CROSS JOIN LATERAL steward_ingest_delta(m.cogmap_id,
                                              cm.steward_watermark_event_id,
                                              cm.steward_boundary_fingerprint) d
     -- Either axis admits the map. `d.boundary_moved` is READ, never recomputed — the comparison has
     -- one home (steward_ingest_delta) so the sweep and the Rust surface cannot drift apart.
     WHERE d.new_resources >= p_threshold
        OR d.boundary_moved
     -- Boundary-moved first: that class is SELF-CLEARING (the run that consumes it stores the digest
     -- and the row does not return) and its new_resources is typically 0, the minimum of the second
     -- key — so under the old ordering alone it would sort behind every map with one new resource.
     -- Count-drift RECURS every tick until distilled, so yielding it a position costs it nothing.
     -- This is presentation, not admission: the sweep has no LIMIT and steward_dispatch_tick
     -- enqueues every returned row before the cap is applied by workflow_job_service::claim.
     -- m.cogmap_id makes the sort total (`20260727000010:89` ends the same way, and no tiebreak at
     -- all is what let `20260705000002` call itself "Deterministic" while sorting on one
     -- non-unique key).
     ORDER BY d.boundary_moved DESC, d.new_resources DESC, m.cogmap_id;
$$;

COMMENT ON FUNCTION steward_drift_sweep(uuid, bigint) IS
$c$Team-joined cogmaps this principal can read that the steward should look at, most-drifted-first.

Re-created by `20260727000040` to admit a SECOND drift axis. The gate is now
`new_resources >= p_threshold OR boundary_moved`: a cogmap whose change-detection frame moved
qualifies even at a zero count, because the frame moves with no event to count (share/unshare are
bare INSERT/DELETE on kb_team_contexts) and a newly-shared context's resources carry
resource_created events BELOW the watermark. boundary_moved is READ from steward_ingest_delta, never
recomputed here.

ORDER BY boundary_moved DESC, new_resources DESC, cogmap_id — the boundary-moved class is
self-clearing and its count is typically 0 (the minimum of the second key), so it leads. This is
presentation, not admission: there is no LIMIT and steward_dispatch_tick enqueues every returned row
before the downstream cap.

The candidate set and its read gate are unchanged: steward_candidate_cogmaps, scoped through
anchor_readable_by_profile — the steward app-principal's broad read comes from grants /
access_mode=open, NOT a bypass.$c$;
