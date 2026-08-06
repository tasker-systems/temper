-- Phase 1 — `/api/search` becomes two arms that are never combined.
--
-- `search_exact` is the arm for "you can quote the words". It returns `fts_norm` and nothing else;
-- no weight, no sum, no second quantity to rank it against. `search_wide` (the vector arm) lands
-- beside it in this same phase; `unified_search` stays until both ship and the read path moves, so
-- this migration is additive and the deploy's additive-only gate passes.
--
-- Task 019fd25e-95f0-7373-9a6e-0574deea5ab3, decision 019fd25a-ef4c-7473-b72e-265a7d36dd65.
--
-- ── WHY A NEW NAME RATHER THAN A REPLACE ─────────────────────────────────────────────────────────
--
-- Scoping in this phase is by anchor pair `(anchor_table, anchor_id)` — one shape for contexts and
-- cogmaps alike, replacing today's split where a context scopes via `p_context_id` and a cogmap
-- scopes via a materialized `uuid[]` from `cogmap_scope_ids_multi`. That is a parameter change, and
-- PostgreSQL keys functions by (name, argument types), so it could only ever be a NEW function.
-- Overloading the incumbent names would leave two same-named functions live, which is a live hazard
-- for `search_surface_a.rs`'s `pinned_ef_search` — it reads `pg_proc WHERE proname = ...` with
-- `fetch_one` and would silently take whichever overload came back first. Distinct names, no
-- ambiguity.
--
-- ── THE PREDICATES STAY ON THE ARM ───────────────────────────────────────────────────────────────
--
-- Visibility, `is_active` and `ingest_state = 'complete'` are carried here, inline, exactly as
-- `search_fts_candidates` carries them (20260801000010:132-135). This is CONFORM, not a new
-- decision: every arm already gates itself today, and `resources_visible_to`'s union has been
-- wrapped in an `is_active` semi-join since 20260708000007, so visibility already implies active.
-- `unified_search`'s `corpus` CTE adds `ingest_state` on top for the graph and seed arms, which have
-- no gate of their own; it is redundant for this arm and dies with those arms, not with this one.
--
-- ANCHOR READABILITY IS A FOURTH PREDICATE, AND IT IS NOT IMPLIED BY THE OTHER THREE. The first
-- draft of this migration carried the anchor pair and `resources_visible_to` but dropped the
-- readability conjunct that `cogmap_scope_ids` had always carried, because replacing a function
-- with an inline predicate silently drops whatever the function did beyond the part being inlined.
-- `resources_visible_to` MASKS that for almost every principal, which is why it very nearly shipped:
-- it bites only where a principal is admitted to the resource by some OTHER path while having lost
-- the map — an ex-member who still OWNS a resource homed there. See the conjunct on each arm.
--
-- Body derived from `search_fts_candidates`' live definition, INCLUDING `websearch_to_tsquery` and
-- ts_rank flag 33 (log-length normalization, 20260801000010). Re-deriving from an older migration
-- silently reverts both.

CREATE OR REPLACE FUNCTION search_exact(
    p_principal    uuid,
    p_query        text,
    p_anchor_table varchar DEFAULT NULL,
    p_anchor_id    uuid    DEFAULT NULL)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
    SELECT r.id,
           (ts_rank(si.search_vector, websearch_to_tsquery('english', p_query), 33))::real
      FROM kb_resource_search_index si
      JOIN kb_resources r                      ON r.id = si.resource_id
      JOIN resources_visible_to(p_principal) v ON v.resource_id = r.id
     WHERE p_query IS NOT NULL AND p_query <> ''
       AND r.is_active
       AND r.ingest_state = 'complete'
       AND si.search_vector @@ websearch_to_tsquery('english', p_query)
       -- The anchor pair. One predicate for both anchor kinds, indexed by
       -- idx_kb_resource_homes_anchor(anchor_table, anchor_id) — which is why a cogmap no longer
       -- needs its members materialized into a uuid[] first. Keyed on the id, not the table: an
       -- anchor_table with no id names nothing, so it scopes nothing.
       --
       -- Guarded by the anchor's own READABILITY: an anchor you cannot read scopes nothing, however
       -- visible the resources homed in it are to you. This is the conjunct `cogmap_scope_ids`
       -- carried and this arm's first draft dropped.
       --
       -- Cogmap-only BY KIND. `cogmap_readable_by_profile` is CALLED, never restated — it is the
       -- predicate. It is guarded by the kind rather than applied unconditionally because this
       -- pair is generic over `p_anchor_table`, and a context anchor must not be run through a
       -- cogmap function. A context anchor is already gated in Rust, by `resolve_context_ref`
       -- (`substrate_read.rs`), and is deliberately left alone here — restoring one dropped
       -- conjunct is not the moment to add a gate the incumbent never had.
       --
       -- `IS DISTINCT FROM` rather than `<>` so a NULL `p_anchor_table` is a clean TRUE instead of
       -- a NULL that has to be reasoned about; such a row is then excluded by the EXISTS anyway,
       -- since `h.anchor_table = NULL` matches nothing.
       AND (p_anchor_id IS NULL OR (
             (p_anchor_table IS DISTINCT FROM 'kb_cogmaps'
                OR cogmap_readable_by_profile(p_principal, p_anchor_id))
             AND EXISTS (
               SELECT 1 FROM kb_resource_homes h
                WHERE h.resource_id = r.id
                  AND h.anchor_table = p_anchor_table
                  AND h.anchor_id = p_anchor_id)));
