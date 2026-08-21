-- The entry read — a degree ranking, and the induced-edge body given a frame-neutral name.
--
-- Chunk A of `internal/superpowers/specs/2026-08-20-grounding-and-navigation-split-design.md`
-- (task 01a023df-f54c-7d90-aa53-1bd66011475c). Serves the door for a reader who has supplied
-- nothing: the K most-connected resources they can see, plus every edge among them.
--
-- ── WHY THIS EXISTS ─────────────────────────────────────────────────────────────────────────────
--
-- The unaddressed entry draws 250 marks of which 244 are unconnected. That is not a rendering bug.
-- `follow-from` returns edges joining a walked node to the seed it was reached from; the entry
-- seeds that walk from every visible resource while drawing 200 rows fetched separately and ordered
-- `r.updated DESC`. The drawn set and the walked set are chosen by unrelated criteria, so nearly
-- every edge has one endpoint off-canvas and is dropped. The 244 are unconnected in the drawing,
-- not in the corpus.
--
-- The rule that fixes it (spec §5.1 constraint 2, as corrected):
--
--     Rank by corpus degree; return the INDUCED SUBGRAPH over the top-K.
--
-- Every drawn edge then has both endpoints drawn by construction. An earlier draft said "degree
-- must be measured over the set that will be drawn", which is circular — the drawn set is chosen
-- BY the ranking, so the ranking cannot be scoped to it.
--
-- ── 1. graph_visible_degree_ranking ─────────────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION graph_visible_degree_ranking(p_profile uuid, p_limit int)
RETURNS TABLE (resource_id uuid, degree int)
LANGUAGE sql
STABLE
AS $$
    -- The degree predicate is NOT restated here. `edges_visible_to(p_profile)` already carries
    -- `NOT e.is_folded` and gates BOTH endpoints, and it is the same set-returning function the
    -- incumbent `graph_atlas_nodes_visible` degree LATERAL joins. Restating `NOT is_folded` inline
    -- would create a second copy that drifts silently from the one the hydration uses -- and a
    -- ranking that disagrees with the degree it displays is this spec's own defect, again.
    WITH vis AS MATERIALIZED (
        SELECT resource_id AS id FROM resources_visible_to(p_profile)
    ),
    e AS MATERIALIZED (
        SELECT e.id, e.source_id, e.target_id
          FROM kb_edges e
          JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
         -- kb_edges may target kb_cogmaps, which are not drawable resource marks (spec §5.1
         -- constraint 3). Both endpoints must be resources or the edge cannot be drawn at all.
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
    ),
    -- UNION, never UNION ALL. A self-loop yields the pair (edge, node) twice and must collapse to
    -- one, so that this count equals the incumbent's
    -- `count(*) ... WHERE (e.source_id = r.id OR e.target_id = r.id)` exactly. UNION ALL would
    -- count a self-loop twice and put this ranking a hair out of step with the displayed degree.
    inc AS MATERIALIZED (
        SELECT id AS eid, source_id AS n FROM e
        UNION
        SELECT id, target_id FROM e
    )
    SELECT v.id, count(inc.eid)::int AS degree
      FROM vis v
      LEFT JOIN inc ON inc.n = v.id   -- LEFT: degree-zero resources are RANKED, never dropped
     GROUP BY v.id
     -- `id` is the tie-break, and it is load-bearing rather than cosmetic: without it two calls
     -- with the same p_limit may return different sets from the same corpus, and a door that
     -- answers differently on refresh cannot be reasoned about.
     ORDER BY degree DESC, v.id
     -- GREATEST clamps rather than errors: Postgres rejects a negative LIMIT outright, and a read
     -- should return an empty answer to a nonsensical bound, not fail. NULL also clamps to 0 --
     -- there is deliberately no "unlimited" spelling, because an unbounded entry read is the
     -- 3574-row draw this chunk exists to stop.
     LIMIT GREATEST(coalesce(p_limit, 0), 0);
$$;

COMMENT ON FUNCTION graph_visible_degree_ranking(uuid, int) IS
$c$The entry read's ranking half (spec §5.1): the resources visible to p_profile ordered by CORPUS
degree descending, id ascending, limited to p_limit. Returns ids and degrees only -- hydration is
`graph_atlas_nodes_visible`, which is the one node shape every graph read uses, so there is no
second place for it to drift.

