-- Drop `graph_subgraph_nodes` and the never-used team-zone trio.
--
-- `graph_subgraph_nodes` backed the legacy Cytoscape context graph. Beat E
-- (20260709000010/11, merged as 087c8b04) replaced that surface with the Atlas
-- context door and removed every Rust/TS caller. This DROP is deliberately a
-- *later* migration than Beat E rather than part of it: each install is an
-- independent Vercel project whose migrations run ahead of its deploy, so
-- dropping a function in the same release that stops calling it would 500 the
-- still-running old code during that window. Beat E is now deployed everywhere.
--
-- `team_viewable_by` / `team_child_zones` / `team_descendants` were born in
-- 20260703000002 for a `TeamZoneMark` surface that was never built. They have
-- never had a Rust, TypeScript, or test caller; the only SQL callers are each
-- other. Dropping them also retires `team_descendants`' `is_active` soft-delete
-- gap — it walks team parentage without filtering deleted rows, so it must not
-- be revived as-is (see docs/code-reviews/2026-07-08-sql-function-audit.md).
--
-- Nothing else in the schema references these four: no view, no function body,
-- no default, no constraint.

DROP FUNCTION IF EXISTS graph_subgraph_nodes(uuid, uuid, text[], int);

-- Callers first, then the shared callee, so the file reads top-down even though
-- Postgres tracks no function-to-function dependency.
DROP FUNCTION IF EXISTS team_viewable_by(uuid, uuid);
DROP FUNCTION IF EXISTS team_child_zones(uuid, uuid);
DROP FUNCTION IF EXISTS team_descendants(uuid);
