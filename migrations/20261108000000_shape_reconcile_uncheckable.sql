-- The reconcile drain's record of the shapes it cannot check, and why. Task
-- 01a15663-4338-750b-9346-957d0c9caf2e.
--
-- A shape stored before compile refused anything but a field tree (#1259) may no longer compile.
-- The drain cannot verdict any artifact under such a shape. Without a record, those artifacts would
-- stay in its backlog for good, and every pass would compile the shape again and fail again. The
-- drain records the shape here once, and its backlog leaves out every artifact whose shape in force
-- is recorded.
--
-- What a later edit must not break:
--   1. A RECORD IS NOT A VERDICT. Reads still derive `shape_state` from kb_data_artifact_verdicts
--      alone, so an artifact under a recorded shape reads `declared_not_yet_checked`, which is
--      true: nobody checked it.
--   2. THE KEY IS THE SHAPE ROW. A shape row is never edited: an amendment folds it and declares a
--      new row with a new id. So a record never goes stale, and an amended shape is tried afresh.
--   3. NO CONTENT. The row holds an id and a closed reason.
--
-- Additive: one CREATE TABLE. No existing object is altered.

CREATE TABLE kb_shape_reconcile_uncheckable (
    shape_id    uuid        PRIMARY KEY REFERENCES kb_data_artifact_shapes (id) ON DELETE CASCADE,
    reason      text        NOT NULL CHECK (reason IN ('schema_uncompilable', 'compile_panicked')),
    recorded_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_shape_reconcile_uncheckable IS
    'The shape-reconcile drain''s record of shapes it cannot check: one row per shape row (one shape version) that does not compile, or whose compile panicked. The drain''s backlog excludes every artifact whose shape in force is recorded here. Not a verdict: shape_state reads from kb_data_artifact_verdicts alone, so such an artifact reads declared_not_yet_checked (20261108000000).';

SELECT declare_migration(
    20261108000000,
    'additive',
    'New table kb_shape_reconcile_uncheckable: the shape-reconcile drain''s record of shapes that do not compile, keyed by shape row, with a closed reason. No existing object is altered; a deployed binary that does not read it is unaffected.'
);
