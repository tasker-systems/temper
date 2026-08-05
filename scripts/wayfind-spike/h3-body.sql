-- H3 — the full deployed wayfind_region_scores body. Reproduce from THIS, not from the migration
-- file and not from memory. Read-only.
SELECT pg_get_functiondef(p.oid)
FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname='public' AND p.proname='wayfind_region_scores';
