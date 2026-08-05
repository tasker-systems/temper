-- Pin `hnsw.ef_search` on `search_vector_candidates`, because the server default is below the k the
-- caller asks for and has been silently truncating the vector arm's draw.
--
-- ── WHAT IS WRONG ────────────────────────────────────────────────────────────────────────────────
--
-- `unified_search` declares `100 AS vector_k` and passes it to `search_vector_candidates`, whose
-- unscoped branch runs `ORDER BY c.embedding <=> p_emb LIMIT 100` against `idx_kb_chunks_embedding`,
-- an HNSW index. `hnsw.ef_search` bounds the dynamic candidate list that scan can build, and on prod
-- it is 40, `source = default` — never set here or anywhere in the repo (a repo-wide grep returns
-- zero hits outside the index DDL). `hnsw.iterative_scan` is likewise `off` at boot default, and the
-- index carries no `reloptions`, so `m=16, ef_construction=64`.
--
-- So the arm delivers roughly a third of what the code asks for. Measured on prod, read-only,
-- 2026-08-04, over the fixed 20-query set `scripts/wayfind-spike/queries.txt`, with real query
-- vectors from `crates/temper-ingest/examples/query_vectors.rs`:
--
--     ef_search   chunks/query   admitted resources/query   % of exact scan   cost
--     ---------   ------------   ------------------------   ---------------   -------------
--     40 (prod)         33.4                       24.3               37%      4.9 ms/query
--     100               84.2                       54.8               83%      5.2 ms/query
--     200               99.8                       65.0               98%      6.1 ms/query
--     400              100.0                       66.2              100%    247.5 ms/query
--
-- HNSW chunks per query: min 8, median 38, max 44 — 18 of 20 queries pinned at or under 40.
-- Validated against the deployed function itself, not merely modelled: calling
-- `search_vector_candidates` over the same 20 vectors returns 486 admitted rows (24.3/query) at
-- ef=40 and 1301 (65.1/query) at ef=200 — identical to the model.
--
-- The 400 row is not a tuning option and is why this migration does not simply pick a large number.
-- `EXPLAIN (ANALYZE)` at ef=400 shows the planner abandoning the index — `Seq Scan on kb_chunks
-- (actual rows=42991)` + top-N heapsort — because pgvector's index cost estimate scales with
-- `ef_search` and eventually loses to a sequential scan. That is why ef=400's numbers match the
-- exact scan exactly: it IS one. The degradation is graceful (exact results, ~40x latency), but the
-- threshold moves with corpus size, cost model and cache state, so it must be re-measured rather
-- than assumed to stay above 200.
--
-- ── SCOPE: EXACTLY ONE CODE PATH ─────────────────────────────────────────────────────────────────
--
-- Six deployed functions use `<=>`; only `search_vector_candidates` and `wayfind_region_scores`
-- carry a LIMIT; and `idx_kb_chunks_embedding` is the ONLY vector index in the database.
-- `wayfind_region_scores` computes `centroid <=> p_emb` over `kb_cogmap_regions`, which has no
-- vector index — an exact sequential computation whose LIMIT applies after a full sort, so it has no
-- `ef_search` exposure. The scoped branch of `search_vector_candidates` carries no top-k either.
-- This reaches the unscoped (plain-search) vector arm and nothing else; wayfind is unaffected.
--
-- ── WHY A FUNCTION ATTRIBUTE ─────────────────────────────────────────────────────────────────────
--
-- `ALTER FUNCTION ... SET` attaches the GUC to this function's own execution (`pg_proc.proconfig`);
-- the session value is untouched before and after. Preferred over `CREATE OR REPLACE FUNCTION`
-- because restating the body is the exact drift hazard 20260801000010's header warns about
-- ("Re-deriving this by hand from an older migration silently reverts websearch_to_tsquery back to
-- plainto_tsquery and drops the ingest_state gate") — nothing here needs the body, so nothing here
-- retypes it. Preferred over `ALTER DATABASE ... SET`, which would reach every future ANN caller
-- including ones written after this decision. Preferred over an `after_connect` pool hook in Rust,
-- which would not travel with the schema and would need duplicating per surface.
--
-- ── CLASS ────────────────────────────────────────────────────────────────────────────────────────
--
-- `additive`. Signature, argument types and return shape are untouched, so a binary deployed either
-- side of this reads the function identically and schema/in-flight-binary cannot disagree — the only
-- question that axis governs (goal 019fb881, refusal face superseded 2026-08-04). Per that same
-- reasoning the semantic risk does not vanish, it moves to witnesses; see below and the PR.
--
-- ── THE REVIEW GATE — THIS CHANGES RANKING SUBSTANTIALLY ─────────────────────────────────────────
--
-- Measured end to end through the deployed `unified_search` (depth 2, limit 10, graph_expand true),
-- same 20 queries, ef=40 vs ef=200, one read:
--
--     * 20 of 20 queries change their top-10 SET
--     * 95 of 200 returned rows replaced — 48% churn, 52% stable
--     * 11 of 20 queries change their #1 RESULT
--
-- Recall is a PROXY. Nothing in this corpus establishes whether those 95 substitutions are better or
-- worse; every figure gathered under goal 019fb559 to date is class composition or reachability,
-- never answer quality, and a share metric that rose while answers got worse would look identical.
-- Three prior interventions in this area moved a proxy and left the outcome flat or worse. What this
-- migration is unambiguously right about is that the code does not do what it says; whether
-- correcting that improves answers is unmeasured, and that question belongs to the reviewer.
--
-- ef=100 is the conservative alternative (83% of exact admission, +0.3 ms/query, further from the
-- planner cliff). The value below is the single knob under review.

-- ── THE WARMUP IS LOAD-BEARING. DO NOT DELETE IT. ────────────────────────────────────────────────
--
-- pgvector registers `hnsw.ef_search` in `_PG_init`, which runs on FIRST USE of a vector type in a
-- backend. Until that happens the name is an unregistered *placeholder*, and PostgreSQL permits
-- setting a placeholder only to a SUPERUSER. A migration runs on a cold connection that has touched
-- no vector, so without this line the next statement fails for an ordinary role with:
--
--     ERROR: permission denied to set parameter "hnsw.ef_search"
--
-- That is exactly how this migration failed its first Vercel deploy. Touching a vector loads the
-- library, after which the GUC is `PGC_USERSET` and any role may set it — proven both ways against a
-- non-superuser role owning its own function: cold session ⇒ denied; warmed ⇒ `{hnsw.ef_search=200}`,
-- including inside a single transaction, which is how sqlx applies a migration.
--
-- **Neither local verification nor CI can catch this class of defect**, and that is worth knowing
-- rather than discovering twice: the docker `temper` role is `usesuper = t`, as is the role in every
-- CI Postgres service container, and for a superuser setting a placeholder is permitted. So this
-- migration passed locally, passed the full test suite, and failed only at deploy. The only
-- environment that exercises the ordinary-role path is a real deploy against Neon.
DO $$ BEGIN PERFORM '[1,2]'::vector <=> '[1,3]'::vector; END $$;

ALTER FUNCTION search_vector_candidates(uuid, vector, integer, uuid, uuid[])
    SET hnsw.ef_search = 200;

COMMENT ON FUNCTION search_vector_candidates(uuid, vector, integer, uuid, uuid[]) IS
$c$Vector candidates for unified_search. The unscoped branch draws a top-k over the HNSW index and
applies visibility/active/complete AFTER it so the index is not defeated; the scoped branch
pre-filters into scoped_res and carries no top-k.

`hnsw.ef_search` is pinned on this function because the server default (40) sits BELOW the 100 the
caller asks for, which truncated the unscoped draw to ~33 chunks per query and admitted 24.3
resources where an exact scan admits 66.2 (measured on prod, 20-query set, 2026-08-04). The GUC is
function-scoped, so no other caller's plan is disturbed. Raising it much past 200 makes the planner
abandon the index for a sequential scan — correct results, roughly 40x the latency.$c$;

SELECT declare_migration(
    20260804000030,
    'additive',
    'Pin hnsw.ef_search=200 on search_vector_candidates. The server default of 40 sits below unified_search vector_k=100, truncating the unscoped ANN draw to ~33 chunks/query and admitting 24.3 of the 66.2 resources an exact scan admits (prod, 20-query set, 2026-08-04). Function-scoped GUC via pg_proc.proconfig; no body, signature or return shape is touched, so a binary either side of this reads the function identically. Ranking output changes substantially: 48% top-10 row churn, 11/20 queries change their top result.'
);
