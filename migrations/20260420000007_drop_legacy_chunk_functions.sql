-- Drop the pre-dedup 2-arg variants of persist_resource_chunks and
-- replace_resource_chunks. The 4-arg revision-aware forms introduced in
-- 20260420000006 are now the only callers (Rust temper-api updated in
-- this commit's parent; TS cloud workflows updated in Task 6).
DROP FUNCTION IF EXISTS persist_resource_chunks(UUID, JSONB);
DROP FUNCTION IF EXISTS replace_resource_chunks(UUID, JSONB);