Degree counts what THIS CALLER can see, following `member_count`'s precedent: two readers of the
same corpus can legitimately see different numbers, and that is the point. The count is delegated to
`edges_visible_to`, which already excludes folded edges -- a retracted assertion must not rank
anything.

Degree-zero resources are ranked, not filtered. The caller decides what to draw; a read must not
make presentation decisions (spec §6).

NOTE: corpus degree is a RANKING input. It is not the quantity a reader should see -- what reaches
the screen is derived from the edges actually drawn (spec §5.3). A high corpus degree does NOT imply
an edge inside the drawn set; measured on production at K=50, 18 of 50 nodes with degree >= 16 had
no induced edge at all.$c$;

-- ── 2. graph_induced_edges ──────────────────────────────────────────────────────────────────────
--
-- This body is NOT new. It is `graph_context_composition_edges` (20260709000012), verbatim, under a
-- name that does not lie about its frame: it was never context-specific -- it takes an arbitrary
-- node-id array -- and chunk E deletes the context door that is currently its only caller.
--
-- At p_depth = 0 the recursive arm is dead (`r.depth < LEAST(0, 3)` is `0 < 0`), so `reached`
-- collapses to the seed array and the final SELECT returns exactly the INDUCED SUBGRAPH over it.
-- That is the shape chunk A needs and the reason no new SQL had to be written for spec §5.4.

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
$c$Edges in `AtlasEdge` shape over an arbitrary node-id array (spec §5.4), serving BOTH the entry
read (chunk A, depth 0) and the traversal read (chunk B). Visibility-scoped on both endpoints,
unfolded-only, anchor-readable, and restricted to kb_resources on both ends.

p_depth = 0 returns the INDUCED SUBGRAPH over p_node_ids -- no expansion, so every returned edge has
both endpoints in the array the caller passed. That is the property protecting the entry read from
the defect it was written to fix. p_depth > 0 expands outward first (clamped to 3) and then returns
the edges among everything reached; that is the composition drill's behaviour.

This is the canonical body. `graph_context_composition_edges` is a delegating wrapper over it and is
deleted with the context door in chunk E.$c$;

-- ── 3. The old name becomes a wrapper ───────────────────────────────────────────────────────────
--
-- One body, two names -- the precedent spec §5.2 names for linking two walks that must agree. Doing
-- it this way rather than by DROP + rename is what keeps this migration `additive`: a binary that
-- does not carry it calls `graph_context_composition_edges` and gets the identical answer, because
-- it IS the identical body. A rename would have been shape-breaking and operator-gated.

CREATE OR REPLACE FUNCTION graph_context_composition_edges(p_profile uuid, p_seed_ids uuid[], p_depth integer)
RETURNS TABLE (id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)
LANGUAGE sql
STABLE
AS $$
    SELECT * FROM graph_induced_edges(p_profile, p_seed_ids, p_depth);
$$;

COMMENT ON FUNCTION graph_context_composition_edges(uuid, uuid[], integer) IS
$c$DEPRECATED wrapper over `graph_induced_edges`, which carries the body verbatim under a name that
does not claim a context frame it never had. Kept only so this migration is additive for a binary
that predates it; deleted with `/api/graph/contexts/composition` in chunk E of the
grounding-and-navigation split.$c$;

SELECT declare_migration(
    20260821000010,
    'additive',
    'The entry read (chunk A of the grounding/navigation split, task 01a023df-f54c-7d90-aa53-1bd66011475c). Adds graph_visible_degree_ranking(profile, limit) -- visible resources ordered by corpus degree, delegating the degree predicate to edges_visible_to rather than restating NOT is_folded -- and graph_induced_edges(profile, node_ids, depth), which is graph_context_composition_edges'' body verbatim under a frame-neutral name; at depth 0 it returns the induced subgraph over the given array, which is what makes every drawn edge have both endpoints drawn. graph_context_composition_edges is REPLACED BY A DELEGATING WRAPPER over the new name, same signature and same columns, so a binary without this migration receives the identical answer -- which is why this is additive rather than a shape-breaking rename. Nothing dropped. The wrapper goes with the context door in chunk E.'
);
