-- Graph Atlas — Chunk B read functions (R2–R5) + shared team-viewable gate.
--
-- Consolidated single migration for the Chunk B reads. Timestamped AFTER the
-- sibling `20260703120000_invitation_partial_unique.sql` (already merged to main)
-- so it applies in order on a prod DB that already ran that migration — see the
-- repo convention of renumbering after merging main (e.g. commit 841e2934).
--
-- Design spec: docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md
--
-- SECURITY INVARIANT: every function that emits or counts an access-controlled
-- row reproduces the FULL canonical visibility predicate for that row type:
--   * resources → resources_visible_to (or resources_in_team_scope ⊆ it)
--   * edges     → edges_visible_to = NOT is_folded AND anchor_readable_by_profile(home)
--                 AND endpoint_readable_by_profile(source) AND endpoint_readable_by_profile(target)
--                 (a trail read may intentionally drop ONLY the NOT-is_folded conjunct,
--                  to show a folded edge's history — the endpoint+anchor conjuncts stay)
--   * regions/cogmaps → cogmap_readable_by_profile
-- Gating an edge on a SUBSET of those conjuncts leaks private relationships.


-- ─────────────────────────────────────────────────────────────────────────────
-- Shared: team-viewable "deny-as-absence" entry gate (member of the team or a
-- descendant). Extracted from the three inline copies (R4/R2 services + R1 graph_scope).
-- ─────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION team_viewable_by(p_profile uuid, p_team uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS(
        SELECT 1 FROM team_descendants(p_team) d
        JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = p_profile
    );
$$;


-- ─────────────────────────────────────────────────────────────────────────────
-- R4 — Atlas neighborhood slice: team-scoped, edge-kind-filtered traversal + node projection.
-- Keeps graph_traverse's recursive-CTE shape but swaps the profile-visibility CTE for a
-- team-scope CTE (resources_in_team_scope), adds an edge-kind filter to both arms, returns weight.
-- ─────────────────────────────────────────────────────────────────────────────

-- Scoped, edge-kind-filtered directed walk. p_edge_kinds empty/NULL => all kinds.
-- Edge inclusion enforces the full edges_visible_to predicate: both endpoints ∈ team scope
-- (⊆ resources_visible_to), NOT is_folded, AND the edge's own home anchor readable — the
-- last conjunct closes the "private edge between two public resources" leak.
CREATE FUNCTION graph_traverse_scoped(
    p_profile     uuid,
    p_team        uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_edge_kinds  edge_kind[]
) RETURNS TABLE(
    source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    walk AS (
        SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, 1 AS depth
        FROM kb_edges e
        JOIN scope ss ON ss.id = e.source_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND e.source_id = ANY(p_seed_ids)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
        UNION
        SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, w.depth + 1
        FROM kb_edges e
        JOIN walk w ON e.source_id = w.target_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND w.depth < LEAST(p_depth, 10)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
    )
    -- DISTINCT: `walk`'s UNION dedups on the full row *including* depth, so an
    -- edge reachable at two different depths (realistic with multiple seeds)
    -- would otherwise survive as two rows once `depth` is dropped here.
    SELECT DISTINCT source_id, target_id, edge_kind, polarity, label, weight FROM walk;
$$;

-- Project Atlas node attributes for a set of ids, clamped to team scope.
-- doc_type is LEFT-joined (nullable). home = cogmap if any cogmap home exists, else context.
CREATE FUNCTION graph_atlas_nodes(
    p_profile uuid, p_team uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree
    FROM ids
    JOIN scope s   ON s.id = ids.id
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;


-- ─────────────────────────────────────────────────────────────────────────────
-- R2 — territory overview: region + context territories, orphan salient nodes
-- (sparsity fallback = edge-degree), and aggregated cross-territory bridges.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE FUNCTION graph_region_territories(
    p_profile uuid, p_team uuid, p_lens uuid
) RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id, reg.label, reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    JOIN kb_team_cogmaps tc ON tc.cogmap_id = reg.cogmap_id
    JOIN team_ancestors(p_team) a ON a.team_id = tc.team_id
    WHERE NOT reg.is_folded
      AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;

CREATE FUNCTION graph_context_territories(
    p_profile uuid, p_team uuid
) RETURNS TABLE(context_id uuid, label text, member_count int) LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    homed AS (
        SELECT h.anchor_id AS context_id, h.resource_id
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_contexts'
    )
    SELECT c.id, c.name, count(homed.resource_id)::int
    FROM homed
    JOIN kb_contexts c ON c.id = homed.context_id
    GROUP BY c.id, c.name;
$$;

-- Orphan salient nodes: in-scope resources whose cogmap home has NO live region,
-- ranked by visible edge-degree. doc_type LEFT-joined (nullable). Bounded in Rust.
CREATE FUNCTION graph_orphan_salient_nodes(
    p_profile uuid, p_team uuid
) RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid)
LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    cogmap_homed AS (
        SELECT h.resource_id, h.anchor_id AS cogmap_id
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps'
    ),
    region_maps AS (
        SELECT DISTINCT cogmap_id FROM kb_cogmap_regions WHERE NOT is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type,
           deg.degree, ch.cogmap_id
    FROM cogmap_homed ch
    LEFT JOIN region_maps rm ON rm.cogmap_id = ch.cogmap_id
    JOIN kb_resources r ON r.id = ch.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE rm.cogmap_id IS NULL  -- home cogmap has no materialized region
    ORDER BY deg.degree DESC;
$$;

-- Aggregated cross-territory bridges: visible edges whose endpoints' cogmap homes differ.
CREATE FUNCTION graph_territory_bridges(
    p_profile uuid, p_team uuid
) RETURNS TABLE(source_territory uuid, target_territory uuid, edge_count int)
LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    homed AS (
        SELECT h.resource_id, h.anchor_id AS territory
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps'
    )
    SELECT LEAST(sh.territory, th.territory), GREATEST(sh.territory, th.territory), count(*)::int
    FROM kb_edges e
    JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
    JOIN homed sh ON sh.resource_id = e.source_id
    JOIN homed th ON th.resource_id = e.target_id
    WHERE NOT e.is_folded AND sh.territory <> th.territory
    GROUP BY LEAST(sh.territory, th.territory), GREATEST(sh.territory, th.territory);
$$;


-- ─────────────────────────────────────────────────────────────────────────────
-- R3 — territory slice: components + visibility-scoped region members.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE FUNCTION graph_region_components(
    p_profile uuid, p_region uuid
) RETURNS TABLE(component_id uuid, member_count int) LANGUAGE sql STABLE AS $$
    SELECT comp.id, cardinality(comp.member_ids)::int
    FROM kb_cogmap_regions reg
    JOIN kb_cogmap_components comp
      ON comp.cogmap_id = reg.cogmap_id AND comp.lens_id = reg.lens_id AND NOT comp.is_folded
    WHERE reg.id = p_region AND NOT reg.is_folded
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;

CREATE FUNCTION graph_region_members(
    p_profile uuid, p_region uuid
) RETURNS TABLE(id uuid, title text, doc_type text, affinity double precision)
LANGUAGE sql STABLE AS $$
    WITH doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    visible AS (SELECT resource_id FROM resources_visible_to(p_profile))
    SELECT r.id, r.title, d.dt AS doc_type, m.affinity
    FROM kb_cogmap_regions reg
    JOIN kb_cogmap_region_members m ON m.region_id = reg.id AND m.member_table = 'kb_resources'
    JOIN visible v ON v.resource_id = m.member_id
    JOIN kb_resources r ON r.id = m.member_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    WHERE reg.id = p_region AND NOT reg.is_folded
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id)
    ORDER BY m.affinity DESC NULLS LAST;
$$;


-- ─────────────────────────────────────────────────────────────────────────────
-- R5 — element event-trail: time-ordered events for a node or edge.
-- ─────────────────────────────────────────────────────────────────────────────

-- Edge trail: every relationship_* payload embeds a stable edge_id. Enforce the
-- full edge visibility EXCEPT the NOT-is_folded conjunct (a folded edge must still
-- show its trail — the fold event is part of the story): home anchor readable AND
-- BOTH endpoints readable. Dropping the endpoint conjuncts would leak the trail of
-- an edge that touches a private endpoint.
CREATE FUNCTION element_trail_edge(
    p_profile uuid, p_edge uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz, metadata jsonb)
LANGUAGE sql STABLE AS $$
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata
    FROM kb_edges edg
    JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = edg.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    WHERE edg.id = p_edge
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$$;

-- Node trail: NO single key exists — union three grounded key-shapes, then gate
-- once via resources_visible_to. (1) resource-keyed events (created/updated/deleted/
-- rehomed/block_created); (2) property events whose owner IS this resource
-- (guard owner.table='kb_resources'); (3) block events that carry only block_id →
-- join kb_content_blocks to attribute them.
CREATE FUNCTION element_trail_node(
    p_profile uuid, p_resource uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz, metadata jsonb)
LANGUAGE sql STABLE AS $$
    WITH ev_ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
        WHERE b.resource_id = p_resource
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    WHERE EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$$;
