-- The staleness clock reads EITHER anchor kind.
--
-- `cogmap_staleness` (20260624000002:527-551) is an ON-READ aggregate, not a denormalized watermark:
-- it compares the stored materialization watermark against the latest event touching the anchor's
-- homed regions/edges. That contract is carried over here UNCHANGED. What changes is the key.
--
-- Design: internal/superpowers/specs/2026-08-23-anchor-shape-envelope-design.md §6.
--
-- THREE CHANGES FROM 20260624000002:527-551, and the first is the one this migration turns on.
--
-- 1. THE REGIONS ARM. It was keyed on `reg.cogmap_id = p_cogmap`
--    (20260624000002:538-541), a FK to kb_cogmaps that is NULL for every context region
--    (`kb_cogmap_regions.home_anchor_table` / `home_anchor_id` carry a context's regions instead --
--    see 20260823000010:66-67, where anchor_shape keys on exactly that pair). It now keys on the
--    anchor pair, which is index-covered: `idx_kb_cogmap_regions_anchor` on
--    (home_anchor_table, home_anchor_id) WHERE NOT is_folded.
--
--    THE TRAP THIS AVOIDS, restated because it is SILENT. Generalize the signature and leave the
--    regions arm on `cogmap_id`, and contexts do not error and do not return nulls. `latest_touch`
--    comes back NULL, so `latest_touch > materialized_at` is NULL, and the COALESCE
--    (20260624000002:549) falls through to `materialized_at IS NULL` -- FALSE for any context that
--    has materialized even once. Every context would report `is_stale = false`, permanently, and
--    nothing would go red. The witness is
--    `crates/temper-api/tests/context_orientation_test.rs::a_touched_context_reports_stale`: it
--    materializes a context AND THEN touches one of its regions with a later event, which is the
--    only shape of fixture that can tell the working function from the broken one.
--
-- 2. THE kb_edges ARM was ALREADY anchor-generic (20260624000002:543-545) -- it reads
--    `e.home_anchor_table` / `e.home_anchor_id`. Only its two literals move to the parameters. The
--    folded-inclusive scan and its covering index `idx_kb_edges_home_all` (20260708000008) are
--    deliberately untouched: a fold event advances the edge's last_event_id and staleness must keep
--    reporting it, which is why that arm carries no `is_folded` predicate. Same for the regions arm.
--
-- 3. THE `mat` CTE reads `shape_materialized_event_id` from whichever table `p_anchor_table` names,
--    using the same UNION ALL shape as anchor_shape's `clock` CTE (20260823000010:74-85) rather
--    than inventing a second way to read one column from one of two anchor tables. As before, an
--    anchor that does not exist yields ZERO rows from `mat`, so the cross join yields zero rows and
--    the function returns nothing -- the behaviour `cogmap_analytics` already depends on
--    (20260628000001:25-26: "cogmap_staleness yields exactly one row").
--
-- No gate here, matching the incumbent: staleness is a clock reading, and the gate lives in the
-- composers (cogmap_analytics, 20260628000001:77-78). Stale reads are allowed and LEGIBLE -- this
-- reports staleness, it never blocks on it.

CREATE FUNCTION anchor_staleness(p_anchor_table text, p_anchor_id uuid)
RETURNS TABLE(materialized_at timestamptz, latest_touch timestamptz, is_stale boolean)
LANGUAGE sql STABLE AS $$
    WITH mat AS (
        SELECT ev.occurred_at AS materialized_at
        FROM (
            SELECT c.shape_materialized_event_id AS eid FROM kb_contexts c
             WHERE p_anchor_table = 'kb_contexts' AND c.id = p_anchor_id
            UNION ALL
            SELECT m.shape_materialized_event_id FROM kb_cogmaps m
             WHERE p_anchor_table = 'kb_cogmaps' AND m.id = p_anchor_id
        ) a
        LEFT JOIN kb_events ev ON ev.id = a.eid
    ),
    touch AS (
        SELECT max(occurred_at) AS latest_touch FROM (
            SELECT ev.occurred_at FROM kb_cogmap_regions reg
              JOIN kb_events ev ON ev.id = reg.last_event_id
             WHERE reg.home_anchor_table = p_anchor_table
               AND reg.home_anchor_id    = p_anchor_id
            UNION ALL
            SELECT ev.occurred_at FROM kb_edges e
              JOIN kb_events ev ON ev.id = e.last_event_id
             WHERE e.home_anchor_table = p_anchor_table
               AND e.home_anchor_id    = p_anchor_id
        ) t
    )
    SELECT mat.materialized_at, touch.latest_touch,
           COALESCE(touch.latest_touch > mat.materialized_at, mat.materialized_at IS NULL)
    FROM mat, touch;
