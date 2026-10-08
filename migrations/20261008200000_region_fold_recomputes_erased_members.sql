-- A materialize recomputes, under its anchor lock, the centroid of every live region it may fold
-- that still lists an erased resource.
--
-- Spec: temper-artifacts specs/2026-09-28-resource-erasure-design.md, rulings from P5 of the
-- v0.6.0 security review (2026-10-08). A folded region is never recomputed, so a region folded with
-- an erased resource's share keeps it for good. The act recomputes the live regions it can see
-- (7c), but a materialize in flight during the act can commit a live region that still lists the
-- resource with its share, after the act. A recompute run before a later materialize takes its
-- lock can miss that region, because the in-flight materialize may not have committed yet. Run
-- after `region_materialize` has updated the anchor row (`_project_region_materialized`), it
-- cannot: every materialize that took the lock first has committed, and READ COMMITTED shows its
-- regions to each later statement of the transaction.
--
--   1. `_region_recompute_centroids(regions)` holds the formula (populate_readouts' centroid
--      statement, previously inlined in the act's recompute), for live regions only.
--   2. `_resource_erasure_recompute_live_centroids` keeps its signature and behaviour and calls it.
--   3. `_region_recompute_erased_member_centroids(anchor, lens)` recomputes the live regions of
--      one anchor under one lens that list an erased resource. Scoped to the anchor the caller
--      holds the lock on: recomputing every anchor holding the resource from inside one anchor's
--      lock could deadlock against another anchor's materialize.

CREATE FUNCTION _region_recompute_centroids(p_regions uuid[])
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_cogmap_regions r SET centroid = coalesce((
           SELECT avg(mv) FROM (
             SELECT avg(ch.embedding) AS mv FROM kb_cogmap_region_members mm
             JOIN kb_chunks ch ON ch.resource_id = mm.member_id AND ch.is_current
             JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded
             WHERE mm.region_id = r.id GROUP BY mm.member_id) per_member
         ), array_fill(0, ARRAY[768])::vector)
     WHERE r.id = ANY(p_regions) AND NOT r.is_folded;
END;
$$;

COMMENT ON FUNCTION _region_recompute_centroids(uuid[]) IS
'Recompute the centroid of each named LIVE region over its members'' current, unfolded embeddings —
populate_readouts'' centroid statement. An erased member''s embeddings are null, so the result is
the survivors'' mean (the zero vector when none survive). Folded regions are skipped.';

CREATE OR REPLACE FUNCTION _resource_erasure_recompute_live_centroids(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM _region_recompute_centroids(ARRAY(
        SELECT mem.region_id FROM kb_cogmap_region_members mem
          JOIN kb_cogmap_regions r ON r.id = mem.region_id
         WHERE NOT r.is_folded
           AND mem.member_table = 'kb_resources' AND mem.member_id = p_resource));
END;
$$;

CREATE FUNCTION _region_recompute_erased_member_centroids(
    p_anchor_table text, p_anchor_id uuid, p_lens_id uuid)
RETURNS integer LANGUAGE plpgsql AS $$
DECLARE v_regions uuid[];
BEGIN
    v_regions := ARRAY(
        SELECT DISTINCT r.id FROM kb_cogmap_regions r
          JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
          JOIN kb_resources res ON res.id = mem.member_id
         WHERE r.home_anchor_table = p_anchor_table AND r.home_anchor_id = p_anchor_id
           AND r.lens_id = p_lens_id AND NOT r.is_folded
           AND mem.member_table = 'kb_resources' AND res.erased_at IS NOT NULL);
    PERFORM _region_recompute_centroids(v_regions);
    RETURN cardinality(v_regions);
END;
$$;

COMMENT ON FUNCTION _region_recompute_erased_member_centroids(text, uuid, uuid) IS
'Recompute the centroid of every LIVE region of one anchor under one lens that lists an erased
resource, returning how many. A materialize calls it inside its transaction, after
region_materialize has locked the anchor row and before it folds, so no region folds with an erased
resource''s share.';

SELECT declare_migration(
    20261008200000,
    'additive',
    'Two new functions, _region_recompute_centroids and _region_recompute_erased_member_centroids, and a CREATE OR REPLACE of _resource_erasure_recompute_live_centroids with an unchanged signature, return type and effect (its formula moves into _region_recompute_centroids). A binary that predates this never calls the new functions and materializes as before. No table, column, constraint or grant changes.'
);
