-- Give both search arms paging and the doc_type filter, in SQL.
--
-- Two defects, one signature change, one cutover. Both were found reviewing PR #656 and both are
-- regressions against `unified_search` rather than pre-existing flaws.
--
-- ── 1. THE ARMS RETURNED THE WHOLE MATCH SET ────────────────────────────────────────────────────
--
-- `unified_search` ended `ORDER BY combined_score DESC, id LIMIT p_limit OFFSET p_offset`, so at
-- most `limit` rows ever crossed the wire. `search_exact` (20260805000020) carried no ORDER BY and
-- no LIMIT at all, and `search_wide`'s scoped branch carried no bound either. `search_select`
-- sorted and sliced in Rust — but only AFTER `enrich_hits` had built a full `ResourceView`, with a
-- per-row LATERAL aggregate over `kb_properties`, for every returned row.
--
-- MEASURED against production (3,402 live complete resources, 2026-08-07), counting the rows
-- `search_exact` would return for a request asking for ten:
--
--     temper    1,883      resource  1,502      data   741
--     test      1,778      design    1,277      search 561
--
-- So a common single-token query hydrated ~55% of the corpus and discarded 99.5% of it, inside a
-- function with a 60-second ceiling, at a cost that scales with the corpus rather than the page.
-- `offset` reduced nothing: `offset 100000` did identical work to `offset 0`.
--
-- The ordering moves back into SQL with it. Rust's tiebreak was `(score DESC, id ASC)`; carried
-- here verbatim, because a LIMIT without a total order makes the page boundary arbitrary — two
-- rows tied on score could swap between pages and a caller walking offsets would see one twice and
-- another never.
--
-- ── 2. `doc_type` WAS ADVERTISED AND FILTERED NOTHING ───────────────────────────────────────────
--
-- `unified_search` took `p_doc_type` and applied it (20260714000001:281-285); `substrate_read.rs`
-- bound `params.doc_type` into it. The two-arm split dropped the parameter while leaving the door
-- open on every surface: `temper search --doc-type <t>` ("Filter by document type"), the
-- `SearchParams` field, three temper-client methods, the MCP tool and the published OpenAPI schema.
-- `temper search "auth" --doc-type decision` returned every doc type at 200 OK with no hint.
--
-- It was retired as if it were a blend internal — filed alongside term zeroing, `w_fts`/`w_vec`,
-- `seed_only` and hop-0 de-bias. Every other item on that list is invisible from outside the
-- function. This one had a CLI flag. The conjunct is carried back verbatim from the incumbent,
-- including `owner_table = 'kb_resources'` — `kb_properties` is polymorphic and a predicate that
-- omits the owner kind is not the same predicate.
--
-- Interim, by decision: the compositional property querying in
-- docs/superpowers/specs/2026-08-05-query-builder-compositional-design.md will likely supersede
-- this with a general property filter. `--doc-type` is among the most-used filters today, so it is
-- restored rather than left broken until that lands. `[decided — 2026-08-07, Pete]`
--
-- WHERE THE CONJUNCT GOES IS NOT FREE, and it differs per branch:
--
--   * `search_exact` — in the WHERE, beside the other row predicates. Nothing is bounded before it.
--   * `search_wide`, UNSCOPED — in `admitted`, AFTER `LIMIT p_k`, never in `ann`. That branch draws
--     the global top-k chunks and applies its predicates afterwards, deliberately, so the HNSW
--     index is not defeated (see 20260805000020's header). Filtering doc_type inside the draw would
--     put a non-indexed predicate under the ANN `ORDER BY ... LIMIT`, which is the exact shape that
--     turns an index scan into a sequential one. The cost of placing it after is that a doc_type
--     filter on an unscoped search filters the top-k rather than searching within the type — the
--     same known asymmetry visibility already has on this branch, and not a new one.
--   * `search_wide`, SCOPED — in `scoped_res`, with the other row predicates. That branch is
--     exhaustive and has no draw to defeat.
--
-- ── WHY A NEW SIGNATURE, AND WHY THAT MAKES THIS shape-breaking ─────────────────────────────────
--
-- A parameter change makes a NEW function in PostgreSQL; functions are keyed by name plus argument
-- types. So this DROPs both arms and recreates them, and a DROP FUNCTION is not additive — the
-- deploy's additive-only gate halts here exactly as it halts at 20260806000010. That costs nothing
-- extra: the operator cutover for that migration is already mandatory, and this rides in the same
-- one. After merge it would cost a second cutover of its own.
--
-- The three new parameters carry DEFAULTs so the 4- and 5-argument call sites in the substrate
-- suite keep working unchanged, and `LIMIT NULL` is PostgreSQL's spelling of "no limit" — so a
-- caller that passes no `p_limit` gets the previous unbounded behaviour rather than an empty page.
-- Defaults are safe here BECAUSE the old signatures are dropped first: two live overloads of one
-- name is what makes `pinned_ef_search` read a nondeterministic `pg_proc` row (30e08f59).
--
-- `p_k` and `p_limit` are DIFFERENT UNITS and both are needed. `p_k` bounds the ANN draw in CHUNKS
-- on the unscoped branch; `p_limit` bounds the answer in RESOURCES, after aggregation, on both.
-- Collapsing them would silently change what the shrinkage factor is computed over.

DROP FUNCTION IF EXISTS search_exact(uuid, text, character varying, uuid);
DROP FUNCTION IF EXISTS search_wide(uuid, vector, integer, character varying, uuid);

CREATE FUNCTION search_exact(
    p_principal    uuid,
    p_query        text,
    p_anchor_table varchar DEFAULT NULL,
    p_anchor_id    uuid    DEFAULT NULL,
    p_doc_type     text    DEFAULT NULL,
    p_limit        int     DEFAULT NULL,
    p_offset       int     DEFAULT 0)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
    -- Wrapped in a subquery so the ORDER BY can name the score once rather than repeating the
    -- ts_rank expression, and so the inner names cannot be read as the RETURNS TABLE out-params.
    SELECT s.rid, s.score
      FROM (
        SELECT r.id AS rid,
               (ts_rank(si.search_vector, websearch_to_tsquery('english', p_query), 33))::real
                   AS score
          FROM kb_resource_search_index si
          JOIN kb_resources r                      ON r.id = si.resource_id
          JOIN resources_visible_to(p_principal) v ON v.resource_id = r.id
         WHERE p_query IS NOT NULL AND p_query <> ''
           AND r.is_active
           AND r.ingest_state = 'complete'
           AND si.search_vector @@ websearch_to_tsquery('english', p_query)
           -- Carried back from unified_search (20260714000001). `owner_table` is load-bearing:
           -- kb_properties is polymorphic and keys are not unique across owner kinds.
           AND (p_doc_type IS NULL OR EXISTS (
                 SELECT 1 FROM kb_properties p
                  WHERE p.owner_table = 'kb_resources' AND p.owner_id = r.id
                    AND p.property_key = 'doc_type' AND NOT p.is_folded
                    AND p.property_value #>> '{}' = p_doc_type))
           -- The anchor pair, and the anchor's own readability. Unchanged from 20260805000020 —
           -- `cogmap_readable_by_profile` is CALLED, never restated, and is guarded by kind because
           -- the pair is generic over p_anchor_table and a context anchor must not be run through a
           -- cogmap function. A context anchor is gated in Rust by resolve_context_ref.
           AND (p_anchor_id IS NULL OR (
                 (p_anchor_table IS DISTINCT FROM 'kb_cogmaps'
                    OR cogmap_readable_by_profile(p_principal, p_anchor_id))
                 AND EXISTS (
                   SELECT 1 FROM kb_resource_homes h
                    WHERE h.resource_id = r.id
                      AND h.anchor_table = p_anchor_table
                      AND h.anchor_id = p_anchor_id)))
      ) s
     ORDER BY s.score DESC, s.rid
     LIMIT p_limit OFFSET p_offset;
$$;

CREATE FUNCTION search_wide(
    p_principal    uuid,
    p_emb          vector,
    p_k            int,
    p_anchor_table varchar DEFAULT NULL,
    p_anchor_id    uuid    DEFAULT NULL,
    p_doc_type     text    DEFAULT NULL,
    p_limit        int     DEFAULT NULL,
    p_offset       int     DEFAULT 0)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE plpgsql STABLE AS $$
BEGIN
  IF p_anchor_id IS NULL THEN
    RETURN QUERY
    WITH ann AS (
      -- The global top-k chunk draw. Predicates that are not index-supported stay OUT of this CTE;
      -- `c.embedding IS NOT NULL` is here only because pgvector's HNSW never indexes a NULL vector,
      -- so it cannot cost a scan (measured, 20260805000020 header).
      SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
        FROM kb_chunks c
       WHERE p_emb IS NOT NULL AND c.is_current AND c.embedding IS NOT NULL
       ORDER BY c.embedding <=> p_emb
       LIMIT p_k
    ),
    admitted AS (
      SELECT DISTINCT a.resource_id
        FROM ann a
        JOIN kb_resources r                      ON r.id = a.resource_id AND r.is_active
                                                AND r.ingest_state = 'complete'
        JOIN resources_visible_to(p_principal) v ON v.resource_id = a.resource_id
       -- doc_type lands HERE, after the draw, for the reason given at length in the header.
       WHERE (p_doc_type IS NULL OR EXISTS (
               SELECT 1 FROM kb_properties p
                WHERE p.owner_table = 'kb_resources' AND p.owner_id = r.id
                  AND p.property_key = 'doc_type' AND NOT p.is_folded
                  AND p.property_value #>> '{}' = p_doc_type))
    ),
    scored AS (
      -- Order statistic shrunk toward the mean by the draws that produced it, re-derived over the
      -- resource's full current EMBEDDED chunk set rather than over `ann` — `ann` is the top-k, so
      -- aggregating there conditions on the winners and the correction collapses to a no-op.
      --
      -- "EMBEDDED" carries two jobs. A resource with no embedded current chunks joins to nothing
      -- and emits no group — absent rather than a NULL vec_norm. And MIN/AVG skip NULLs while
      -- count(*) does not, so an unembedded chunk reaching here would inflate the shrinkage
      -- denominator: a wrong number rather than an error.
      SELECT ad.resource_id AS rid,
             (1.0 - (MIN(c.embedding <=> p_emb)
                   + (AVG(c.embedding <=> p_emb) - MIN(c.embedding <=> p_emb))
                     * (1.0 - 1.0 / sqrt(count(*)::float8))
                    ) / 2.0)::real AS score
        FROM admitted ad
        JOIN kb_chunks c ON c.resource_id = ad.resource_id AND c.is_current
                        AND c.embedding IS NOT NULL
       GROUP BY ad.resource_id
    )
    SELECT s.rid, s.score
      FROM scored s
     ORDER BY s.score DESC, s.rid
     LIMIT p_limit OFFSET p_offset;
  ELSE
    -- The same anchor-readability gate search_exact carries, in the form this arm can express: a
    -- guard clause, because p_anchor_id is non-NULL on this branch by construction. An unreadable
    -- map returns empty WITHOUT scanning — the ANN work is never started rather than filtered after.
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
         -- This branch is exhaustive — no draw to defeat — so doc_type sits with the other row
         -- predicates rather than after a top-k.
         AND (p_doc_type IS NULL OR EXISTS (
               SELECT 1 FROM kb_properties p
                WHERE p.owner_table = 'kb_resources' AND p.owner_id = v.resource_id
                  AND p.property_key = 'doc_type' AND NOT p.is_folded
                  AND p.property_value #>> '{}' = p_doc_type))
    ),
    ann AS (
      -- The guard is strictly load-bearing here: this branch has no top-k, so its aggregate reads
      -- `ann` directly and an unembedded chunk would otherwise reach count(*).
      SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
        FROM kb_chunks c
        JOIN scoped_res s ON s.id = c.resource_id
       WHERE p_emb IS NOT NULL AND c.is_current AND c.embedding IS NOT NULL
    ),
    scored AS (
      -- Aggregating over `ann` is correct HERE and only here: no top-k, so `ann` is the full draw.
      SELECT a.resource_id AS rid,
             (1.0 - (MIN(a.dist)
                   + (AVG(a.dist) - MIN(a.dist)) * (1.0 - 1.0 / sqrt(count(*)::float8))
                    ) / 2.0)::real AS score
        FROM ann a
       GROUP BY a.resource_id
    )
    SELECT s.rid, s.score
      FROM scored s
     ORDER BY s.score DESC, s.rid
     LIMIT p_limit OFFSET p_offset;
  END IF;
END;
$$;

-- ── THE ef_search PIN DOES NOT INHERIT, AND THIS SIGNATURE IS NEW ───────────────────────────────
--
-- `pg_proc.proconfig` binds to a SIGNATURE. The pin 20260805000020 set on
-- `search_wide(uuid, vector, integer, varchar, uuid)` died with that function three statements ago;
-- the 8-argument function created above inherits NOTHING and would run at the server default of 40
-- — below any k the caller passes, making `LIMIT p_k` unreachable and truncating the draw silently.
-- Measured on prod before the original fix: 33.4 chunks and 24.3 admitted resources per query where
-- an exact scan admits 66.2. This is the single most dangerous line in this migration to omit,
-- because nothing fails: results merely get quietly worse.
--
-- THE WARMUP IS LOAD-BEARING. DO NOT DELETE IT. pgvector registers `hnsw.ef_search` in `_PG_init`,
-- which runs on FIRST USE of a vector type in a backend. Until then the name is an unregistered
-- placeholder, and PostgreSQL lets only a SUPERUSER set a placeholder. A migration runs on a cold
-- connection, so without this line the next statement fails for an ordinary role with
-- `permission denied to set parameter "hnsw.ef_search"` — which is how 20260804000030 failed its
-- first Vercel deploy.
--
-- NEITHER LOCAL VERIFICATION NOR CI CAN CATCH ITS ABSENCE: the docker `temper` role is
-- `usesuper = t`, as is the role in every CI Postgres service container, and for a superuser
-- setting a placeholder is permitted. Only a real deploy against Neon exercises the ordinary role.
DO $$ BEGIN PERFORM '[1,2]'::vector <=> '[1,3]'::vector; END $$;

ALTER FUNCTION search_wide(uuid, vector, integer, varchar, uuid, text, int, int)
    SET hnsw.ef_search = 200;

COMMENT ON FUNCTION search_exact(uuid, text, varchar, uuid, text, int, int) IS
$c$The exact arm of /api/search: term matching over the FTS index, ordered by `fts_norm` alone.

`fts_norm` is ts_rank with flag 33 (32's rank/(rank+1) normalization plus flag 1's log-length
normalization). It is never combined with the wide arm's `vec_norm`; the two measure different
things on different scales and no field anywhere ranks one against the other.

Paging is `ORDER BY fts_norm DESC, resource_id` then LIMIT/OFFSET. The tiebreak is part of the
contract, not a detail: without a total order a tied row can appear on two pages or none.
`p_limit` NULL means unbounded.

`p_doc_type` filters on the resource's `doc_type` property. NULL means no filter.$c$;

COMMENT ON FUNCTION search_wide(uuid, vector, integer, varchar, uuid, text, int, int) IS
$c$The wide arm of /api/search: nearest-neighbour retrieval over chunk embeddings, ordered by
`vec_norm` alone.

`vec_norm` rescales the pgvector cosine DISTANCE — which spans [0,2] — as `1 - d/2`, so it lies in
[0,1]. Note that `wayfind_region_scores.query_cos` rescales the SAME operator as `1 - d`, spanning
[-1,1]; the two are not the same quantity and neither column name says so. This arm's number is
never combined with the exact arm's `fts_norm`.

`p_k` bounds the ANN draw in CHUNKS and applies to the unscoped branch only; `p_limit` bounds the
answer in RESOURCES and applies to both. They are different units.

THE ARM CHANGES ALGORITHM ON SCOPE: unscoped is an approximate global top-k, scoped is exhaustive.
Adding a scope can therefore surface resources an unscoped search structurally cannot. Deliberate —
it is what keeps the HNSW index usable.$c$;

SELECT declare_migration(
    20260806000020,
    'shape-breaking',
    'Gives search_exact and search_wide SQL-side paging (ORDER BY score DESC, resource_id + LIMIT/OFFSET) and restores the p_doc_type filter the two-arm split dropped while leaving its CLI flag, SearchParams field, MCP parameter and OpenAPI schema in place. Both arms previously returned their entire match set — measured at 1,883 rows against a 3,402-resource production corpus for a request asking for ten — and every row was hydrated into a full ResourceView before Rust sliced the page, so cost scaled with the corpus rather than the page and offset reduced no work. Shape-breaking because changing a function signature in PostgreSQL means DROP and CREATE rather than REPLACE, and a DROP FUNCTION is not additive; it rides the cutover 20260806000010 already requires. The hnsw.ef_search pin is re-applied to the new 8-argument search_wide signature, preceded by the _PG_init vector warmup — proconfig binds to a signature and inherits nothing, and its absence has no symptom other than a silently truncated ANN draw.'
);