$$;

COMMENT ON FUNCTION anchor_staleness(text, uuid) IS
'ON-READ staleness for EITHER anchor kind (kb_contexts or kb_cogmaps): compares the anchor''s stored shape_materialized_event_id watermark against the latest event touching its homed regions and edges. Keyed on the anchor pair (home_anchor_table, home_anchor_id) -- the same key anchor_shape uses -- NOT on the vestigial kb_cogmap_regions.cogmap_id, which is NULL for every context region and would silently report every context permanently fresh. Folded regions and edges are deliberately included: a fold advances last_event_id and is a touch. Ungated by design; the gate lives in the composers. Yields exactly one row for an anchor that exists, zero rows for one that does not. Staleness is LEGIBLE -- reported, never blocking.';

-- The cogmap name stays, delegating, so `cogmap_analytics` (20260628000001:63-77) and the scenario
-- runner's `SELECT is_stale FROM cogmap_staleness($1)`
-- (crates/temper-substrate/src/scenario/runner.rs:486, compile-time-checked and therefore pinned to
-- this exact signature and column set) keep working untouched. CREATE OR REPLACE is legal here
-- precisely because the return type does not move; the body is what moves.
--
-- The delegation does not change the answer for a cogmap. The regions arm now reads the anchor pair
-- where it used to read cogmap_id, and for cogmap regions those two are the SAME value by
-- construction at both ends: the writer sets both in one INSERT
-- (crates/temper-substrate/src/write.rs:688-696) and every pre-anchor row was backfilled
-- `home_anchor_table = 'kb_cogmaps', home_anchor_id = cogmap_id` (20260712000030:44, re-run for
-- stragglers at 20260712000040:21-22). It is equality by backfill and convention, though, not by
-- constraint -- nothing in the schema FORCES the pair to agree with cogmap_id -- which is one reason
-- this migration declares shape-breaking rather than additive.
CREATE OR REPLACE FUNCTION cogmap_staleness(p_cogmap uuid)
RETURNS TABLE(materialized_at timestamptz, latest_touch timestamptz, is_stale boolean)
LANGUAGE sql STABLE AS $$
    SELECT s.materialized_at, s.latest_touch, s.is_stale
      FROM anchor_staleness('kb_cogmaps', p_cogmap) s;
$$;

SELECT declare_migration(
    20260823000020,
    'shape-breaking',
    'The staleness clock is generalized to either anchor kind (task 01a02ebd-c153-7d22-acb6-d9fdec1b0f16). New anchor_staleness(p_anchor_table text, p_anchor_id uuid) returns the same three columns as cogmap_staleness (materialized_at, latest_touch, is_stale) but keys the regions arm on the anchor pair (kb_cogmap_regions.home_anchor_table / home_anchor_id) instead of the vestigial cogmap_id FK, which is NULL for every context region; the kb_edges arm was already anchor-generic and only its two literals move to the parameters; and the mat CTE reads shape_materialized_event_id from kb_contexts or kb_cogmaps via the same UNION ALL shape anchor_shape''s clock CTE uses. Left un-generalized this would have failed SILENTLY rather than loudly: latest_touch NULL makes latest_touch > materialized_at NULL, the COALESCE falls through to materialized_at IS NULL, and every materialized context reports is_stale = false forever. Classed shape-breaking, not additive, because cogmap_staleness(uuid) is REPLACED with a delegating wrapper: the name, argument and column set are byte-identical, so the compile-time-checked caller at crates/temper-substrate/src/scenario/runner.rs:486 and cogmap_analytics both keep working untouched, but the rows it returns for a cogmap are now computed off the anchor pair rather than off cogmap_id. For every cogmap region in the substrate those two are the same value -- the writer sets both in one INSERT (crates/temper-substrate/src/write.rs:688-696) and the pre-anchor rows were backfilled at 20260712000030:44 and again at 20260712000040:21 -- but that equality is convention and backfill, not a schema constraint, so a row where they disagree would give a binary paired with the prior migration a different staleness verdict for the same map. Silence about that is not a classification. No gate is added: staleness stays a clock reading and the access gate stays in the composers, matching the incumbent. Folded regions and edges remain included, because a fold advances last_event_id and is a touch (20260708000008). Design: internal/superpowers/specs/2026-08-23-anchor-shape-envelope-design.md.'
);
