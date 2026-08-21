-- The entry read: a degree ranking, the induced-edge body under a frame-neutral name, and two
-- columns on the node projection. Chunk A of the grounding/navigation split.
--
-- Design, the measured degree distribution, and why K is a floor rather than a count:
--   internal/superpowers/specs/2026-08-20-grounding-and-navigation-split-design.md §5.1, §5.4, §10.1.1
-- Task 01a023df-f54c-7d90-aa53-1bd66011475c.

CREATE OR REPLACE FUNCTION graph_visible_degree_ranking(
    p_profile uuid, p_anchor_ids uuid[], p_min_degree int, p_limit int)
RETURNS TABLE (resource_id uuid, degree int)
LANGUAGE sql
STABLE
AS $$
    -- No `NOT is_folded` here: `edges_visible_to` already carries it, and it is the same set the
    -- incumbent `graph_atlas_nodes_visible` degree LATERAL counts. A second copy would drift.
    WITH vis AS MATERIALIZED (
        SELECT rv.resource_id AS id
          FROM resources_visible_to(p_profile) rv
         WHERE p_anchor_ids IS NULL
            OR array_length(p_anchor_ids, 1) IS NULL
            OR EXISTS (SELECT 1 FROM kb_resource_homes h
                        WHERE h.resource_id = rv.resource_id
                          AND h.anchor_id = ANY(p_anchor_ids))
    ),
    e AS MATERIALIZED (
        SELECT e.id, e.source_id, e.target_id
          FROM kb_edges e
          JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
    ),
    -- UNION, never UNION ALL: a self-loop yields (edge, node) twice and must collapse to one, or
    -- this count stops matching the incumbent's `WHERE source = r.id OR target = r.id`.
    inc AS MATERIALIZED (
        SELECT id AS eid, source_id AS n FROM e
        UNION
        SELECT id, target_id FROM e
    ),
    deg AS (
        SELECT v.id, count(inc.eid)::int AS degree
          FROM vis v
          LEFT JOIN inc ON inc.n = v.id   -- LEFT: a degree-zero resource is COUNTED before it is cut
         GROUP BY v.id
    )
    SELECT d.id, d.degree
      FROM deg d
     WHERE d.degree >= coalesce(p_min_degree, 1)
     -- `id` is load-bearing, not cosmetic: without it two calls with the same p_limit can return
     -- different sets from one corpus, and a door that answers differently on refresh is unusable.
     ORDER BY d.degree DESC, d.id
     -- Postgres rejects a negative LIMIT outright; clamp so a nonsensical bound returns nothing
     -- rather than erroring. NULL clamps to 0 too -- there is deliberately no "unlimited" spelling.
     LIMIT GREATEST(coalesce(p_limit, 0), 0);
$$;

COMMENT ON FUNCTION graph_visible_degree_ranking(uuid, uuid[], int, int) IS
$c$Visible resources ordered by corpus degree desc, id asc, confined to p_anchor_ids when given and
floored at p_min_degree. Returns ids and degrees; hydration is graph_atlas_nodes_visible.

Degree counts what THIS CALLER can see, as member_count does. It is a RANKING input, not the number
a reader should see -- a high corpus degree does not imply an edge inside the drawn set. This
function cannot report what the floor excluded; graph_visible_degree_bounds does.$c$;

CREATE OR REPLACE FUNCTION graph_visible_degree_bounds(
    p_profile uuid, p_anchor_ids uuid[], p_min_degree int)
RETURNS TABLE (in_scope int, eligible int)
LANGUAGE sql
STABLE
AS $$
    WITH vis AS MATERIALIZED (
        SELECT rv.resource_id AS id
          FROM resources_visible_to(p_profile) rv
         WHERE p_anchor_ids IS NULL
            OR array_length(p_anchor_ids, 1) IS NULL
            OR EXISTS (SELECT 1 FROM kb_resource_homes h
                        WHERE h.resource_id = rv.resource_id
                          AND h.anchor_id = ANY(p_anchor_ids))
    ),
    e AS MATERIALIZED (
        SELECT e.id, e.source_id, e.target_id
          FROM kb_edges e
          JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
    ),
    inc AS MATERIALIZED (
        SELECT id AS eid, source_id AS n FROM e
        UNION
        SELECT id, target_id FROM e
    ),
    deg AS (
        SELECT v.id, count(inc.eid)::int AS degree
          FROM vis v LEFT JOIN inc ON inc.n = v.id GROUP BY v.id
    )
    -- `::int`, not bigint: ts-rs maps a 64-bit count to `bigint`, which cannot survive
    -- JSON.stringify across the server/client boundary.
    SELECT count(*)::int AS in_scope,
           count(*) FILTER (WHERE degree >= coalesce(p_min_degree, 1))::int AS eligible
      FROM deg;
$$;

COMMENT ON FUNCTION graph_visible_degree_bounds(uuid, uuid[], int) IS
$c$What the entry read is choosing from: in_scope is every resource visible to the caller (within
p_anchor_ids), eligible is how many clear p_min_degree. The difference is what the read does not
draw, and naming it is what keeps that omission declared.

Separate from the ranking because the case that most needs these numbers is the one where the
ranking returns no rows at all -- counts carried on result rows vanish exactly there.$c$;

