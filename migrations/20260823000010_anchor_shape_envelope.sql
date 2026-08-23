-- The shape read gains an anchor-level envelope, so an empty answer can say why it is empty.
--
-- Non-additive on purpose: Postgres cannot CREATE OR REPLACE across a return-type change, so this
-- DROPs and re-CREATEs. Design: internal/superpowers/specs/2026-08-23-anchor-shape-envelope-design.md
--
-- The region select, the member gate and the cogmap self-read exemption are carried over UNCHANGED
-- from 20260713000050:99-130. The argument for each is at 20260713000050:41-77 and still holds.
-- What is new is: (a) `regs` no longer applies p_lens (it is the ALL-LENS set, so `population` is a
-- real denominator rather than a restatement of the row count), and (b) the LEFT JOIN ON true, which
-- guarantees exactly one row even for an empty or unreadable anchor — the sentinel the envelope
-- speaks from.

DROP FUNCTION IF EXISTS anchor_shape(text, uuid, text, uuid, uuid);

CREATE FUNCTION anchor_shape(
    p_anchor_table   text,
    p_anchor_id      uuid,
    p_principal_kind text,
    p_principal_id   uuid,
    p_lens           uuid DEFAULT NULL
)
RETURNS TABLE(
    population       integer,
    emptiness        text,
    materialized_at  timestamptz,
    region_id        uuid,
    lens_id          uuid,
    salience         double precision,
    content_cohesion double precision,
    label            text,
    member_count     integer
)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        -- Computed ONCE for both the rows and the population. Empty for a non-profile principal.
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    ),
    gate AS (
        -- Always exactly one row (no FROM), which is what keeps `env` non-empty for an anchor that
        -- is unreadable OR does not exist. Disjunction carried over from 20260713000050:126-132.
        SELECT (
            (p_principal_kind = 'profile'
                 AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
            OR (p_principal_kind = 'cogmap'
                 AND p_anchor_table = 'kb_cogmaps'
                 AND p_principal_id = p_anchor_id)
        ) AS readable
    ),
    regs AS (
        SELECT reg.id AS region_id, reg.lens_id, reg.salience, reg.content_cohesion,
               COALESCE(reg.label, seen.rep_title) AS label,
               CASE
                   WHEN p_principal_kind = 'cogmap' THEN reg.member_count
                   ELSE seen.visible_members
               END AS member_count
        FROM kb_cogmap_regions reg
        CROSS JOIN LATERAL (
            SELECT count(*)::int AS visible_members,
                   (array_agg(r.title ORDER BY m.affinity DESC NULLS LAST))[1] AS rep_title
            FROM kb_cogmap_region_members m
            JOIN vis v ON v.resource_id = m.member_id
            JOIN kb_resources r ON r.id = m.member_id AND r.is_active
            WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ) seen
        WHERE reg.home_anchor_table = p_anchor_table
          AND reg.home_anchor_id    = p_anchor_id
          AND NOT reg.is_folded
          -- A region you can see nothing in is not a region you can see. (Cogmap arm exempt.)
          AND (p_principal_kind = 'cogmap' OR seen.visible_members > 0)
          AND (SELECT readable FROM gate)
        -- DELIBERATELY no p_lens predicate: `regs` is the ALL-LENS set. The lens narrows the ROWS
        -- returned, below; it must not narrow the denominator.
    ),
    clock AS (
        SELECT a.eid, ev.occurred_at AS materialized_at
        FROM (
            SELECT c.shape_materialized_event_id AS eid FROM kb_contexts c
             WHERE p_anchor_table = 'kb_contexts' AND c.id = p_anchor_id
            UNION ALL
            SELECT m.shape_materialized_event_id FROM kb_cogmaps m
             WHERE p_anchor_table = 'kb_cogmaps' AND m.id = p_anchor_id
        ) a
        LEFT JOIN kb_events ev ON ev.id = a.eid
    ),
    env AS (
        SELECT
            CASE WHEN g.readable THEN (SELECT count(*)::int FROM regs) ELSE 0 END AS population,
            CASE WHEN g.readable THEN (SELECT k.materialized_at FROM clock k) ELSE NULL END
                AS materialized_at,
            -- Precedence is load-bearing, in two places.
            --
            -- The FIRST arm guards the field's own contract: `emptiness` explains an EMPTY row set
            -- and nothing else. Without it, a readable anchor that holds visible regions but was
            -- never materialized returns rows AND 'never_clustered' -- a named cause attached to a
            -- non-empty answer, which contradicts the column's documented meaning. That fact is not
            -- lost by suppressing it here: `materialized_at` is NULL for exactly that anchor, which
            -- is the field that is actually about the clock. (An unreadable anchor never reaches
            -- this arm -- it has no rows.)
            --
            -- Then 'never_clustered' MUST precede 'nothing_visible', or a never-clustered anchor
            -- reports 'nothing_visible' and the distinction this function exists to draw is lost.
            CASE
                WHEN (SELECT count(*) FROM regs rr
                       WHERE p_lens IS NULL OR rr.lens_id = p_lens) > 0 THEN NULL
                WHEN NOT g.readable                        THEN 'unreadable_or_absent'
                WHEN (SELECT k.eid FROM clock k) IS NULL   THEN 'never_clustered'
                WHEN (SELECT count(*) FROM regs) = 0       THEN 'nothing_visible'
                ELSE 'lens_narrowed'
            END AS emptiness
        FROM gate g
    )
    SELECT env.population, env.emptiness, env.materialized_at,
           r.region_id, r.lens_id, r.salience, r.content_cohesion, r.label, r.member_count
    FROM env
    LEFT JOIN (SELECT * FROM regs rr WHERE p_lens IS NULL OR rr.lens_id = p_lens) r ON true
    ORDER BY r.salience DESC NULLS LAST, r.region_id;
$$;

COMMENT ON FUNCTION anchor_shape(text, uuid, text, uuid, uuid) IS
'Surface-tier read of an anchor''s materialized regions plus an anchor-level envelope, for EITHER anchor kind. Returns AT LEAST ONE ROW always: an empty or unreadable anchor yields a single row with region_id NULL, carrying the envelope. `population` is the member-gated region count across ALL lenses (a real denominator under a lens filter); `emptiness` names why the row set is empty (unreadable_or_absent / never_clustered / nothing_visible / lens_narrowed, NULL when non-empty); `materialized_at` is the shape watermark, NULL when never clustered. Deny and absent collapse into ONE arm and disclose neither population nor clock — no existence oracle. The gate is inside the SQL. The member gate, label fallback and cogmap self-read exemption are carried unchanged from 20260713000050.';

-- The wrapper is dead (no SQL or Rust caller reaches it), but DROPping anchor_shape strands it, and
-- a non-additive migration should not also be a silent removal. Pinned to the six original columns
-- by explicit select-list. Retiring the name belongs to M3 (20260713000010:185).
CREATE OR REPLACE FUNCTION cogmap_shape(
    p_cogmap uuid, p_principal_kind text, p_principal_id uuid, p_lens uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, salience double precision,
              content_cohesion double precision, label text, member_count integer)
LANGUAGE sql STABLE AS $$
    SELECT s.region_id, s.lens_id, s.salience, s.content_cohesion, s.label, s.member_count
      FROM anchor_shape('kb_cogmaps', p_cogmap, p_principal_kind, p_principal_id, p_lens) s
     WHERE s.region_id IS NOT NULL;
$$;

SELECT declare_migration(
    20260823000010,
    'shape-breaking',
    'The shape read gains an anchor-level envelope (task 01a02ebd-c153-7d22-acb6-d9fdec1b0f16). anchor_shape is DROPped and re-CREATEd with three envelope columns prepended (population, emptiness, materialized_at) and now returns AT LEAST ONE ROW always -- an empty or unreadable anchor yields a single row with region_id NULL, which is the mechanism that lets an empty answer state its cause. Shape-breaking on both counts: the return type changes, so a binary paired with the prior migration selects columns that no longer exist in that order; and the guaranteed sentinel row means an old binary reading region_id AS "region_id!" would hit a NULL it declared impossible. population is the member-gated region count across ALL lenses, so it is a real denominator under a lens filter rather than a restatement of the row count -- the regs CTE deliberately omits p_lens for exactly this reason. Deny and absent collapse into one emptiness arm disclosing neither population nor clock, so the envelope is not an existence oracle. The member gate, label fallback and cogmap self-read exemption are carried over unchanged from 20260713000050. cogmap_shape is re-CREATEd pinned to its six original columns: it has no callers, but dropping anchor_shape strands it, and a non-additive migration should not also be a silent removal -- retiring that name belongs to M3. Design: internal/superpowers/specs/2026-08-23-anchor-shape-envelope-design.md.'
);
