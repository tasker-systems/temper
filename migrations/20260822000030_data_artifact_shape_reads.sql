-- Data-artifact shape registry enumeration reads — visibility-gated retrieval of declared shapes.
--
-- Beat C, Task 5 of the shape registry plan. The write path (Beat A, Task 1) shipped in
-- 20260822000010; the verdict read-model (Beat C, Task 4) shipped in 20260822000020. This
-- migration adds the two read surfaces that make the registry enumerable outside a test.
--
-- Design and rationale: internal/superpowers/specs/2026-08-21-data-artifact-shape-registry-design.md.
--
-- The visibility gate is anchor_readable_by_profile — the same container visibility spine every
-- read gated on a home anchor uses (edges_visible_to, anchor_shape, cogmap_analytics, etc.). A
-- shape is homed polymorphically over (kb_contexts, kb_cogmaps), and a principal who cannot read
-- the home context/cogmap sees zero shapes — fail closed, never an error.
--
-- CONFORM warning, carried from the sibling migration's own header (20260820000030:20-27): the
-- gate must fail closed when the visible set is empty. anchor_readable_by_profile is a scalar
-- boolean — it returns false for each unreadable anchor, so every row is filtered and zero rows
-- come back. This is NOT an array_agg into a NULL-means-unbounded predicate; the
-- array_agg-over-empty-scope-returns-NULL fall-open scar
-- (vault memory 019fc290-b5c6-7160-a9a5-db40f3fff2d2) does not apply. If a future change
-- restructures these reads to collect anchor IDs into an array, COALESCE the aggregate to
-- ARRAY[]::uuid[] or the gate falls open.

-- All live (non-folded) shapes for one home. A principal who cannot read the home anchor sees
-- zero rows. Folded shapes are excluded — this is the enumeration of shapes in force, not the
-- revision history. Ordered by (created, id) for determinism, matching the artifact reads.
CREATE FUNCTION shapes_for_home(p_profile uuid, p_anchor_table varchar, p_anchor_id uuid)
RETURNS TABLE(
    shape_id            uuid,
    home_anchor_table   varchar,
    home_anchor_id      uuid,
    kind_owner_table    varchar,
    kind_owner_id       uuid,
    artifact_kind       text,
    schema              jsonb,
    enforcement         text,
    shape_version       int,
    is_folded           boolean,
    created             timestamptz
)
LANGUAGE sql STABLE AS $$
    SELECT s.id,
           s.home_anchor_table,
           s.home_anchor_id,
           s.kind_owner_table,
           s.kind_owner_id,
           s.artifact_kind,
           s.schema,
           s.enforcement,
           s.shape_version,
           s.is_folded,
           s.created
      FROM kb_data_artifact_shapes s
     WHERE s.home_anchor_table = p_anchor_table
       AND s.home_anchor_id = p_anchor_id
       AND anchor_readable_by_profile(p_profile, s.home_anchor_table, s.home_anchor_id)
       AND NOT s.is_folded
     ORDER BY s.created, s.id
$$;

-- A single shape by id. Never trusts the caller — even if the caller knows the shape id, the
-- owning home anchor must be readable to the profile or the shape is absent (fail closed,
-- returned as zero rows → Ok(None) on the Rust side). Includes folded shapes: a caller that
-- knows a shape id may legitimately need to inspect a folded prior version (audit/history).
CREATE FUNCTION shape_by_id(p_profile uuid, p_shape_id uuid)
RETURNS TABLE(
    shape_id            uuid,
    home_anchor_table   varchar,
    home_anchor_id      uuid,
    kind_owner_table    varchar,
    kind_owner_id       uuid,
    artifact_kind       text,
    schema              jsonb,
    enforcement         text,
    shape_version       int,
    is_folded           boolean,
    created             timestamptz
)
LANGUAGE sql STABLE AS $$
    SELECT s.id,
           s.home_anchor_table,
           s.home_anchor_id,
           s.kind_owner_table,
           s.kind_owner_id,
           s.artifact_kind,
           s.schema,
           s.enforcement,
           s.shape_version,
           s.is_folded,
           s.created
      FROM kb_data_artifact_shapes s
     WHERE s.id = p_shape_id
       AND anchor_readable_by_profile(p_profile, s.home_anchor_table, s.home_anchor_id)
$$;

SELECT declare_migration(
    20260822000030,
    'additive',
    'Data-artifact shape registry enumeration reads (Beat C, Task 5): shapes_for_home (all live non-folded shapes for one home anchor, visibility-gated by anchor_readable_by_profile — the same container visibility spine edges_visible_to/anchor_shape/cogmap_analytics use) and shape_by_id (single shape by id, gated on the owning home anchor, includes folded shapes for audit). Both fail closed: a principal who cannot read the home context/cogmap sees zero shapes, never an error. The gate is a scalar predicate (anchor_readable_by_profile), not an array_agg into a NULL-means-unbounded predicate — the fall-open scar does not apply. Design: internal/superpowers/specs/2026-08-21-data-artifact-shape-registry-design.md.'
);
