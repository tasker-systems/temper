-- T6 — two clocks: telos drift as a first-class signal, and the snapshot that makes it computable.
--
-- Spec §3.5. Formation is expensive and depends on membership inputs; salience is a handful of cosines
-- and depends on the telos. In a cogmap they move together, so running the readouts only inside
-- `materialize` is fine. In a CONTEXT they come apart — goals open and close without any region's
-- membership changing — and the two input sets are provably disjoint: formation reads members, edges,
-- facets (`property_key='facet'` only) and embeddings; liveness reads `temper-stage` rows and
-- `advances` edges. Closing a task rewrites a `temper-stage` row, which is in the second set and not
-- the first, so membership CANNOT move while the telos MUST. (T5 pinned this in CI:
-- `context_telos_salience.rs::closing_a_goal_moves_salience_without_changing_region_membership`.)
--
-- This migration adds the cheap clock's two missing pieces: a drift READING, and — for a cogmap — the
-- snapshot to read it against.
--
-- ADDITIVE ONLY (the `main` invariant, DEPLOYING.md): new nullable column, new defaulted column, and
-- CREATE OR REPLACE of two functions. Every existing lens row takes the column default, and every
-- pre-T6 `lens_created` event replays to exactly the row it already produced.

-- ---------------------------------------------------------------------------
-- 1. Cogmaps gain the telos snapshot too.
--
-- T2 (20260712000030) added `telos_centroid` to kb_contexts ONLY, reasoning that "a cogmap does not
-- carry one because its telos is a DECLARED resource whose embedding can be read directly." True for
-- reading the telos — but drift is not a reading of the telos, it is a reading of how far the telos has
-- MOVED since the shape was last computed, and that needs a snapshot on BOTH anchors. A cogmap's telos
-- moves whenever its charter is edited; without this column that motion is invisible, and
-- `anchor_telos_drift` could only ever return NULL for a cogmap — failing T6's acceptance ("returns a
-- sane value for both anchor kinds") and denying cogmaps the cheap clock for no reason.
--
-- Nullable, exactly like the context column: NULL means "never materialized", which is a state the
-- gate must be able to see (there are no regions to refresh yet — the formation clock owns that trip).
ALTER TABLE kb_cogmaps
    ADD COLUMN telos_centroid vector(768);

COMMENT ON COLUMN kb_cogmaps.telos_centroid IS
    'The telos vector as of the last materialize — the snapshot anchor_telos_drift() compares the '
    'current telos against (spec §3.5). For a cogmap the telos is its charter''s pooled embedding, so '
    'this moves when the charter is edited. NULL before the first materialize.';

-- ---------------------------------------------------------------------------
-- 2. The drift gate's threshold, lens-resident.
--
-- A tuning constant, so it lives in SQL beside the other lens calibration rather than as a Rust
-- constant — same reasoning as the telos weights (§3.4) and the wayfind blend.
--
-- The default is small ON PURPOSE, and it is not arbitrary. The telos is a liveness-WEIGHTED centroid:
--
--     telos = Σ(liveness_g · v_g) / Σ(liveness_g),   liveness_g = damper_g · sqrt(mass_g)
--     mass_g = Σ_tasks  stage_weight · exp(−idle_days / halflife)
--
-- When wall-clock time advances by Δt and nothing else happens, EVERY task's idle grows by the same
-- Δt, so every goal's mass scales by the same factor exp(−Δt/halflife); sqrt preserves that uniformity
-- and the dampers are time-independent, so every liveness scales by one common factor — which CANCELS
-- in the normalisation. **Pure time passage cannot rotate the telos at all.** Drift is therefore not a
-- noisy signal that needs a generous deadband: it is ~0 (float noise) until the goal census actually
-- changes, and jumps by orders of magnitude more when it does.
--
-- So epsilon exists to clear float noise, not to suppress a drifting baseline. Making it large would
-- buy nothing and cost the thing the cheap clock is FOR — a task closing must move salience now, not
-- eventually. (`context_telos_drift.rs` pins both halves: a uniform time advance stays under epsilon;
-- one task changing stage goes over it.)
ALTER TABLE kb_cogmap_lenses
    ADD COLUMN telos_drift_epsilon DOUBLE PRECISION NOT NULL DEFAULT 1e-6;

COMMENT ON COLUMN kb_cogmap_lenses.telos_drift_epsilon IS
    'Cosine-distance threshold above which a telos move triggers a salience-only refresh (spec §3.5, '
    'the cheap clock). Small by design: the telos is scale-invariant to pure time decay, so drift is '
    'float noise until the goal census genuinely changes.';

