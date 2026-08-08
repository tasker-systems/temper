-- The composable find twins: one body per arm, and a bound set is a scope.
--
-- `/api/query` compiles a composition into ONE statement in which a stage may be narrowed by the
-- ids an upstream stage produced. The deployed find arms cannot express that: `search_exact` (7
-- args) and `search_wide` (8) take an ANCHOR PAIR — one `(table, id)` — and have no resource-id
-- array. `search_graph_expand`'s `p_seed_ids uuid[]` is the only fragment input already speaking the
-- composition's currency, which is why the compiler's CTE skeleton fits it and only it.
--
-- Post-filtering is NOT the escape. `20260806000020` put `ORDER BY score DESC, resource_id
-- LIMIT p_limit OFFSET p_offset` INSIDE both arms, so a bound applied to their output filters after
-- truncation — the wide-then-filter defect this arc retired. A correctness rule, not a tuning choice.
--
-- And the incumbent signatures may not change: DROP/CREATE is shape-breaking and needs an operator
-- cutover, while CREATE OR REPLACE at an unchanged signature is additive and auto-deploys. So this
-- adds NEW functions beside them and re-points the incumbents at those, rather than altering either.
--
-- ── ONE BODY PER ARM ────────────────────────────────────────────────────────────────────────────
--
-- The twins are created by MOVING 20260808000020's now-clean bodies, and the incumbents become
-- delegating wrappers passing `p_bound_ids => NULL`. After this migration `search_exact` IS
-- `query_find_exact` with NULL bounds — not a second implementation that agrees with it today. That
-- distinction is the whole point: two bodies drift, and the drift would be silent because both
-- would keep returning plausible rows.
--
-- ── THREE SEMANTICS, none of them inferable from the signature ──────────────────────────────────
--
-- 1. A BOUND SET IS A SCOPE. In `query_find_wide` the branch predicate becomes
--    `IF p_anchor_id IS NULL AND p_bound_ids IS NULL THEN <top-k> ELSE <exhaustive>`. The exhaustive
--    branch has no truncation to defeat, so the rank-shaped rule — visibility and narrowing must sit
--    BENEATH any ORDER BY/LIMIT over a score — is satisfied STRUCTURALLY rather than by a rule
--    someone has to remember. The fork `search_wide` already carries is the fork the rule describes.
--
-- 2. EMPTY IS NOT ABSENT. `p_bound_ids = '{}'` means bounded-to-nothing ⇒ zero rows; only NULL means
--    unbounded. An upstream stage that returned empty must never silently become an unbounded
--    search — "an empty upstream is a disclosed disposition, never a substitution." Written as
--    `(p_bound_ids IS NULL OR x = ANY(p_bound_ids))`, NEVER as a cardinality test: `'{}'` makes
--    `= ANY` false for every row, which is exactly right, whereas `cardinality(...) = 0 OR ...`
--    would silently widen.
--
-- 3. `query_find_wide` CARRIES ITS OWN ef_search PIN. `proconfig` binds to a SIGNATURE and does not
--    inherit. A pin on a wrapper does reach a nested call, so `/api/search` stays safe through the
--    delegation — but `/api/query` calls the twin DIRECTLY and would otherwise draw at the server
--    default of 40, below any k the caller passes, truncating with no error and no symptom.
--    The incumbent pin test reads `WHERE proname = 'search_wide'` and cannot see this door.