$$;

COMMENT ON FUNCTION search_exact(uuid, text, varchar, uuid) IS
$c$The exact arm of /api/search: term matching over the FTS index, ordered by `fts_norm` alone.

`fts_norm` is ts_rank with flag 33 (32's rank/(rank+1) normalization plus flag 1's log-length
division), so it lies in [0,1). It is this arm's own quantity and is never combined with the wide
arm's `vec_norm` — the two measure different things and summing them is a category error, which is
what this phase exists to stop.$c$;

-- ── search_wide ──────────────────────────────────────────────────────────────────────────────────
--
-- The arm for "you have the idea, not the words". Returns `vec_norm` alone.
--
-- Both branches are carried over from `search_vector_candidates`' live definition, INCLUDING the
-- deliberate asymmetry between them (20260801000010:139-145): the unscoped branch applies
-- `LIMIT p_k` inside `ann` and lands visibility/active/complete AFTER it, because applying them
-- inside forces a seq-scan and defeats idx_kb_chunks_embedding; the scoped branch pre-filters into
-- `scoped_res` and carries NO top-k, so its aggregate over `ann` already sees every current chunk of
-- every scoped resource.
--
-- That asymmetry means the arm changes ALGORITHM on scope, not merely filter — scoped is exhaustive,
-- unscoped is approximate — so adding a scope can surface resources an unscoped search structurally
-- cannot. Carried forward deliberately and unchanged; disclosing it is the read path's problem, and
-- it is named in the task's Still open.
--
-- The only edit is the scope predicate: the anchor pair replaces `p_context_id` + `p_scope_ids`,
-- guarded by the same cogmap-readability gate `search_exact` carries (see there for why it is a
-- separate predicate and why it is keyed on the anchor kind).
CREATE OR REPLACE FUNCTION search_wide(
    p_principal    uuid,
    p_emb          vector,
    p_k            int,
    p_anchor_table varchar DEFAULT NULL,
    p_anchor_id    uuid    DEFAULT NULL)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE plpgsql STABLE AS $$
BEGIN
  IF p_anchor_id IS NULL THEN
    RETURN QUERY
    WITH ann AS (
      SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
        FROM kb_chunks c
       WHERE p_emb IS NOT NULL AND c.is_current
       ORDER BY c.embedding <=> p_emb
       LIMIT p_k
    ),
    admitted AS (
      SELECT DISTINCT a.resource_id
        FROM ann a
        JOIN kb_resources r                      ON r.id = a.resource_id AND r.is_active
                                                AND r.ingest_state = 'complete'
        JOIN resources_visible_to(p_principal) v ON v.resource_id = a.resource_id
    )
    -- Order statistic shrunk toward the mean by the draws that produced it, re-derived over the
    -- resource's FULL current chunk set rather than over `ann` — `ann` is the top-k, so aggregating
    -- there conditions on the winners and the correction collapses to a no-op.
    SELECT ad.resource_id,
           (1.0 - (MIN(c.embedding <=> p_emb)
                 + (AVG(c.embedding <=> p_emb) - MIN(c.embedding <=> p_emb))
                   * (1.0 - 1.0 / sqrt(count(*)::float8))
                  ) / 2.0)::real
      FROM admitted ad
      JOIN kb_chunks c ON c.resource_id = ad.resource_id AND c.is_current
     GROUP BY ad.resource_id;
  ELSE
    -- The same anchor-readability gate `search_exact` carries, in the form this arm can express:
    -- a guard clause, because `p_anchor_id` is non-NULL on this branch by construction and
    -- `cogmap_readable_by_profile` does not depend on the row. An unreadable map returns the empty
    -- set WITHOUT scanning — the ANN work is never started rather than filtered afterwards.
    --
    -- Cogmap-only by kind, and the predicate is CALLED not restated, for the reasons given at
    -- length on `search_exact`'s conjunct above.
    IF p_anchor_table = 'kb_cogmaps'
       AND NOT cogmap_readable_by_profile(p_principal, p_anchor_id) THEN
      RETURN;
    END IF;

    RETURN QUERY
    WITH scoped_res AS (
      SELECT v.resource_id AS id
        FROM resources_visible_to(p_principal) v
        JOIN kb_resources r ON r.id = v.resource_id AND r.is_active
                           AND r.ingest_state = 'complete'
       WHERE EXISTS (
               SELECT 1 FROM kb_resource_homes h
                WHERE h.resource_id = v.resource_id
                  AND h.anchor_table = p_anchor_table
                  AND h.anchor_id = p_anchor_id)
    ),
    ann AS (
      SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
        FROM kb_chunks c
        JOIN scoped_res s ON s.id = c.resource_id
       WHERE p_emb IS NOT NULL AND c.is_current
    )
    -- Aggregating over `ann` is correct HERE and only here: this branch carries no top-k, so `ann`
    -- already holds the full draw set.
    SELECT a.resource_id,
           (1.0 - (MIN(a.dist)
                 + (AVG(a.dist) - MIN(a.dist)) * (1.0 - 1.0 / sqrt(count(*)::float8))
                  ) / 2.0)::real
      FROM ann a
     GROUP BY a.resource_id;
  END IF;
