-- Migration: edge-owned properties — anchor resolution per owner kind, and fold cascade.
--
-- `kb_properties.owner_table` has admitted 'kb_edges' since the canonical schema, whose own DDL
-- comment says why ("§4a edges carry facets"). Nothing ever wrote one: production, 2026-07-27, has
-- kb_resources 10692 / kb_content_blocks 37 / kb_edges 0 / kb_cogmaps 0. This migration builds the
-- write path the DDL always described.
--
-- ── What was actually blocking it (established by execution, 2026-07-27) ─────────────────────────
-- The projectors were never the problem. `_project_property_asserted` reads `owner.table` and
-- `owner.id` straight from the payload and inserts them verbatim; handed an edge owner with an
-- anchor, it projects a kb_edges-owned row cleanly. The blocker is one line in each of the two
-- *wrapper* functions, which resolve the event's home anchor by treating the owner id as a RESOURCE
-- id:
--
--     SELECT anchor_table, anchor_id FROM kb_resource_homes WHERE resource_id = v_owner
--     IF v_anchor IS NULL THEN RAISE EXCEPTION '…: resource % has no home to anchor the property event'
--
-- An edge has no `kb_resource_homes` row, so the guard fires:
--     ERROR: facet_set: resource 019fa500-…-0000000ed001 has no home to anchor the property event
--
-- ── Beat 1: one anchor resolver, shared by both wrappers ────────────────────────────────────────
-- An edge already carries its own home (`kb_edges.home_anchor_table/home_anchor_id`) and
-- `relationship_fold` already anchors its event from exactly those columns. This resolver conforms
-- to that incumbent rather than inventing a second way to answer the same question, and it is one
-- function rather than a copy in each wrapper — the two would drift.
--
-- Only the two owner kinds that have a write path are handled. `kb_cogmaps` and `kb_content_blocks`
-- are admitted by the CHECK but have no caller: a cogmap-owned property has never existed, and
-- block properties are written by `_project_blocks` directly, never through these wrappers. They
-- RAISE rather than getting a speculative branch — an unused arm is state we do not need, and a
-- silent NULL anchor would be worse than a clear refusal.
CREATE FUNCTION _property_owner_anchor(p_owner_table text, p_owner uuid,
                                       OUT anchor_table text, OUT anchor_id uuid)
LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF p_owner_table = 'kb_resources' THEN
        -- Incumbent behaviour, unchanged: a resource is anchored by its home. The cogmap-first
        -- ORDER BY is carried verbatim from the wrappers this replaces — a resource homed in both a
        -- context and a cogmap anchors on the cogmap.
        SELECT h.anchor_table, h.anchor_id INTO anchor_table, anchor_id
          FROM kb_resource_homes h
         WHERE h.resource_id = p_owner
         ORDER BY (h.anchor_table = 'kb_cogmaps') DESC
         LIMIT 1;
    ELSIF p_owner_table = 'kb_edges' THEN
        -- An edge carries its own home. Same source `relationship_fold` reads.
        SELECT e.home_anchor_table, e.home_anchor_id INTO anchor_table, anchor_id
          FROM kb_edges e
         WHERE e.id = p_owner;
    ELSE
        RAISE EXCEPTION 'property owner_table % has no anchor rule', p_owner_table
            USING HINT = 'only kb_resources and kb_edges can own a property through facet_set/property_set';
    END IF;

    IF anchor_id IS NULL THEN
        RAISE EXCEPTION '% % has no home to anchor the property event', p_owner_table, p_owner;
    END IF;
END;
$$;

COMMENT ON FUNCTION _property_owner_anchor(text, uuid) IS
'Resolve the home anchor for a property event, by owner kind. kb_resources → kb_resource_homes; '
'kb_edges → the edge''s own home_anchor_* columns (the same source relationship_fold reads). Any '
'other owner kind raises: it has no write path, and a speculative branch would be unused state.';

-- ── Beat 2: both wrappers dispatch on owner.table instead of assuming a resource ─────────────────
-- Same signatures, same events, same projectors. The ONLY change is where the anchor comes from,
-- so a kb_resources owner takes a path identical to the one it took before.
DROP FUNCTION facet_set(jsonb, uuid, jsonb, uuid, uuid);
CREATE FUNCTION facet_set(p_payload jsonb, p_emitter uuid,
                          p_metadata jsonb DEFAULT '{}'::jsonb,
                          p_invocation uuid DEFAULT NULL, p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner_tbl text := COALESCE(p_payload#>>'{owner,table}', 'kb_resources');
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT a.anchor_table, a.anchor_id INTO v_anchor_tbl, v_anchor
      FROM _property_owner_anchor(v_owner_tbl, v_owner) a;
    v_ev := _event_append('property_asserted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_property_asserted(v_ev, p_payload);
END;
$$;

DROP FUNCTION property_set(jsonb, uuid, jsonb, uuid, uuid);
CREATE FUNCTION property_set(p_payload jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}'::jsonb,
                             p_invocation uuid DEFAULT NULL, p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner_tbl text := COALESCE(p_payload#>>'{owner,table}', 'kb_resources');
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT a.anchor_table, a.anchor_id INTO v_anchor_tbl, v_anchor
      FROM _property_owner_anchor(v_owner_tbl, v_owner) a;
    v_ev := _event_append('property_set', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_property_set(v_ev, p_payload);
END;
$$;

-- ── Beat 3: folding an edge folds the properties it owns ─────────────────────────────────────────
-- Measured before this migration: folding an edge left its properties live
-- (edge_folded = t, property_folded = f). `_project_relationship_folded` touched kb_edges only.
--
-- That orphan is not survivable for what edge facets are FOR. The clause citation on an `advances`
-- edge exists so that "this task witnesses clause X of goal G" cannot diverge from the link it
-- qualifies. A live citation hanging off a retracted link is precisely that divergence, re-created
-- one level down.
--
-- The cascade loses no history: `is_folded` IS the retention mechanism here — folded rows are kept,
-- never deleted, and `uq_kb_properties_active` is partial on `NOT is_folded` exactly so that
-- fold-then-reinsert works. The alternative (leave them live, make every reader join kb_edges and
-- filter) puts one invariant in N readers instead of one writer, which is the shape this whole
-- design set out to make unrepresentable.
--
-- `last_event_id` on the cascaded property points at the FOLD event, so the trail records which act
-- retired it.
CREATE OR REPLACE FUNCTION _project_relationship_folded(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
BEGIN
    UPDATE kb_edges SET is_folded = true, last_event_id = p_event WHERE id = v_edge;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    -- Cascade: an edge's facets do not outlive the edge.
    UPDATE kb_properties SET is_folded = true, last_event_id = p_event
     WHERE owner_table = 'kb_edges' AND owner_id = v_edge AND NOT is_folded;
    RETURN v_edge;
END;
$$;