-- ---------------------------------------------------------------------------
-- 3. The ledger projection must carry the new column.
--
-- THIS IS THE STEP THAT KEEPS BEING MISSED. T4 widened `_project_lens_created` and its migration noted
-- "T2 added TWO groups of columns and this function was missing BOTH" — T2 had actually added THREE,
-- and the telos group was still absent, so a lens minted through the ledger silently took column
-- defaults and any tuned value was dropped on the floor. Tunable in the DDL, untunable in practice,
-- with no error and every test green. T5 fixed that; this migration adds a column, so it inherits the
-- obligation. The function is re-created WHOLE below (not patched) precisely so the column list can be
-- read against `\d kb_cogmap_lenses` in one glance.
--
-- COALESCE on the new key for the append-only reason: `kb_events` is immutable, so every `lens_created`
-- event written before T6 carries no `{telos,drift_epsilon}` and must still replay to exactly the row
-- it already produced. The default here, the SQL column default, and `TelosConstants::drift_epsilon`
-- in Rust are one calibration declared in three places that must agree.
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
         damper_paused, damper_completed, telos_drift_epsilon,
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
            COALESCE((p_payload#>>'{telos,drift_epsilon}')::double precision, 1e-6),
            (p_payload#>>'{salience,telos}')::double precision,
            (p_payload#>>'{salience,ref}')::double precision,
            (p_payload#>>'{salience,central}')::double precision,
            (p_payload->>'resolution')::double precision,
            p_event, v_occurred);
    RETURN v_lens;
END;
$$;

-- ---------------------------------------------------------------------------
-- 4. Telos drift — the cheap clock's reading, and a first-class queryable signal.
--
-- How far has this anchor's PURPOSE moved since its SHAPE was last computed? The context analogue of
-- `cogmap_staleness`, and the gate condition of spec §3.5's clock 1:
--
--     d = 1 − cos(telos_now(A), A.telos_centroid)          -- one cosine
--     if d > epsilon:  refresh salience (no clustering); A.telos_centroid := telos_now(A)
--
-- pgvector's `<=>` IS cosine distance (1 − cosine similarity), so it is exactly the spec's `d` — no
-- arithmetic of our own, and no chance of getting the sign backwards.
--
-- NULL, deliberately, in three cases, all meaning "there is no drift question to ask yet":
--   * never materialized (telos_centroid IS NULL) — no regions exist to refresh; the FORMATION clock
--     owns that trip, and a NULL here must NOT read as "no drift, skip".
--   * no telos (a context with no live, embedded goals) — nothing to compare.
--   * unknown anchor kind — the CASE falls through.
-- The caller compares `drift > epsilon`, and NULL > x is NULL is not-true, so a NULL correctly declines
-- to fire the cheap clock rather than firing it spuriously.
CREATE OR REPLACE FUNCTION anchor_telos_drift(
    p_anchor_table varchar, p_anchor_id uuid, p_lens uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_contexts' THEN (
            SELECT t.v <=> c.telos_centroid
              FROM kb_contexts c
              CROSS JOIN LATERAL (
                  SELECT anchor_telos_embedding(p_anchor_table, p_anchor_id, p_lens) AS v
              ) t
             WHERE c.id = p_anchor_id
               AND c.telos_centroid IS NOT NULL
               AND t.v IS NOT NULL
        )
        WHEN 'kb_cogmaps' THEN (
            SELECT t.v <=> m.telos_centroid
              FROM kb_cogmaps m
              CROSS JOIN LATERAL (
                  SELECT anchor_telos_embedding(p_anchor_table, p_anchor_id, p_lens) AS v
              ) t
             WHERE m.id = p_anchor_id
               AND m.telos_centroid IS NOT NULL
               AND t.v IS NOT NULL
        )
    END;
$$;

COMMENT ON FUNCTION anchor_telos_drift(varchar, uuid, uuid) IS
    'How far an anchor''s telos has moved since its shape was last materialized — cosine distance '
    'between the current telos and the snapshot (spec §3.5). Gate 1 of the two-clock trigger, and a '
    'queryable staleness signal in its own right. NULL before the first materialize, or when the '
    'anchor has no telos: both mean "no drift question to ask", and NULL > epsilon is not true, so a '
    'NULL declines to fire the cheap clock.';

-- ---------------------------------------------------------------------------
-- 5. The telos snapshot must be a PROJECTION, not a side-write.
--
-- `kb_cogmaps` and `kb_contexts` are projection tables: every column must be reproducible by replaying
-- the ledger through the `_project_*` halves (`replay_roundtrip.rs` compares them byte-for-byte). T5
-- wrote `kb_contexts.telos_centroid` DIRECTLY from the materialize path, which is not an event
-- projection — so replay could not reproduce it. That went unnoticed only because `kb_contexts` is not
-- in `PROJECTION_DUMPS`; giving cogmaps the same column (step 1) surfaced it immediately.
--
-- Recomputing the telos at replay time is NOT the fix. Liveness carries an `exp(−idle/halflife)` term,
-- so replay's later `now()` gives every goal a weight differing in the ~16th digit. The centroid is
-- mathematically invariant to that (the uniform factor cancels — see step 2), but the float rounding is
-- not PROVABLY identical after the cast to `vector(768)`, and a test that is only probabilistically
-- byte-identical is a flake, not a proof.
--
-- So the act RECORDS the telos it computed, and the projection writes the recorded bytes. Replay then
-- reproduces it exactly, with no float question at all — and the payload gains an honest statement it
-- should always have made: *at watermark W, this anchor's telos was V*.
--
-- `jsonb_exists` rather than COALESCE, because "key absent" and "key present and null" are different
-- facts: a pre-T6 event carries no key and must leave the column alone, while a context whose last live
-- goal just closed genuinely HAS no telos and must snapshot NULL. COALESCE would conflate them and pin
-- a stale vector forever.
CREATE OR REPLACE FUNCTION _project_region_materialized(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_table text := coalesce(p_payload->>'home_anchor_table', 'kb_cogmaps');
    v_id    uuid := coalesce((p_payload->>'home_anchor_id')::uuid, (p_payload->>'cogmap_id')::uuid);
    v_has   boolean := jsonb_exists(p_payload, 'telos_centroid');
    v_telos vector := (p_payload->>'telos_centroid')::vector;
BEGIN
    IF v_table = 'kb_cogmaps' THEN
        UPDATE kb_cogmaps SET
            shape_materialized_event_id = p_event,
            telos_centroid = CASE WHEN v_has THEN v_telos ELSE telos_centroid END
         WHERE id = v_id;
    ELSIF v_table = 'kb_contexts' THEN
        UPDATE kb_contexts SET
            shape_materialized_event_id = p_event,
            telos_centroid = CASE WHEN v_has THEN v_telos ELSE telos_centroid END
         WHERE id = v_id;
    ELSE
        RAISE EXCEPTION 'region_materialized: unknown home_anchor_table %', v_table;
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 6. `salience_refreshed` — the cheap clock's act.
--
-- The cheap clock re-arms the drift gate by re-snapshotting the telos. That is a write to a projection
-- table, so by the same argument as step 5 it needs an event behind it, or replay diverges again.
--
-- **It is deliberately NOT a formation event.** `formation_touched_count_since` counts
-- `STRUCTURAL_EVENTS ∪ CONTENT_EVENTS` (replay.rs) and this name is in neither, so a salience refresh
-- can never advance the threshold the EXPENSIVE clock gates on. That is the whole point of two clocks:
-- if the cheap trip nudged the anchor toward a re-cluster, every closing task would eventually force
-- the very formation pass the separation exists to avoid.
--
-- It also does not touch `shape_materialized_event_id` — that is the FORMATION watermark, and salience
-- is not formation. The two clocks keep two watermarks.
--
-- Free benefit: telos drift becomes auditable. The ledger now records every time an anchor's purpose
-- moved far enough to matter, with the vector it moved to.
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('salience_refreshed', NULL, 1)
ON CONFLICT (name) DO NOTHING;

CREATE OR REPLACE FUNCTION _project_salience_refreshed(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_table text := p_payload->>'home_anchor_table';
    v_id    uuid := (p_payload->>'home_anchor_id')::uuid;
    v_telos vector := (p_payload->>'telos_centroid')::vector;
BEGIN
    -- Only the telos snapshot. The region readouts this act recomputed are DERIVED compute (Rust-side,
    -- like the region rows themselves), re-provable by re-deriving — not replayed from the payload.
    IF v_table = 'kb_cogmaps' THEN
        UPDATE kb_cogmaps  SET telos_centroid = v_telos WHERE id = v_id;
    ELSIF v_table = 'kb_contexts' THEN
        UPDATE kb_contexts SET telos_centroid = v_telos WHERE id = v_id;
    ELSE
        RAISE EXCEPTION 'salience_refreshed: unknown home_anchor_table %', v_table;
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION salience_refresh(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('salience_refreshed', p_emitter,
                          p_payload->>'home_anchor_table',
                          (p_payload->>'home_anchor_id')::uuid,
                          p_payload);
    PERFORM _project_salience_refreshed(v_ev, p_payload);
    RETURN v_ev;
END;
$$;

COMMENT ON FUNCTION salience_refresh(jsonb, uuid) IS
    'The cheap clock''s act (spec §3.5): re-snapshot an anchor''s telos after a salience-only refresh. '
    'Deliberately NOT a formation event — it appears in neither STRUCTURAL_EVENTS nor CONTENT_EVENTS, '
    'so it never advances the threshold the expensive clock gates on.';
