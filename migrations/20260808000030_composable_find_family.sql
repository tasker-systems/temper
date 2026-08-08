-- The find family, in final form: one body per arm, a bound set is a scope, and the gate is
-- computed once no matter how many stages ask.
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
-- ── THE THREE-LEVEL CHAIN, AND WHY IT IS THREE ──────────────────────────────────────────────────
--
--   search_exact(p_principal, …)              -- the deployed door. Unchanged signature.
--     -> query_find_exact(p_principal, …, p_bound_ids)   -- composable, GATED: computes the verdict
--       -> __temper_ungated_find_exact(p_visible_ids, …) -- the real body. Applies NO gate.
--
-- The middle level exists because `/api/query` needs `p_bound_ids` and `/api/search` must not change
-- shape. The bottom level exists because a composition must compute visibility ONCE: every stage
-- calling a gate-internally fragment means an N-stage composition evaluates `resources_visible_to`
-- N times, each with its own `Recursive Union` team closure. The planner does not dedupe those —
-- `provolatile = 's'` permits caching within a scan, not common-subexpression elimination across
-- call sites. Measured, and corpus-independent.
--
-- A CTE cannot be passed to a function, so the only way to hand one verdict to N stages is to hand
-- it as a VALUE. Hence `p_visible_ids uuid[]`.
--
-- ONE BODY PER ARM is the invariant the whole chain exists to preserve: `search_exact` IS
-- `query_find_exact` at NULL bounds IS `__temper_ungated_find_exact` over the caller's own visible
-- set. Not three implementations that agree today — two bodies drift, and the drift is silent
-- because both keep returning plausible rows.
--
-- ── WHY ROUTING /api/search THROUGH AN UNGATED CORE IS SAFE ─────────────────────────────────────
--
-- `/api/search` also routes through the core and so also pays the visible-set materialization. That
-- is deliberate and is not available to avoid: letting the incumbent keep its own gated join would
-- restore two bodies per arm and forfeit the point.
--
-- Converting a gate JOIN into an array is PR #659's no-equivalence-class hazard run in reverse, so
-- it was MEASURED before being written (plan Task 6, evidence
-- `docs/superpowers/plans/evidence/2026-08-08-task6-gate-shape.md`). Over a 4802-resource corpus
-- with a principal seeing 3201 of them, total cost including array construction was 6.95 ms
-- incumbent against 7.13 ms array — ~2.6%, inside the run-to-run spread of either column. Row-set
-- equality (3201 both ways) was confirmed before any timing was compared. Given the set already
-- built, the array path is ~4x faster, which is what a composition gains.
--
-- **`unnest`, NEVER `= ANY`.** `= ANY(uuid[])` is precisely the form that establishes no equivalence
-- class and so cannot propagate a join condition. The measurement compared `JOIN unnest($1::uuid[])`
-- and that is what is written below; `= ANY` would be a different query than the one cleared.
--
-- The wide arm's unscoped branch joins the gate AFTER its top-k, which reads like it only needed to
-- check `p_k` rows — but `resources_visible_to` is parameterized by principal alone, so the
-- incumbent already evaluates the whole set and hash-joins ≤k rows against it. The array path does
-- the same work plus the array build, exactly as measured on the exact arm. No separate measurement
-- is claimed for it; the shapes are the same shape.
--
-- ── THREE SEMANTICS, none of them inferable from the signature ──────────────────────────────────
--
-- 1. A BOUND SET IS A SCOPE. In the wide body the branch predicate is
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
-- 3. THE ef_search PIN DOES NOT INHERIT. `proconfig` binds to a SIGNATURE. A pin on an outer
--    function does reach a nested call, but a function called DIRECTLY gets only its own — and both
--    `query_find_wide` (called by `/api/query` before the cores landed) and the core itself are
--    directly callable. All three levels are pinned; see the pin block for which claim each makes.
--
-- ── TWO uuid[] PARAMETERS, OPPOSITE NULL SEMANTICS ──────────────────────────────────────────────
--
-- `p_visible_ids` NULL admits NOTHING. `unnest(NULL::uuid[])` yields zero rows, so a caller that
-- computed no verdict gets no rows rather than the corpus — fail-closed, structurally.
-- `p_bound_ids` NULL is UNBOUNDED, per semantic 2 above. Two same-typed parameters three positions
-- apart that mean opposite things by NULL; both directions are witnessed by
-- `a_null_visible_set_admits_nothing_though_a_null_bound_set_is_unbounded`.
--
-- ── THE CORES ARE DELIBERATELY UNSAFE, AND THAT IS WHAT THE NAME SAYS ───────────────────────────
--
-- `__temper_ungated_` follows the `__temper_unbound_act` convention beat C established for an
-- identifier that must never be read as ordinary. `_private` would say "internal detail" and invite
-- a caller; this cannot be misread.
--
-- **Naming is not the guard.** `.github/scripts/audit-ungated-fragments.sh` derives the call-site set
-- from the prefix and fails closed on any addition, and the compiler emits these calls from a single
-- function whose id source is not a parameter. Both are SOURCE DISCIPLINE, not a database
-- permission: anyone holding psql, a Neon console, or the app credentials can call a core with an
-- arbitrary `uuid[]` and receive ungated rows. `REVOKE` buys nothing, because the application
-- connects as the owning role. This is a real, accepted residue of the split (spec §6), stated here
-- so it is not a discovery later.
--
-- ── ANCHOR READABILITY IS A SECOND QUESTION AND CANNOT RIDE IN THE ID SET ───────────────────────
--
-- Two different authorizations travel through these functions. *Which resources may this principal
-- see* is a row set — hoisting it is the entire point. *May this principal use this cogmap as a
-- scope* is one boolean per call, is a property of no row, and therefore cannot be expressed as
-- `p_visible_ids` at all. So the cores keep that check and take `p_anchor_reader` for it.
--
-- **`p_anchor_reader` IS NOT A VISIBILITY GATE.** Row visibility comes exclusively from
-- `p_visible_ids`; this parameter is read only by `cogmap_readable_by_profile`, and a core called
-- with a full `p_visible_ids` returns rows the reader cannot see. It is a principal rather than a
-- caller-asserted boolean on purpose: `p_visible_ids` is already one asserted verdict, and a second
-- one would let a caller declare its own anchor authorization. The core cannot be lied to about
-- readability.
--
-- Dropping this check would leak no resource — every row still comes from the caller's asserted
-- visible set — but it would leak MEMBERSHIP of a map the principal cannot read, which is the
-- disclosure `an_unreadable_cogmap_anchor_scopes_nothing_in_either_arm` pins one level up.
--
-- Wrapped in `COALESCE(…, false)` in both cores so a NULL `p_anchor_reader` denies rather than
-- yielding a NULL that neither admits nor refuses. `cogmap_readable_by_profile` returns
-- `EXISTS(…) OR profile_explicit_grant(…)` and so is believed never to be NULL — the COALESCE is
-- here so this function's fail-closed behaviour does not depend on reading a third one.