END;
$$;

-- ── THE ef_search PIN DOES NOT INHERIT. THIS BLOCK IS WHY. ───────────────────────────────────────
--
-- 20260804000030 pinned `hnsw.ef_search = 200` on `search_vector_candidates(uuid, vector, integer,
-- uuid, uuid[])` via `pg_proc.proconfig`. That binding is to one SIGNATURE. `search_wide` is a new
-- function, so it inherits NOTHING and would run at the server default of 40 — below any k the
-- caller passes, making `LIMIT p_k` unreachable and truncating the draw silently. Measured on prod
-- before the original fix: 33.4 chunks and 24.3 admitted resources per query where an exact scan
-- admits 66.2.
--
-- THE WARMUP IS LOAD-BEARING. DO NOT DELETE IT. pgvector registers `hnsw.ef_search` in `_PG_init`,
-- which runs on FIRST USE of a vector type in a backend. Until then the name is an unregistered
-- placeholder, and PostgreSQL lets only a SUPERUSER set a placeholder. A migration runs on a cold
-- connection, so without this line the next statement fails for an ordinary role with
-- `permission denied to set parameter "hnsw.ef_search"` — which is exactly how 20260804000030
-- failed its first Vercel deploy.
--
-- NEITHER LOCAL VERIFICATION NOR CI CAN CATCH ITS ABSENCE: the docker `temper` role is
-- `usesuper = t`, as is the role in every CI Postgres service container, and for a superuser
-- setting a placeholder is permitted. Only a real deploy against Neon exercises the ordinary-role
-- path.
DO $$ BEGIN PERFORM '[1,2]'::vector <=> '[1,3]'::vector; END $$;

ALTER FUNCTION search_wide(uuid, vector, integer, varchar, uuid)
    SET hnsw.ef_search = 200;

COMMENT ON FUNCTION search_wide(uuid, vector, integer, varchar, uuid) IS
$c$The wide arm of /api/search: nearest-neighbour retrieval over chunk embeddings, ordered by
`vec_norm` alone.

`vec_norm` rescales the pgvector cosine DISTANCE — which spans [0,2] — as `1 - d/2`, so it lies in
[0,1]. Note that `wayfind_region_scores.query_cos` rescales the SAME operator as `1 - d`, spanning
[-1,1]; the two are not the same quantity and neither column name says so. This arm's number is
never combined with the exact arm's `fts_norm`.

`hnsw.ef_search` is pinned on this function because the server default (40) sits below the k callers
ask for. The pin does NOT inherit from search_vector_candidates — proconfig binds to a signature.$c$;

SELECT declare_migration(
    20260805000020,
    'additive',
    'Phase 1 (task 019fd25e): search_exact and search_wide, the two arms of /api/search, added beside unified_search. Each returns its own quantity (fts_norm, vec_norm) with no weight and no companion to rank it against. Scoping is the anchor pair (anchor_table, anchor_id) for both, replacing the context-EXISTS/cogmap-uuid[] split, and is guarded by cogmap_readable_by_profile on the cogmap kind — the conjunct cogmap_scope_ids carried, which resources_visible_to does NOT imply for an ex-member who still owns a homed resource. New names rather than replaces because a parameter change makes a new function in PostgreSQL; overloading would also make search_surface_a.rs pinned_ef_search read a nondeterministic pg_proc row. hnsw.ef_search is re-pinned on search_wide because proconfig binds to a signature and does not inherit. Nothing is dropped or altered, so a binary either side of this is unaffected.'
);
