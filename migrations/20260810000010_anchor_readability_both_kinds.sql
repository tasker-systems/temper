-- Anchor readability now covers BOTH anchor kinds, and the exact wrapper gets the wide one's
-- guaranteed-empty guard. Replace-in-place at byte-identical signatures; no shape change.
--
-- ── THE GAP (adjudicated 2026-08-10, Pete — ADJ-1) ──────────────────────────────────────────────
--
-- The ungated find cores in `20260808000030` checked anchor readability for cogmaps ONLY. The
-- exact core's guard disjunct — `p_anchor_table IS DISTINCT FROM 'kb_cogmaps' OR
-- COALESCE(cogmap_readable_by_profile(…), false)` — is unconditionally TRUE for `kb_contexts`,
-- and the wide core's early-RETURN guard likewise fired only when `p_anchor_table = 'kb_cogmaps'`.
-- Consequence: a caller could bound a stage by a CONTEXT it cannot read and learn its membership —
-- exactly the disclosure the cogmap arm exists to refuse, reachable through the sibling kind. No
-- resource leaked (every row still comes from the caller's asserted visible set); what leaked was
-- which resources are homed where, for a home the principal has no right to see.
--
-- Ruling ADJ-1, option (a): close it in SQL, symmetrically, using
-- `anchor_readable_by_profile(p_profile, p_anchor_table, p_anchor_id)` — which already exists
-- (`20260712000010`), dispatches on the table, and fails CLOSED (`ELSE false`) on an unknown or
-- NULL table. Both cores now ask the one dispatching predicate instead of special-casing one kind.
--
-- THE RENDERING RULE: an unreadable anchor of EITHER kind renders as an EMPTY STAGE, never an
-- error — the existence-oracle rule. A refusal that errored would tell the caller the anchor
-- exists; zero rows is indistinguishable from an anchor with nothing homed in it, exactly as the
-- cogmap arm already rendered.
--
-- ── THE ADJ-6 GUARD, ruled rather than measured ─────────────────────────────────────────────────
--
-- `query_find_wide` carries a `CASE WHEN p_emb IS NULL THEN NULL::uuid[] ELSE ARRAY(…) END` so a
-- guaranteed-empty call never pays the visibility-gate expansion. `query_find_exact` did not,
-- though its core's first qual (`p_query IS NOT NULL AND p_query <> ''`) makes a NULL/empty-query
-- call guaranteed-empty the same way — a function's arguments are evaluated before its body, so
-- the wrapper materialized the whole visible set and handed it to a body that returns nothing.
-- Task 6 never measured this arm; the wide arm's cost (~3.9 ms of expansion for zero rows) was
-- measured and this is the same shape. Guarded symmetrically BY RULING rather than by a fresh
-- measurement (ADJ-6). Handing NULL is not a shortcut around the verdict: a NULL `p_visible_ids`
-- admits NOTHING, the same zero rows the empty query produces inside the body.
--
-- ── THE `p_anchor_reader` CHARTER ───────────────────────────────────────────────────────────────
--
-- `p_anchor_reader` exists for ANCHOR READABILITY ONLY, now covering both anchor kinds, and must
-- never gain another use. It is the ONE authorization the "ungated" cores perform — row visibility
-- comes exclusively from `p_visible_ids`, and a core given a full visible set returns rows this
-- reader cannot see; that is the point of the split. No prior design doc specified this parameter;
-- this header is its authorization (adjudicated P4/ADJ-1). A future edit that reads it anywhere
-- other than the anchor-readability predicate is widening the cores' authorization surface and
-- needs its own adjudication, not a refactor's shrug.

-- ── 1. The exact core ───────────────────────────────────────────────────────────────────────────
--
-- Body copied verbatim from `20260808000030`, except the anchor guard: the cogmap-only disjunct
-- becomes the dispatching predicate, so a context anchor is now checked too.
CREATE OR REPLACE FUNCTION __temper_ungated_find_exact(
    p_visible_ids   uuid[],
    p_query         text,
    p_bound_ids     uuid[]  DEFAULT NULL,
    p_anchor_table  varchar DEFAULT NULL,
    p_anchor_id     uuid    DEFAULT NULL,
    p_anchor_reader uuid    DEFAULT NULL,
    p_doc_type      text    DEFAULT NULL,
    p_limit         int     DEFAULT NULL,
    p_offset        int     DEFAULT 0)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
    SELECT s.rid, s.score
      FROM (
        SELECT r.id AS rid,
               (ts_rank(si.search_vector, websearch_to_tsquery('english', p_query), 33))::real
                   AS score
          FROM kb_resource_search_index si
          JOIN kb_resources_live r                     ON r.id = si.resource_id
          -- The verdict, handed in. NOT `= ANY` — see 20260808000030's header.
          JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = r.id
         WHERE p_query IS NOT NULL AND p_query <> ''
           AND si.search_vector @@ websearch_to_tsquery('english', p_query)
           -- The bound. NULL is unbounded; '{}' admits nothing.
           AND (p_bound_ids IS NULL OR r.id = ANY(p_bound_ids))
           AND (p_doc_type IS NULL OR EXISTS (
                 SELECT 1 FROM kb_resource_doc_type dt
                  WHERE dt.resource_id = r.id AND dt.doc_type = p_doc_type))
           -- The anchor guard, BOTH kinds (ADJ-1). `anchor_readable_by_profile` dispatches on the
           -- table and fails closed on an unknown or NULL one; `COALESCE(…, false)` so a NULL
           -- reader denies. An unreadable anchor renders as an empty stage, never an error.
           AND (p_anchor_id IS NULL OR (
                 COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                          false)
                 AND EXISTS (
                   SELECT 1 FROM kb_resource_homes h
                    WHERE h.resource_id = r.id
                      AND h.anchor_table = p_anchor_table
                      AND h.anchor_id = p_anchor_id)))
      ) s
     ORDER BY s.score DESC, s.rid
     LIMIT p_limit OFFSET p_offset;
$$;

-- ── 2. The wide core ────────────────────────────────────────────────────────────────────────────
--
-- Body copied verbatim from `20260808000030`, both branches intact, except the early-RETURN guard:
-- cogmap-only becomes both-kinds.
CREATE OR REPLACE FUNCTION __temper_ungated_find_wide(
    p_visible_ids   uuid[],
    p_emb           vector,
    p_k             int,
    p_bound_ids     uuid[]  DEFAULT NULL,
    p_anchor_table  varchar DEFAULT NULL,
    p_anchor_id     uuid    DEFAULT NULL,
    p_anchor_reader uuid    DEFAULT NULL,
    p_doc_type      text    DEFAULT NULL,
    p_limit         int     DEFAULT NULL,
    p_offset        int     DEFAULT 0)
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
        JOIN kb_resources_live r                     ON r.id = a.resource_id
        JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = a.resource_id
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
    -- The anchor-readability guard, BOTH kinds (ADJ-1). Guarded by `p_anchor_id IS NOT NULL`
    -- because this branch is also reached by a BOUNDED call with no anchor at all — no anchor
    -- means nothing to refuse. With an anchor, `anchor_readable_by_profile` dispatches on the
    -- table and fails closed (`ELSE false`) on an unknown or NULL one; `COALESCE(…, false)` so a
    -- NULL reader denies. An unreadable anchor of either kind renders as an EMPTY STAGE, never an
    -- error — the existence-oracle rule.
    IF p_anchor_id IS NOT NULL
       AND NOT COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                        false) THEN
      RETURN;
    END IF;

    RETURN QUERY
    WITH scoped_res AS (
      -- DISTINCT, because `unnest` of an array carries no uniqueness while the
      -- `resources_visible_to` join this replaced did: that function is four UNIONed arms, so its
      -- output is distinct BY CONSTRUCTION. A duplicated id here would duplicate chunk rows into
      -- `ann`, inflating `count(*)` and shifting MIN/AVG inside `shrunk_best_of_n` — a silently
      -- WRONG SCORE rather than an error. The sibling `admitted` CTE in the other branch already
      -- carries DISTINCT for the same reason; this branch had lost it in the move.
      SELECT DISTINCT v.resource_id AS id
        FROM unnest(p_visible_ids) AS v(resource_id)
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

-- ── 3. The exact wrapper gains the guaranteed-empty guard (ADJ-6) ───────────────────────────────
--
-- **THE `CASE` IS NOT A FLOURISH — WITHOUT IT A QUERY-LESS CALL PAYS A FULL GATE EXPANSION.**
--
-- The exact core's first qual (`p_query IS NOT NULL AND p_query <> ''`) makes a NULL/empty-query
-- call guaranteed-empty, the same way `p_emb IS NULL` does on the wide arm. A function's arguments
-- are evaluated BEFORE its body, so an unconditional `ARRAY(SELECT … FROM resources_visible_to(…))`
-- here materializes the principal's entire visible set — recursive team closure and all — and hands
-- it to a body that returns zero rows anyway. Task 6 cleared the array path against a query that
-- RETURNS ROWS; it never measured this arm's guaranteed-empty call. Guarded symmetrically with the
-- wide wrapper BY RULING rather than measurement (ADJ-6).
--
-- `CASE` evaluates only the selected result expression, so the empty case never touches the gate.
-- Handing NULL is not a shortcut around the verdict — a NULL `p_visible_ids` admits NOTHING, which
-- is the same zero rows the empty query produces inside the body, reached without the work.
CREATE OR REPLACE FUNCTION query_find_exact(
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
    SELECT c.resource_id, c.fts_norm
      FROM __temper_ungated_find_exact(
             CASE WHEN p_query IS NULL OR p_query = '' THEN NULL::uuid[]
                  ELSE ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v) END,
             p_query, p_bound_ids, p_anchor_table, p_anchor_id, p_principal,
             p_doc_type, p_limit, p_offset) c;
$$;

-- ── 4. Re-pin the wide core. ────────────────────────────────────────────────────────────────────
--
-- `CREATE OR REPLACE` above discarded the `hnsw.ef_search` pin `20260808000030` applied to
-- `__temper_ungated_find_wide` — proconfig is reset wholesale by a replace, measured — so it is
-- re-applied here, warmup first. `query_find_wide` and `search_wide` were not replaced, so their
-- pins stand; `query_find_exact` never carried one (the exact arm draws no ANN).
--
-- THE WARMUP IS LOAD-BEARING. DO NOT DELETE IT. pgvector registers `hnsw.ef_search` in `_PG_init`,
-- which runs on FIRST USE of a vector type in a backend. Until then the name is an unregistered
-- placeholder and only a SUPERUSER may set one. A migration runs on a cold connection, so without
-- this line the next statement fails for an ordinary role with `permission denied to set parameter
-- "hnsw.ef_search"` — how 20260804000030 failed its first Vercel deploy. Neither local verification
-- nor CI can catch its absence: those roles are superusers. Only a real deploy exercises it.
DO $$ BEGIN PERFORM '[1,2]'::vector <=> '[1,3]'::vector; END $$;

ALTER FUNCTION __temper_ungated_find_wide(
        uuid[], vector, integer, uuid[], varchar, uuid, uuid, text, int, int)
    SET hnsw.ef_search = 200;

COMMENT ON FUNCTION __temper_ungated_find_exact(
        uuid[], text, uuid[], varchar, uuid, uuid, text, int, int) IS
$c$THE EXACT FIND BODY, WITH NO VISIBILITY GATE. Do not call this from anywhere that has not already
established an RBAC verdict.

`p_visible_ids` IS that verdict, handed in as a value because a CTE cannot be passed to a function.
The caller is trusted absolutely for it. NULL admits NOTHING (`unnest(NULL)` is zero rows), which is
the fail-closed direction — note this is the OPPOSITE of `p_bound_ids`, where NULL means unbounded
and only '{}' narrows to nothing.

`p_visible_ids` MUST BE DISTINCT. `resources_visible_to` is four UNIONed arms and so is distinct by
construction, but `unnest` of an arbitrary array is not, and a duplicate is not merely redundant: on
the exact arm it duplicates an output row, and on the wide arm it duplicates chunk rows into the
score aggregate, shifting MIN/AVG and inflating count(*) inside `shrunk_best_of_n` — a silently
wrong score rather than an error. The wide arm defends itself with DISTINCT; this is stated as a
precondition because a direct caller can violate it.

`p_anchor_reader` is NOT a visibility gate, and anchor readability is its ONLY use (its charter —
20260810000010). It is read only by `anchor_readable_by_profile`, which dispatches on the anchor
table and covers BOTH kinds (ADJ-1: a context anchor is checked exactly as a cogmap one, because
"may this principal use this anchor as a scope" is one boolean per call and a property of no row, so
it cannot ride in `p_visible_ids`). An unreadable anchor of either kind renders as an empty stage,
never an error. A core given a full `p_visible_ids` will return rows that reader cannot see — that
is the point of the function.

Reached by `/api/search` through `search_exact` -> `query_find_exact`, and by `/api/query` through a
single compiler emitter whose id source is fixed to the hoisted `__temper_vis` CTE. The CI tripwire
`audit-ungated-fragments.sh` derives the call-site set from this prefix. Both are source discipline,
NOT a database permission: the application connects as the owning role, so anyone with a psql
connection can call this directly. Accepted residue, spec §6.$c$;

COMMENT ON FUNCTION __temper_ungated_find_wide(
        uuid[], vector, integer, uuid[], varchar, uuid, uuid, text, int, int) IS
$c$THE WIDE FIND BODY, WITH NO VISIBILITY GATE. Same contract as `__temper_ungated_find_exact` —
read that comment for `p_visible_ids`, `p_anchor_reader` (anchor readability, BOTH kinds, and its
only use), and the accepted residue.

A BOUND SET IS A SCOPE. Supplying one selects the exhaustive branch, the same branch an anchor
selects, because the approximate branch draws a global top-k before any narrowing and a bound applied
to that output would filter after truncation.

Holds the family's only ANN draw, so its `hnsw.ef_search` pin is the one about a body rather than a
call.$c$;

COMMENT ON FUNCTION query_find_exact(uuid, text, uuid[], varchar, uuid, text, int, int) IS
$c$The exact find fragment, composable and GATED: computes the caller's visible set once and hands it
to `__temper_ungated_find_exact`.

`search_exact` IS this function with NULL bounds — it delegates here, so there is one body and one
scoring revision across the whole family rather than several that agree today.

`p_bound_ids` NULL means unbounded; '{}' means bounded to nothing and returns zero rows. The bound is
applied BENEATH the ORDER BY/LIMIT, so a bounded page is never a page thinned after truncation.

A NULL or empty query skips the visible-set expansion entirely (the CASE — ADJ-6): the core's first
qual makes such a call guaranteed-empty, and a NULL `p_visible_ids` is the same zero rows reached
without materializing the gate.$c$;

SELECT declare_migration(
    20260810000010,
    'additive',
    'Closes the context-anchor readability gap in the ungated find cores (ADJ-1, 2026-08-10): both cores checked anchor readability for cogmaps only, so a caller could bound a stage by a context it cannot read and learn its membership. CREATE OR REPLACE at byte-identical signatures on __temper_ungated_find_exact, __temper_ungated_find_wide and query_find_exact — replace-in-place, no shape change, no DROP. The cogmap-only guards become COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id), false), which dispatches on the anchor table and fails closed on an unknown or NULL one; an unreadable anchor of EITHER kind renders as an empty stage, never an error (the existence-oracle rule). query_find_exact additionally gains the wide wrapper''s CASE guard (ADJ-6, ruled not measured): a NULL/empty query is guaranteed-empty by the core''s first qual, so the wrapper hands NULL::uuid[] instead of materializing the visible set for an arm that returns nothing. The wide core''s hnsw.ef_search pin is re-applied after the replace (proconfig is reset wholesale), warmup first; the wrappers'' pins were not replaced and stand.'
);
