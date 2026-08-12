-- Extract the interiority search_exact and search_wide each carry several copies of.
--
-- Behaviour-preserving by construction: two views, one IMMUTABLE function, and both arms rewritten
-- onto them at BYTE-IDENTICAL signatures. Nothing about which rows come back, or what they score,
-- changes — which is why this is its own migration, and why the deliverable is that
-- `search_exact_and_wide.rs` passes WITHOUT EDIT. A test file that had to be touched here would
-- mean behaviour moved and the separation was pointless.
--
-- Counted from the bodies of 20260806000020, not remembered:
--   * the doc_type property lookup   — 3 copies (search_exact; search_wide's `admitted`; its `scoped_res`)
--   * the is_active/ingest_state pair — 3 copies (same three sites)
--   * the shrunk best-of-N score      — 2 copies (search_wide's two branches)
--
-- ── WHY VIEWS AND NOT SCALAR FUNCTIONS ──────────────────────────────────────────────────────────
--
-- The obvious extraction — a `LANGUAGE sql STABLE` predicate function — is a PERFORMANCE
-- REGRESSION, not a cleanup, and it was measured before this was written:
--
--   INCUMBENT (inline EXISTS)              VIA SCALAR FUNCTION
--    Seq Scan on kb_resources r             Seq Scan on kb_resources r
--      Filter: (ANY (r.id = (hashed           Filter: spike_has_doc_type(r.id, 'note'::text)
--              SubPlan 2).col1))
--      SubPlan 2
--        -> Index Only Scan using
--           uq_kb_properties_active
--
-- The index path disappears and a hashed set becomes a per-row call. The blocker is the sublink in
-- the body. The same predicate sourced from a VIEW is plan-identical to the incumbent, down to the
-- same hashed SubPlan and the same index — and with p_doc_type NULL the OR collapses and no SubPlan
-- is planned at all. See spec §2.1/§2.2; do not re-derive this.
--
-- `shrunk_best_of_n` is a function rather than a view because it is arithmetic over ALREADY
-- AGGREGATED values — called once per group, not once per row — so the hazard above does not reach
-- it. Its body contains no sub-select.
--
-- ── WHAT IS DELIBERATELY NOT EXTRACTED ──────────────────────────────────────────────────────────
--
-- The anchor EXISTS over kb_resource_homes: `\d kb_resource_homes` carries no is_folded/is_current/
-- is_active column, so the predicate is complete as written and there is no hidden knowledge for a
-- view to protect.
--
-- The anchor readability gate: it already CALLS `cogmap_readable_by_profile` rather than restating
-- it, and its two forms are load-bearing per-arm differences rather than drift — search_exact
-- carries it as a WHERE conjunct, search_wide as a guard clause with an early RETURN. The guard
-- returns WITHOUT SCANNING, which a conjunct could not do.
--
-- Also not extracted: a wide pre-joined view gathering doc_type alongside the rest of the resource
-- shape. Refused already, on grounds still live — routing an arm through a pre-joined view puts the
-- join back inside the ranked query, which is exactly what the two-arm split removed.
-- `kb_resource_doc_type` is not that view: it is a narrow PREDICATE SOURCE (one table, three
-- constant filters), living in an EXISTS under the WHERE, contributing no output columns and
-- carrying no visibility claim. Predicate source vs. hydration spine is the distinction.

-- ── 1. The doc_type property lookup, once ───────────────────────────────────────────────────────
--
-- `owner_table` is load-bearing and is the knowledge this view exists to home: kb_properties is
-- polymorphic and property keys are NOT unique across owner kinds, so a lookup that forgets it
-- silently matches a cogmap's or a block's `doc_type`. The two-arm split dropped that filter once
-- already and 20260806000020 had to restore it; with one definition there is nowhere for a second
-- copy to drop it from.
CREATE VIEW kb_resource_doc_type AS
    SELECT p.owner_id AS resource_id,
           p.property_value #>> '{}' AS doc_type
      FROM kb_properties p
     WHERE p.owner_table = 'kb_resources'
       AND p.property_key = 'doc_type'
       AND NOT p.is_folded;

COMMENT ON VIEW kb_resource_doc_type IS
$c$The resource doc_type lookup, defined once. A PREDICATE SOURCE for an EXISTS, never a join for
hydration — routing a ranked arm through a pre-joined view is separately refused.

Homes the polymorphic-key knowledge (`owner_table`, `property_key`, `is_folded`, `#>> '{}'`) that
kb_properties requires and that a hand-written copy has already been observed to lose.$c$;

-- ── 2. The live-row predicate, once ─────────────────────────────────────────────────────────────
--
-- Makes "`ingest_state = 'complete'` goes exactly where `r.is_active` goes" a STRUCTURAL property
-- rather than a rule each new call site has to remember.
--
-- ID-ONLY, DELIBERATELY. `SELECT r.*` would be wrong in a way that is invisible: PostgreSQL expands
-- `*` at view-CREATION time, so a column added to kb_resources later would simply never appear
-- through this view — a view that looks like it tracks the table and does not. Both arms read only
-- `r.id` from this relation anyway, so the id is the whole of what is needed.
CREATE VIEW kb_resources_live AS
    SELECT r.id
      FROM kb_resources r
     WHERE r.is_active
       AND r.ingest_state = 'complete';

COMMENT ON VIEW kb_resources_live IS
$c$Resources that are both undeleted and fully ingested — the pair of predicates every read floor
carries. Id-only on purpose: a `SELECT *` view freezes its column list at creation time.

An interrupted ingest is not a document: `ingest_state = 'complete'` belongs exactly where
`is_active` does, and this view is what makes that structural.$c$;

-- ── 3. The shrunk best-of-N score, once ─────────────────────────────────────────────────────────
--
-- IMMUTABLE and called on already-aggregated values, so the non-inlining hazard above does not
-- apply. Two copies of a scoring formula WILL drift, and search_wide's two branches would then
-- silently score differently — the same corpus ranked two ways depending on whether the caller
-- passed a scope.
CREATE FUNCTION shrunk_best_of_n(
    p_min double precision,
    p_avg double precision,
    p_n   bigint)
RETURNS real
LANGUAGE sql IMMUTABLE AS $$
    SELECT (1.0 - (p_min + (p_avg - p_min) * (1.0 - 1.0 / sqrt(p_n::float8))) / 2.0)::real
$$;

COMMENT ON FUNCTION shrunk_best_of_n(double precision, double precision, bigint) IS
$c$Best-of-N distance shrunk toward the mean by the number of draws that produced it, rescaled from
pgvector's cosine DISTANCE span [0,2] into [0,1].

Takes aggregates rather than computing them: the caller decides what population it is aggregating
over, and that choice is not the same in both of search_wide's branches — the unscoped branch must
re-derive over the resource's full embedded chunk set (aggregating over its own top-k would
condition on the winners and collapse the correction to a no-op), while the scoped branch has no
top-k and aggregates its draw directly.$c$;

-- ── 4. Both arms, rewritten onto the three. Signatures byte-identical ⇒ additive. ───────────────

CREATE OR REPLACE FUNCTION search_exact(
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
          -- was: JOIN kb_resources r ... AND r.is_active AND r.ingest_state = 'complete'
          JOIN kb_resources_live r                 ON r.id = si.resource_id
          JOIN resources_visible_to(p_principal) v ON v.resource_id = r.id
         WHERE p_query IS NOT NULL AND p_query <> ''
           AND si.search_vector @@ websearch_to_tsquery('english', p_query)
           AND (p_doc_type IS NULL OR EXISTS (
                 SELECT 1 FROM kb_resource_doc_type dt
                  WHERE dt.resource_id = r.id AND dt.doc_type = p_doc_type))
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

CREATE OR REPLACE FUNCTION search_wide(
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
        JOIN kb_resources_live r                 ON r.id = a.resource_id
        JOIN resources_visible_to(p_principal) v ON v.resource_id = a.resource_id
       -- doc_type lands HERE, after the draw, for the reason given at length in 20260806000020.
       WHERE (p_doc_type IS NULL OR EXISTS (
               SELECT 1 FROM kb_resource_doc_type dt
                WHERE dt.resource_id = r.id AND dt.doc_type = p_doc_type))
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
             shrunk_best_of_n(MIN(c.embedding <=> p_emb),
                              AVG(c.embedding <=> p_emb),
                              count(*)) AS score
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
        JOIN kb_resources_live r ON r.id = v.resource_id
       WHERE EXISTS (
               SELECT 1 FROM kb_resource_homes h
                WHERE h.resource_id = v.resource_id
                  AND h.anchor_table = p_anchor_table
                  AND h.anchor_id = p_anchor_id)
         -- This branch is exhaustive — no draw to defeat — so doc_type sits with the other row
         -- predicates rather than after a top-k.
         AND (p_doc_type IS NULL OR EXISTS (
               SELECT 1 FROM kb_resource_doc_type dt
                WHERE dt.resource_id = v.resource_id AND dt.doc_type = p_doc_type))
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
             shrunk_best_of_n(MIN(a.dist), AVG(a.dist), count(*)) AS score
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

-- ── 5. RE-APPLY THE ef_search PIN. CREATE OR REPLACE DROPS proconfig. ───────────────────────────
--
-- MEASURED, not assumed, because this is the kind of thing that is easy to reason wrongly about:
--
--   ALTER FUNCTION __probe() SET hnsw.ef_search = 200;
--   SELECT proconfig FROM pg_proc WHERE proname='__probe';   -- {hnsw.ef_search=200}
--   CREATE OR REPLACE FUNCTION __probe() ... ;
--   SELECT proconfig FROM pg_proc WHERE proname='__probe';   -- NULL
--
-- `CREATE OR REPLACE FUNCTION` redefines the function's properties wholesale, so every SET clause
-- applied by a previous `ALTER FUNCTION` is discarded. Replacing search_wide above therefore UNPINS
-- it, and it would run at the server default of 40 — below any k the caller passes, making
-- `LIMIT p_k` unreachable and truncating the ANN draw with no error and no symptom. 20260806000020
-- calls the equivalent line "the single most dangerous line in this migration to omit, because
-- nothing fails: results merely get quietly worse."
--
-- `search_exact` needs no pin: it draws no ANN candidates.
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

SELECT declare_migration(
    20260808000020,
    'additive',
    'Extracts the three predicate families search_exact and search_wide each carried multiple copies of — the doc_type property lookup (3 copies), the is_active/ingest_state live-row pair (3), and the shrunk best-of-N score (2) — into two views and one IMMUTABLE function, and rewrites both arms onto them at unchanged signatures. Extraction is by VIEW and not by scalar function because a LANGUAGE sql STABLE predicate whose body contains a sublink does not inline: measured, the doc_type EXISTS loses its Index Only Scan on uq_kb_properties_active and becomes a per-row call, while the view form is plan-identical to the incumbent. The hnsw.ef_search pin is re-applied to search_wide because CREATE OR REPLACE FUNCTION discards proconfig — also measured — and an unpinned ANN draw truncates silently with no error. Additive: two CREATE VIEW, one CREATE FUNCTION, CREATE OR REPLACE on two existing functions at byte-identical signatures, and one ALTER FUNCTION SET; no DROP. Row sets and scores are unchanged, which the existing search_exact_and_wide suite asserts without edit.'
);