-- ── 1. The exact core ───────────────────────────────────────────────────────────────────────────
--
-- Body moved from 20260808000020's `search_exact`. The additions are the bound conjunct and the
-- `unnest(p_visible_ids)` gate join; the anchor guard's principal becomes `p_anchor_reader`.
--
-- The bound sits INSIDE the inner subquery — BENEATH the `ORDER BY`/`LIMIT`, which is the whole
-- point. The exact arm's ordering is by `fts_norm`, so narrowing above the LIMIT would return a page
-- thinned after truncation exactly as it would on the wide arm.
CREATE FUNCTION __temper_ungated_find_exact(
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
          -- The verdict, handed in. NOT `= ANY` — see the header.
          JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = r.id
         WHERE p_query IS NOT NULL AND p_query <> ''
           AND si.search_vector @@ websearch_to_tsquery('english', p_query)
           -- The bound. NULL is unbounded; '{}' admits nothing.
           AND (p_bound_ids IS NULL OR r.id = ANY(p_bound_ids))
           AND (p_doc_type IS NULL OR EXISTS (
                 SELECT 1 FROM kb_resource_doc_type dt
                  WHERE dt.resource_id = r.id AND dt.doc_type = p_doc_type))
           AND (p_anchor_id IS NULL OR (
                 (p_anchor_table IS DISTINCT FROM 'kb_cogmaps'
                    OR COALESCE(cogmap_readable_by_profile(p_anchor_reader, p_anchor_id), false))
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
-- Body moved from 20260808000020's `search_wide`, both branches intact.
CREATE FUNCTION __temper_ungated_find_wide(
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
    -- The anchor-readability guard. Guarded by `p_anchor_id IS NOT NULL` because this branch is also
    -- reached by a BOUNDED call with no anchor at all — where p_anchor_table is NULL, the `=` yields
    -- NULL rather than true, so the guard correctly does not fire. Stated rather than left to the
    -- reader: the branch condition is no longer "an anchor was supplied".
    -- `COALESCE(…, false)` so a NULL reader denies.
    IF p_anchor_id IS NOT NULL AND p_anchor_table = 'kb_cogmaps'
       AND NOT COALESCE(cogmap_readable_by_profile(p_anchor_reader, p_anchor_id), false) THEN
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

-- ── 3. The composable twins: gated wrappers ─────────────────────────────────────────────────────
--
-- Each computes the caller's visible set ONCE and hands it down; the principal continues down as
-- `p_anchor_reader`, which is the only thing the core still asks a principal for.
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
    SELECT c.resource_id, c.fts_norm
      FROM __temper_ungated_find_exact(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_query, p_bound_ids, p_anchor_table, p_anchor_id, p_principal,
             p_doc_type, p_limit, p_offset) c;
$$;

-- **THE `CASE` IS NOT A FLOURISH — WITHOUT IT A TEXT-ONLY SEARCH PAYS A FULL GATE EXPANSION.**
--
-- `/api/search` runs both arms unconditionally and in parallel: `substrate_read::search_select`
-- calls `search_wide` even when the caller supplied no embedding, binding NULL (`readback::
-- search_wide`: "NULL when None → p_emb NULL → the arm returns nothing"). That is the plain
-- text-only query and the degraded path where embedding failed.
--
-- A function's arguments are evaluated BEFORE its body, so an unconditional
-- `ARRAY(SELECT … FROM resources_visible_to(…))` here would materialize the principal's entire
-- visible set — recursive team closure and all — and then hand it to a body that returns zero rows
-- because `p_emb IS NULL`. Measured on the 4802-resource / 3201-visible corpus: ~1.2 ms before this
-- split, ~4.1 ms after, of which ~3.9 ms is the expansion. Task 6 cleared the array path against a
-- query that RETURNS ROWS; it never measured the guaranteed-empty one.
--
-- `CASE` evaluates only the selected result expression, so the empty case never touches the gate.
-- Handing NULL is not a shortcut around the verdict — a NULL `p_visible_ids` admits NOTHING, which
-- is the same zero rows `p_emb IS NULL` produces inside the body, reached without the work.
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
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.vec_norm
      FROM __temper_ungated_find_wide(
             CASE WHEN p_emb IS NULL THEN NULL::uuid[]
                  ELSE ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v) END,
             p_emb, p_k, p_bound_ids, p_anchor_table, p_anchor_id, p_principal,
             p_doc_type, p_limit, p_offset) c;
$$;

-- ── 4. The incumbents become delegating wrappers ────────────────────────────────────────────────
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

-- ── 5. Pins. ALL THREE LEVELS, and each makes a different claim. ────────────────────────────────
--
-- `__temper_ungated_find_wide` holds the only ANN body in the family, so its pin is about the DRAW:
-- without it the HNSW search draws at the server default of 40, below any k a caller passes,
-- truncating with no error and no symptom.
--
-- `query_find_wide` and `search_wide` no longer draw — they delegate. Their pins are about the CALL:
-- a pin on an outer function DOES reach a nested one, so each entry point asserts that any ANN draw
-- beneath it runs at 200. `search_wide` is re-pinned because `CREATE OR REPLACE` above discarded the
-- pin `20260808000020` applied — proconfig is reset wholesale by a replace, measured — so omitting
-- that line would silently unpin the door `/api/search` uses. A `LANGUAGE sql` wrapper carrying a
-- SET clause is also ineligible for inlining, which keeps each delegation an actual function call
-- rather than something the planner flattens.
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

ALTER FUNCTION query_find_wide(uuid, vector, integer, uuid[], varchar, uuid, text, int, int)
    SET hnsw.ef_search = 200;

ALTER FUNCTION search_wide(uuid, vector, integer, varchar, uuid, text, int, int)
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

`p_anchor_reader` is NOT a visibility gate. It is read only by `cogmap_readable_by_profile`, because
"may this principal use this map as a scope" is one boolean per call and a property of no row, so
it cannot ride in `p_visible_ids`. A core given a full `p_visible_ids` will return rows that reader
cannot see — that is the point of the function.

Reached by `/api/search` through `search_exact` -> `query_find_exact`, and by `/api/query` through a
single compiler emitter whose id source is fixed to the hoisted `__temper_vis` CTE. The CI tripwire
`audit-ungated-fragments.sh` derives the call-site set from this prefix. Both are source discipline,
NOT a database permission: the application connects as the owning role, so anyone with a psql
connection can call this directly. Accepted residue, spec §6.$c$;

COMMENT ON FUNCTION __temper_ungated_find_wide(
        uuid[], vector, integer, uuid[], varchar, uuid, uuid, text, int, int) IS
$c$THE WIDE FIND BODY, WITH NO VISIBILITY GATE. Same contract as `__temper_ungated_find_exact` —
read that comment for `p_visible_ids`, `p_anchor_reader`, and the accepted residue.

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
applied BENEATH the ORDER BY/LIMIT, so a bounded page is never a page thinned after truncation.$c$;

COMMENT ON FUNCTION query_find_wide(uuid, vector, integer, uuid[], varchar, uuid, text, int, int) IS
$c$The wide find fragment, composable and GATED: computes the caller's visible set once and hands it
to `__temper_ungated_find_wide`, which holds the branch logic and the ANN draw.

`p_bound_ids` NULL means unbounded; '{}' means bounded to nothing. Bound and anchor are independent:
either, both, or neither may be supplied.

Carries an `hnsw.ef_search` pin because /api/query may call it directly and proconfig does not
inherit; a pin on a wrapper reaches the nested draw.$c$;

SELECT declare_migration(
    20260808000030,
    'additive',
    'Builds the composable find family in one step. Adds __temper_ungated_find_exact/__temper_ungated_find_wide — the deployed arms'' bodies with their resources_visible_to join replaced by JOIN unnest(p_visible_ids uuid[]), never = ANY, which forms no equivalence class — plus query_find_exact/query_find_wide as GATED wrappers that compute the visible set once, and re-points search_exact/search_wide at those as delegating wrappers at byte-identical signatures. The chain is search_exact -> query_find_exact -> __temper_ungated_find_exact, so there is ONE BODY PER ARM rather than several that can drift, and a /api/query composition computes the visibility relation once instead of once per stage — the planner does not dedupe resources_visible_to across call sites, so an N-stage composition otherwise pays N Recursive Union team closures. /api/search routes through the core too; that array path was measured at ~2.6% against the incumbent join, inside run-to-run noise, before this was written. p_bound_ids adds the currency /api/query compositions speak: NULL is unbounded and ''{}'' returns zero rows, so an upstream stage that produced nothing cannot silently widen into a global search, and a bound selects query_find_wide''s exhaustive branch, which has no top-k, so narrowing can never be applied after truncation. p_visible_ids NULL admits nothing — fail-closed, and the OPPOSITE of p_bound_ids. Anchor readability cannot ride in an id set, so the cores take p_anchor_reader and keep the cogmap check, COALESCEd to false; without it a scope through an unreadable map would disclose its membership. All three levels pin hnsw.ef_search: the core because it holds the only ANN body, the two wrappers because proconfig binds to a signature and each is directly callable, and search_wide because CREATE OR REPLACE discards proconfig. The __temper_ungated_ prefix is source discipline enforced by CI and a single compiler emitter, NOT a database permission — the app connects as the owning role. Additive: four CREATE FUNCTION, CREATE OR REPLACE on two existing functions at byte-identical signatures, three ALTER FUNCTION SET; no DROP.'
);
