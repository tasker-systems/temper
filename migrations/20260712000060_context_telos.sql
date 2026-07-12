-- T5 — a context's telos: goal liveness from the task census.
--
-- Spec §3.4. A cogmap orients by a DECLARED charter (kb_cogmaps.telos_resource_id). A context has no
-- charter and should not be made to author one — its purpose is legible from the goals it already
-- holds, weighted by the state of the work beneath them.

-- ─────────────────────────────────────────────────────────────────────────────
-- 1. `_project_lens_created`: the THIRD group of T2's columns.
--
-- T4 (20260712000050) widened this function and its own comment says "T2 added TWO groups of columns
-- to kb_cogmap_lenses, and this function was missing BOTH". That was wrong: T2 added THREE. The
-- kernel trio (w_cos/knn_k/cos_floor) and the anchor pair (home_anchor_*) were fixed there; the
-- telos/liveness constants below — the ones THIS migration makes load-bearing — were not, and are
-- still absent from the INSERT. Every lens minted through the ledger silently takes the column
-- defaults for them.
--
-- Today that is invisible, because the defaults equal the intended values. It stops being invisible
-- the moment anyone tunes a constant: `lens_create` would accept the payload, drop the value on the
-- floor, and the lens would behave as though the tuning never happened. That is precisely T5's
-- acceptance criterion ("constants land as lens columns, tunable") failing silently.
--
-- COALESCE, not a bare read: `kb_events` is append-only, so the pre-T5 `lens_created` events are
-- immortal and carry no telos keys. The COALESCE defaults mirror the column defaults (as amended in
-- §2 below) and the serde defaults on `payloads::LensCreated` — one value, declared in three places
-- that must agree, so replaying an old event reproduces exactly the row that already exists.
CREATE OR REPLACE FUNCTION _project_lens_created(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_lens uuid := (p_payload->>'lens_id')::uuid;
        v_cogmap uuid := (p_payload->>'cogmap_id')::uuid;   -- NULL for a global lens
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_cogmap_lenses
        (id, cogmap_id, home_anchor_table, home_anchor_id, name, selection_kind,
         w_express, w_contains, w_leads_to, w_near, w_prop,
         w_cos, knn_k, cos_floor, kappa_anchor_prior,
         telos_halflife_days, sw_in_progress, sw_backlog, sw_done,
         damper_paused, damper_completed,
         s_telos, s_ref, s_central, resolution, asserted_by_event_id, created)
    VALUES (v_lens,
            v_cogmap,
            CASE WHEN v_cogmap IS NULL THEN NULL ELSE 'kb_cogmaps' END,
            v_cogmap,
            p_payload->>'name', p_payload->>'selection_kind',
            (p_payload#>>'{weights,express}')::double precision,
            (p_payload#>>'{weights,contains}')::double precision,
            (p_payload#>>'{weights,leads_to}')::double precision,
            (p_payload#>>'{weights,near}')::double precision,
            (p_payload#>>'{weights,prop}')::double precision,
            COALESCE((p_payload#>>'{weights,cos}')::double precision, 0.0),
            COALESCE((p_payload->>'knn_k')::int, 12),
            COALESCE((p_payload->>'cos_floor')::double precision, 0.55),
            COALESCE((p_payload->>'kappa_anchor_prior')::double precision, 0.0),
            COALESCE((p_payload#>>'{telos,halflife_days}')::double precision, 30.0),
            COALESCE((p_payload#>>'{telos,sw_in_progress}')::double precision, 1.0),
            COALESCE((p_payload#>>'{telos,sw_backlog}')::double precision, 0.35),
            COALESCE((p_payload#>>'{telos,sw_done}')::double precision, 0.0),
            COALESCE((p_payload#>>'{telos,damper_paused}')::double precision, 0.3),
            COALESCE((p_payload#>>'{telos,damper_completed}')::double precision, 0.4),
            (p_payload#>>'{salience,telos}')::double precision,
            (p_payload#>>'{salience,ref}')::double precision,
            (p_payload#>>'{salience,central}')::double precision,
            (p_payload->>'resolution')::double precision,
            p_event, v_occurred);
    RETURN v_lens;
END;
$$;

-- ─────────────────────────────────────────────────────────────────────────────
-- 2. Recalibrate `sw_done`: 0.15 → 0.0. The spec's own fixture refutes 0.15.
--
-- §3.4 argues that `done` must be "weighted low and decay, so a graveyard does not masquerade as a
-- heartbeat", and picks 0.15. Run against the live @me/temper census the spec nominates as its
-- calibration fixture, 0.15 produces very nearly the INVERSE of the ranking §3.4 demands:
--
--     goal            | spec (sw_done=0.15) | required by §3.4 / §5
--     ----------------+---------------------+---------------------------------
--     Maintenance     |  2.55  — RANK #1    | "faintly warm — a container, not a driver"
--     Temper Cloud    |  1.19  — rank #6    | "must fall OUT of the telos"
--     path-to-alpha   |  1.13  — rank #7    | "must fall OUT of the telos"
--     Graph Atlas     |  0.96  — rank #9    | "must rank at the top"
--
-- The mechanism is arithmetic, and the decay cannot fix it. A weight of 0.15 is small; SIXTY-EIGHT of
-- them is not. And because marking a task done *touches* it, `exp(-idle/halflife)` is ~1.0 for
-- exactly the tasks that just closed — so a goal that is finishing looks maximally alive. Shortening
-- the halflife does not help either (measured: at 7d Maintenance is still warmer than Graph Atlas),
-- because Maintenance closes tasks CONTINUOUSLY: no decay rate distinguishes "a steady drip of
-- completed chores" from "alive". The count is the problem, not the age.
--
-- At sw_done = 0.0 every one of the fixture's labels is satisfied: Temper Cloud and path-to-alpha go
-- to exactly 0.0 and leave the telos; Maintenance falls to 0.98 — kept faintly warm by its 3 BACKLOG
-- items, which is precisely the "container, not a driver" the fixture asks for; Graph Atlas and
-- Substrate-kernel rank at the top alongside the two arcs actually under active development.
--
-- This is not a retreat from the spec — it is the spec's own sentence, which its number contradicted:
-- "Old completed work is *history*, not *purpose*." Closing a task is still rewarded, by removing it
-- from the open set. The column survives and stays tunable; only its calibration changes.
ALTER TABLE kb_cogmap_lenses ALTER COLUMN sw_done SET DEFAULT 0.0;

-- Safe to UPDATE a row of an otherwise-IMMUTABLE table: these columns have never been READ by any
-- code path (T5 is their first consumer), so no observer can have depended on the prior value. And
-- because §1's COALESCE default now also says 0.0, replaying the original `lens_created` events
-- reproduces exactly these rows — the ledger and the projection still agree.
UPDATE kb_cogmap_lenses SET sw_done = 0.0 WHERE sw_done = 0.15;

COMMENT ON COLUMN kb_cogmap_lenses.sw_done IS
    'Stage weight for a `done` task in the goal-liveness census (spec §3.4). CALIBRATED TO 0.0. A '
    'positive weight here is summed over EVERY closed task under a goal, and closing a task touches '
    'it — so any positive value lets a large graveyard of recently-finished work outrank a goal with '
    'one live task. Measured on the @me/temper fixture: at 0.15 the Maintenance goal (68 done, 0 '
    'in-progress) ranked #1 of 32. Old completed work is history, not purpose. Tunable, but raise it '
    'only with the census in front of you.';

-- ─────────────────────────────────────────────────────────────────────────────
-- 3. Goal liveness from the task census.
--
--   liveness(g) = damper(g) × sqrt( Σ  stage_weight(stage(t)) · exp(−idle_days(t)/halflife) )
--                                  t advances g
--
-- Reads liveness off the layer that reliably TERMINATES (tasks), and lets the layer that does not
-- (goals) inherit it. The declared `temper-status` survives only as a damper: it cannot resurrect a
-- goal with no work, nor kill one with a task in progress. Prod shows it stale in both directions.
--
-- `sqrt` compresses scale so an 84-task container tilts against a 4-task new goal without swamping
-- it — goal scale infers itself from the work beneath, so nobody has to declare epic-vs-milestone.
--
-- `NOT is_folded` on EVERY kb_properties read is load-bearing. The table's unique index is
-- (owner, key, VALUE) WHERE NOT is_folded — superseded rows are retained, folded. Omit the filter and
-- a task that has moved backlog → in-progress → done counts in all THREE stage buckets at once.
--
-- STABLE, not IMMUTABLE: `now()` is an input. That is correct and deliberate — this feeds SALIENCE, a
-- readout, never formation. Affinity and `membership_fingerprint` must stay deterministic; salience is
-- expected to move with the clock. That divergence IS the two-clock model (spec §3.5).
CREATE OR REPLACE FUNCTION context_goal_liveness(p_context uuid, p_lens uuid)
RETURNS TABLE(goal_id uuid, liveness double precision) LANGUAGE sql STABLE AS $$
    WITH l AS (
        SELECT telos_halflife_days, sw_in_progress, sw_backlog, sw_done,
               damper_paused, damper_completed
        FROM kb_cogmap_lenses WHERE id = p_lens
    ),
    goals AS (
        SELECT r.id,
               -- absent status reads as `active`: the damper damps, it never gates.
               coalesce((SELECT p.property_value #>> '{}'
                           FROM kb_properties p
                          WHERE p.owner_table = 'kb_resources' AND p.owner_id = r.id
                            AND p.property_key = 'temper-status' AND NOT p.is_folded),
                        'active') AS declared
          FROM kb_resources r
          JOIN kb_resource_homes h  ON h.resource_id = r.id
                                   AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_context
          JOIN kb_properties dt     ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
                                   AND dt.property_key = 'doc_type' AND NOT dt.is_folded
                                   AND dt.property_value #>> '{}' = 'goal'
         WHERE r.is_active
    ),
    mass AS (
        SELECT g.id, g.declared,
               coalesce(sum(
                   CASE ts.stage
                       WHEN 'in-progress' THEN l.sw_in_progress
                       WHEN 'backlog'     THEN l.sw_backlog
                       WHEN 'done'        THEN l.sw_done
                       ELSE 0.0                         -- cancelled, and any unknown stage
                   END
                   * exp(- (extract(epoch FROM (now() - t.updated)) / 86400.0)
                         / l.telos_halflife_days)
               ), 0.0) AS m
          FROM goals g
          CROSS JOIN l
          -- the `advances` edge is minted task → goal as (leads_to, forward, 'advances')
          LEFT JOIN kb_edges e      ON e.target_table = 'kb_resources' AND e.target_id = g.id
                                   AND e.edge_kind = 'leads_to' AND e.label = 'advances'
                                   AND NOT e.is_folded
          LEFT JOIN kb_resources t  ON t.id = e.source_id AND t.is_active
          LEFT JOIN LATERAL (
              SELECT p.property_value #>> '{}' AS stage
                FROM kb_properties p
               WHERE p.owner_table = 'kb_resources' AND p.owner_id = t.id
                 AND p.property_key = 'temper-stage' AND NOT p.is_folded
          ) ts ON true
         GROUP BY g.id, g.declared
    )
    SELECT m.id,
           CASE m.declared
               WHEN 'paused'    THEN l.damper_paused
               WHEN 'completed' THEN l.damper_completed
               ELSE 1.0
           END * sqrt(m.m)
      FROM mass m CROSS JOIN l;
$$;

COMMENT ON FUNCTION context_goal_liveness(uuid, uuid) IS
    'Liveness of each goal homed in a context, read off the census of tasks that `advances` it (spec '
    '§3.4). A goal is as real as the work beneath it: zero live tasks means zero contribution, '
    'whatever `temper-status` claims.';

-- ─────────────────────────────────────────────────────────────────────────────
-- 4. The telos embedding, one function with two branches.
--
--   kb_cogmaps  → the charter resource's pooled chunk embeddings (unchanged behavior — this is the
--                 body of cogmap_region_telos_alignment's `telos` CTE, lifted verbatim).
--   kb_contexts → the liveness-weighted centroid of the context's goals.
--
-- pgvector here exposes only vector⊗vector (`\do *` → no `vector * float8`, no division), so the
-- scalar weight is applied by elementwise-multiplying against a filled vector. `vector_dims` rather
-- than a hardcoded 768 so this survives an embedding-model change.
--
-- Goals with NO live embedded chunks drop out of BOTH numerator and denominator, via the JOIN — a
-- deliberate graceful degradation, not an accident. Prod carries 7 such goals (created through MCP
-- before the post-embed flow existed); they contribute liveness but no vector, and a re-embed backfill
-- would simply fold them back in. Normalising by the sum over CONTRIBUTING goals only keeps the
-- centroid a true weighted mean of what actually landed.
--
-- Zero live goals (or zero embedded ones) → sum over an empty set → NULL. That is the graceful path:
-- telos_alignment goes NULL, `coalesce(…, 0)` applies in populate_readouts, and salience falls back
-- to reference-standing + centrality.
CREATE OR REPLACE FUNCTION anchor_telos_embedding(
    p_anchor_table varchar, p_anchor_id uuid, p_lens uuid)
RETURNS vector LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps' THEN (
            SELECT avg(ch.embedding)
              FROM kb_cogmaps c
              JOIN kb_chunks ch        ON ch.resource_id = c.telos_resource_id AND ch.is_current
              JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded
             WHERE c.id = p_anchor_id
        )
        WHEN 'kb_contexts' THEN (
            WITH contributing AS (
                SELECT gl.liveness AS w, ge.v
                  FROM context_goal_liveness(p_anchor_id, p_lens) gl
                  JOIN LATERAL (
                      SELECT avg(ch.embedding) AS v
                        FROM kb_chunks ch
                        JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded
                       WHERE ch.resource_id = gl.goal_id AND ch.is_current
                  ) ge ON ge.v IS NOT NULL
                 WHERE gl.liveness > 0.0
            ),
            tot AS (SELECT sum(w) AS s FROM contributing)
            SELECT sum(c.v * array_fill(c.w / tot.s, ARRAY[vector_dims(c.v)])::vector)
              FROM contributing c CROSS JOIN tot
             WHERE tot.s > 0.0
        )
    END;
$$;

COMMENT ON FUNCTION anchor_telos_embedding(varchar, uuid, uuid) IS
    'The telos vector for either anchor kind (spec §3.4). A cogmap declares its telos (a charter '
    'resource); a context COMPUTES one from the liveness-weighted centroid of its goals. NULL when a '
    'context has no live, embedded goals — callers must coalesce.';

-- ─────────────────────────────────────────────────────────────────────────────
-- 5. Region ↔ telos alignment, dispatched on the anchor.
--
-- The cogmap branch DELEGATES to cogmap_region_telos_alignment rather than reimplementing it. That
-- function is untouched by this migration and stays the regression floor the whole arc is measured
-- against (spec §5: "every existing scenario fixture must produce identical region membership and
-- identical fingerprints"). Reimplementing it here — even 'equivalently' — would put the floor at
-- risk for no gain.
CREATE OR REPLACE FUNCTION anchor_region_telos_alignment(
    p_region uuid, p_anchor_table varchar, p_anchor_id uuid, p_lens uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps'  THEN cogmap_region_telos_alignment(p_region, p_anchor_id)
        WHEN 'kb_contexts' THEN (
            SELECT 1 - (r.centroid <=> t.v)
              FROM kb_cogmap_regions r
              CROSS JOIN LATERAL (
                  SELECT anchor_telos_embedding(p_anchor_table, p_anchor_id, p_lens) AS v
              ) t
             WHERE r.id = p_region AND t.v IS NOT NULL
        )
    END;
$$;

COMMENT ON FUNCTION anchor_region_telos_alignment(uuid, varchar, uuid, uuid) IS
    'Cosine of a region centroid against its anchor''s telos (spec §3.4). Dispatches on anchor kind; '
    'the kb_cogmaps branch delegates to cogmap_region_telos_alignment unchanged, preserving the '
    'byte-identical cogmap regime that is this arc''s regression floor.';
