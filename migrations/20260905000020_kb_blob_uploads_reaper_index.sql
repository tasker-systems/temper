-- idx_kb_blob_uploads_updated: the abandoned-staging reaper's steady-state read.
--
-- The TTL reaper (rulings 2026-09-05, task 01a0715d — closes the S4 declared hole the
-- 20260903000040 header named) judges a staged session's age on `updated`, the last
-- begin/append touch. Without this index its daily no-op pass is a sequential scan of
-- kb_blob_uploads, and a reaper whose healthy pass gets slower the longer it goes unused
-- is backwards in exactly the way `idx_kb_saml_replay_expires` (20260701000006) was built
-- to avoid for the AS sweep: the index exists so the pass that deletes nothing is the
-- cheap one. Same role, same shape — plain btree on the freshness column, built for the
-- sweep that needs it.

CREATE INDEX idx_kb_blob_uploads_updated ON kb_blob_uploads(updated);

SELECT declare_migration(
    20260905000020,
    'additive',
    'idx_kb_blob_uploads_updated: the abandoned-staging TTL reaper''s steady-state read — keeps the delete-nothing pass an index probe, the same role idx_kb_saml_replay_expires plays for the AS sweep. The reaper itself is binary-side only (no DDL).'
);
