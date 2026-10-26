-- The reconcile drain's record of the artifacts it did not verdict, and why: a row names the
-- staleness triple it was set aside under, so the drain's backlog leaves it out until the artifact,
-- or the shape in force for it, changes. Task 01a15663-4338-750b-9346-957d0c9caf2e.
--
-- What a later edit must not break:
--   1. A SET-ASIDE IS NOT A VERDICT. Reads still derive `shape_state` from kb_data_artifact_verdicts
--      alone, so a set-aside artifact reads `declared_not_yet_checked`, which is true: nobody
--      checked it. Nothing here may be read as satisfied or not.
--   2. THE TRIPLE IS THE KEY TO LEAVING THE BACKLOG. The drain excludes an artifact only while its
--      row's (shape_id, shape_version, content_hash) is the artifact's triple now. A row for an old
--      triple excludes nothing, and the artifact is checked again.
--   3. 'checking' MARKS A CHECK IN FLIGHT. The drain writes it before a check and removes it after
--      the verdict. One job per anchor is in flight (the single-flight index), so a 'checking' row
--      a pass finds for its own anchor was left by a run that was killed mid-check; the pass
--      re-marks it 'check_did_not_finish', and the artifact costs one killed run, never three.
--   4. NO CONTENT. The row holds ids, a version, a sha256 and a closed reason.
--
-- Additive: one CREATE TABLE. No existing object is altered.

CREATE TABLE kb_shape_reconcile_set_aside (
    artifact_id   uuid        PRIMARY KEY REFERENCES kb_data_artifacts (id) ON DELETE CASCADE,
    shape_id      uuid        NOT NULL REFERENCES kb_data_artifact_shapes (id),
    shape_version int         NOT NULL,
    content_hash  text        NOT NULL,
    reason        text        NOT NULL CHECK (reason IN (
                                  'checking',
                                  'check_did_not_finish',
                                  'too_large_to_check',
                                  'schema_too_large',
                                  'schema_uncompilable',
                                  'pattern_too_costly')),
    recorded_at   timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_shape_reconcile_set_aside IS
    'The shape-reconcile drain''s set-aside artifacts: one row per artifact it did not verdict, naming the staleness triple (shape_id, shape_version, content_hash) and the reason. The drain''s backlog excludes an artifact while the row''s triple is its own. Not a verdict: shape_state reads from kb_data_artifact_verdicts alone, so a set-aside artifact reads declared_not_yet_checked. reason ''checking'' marks a check in flight; one found by a later pass of the same anchor is re-marked check_did_not_finish (20261108000000).';

SELECT declare_migration(
    20261108000000,
    'additive',
    'New table kb_shape_reconcile_set_aside: the shape-reconcile drain''s record of artifacts it did not verdict and why, keyed by artifact with the staleness triple it was set aside under. No existing object is altered; a deployed binary that does not read it is unaffected.'
);
