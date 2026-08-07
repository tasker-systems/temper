-- Retire the blended `unified_search` mechanism and the wayfind SCOPE machinery.
--
-- `/api/search` is now two independent arms — `search_exact` and `search_wide` (20260805000020) —
-- and the read path moved to them in this same branch. Nothing in the repo calls the eight
-- functions below any more; this migration removes them from the schema so that "the mechanism is
-- gone" is a fact about the database and not only about the Rust.
--
-- ── WHY THIS IS `shape-breaking`, NOT `additive` ─────────────────────────────────────────────────
--
-- A binary deployed BEFORE this branch reads `/api/search` through `unified_search`. Applying this
-- ahead of that binary makes every search 500 — not degradation. That is the axis the
-- additive/shape-breaking split governs, so the declaration below is a statement of fact about the
-- schema change, not a deployment plan.
--
-- What the classification mechanically causes: `temper-migrate --additive-only` applies the pending
-- set until it reaches a migration that is not additive, then halts and exits non-zero
-- (`run_additive_only` → `HALTED`, crates/temper-migrate/src/main.rs). `scripts/vercel-build.sh`
-- runs that binary as its build command, so the halt surfaces as a failed build rather than a
-- partially-migrated schema. Nothing after the halt is applied; halting is stopping, not skipping.
--
-- Preview and production are NOT the same event. A preview build applies the PR's own migration to
-- the PR's own Neon branch — the rehearsal half (scripts/vercel-build.sh header). Production carries
-- the live binary that still calls `unified_search`. And `MIGRATE_CMD` is an overridable env var
-- whose DEFAULT carries `--additive-only` (vercel-build.sh:151), so what any given target actually
-- runs is an environment fact, not a repo fact — read the build log, not this comment.
--
-- `[decided — 2026-08-06, Pete]` The retirement rides with the code that supersedes it rather than
-- behind a separate later change. Compare 20260709000020, which dropped a function only once the
-- release that stopped calling it was deployed everywhere; that option is deliberately not taken
-- here. How the halt is handled per target is an operational question this file does not answer —
-- the binary's own HALTED message points at DEPLOYING.md, which is where that belongs.
--
-- ── WHAT SURVIVES, AND WHY IT MUST NOT BE SWEPT UP HERE ──────────────────────────────────────────
--
-- Three functions share this vocabulary and are NOT retired. A name-shaped grep catches all three;
-- each is load-bearing for the next phase (`/api/query`):
--
--   * `wayfind_region_scores(uuid, uuid, vector, integer, varchar, uuid)` — the `survey` act's
--     mechanic. `/api/query` binds it directly. What dies below is the SCOPE funnel built on top of
--     it (`wayfind_scope_ids` / `wayfind_scope_reach`, which existed to hand `unified_search` a
--     flattened `p_scope_ids`), never the scoring itself.
--   * `wayfind_region_diagnostics(uuid, uuid, vector, integer, varchar, uuid)` — a thin wrapper over
--     `wayfind_region_scores` plus anchor-name resolution, and the ONLY read surface over that
--     function's internals (`sal_norm`, `query_cos`, `region_score`, `in_top_n`). Its witness set is
--     already depleted by this branch; dropping it would deepen a hole the next phase depends on.
--   * `search_graph_expand(uuid, uuid[], integer, text[], double precision)` — the `follow-from`
--     act's mechanic. It loses its last caller when `unified_search` goes. That is expected and
--     correct: a stranded mechanic with no door, kept so the record that it exists is not lost.
--
-- ── `atlas_search` IS THE SUBTLE ONE ─────────────────────────────────────────────────────────────
--
-- `atlas_search` (20260704000008) has zero callers anywhere — its planned Rust half,
-- `graph_service::atlas_search`, was never built. It is nonetheless live, and its `LANGUAGE sql`
-- body CALLS `unified_search` through a defaulted 13th parameter. PostgreSQL records no
-- function-to-function dependency and resolves the body at RUNTIME, so dropping `unified_search`
-- alone would leave `atlas_search` in the catalog, callable, and erroring. It goes in the same
-- migration.
--
-- ── VERIFIED CALLER SET (live dev database, 2026-08-06) ──────────────────────────────────────────
--
-- Scanning every `pg_proc.prosrc` in `public` for references to the eight names below returns
-- exactly four edges, all internal to the set being dropped:
--
--     atlas_search           -> unified_search
--     cogmap_scope_ids_multi -> cogmap_scope_ids
--     unified_search         -> search_fts_candidates, search_vector_candidates
--     wayfind_scope_ids      -> wayfind_scope_reach
--
-- No view or materialized view references any of them. The three survivors above reference none of
-- them. `search_exact`'s body matches the grep only in a COMMENT, which cites `cogmap_scope_ids`'
-- readability conjunct as the thing it now carries inline.
--
-- Drops are ordered caller-before-callee so the file reads top-down, even though Postgres tracks no
-- function-to-function dependency and would accept any order.

-- The blend, and the never-built Atlas surface layered on it.
DROP FUNCTION IF EXISTS atlas_search(uuid, uuid, text, integer);
DROP FUNCTION IF EXISTS unified_search(uuid, text, vector, uuid[], integer, text[], uuid, text, boolean, integer, integer, uuid[], boolean);

-- The blend's two candidate arms. `search_exact` and `search_wide` carry their own visibility,
-- `is_active` and `ingest_state` predicates inline, so nothing is left holding these.
DROP FUNCTION IF EXISTS search_vector_candidates(uuid, vector, integer, uuid, uuid[]);
DROP FUNCTION IF EXISTS search_fts_candidates(uuid, text);

-- Cogmap scope materialization. The arms scope by the anchor pair `(anchor_table, anchor_id)` inside
-- SQL, so a `uuid[]` of members no longer has to be assembled in Rust and handed back down.
DROP FUNCTION IF EXISTS cogmap_scope_ids_multi(uuid, uuid[]);
DROP FUNCTION IF EXISTS cogmap_scope_ids(uuid, uuid);

-- The wayfind SCOPE funnel — the `p_scope_ids` producer, not the region scoring.
DROP FUNCTION IF EXISTS wayfind_scope_ids(uuid, uuid, vector, integer, character varying, uuid);
DROP FUNCTION IF EXISTS wayfind_scope_reach(uuid, uuid, vector, integer, character varying, uuid);

SELECT declare_migration(
    20260806000010,
    'shape-breaking',
    'Retires the blended search mechanism now that /api/search runs on search_exact and search_wide (20260805000020): drops unified_search, its two candidate arms (search_fts_candidates, search_vector_candidates), the cogmap scope materializers (cogmap_scope_ids, cogmap_scope_ids_multi), the wayfind scope funnel (wayfind_scope_ids, wayfind_scope_reach), and atlas_search — which has no caller of its own but calls unified_search from a LANGUAGE sql body that PostgreSQL resolves at runtime, so leaving it would strand a callable function that errors and records no dependency. Shape-breaking because a binary deployed before this branch reads /api/search through unified_search and would 500 the moment this applies; the additive-only deploy gate therefore halts here and exits non-zero rather than running past it. wayfind_region_scores, wayfind_region_diagnostics and search_graph_expand are deliberately NOT dropped — they are the survey and follow-from mechanics that /api/query binds next, and the diagnostics wrapper is the only read surface over the scoring internals.'
);