-- ── 1. The exact twin ───────────────────────────────────────────────────────────────────────────
--
-- Body moved from 20260808000020's `search_exact`. The ONLY addition is the bound conjunct, and it
-- sits inside the inner subquery — BENEATH the `ORDER BY`/`LIMIT`, which is the whole point. The
-- exact arm's ordering is by `fts_norm`, so narrowing above the LIMIT would return a page thinned
-- after truncation exactly as it would on the wide arm.
CREATE FUNCTION query_find_exact(
    p_principal    uuid,
    p_query        text,
    p_bound_ids    uuid[]  DEFAULT NULL,
    p_anchor_table varchar DEFAULT NULL,
    p_anchor_id    uuid    DEFAULT NULL,
    p_doc_type     text    DEFAULT NULL,
    p_limit        int     DEFAULT NULL,
    p_offset       int     DEFAULT 0)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
    SELECT s.rid, s.score
      FROM (
        SELECT r.id AS rid,
               (ts_rank(si.search_vector, websearch_to_tsquery('english', p_query), 33))::real
                   AS score
          FROM kb_resource_search_index si
          JOIN kb_resources_live r                 ON r.id = si.resource_id
          JOIN resources_visible_to(p_principal) v ON v.resource_id = r.id
         WHERE p_query IS NOT NULL AND p_query <> ''
           AND si.search_vector @@ websearch_to_tsquery('english', p_query)
           -- The bound. NULL is unbounded; '{}' admits nothing.
           AND (p_bound_ids IS NULL OR r.id = ANY(p_bound_ids))
           AND (p_doc_type IS NULL OR EXISTS (
                 SELECT 1 FROM kb_resource_doc_type dt
                  WHERE dt.resource_id = r.id AND dt.doc_type = p_doc_type))
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

-- ── 2. The wide twin ────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION query_find_wide(
    p_principal    uuid,
    p_emb          vector,
    p_k            int,
    p_bound_ids    uuid[]  DEFAULT NULL,
    p_anchor_table varchar DEFAULT NULL,
    p_anchor_id    uuid    DEFAULT NULL,
    p_doc_type     text    DEFAULT NULL,
    p_limit        int     DEFAULT NULL,
    p_offset       int     DEFAULT 0)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE plpgsql STABLE AS $$
BEGIN
  -- A BOUND SET IS A SCOPE: it takes the exhaustive branch, which has no top-k to defeat.
  IF p_anchor_id IS NULL AND p_bound_ids IS NULL THEN
    RETURN QUERY
    WITH ann AS (
      -- The global top-k chunk draw. Predicates that are not index-supported stay OUT of this CTE;
      -- `c.embedding IS NOT NULL` is here only because pgvector's HNSW never indexes a NULL vector,
      -- so it cannot cost a scan.
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
       WHERE (p_doc_type IS NULL OR EXISTS (
               SELECT 1 FROM kb_resource_doc_type dt
                WHERE dt.resource_id = r.id AND dt.doc_type = p_doc_type))
    ),
    scored AS (
      -- Re-derived over the resource's full current EMBEDDED chunk set rather than over `ann` —
      -- `ann` is the top-k, so aggregating there conditions on the winners and the shrinkage
      -- collapses to a no-op. "EMBEDDED" also keeps an unembedded chunk out of count(*), which
      -- would inflate the denominator: a wrong number rather than an error.
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
    -- The anchor-readability guard, unchanged. Guarded by `p_anchor_id IS NOT NULL` because this
    -- branch is now also reached by a BOUNDED call with no anchor at all — where p_anchor_table is
    -- NULL, the `=` yields NULL rather than true, so the guard correctly does not fire. Stated
    -- rather than left to the reader: the branch condition is no longer "an anchor was supplied".
    IF p_anchor_id IS NOT NULL AND p_anchor_table = 'kb_cogmaps'
       AND NOT cogmap_readable_by_profile(p_principal, p_anchor_id) THEN
      RETURN;
    END IF;

    RETURN QUERY
    WITH scoped_res AS (
      SELECT v.resource_id AS id
        FROM resources_visible_to(p_principal) v
        JOIN kb_resources_live r ON r.id = v.resource_id
       -- The bound and the anchor are INDEPENDENT narrowings, and either may be absent: this branch
       -- is reached by an anchored call, a bounded call, or both. Each is therefore its own
       -- NULL-guarded conjunct rather than an either/or.
       WHERE (p_bound_ids IS NULL OR v.resource_id = ANY(p_bound_ids))
         AND (p_anchor_id IS NULL OR EXISTS (
               SELECT 1 FROM kb_resource_homes h
                WHERE h.resource_id = v.resource_id
                  AND h.anchor_table = p_anchor_table
                  AND h.anchor_id = p_anchor_id))
         -- This branch is exhaustive — no draw to defeat — so doc_type sits with the other row
         -- predicates rather than after a top-k.
         AND (p_doc_type IS NULL OR EXISTS (
               SELECT 1 FROM kb_resource_doc_type dt
                WHERE dt.resource_id = v.resource_id AND dt.doc_type = p_doc_type))
    ),
    ann AS (
      -- The embedding guard is strictly load-bearing here: this branch has no top-k, so its
      -- aggregate reads `ann` directly and an unembedded chunk would otherwise reach count(*).
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

-- ── 3. The incumbents become delegating wrappers ────────────────────────────────────────────────
--
-- Signatures byte-identical, so this is CREATE OR REPLACE and additive. `NULL::uuid[]` is cast
-- explicitly: a bare NULL is ambiguous against a DEFAULTed parameter list.
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
    SELECT q.resource_id, q.fts_norm
      FROM query_find_exact(p_principal, p_query, NULL::uuid[],
                            p_anchor_table, p_anchor_id, p_doc_type, p_limit, p_offset) q;
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
LANGUAGE sql STABLE AS $$
    SELECT q.resource_id, q.vec_norm
      FROM query_find_wide(p_principal, p_emb, p_k, NULL::uuid[],
                           p_anchor_table, p_anchor_id, p_doc_type, p_limit, p_offset) q;
$$;

-- ── 4. Pins. BOTH functions, and the redundancy is deliberate. ──────────────────────────────────
--
-- `query_find_wide` needs one because `/api/query` calls it directly and proconfig does not inherit.
--
-- `search_wide` keeps one even though it now only delegates, and that is NOT belt-and-braces for its
-- own sake: `CREATE OR REPLACE` above discarded the pin 20260808000020 applied (proconfig is reset
-- wholesale by a replace — measured), so omitting this line would UNPIN the door `/api/search` uses.
-- A `LANGUAGE sql` wrapper carrying a SET clause is also ineligible for inlining, which keeps the
-- delegation an actual function call rather than something the planner flattens.
--
-- THE WARMUP IS LOAD-BEARING. DO NOT DELETE IT. pgvector registers `hnsw.ef_search` in `_PG_init`,
-- which runs on FIRST USE of a vector type in a backend. Until then the name is an unregistered
-- placeholder and only a SUPERUSER may set one. A migration runs on a cold connection, so without
-- this line the next statement fails for an ordinary role with `permission denied to set parameter
-- "hnsw.ef_search"` — how 20260804000030 failed its first Vercel deploy. Neither local verification
-- nor CI can catch its absence: those roles are superusers. Only a real deploy exercises it.
DO $$ BEGIN PERFORM '[1,2]'::vector <=> '[1,3]'::vector; END $$;

ALTER FUNCTION query_find_wide(uuid, vector, integer, uuid[], varchar, uuid, text, int, int)
    SET hnsw.ef_search = 200;

ALTER FUNCTION search_wide(uuid, vector, integer, varchar, uuid, text, int, int)
    SET hnsw.ef_search = 200;

COMMENT ON FUNCTION query_find_exact(uuid, text, uuid[], varchar, uuid, text, int, int) IS
$c$The exact find fragment, composable: `search_exact` plus `p_bound_ids`.

`search_exact` IS this function with NULL bounds — it delegates here, so there is one body and one
scoring revision, not two that agree today.

`p_bound_ids` NULL means unbounded; '{}' means bounded to nothing and returns zero rows. The bound
is applied BENEATH the ORDER BY/LIMIT, so a bounded page is never a page thinned after
truncation.$c$;

COMMENT ON FUNCTION query_find_wide(uuid, vector, integer, uuid[], varchar, uuid, text, int, int) IS
$c$The wide find fragment, composable: `search_wide` plus `p_bound_ids`.

A BOUND SET IS A SCOPE. Supplying one selects the exhaustive branch — the same branch an anchor
selects — because the approximate branch draws a global top-k before any narrowing, and a bound
applied to that output would filter after truncation. The rank-shaped rule is satisfied structurally
here rather than by convention.

`p_bound_ids` NULL means unbounded; '{}' means bounded to nothing. Bound and anchor are independent:
either, both, or neither may be supplied.

Carries its own `hnsw.ef_search` pin — proconfig binds to a signature and does not inherit, and
/api/query calls this function directly rather than through `search_wide`.$c$;

SELECT declare_migration(
    20260808000030,
    'additive',
    'Adds query_find_exact and query_find_wide — the deployed find arms plus a p_bound_ids uuid[], the currency /api/query compositions speak and the one search_graph_expand.p_seed_ids already accepts — and re-points search_exact/search_wide at them as delegating wrappers, so there is one body per arm rather than two that can drift. A bound set is a SCOPE: it selects query_find_wide''s exhaustive branch, which has no top-k, so narrowing can never be applied after truncation. Empty is not absent: p_bound_ids = ''{}'' returns zero rows and only NULL is unbounded, so an upstream stage that produced nothing cannot silently widen into a global search. query_find_wide carries its own hnsw.ef_search pin because proconfig binds to a signature and /api/query calls it directly; search_wide''s pin is re-applied because CREATE OR REPLACE discards proconfig. Additive: two CREATE FUNCTION, CREATE OR REPLACE on two existing functions at byte-identical signatures, two ALTER FUNCTION SET; no DROP.'
);