-- `graph_context_composition_edges`' body verbatim under a name that does not claim a context
-- frame it never had; chunk E deletes the context door that is currently its only caller.
CREATE OR REPLACE FUNCTION graph_induced_edges(p_profile uuid, p_node_ids uuid[], p_depth integer)
RETURNS TABLE (id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)
LANGUAGE sql
STABLE
AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    seeds AS (SELECT DISTINCT s.id FROM unnest(p_node_ids) s(id) JOIN vis v ON v.id = s.id),
    reached AS (
        SELECT id AS node_id, 0 AS depth FROM seeds
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END,
               r.depth + 1
          FROM reached r
          JOIN kb_edges e ON (e.source_id = r.node_id OR e.target_id = r.node_id)
          JOIN vis vs ON vs.id = e.source_id
          JOIN vis vt ON vt.id = e.target_id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND NOT e.is_folded
           AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
           -- At p_depth = 0 this arm is dead (`0 < 0`), so `reached` stays the seed array and the
           -- final SELECT is the INDUCED subgraph. The entry read depends on that; raising the
           -- floor to 1 would silently put undrawn endpoints back on the canvas.
           AND r.depth < LEAST(p_depth, 3)
    )
    SELECT DISTINCT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
      FROM kb_edges e
      JOIN reached rs ON rs.node_id = e.source_id
      JOIN reached rt ON rt.node_id = e.target_id
      JOIN vis vs ON vs.id = e.source_id
      JOIN vis vt ON vt.id = e.target_id
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
       AND NOT e.is_folded
       AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id);
$$;

COMMENT ON FUNCTION graph_induced_edges(uuid, uuid[], integer) IS
$c$Edges in AtlasEdge shape over an arbitrary node-id array, serving the entry read (depth 0) and the
traversal read (depth > 0). Visibility-scoped on both endpoints, unfolded-only, anchor-readable,
kb_resources both ends.

p_depth = 0 returns the INDUCED subgraph over p_node_ids -- every returned edge has both endpoints in
the array the caller passed. Above 0 it expands outward first (clamped to 3), then returns the edges
among everything reached.$c$;

-- A delegating wrapper, not a copy: this is what keeps the migration additive, since a binary
-- without it calls the old name and gets the identical body. Deleted with the endpoint in chunk E.
CREATE OR REPLACE FUNCTION graph_context_composition_edges(p_profile uuid, p_seed_ids uuid[], p_depth integer)
RETURNS TABLE (id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)
LANGUAGE sql
STABLE
AS $$
    SELECT * FROM graph_induced_edges(p_profile, p_seed_ids, p_depth);
$$;

COMMENT ON FUNCTION graph_context_composition_edges(uuid, uuid[], integer) IS
$c$DEPRECATED wrapper over graph_induced_edges, kept only so the migration adding it is additive for a
binary that predates it. Deleted with /api/graph/contexts/composition in chunk E.$c$;

-- Adds `home_id` and `updated`. `home_id` is the anchor's id and not a decorated ref because
-- building `@owner/slug` here would copy `graph_home_contexts`' owner_ref CASE (20260707140000:63);
-- the client already holds every anchor it can read. DROP first because CREATE OR REPLACE cannot
-- widen a RETURNS TABLE -- the ARGUMENT signature is unchanged, so existing callers are unaffected.
DROP FUNCTION IF EXISTS graph_atlas_nodes_visible(uuid, uuid[]);

CREATE FUNCTION graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[])
RETURNS TABLE (id uuid, title text, doc_type text, home text, degree integer,
               first_chunk text, stage text, home_id uuid, updated timestamptz)
LANGUAGE sql
STABLE
AS $$
    WITH vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    ),
    stg AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS st
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'temper-stage' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree,
           (SELECT cc.content FROM kb_chunks ch
              JOIN kb_content_blocks b ON b.id = ch.block_id
              JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
             WHERE ch.resource_id = r.id AND ch.is_current AND NOT b.is_folded
             ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk,
           s.st AS stage,
           h.home_id,
           r.updated
    FROM ids
    JOIN vis v ON v.id = ids.id           -- deny-as-absence: unseen ids drop out
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN stg s ON s.rid = r.id
    LEFT JOIN LATERAL (
        -- `bool_or` is carried over verbatim rather than simplified to a LIMIT 1: with one home
        -- they agree, with more than one they do not. Postgres has no `min(uuid)`, hence array_agg.
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home,
               (array_agg(h2.anchor_id))[1] AS home_id
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

SELECT declare_migration(
    20260821000010,
    'additive',
    'The entry read (chunk A of the grounding/navigation split, task 01a023df-f54c-7d90-aa53-1bd66011475c). Adds graph_visible_degree_ranking(profile, anchor_ids, min_degree, limit) and graph_visible_degree_bounds(profile, anchor_ids, min_degree). Renames graph_context_composition_edges'' body to graph_induced_edges and leaves the old name as a delegating wrapper over it -- same signature, same columns, so a binary without this migration gets the identical answer, which is why this is additive rather than a shape-breaking rename. Also widens graph_atlas_nodes_visible with two trailing columns, home_id and updated; the argument signature is unchanged and callers name their own columns. Nothing dropped. The wrapper and the context door go together in chunk E.'
);
