-- The reconcile drain's records of what it cannot check, and why. Task
-- 01a15663-4338-750b-9346-957d0c9caf2e.
--
-- Two tables, for two grains:
--
--   kb_shape_reconcile_uncheckable — a SHAPE the drain cannot compile. A shape stored before
--   compile refused anything but a field tree (#1259) may no longer compile, and then no artifact
--   under it can be verdicted. The drain records the shape row once, and its backlog leaves out
--   every artifact whose shape in force is recorded.
--
--   kb_shape_reconcile_unreadable — an ARTIFACT whose content the drain cannot read, whatever its
--   shape: content that `jsonb` prints past the drain's one-artifact ceiling, or past what Postgres
--   can render as text at all. The drain records it under the artifact's staleness triple
--   (shape_id, shape_version, content_hash), and its backlog leaves the artifact out until the
--   artifact or its shape in force changes.
--
-- Without them those artifacts would stay in the backlog for good: every pass would fail them
-- again, and their anchor's job would end dead with the reason only in a log.
--
-- What a later edit must not break:
--   1. A RECORD IS NOT A VERDICT. Reads still derive `shape_state` from kb_data_artifact_verdicts
--      alone, so a recorded artifact reads `declared_not_yet_checked`, which is true: nobody
--      checked it.
--   2. THE KEYS. A shape row is never edited: an amendment folds it and declares a new row with a
--      new id, which is tried afresh. An artifact's record excludes it only while the record's
--      triple is the artifact's triple now. Neither key covers the COMPILER: a release that widens
--      what compiles leaves recorded shapes excluded. The operator's remedy is to delete the
--      shape's row here and enqueue a shape-reconcile job for its home.
--   3. NO CONTENT. The rows hold ids, a version, a sha256 and a closed reason.
--
-- Additive: two CREATE TABLEs and one new function. No existing object is altered.

CREATE TABLE kb_shape_reconcile_uncheckable (
    shape_id    uuid        PRIMARY KEY REFERENCES kb_data_artifact_shapes (id) ON DELETE CASCADE,
    reason      text        NOT NULL CHECK (reason IN ('schema_uncompilable')),
    recorded_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_shape_reconcile_uncheckable IS
    'The shape-reconcile drain''s record of shapes it cannot check: one row per shape row (one shape version) that does not compile. The drain''s backlog excludes every artifact whose shape in force is recorded here. Not a verdict: shape_state reads from kb_data_artifact_verdicts alone, so such an artifact reads declared_not_yet_checked (20261108000000).';

CREATE TABLE kb_shape_reconcile_unreadable (
    artifact_id   uuid        PRIMARY KEY REFERENCES kb_data_artifacts (id) ON DELETE CASCADE,
    shape_id      uuid        NOT NULL REFERENCES kb_data_artifact_shapes (id) ON DELETE CASCADE,
    shape_version int         NOT NULL,
    content_hash  text        NOT NULL,
    reason        text        NOT NULL CHECK (reason IN ('content_too_large', 'content_unrenderable')),
    recorded_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX kb_shape_reconcile_unreadable_shape_idx ON kb_shape_reconcile_unreadable (shape_id);

COMMENT ON TABLE kb_shape_reconcile_unreadable IS
    'The shape-reconcile drain''s record of artifacts whose content it cannot read: one row per artifact, naming the staleness triple (shape_id, shape_version, content_hash) it was recorded under and a closed reason. The drain''s backlog excludes the artifact while the triple is its own. Not a verdict: shape_state reads from kb_data_artifact_verdicts alone, so such an artifact reads declared_not_yet_checked (20261108000000).';

-- The one measure of an artifact's stored content: how many bytes a read receives. jsonb is sent
-- as its text (after a version byte), and that text prints numbers in full, so it can be far longer
-- than the content_bytes the commit counted. The drain bounds what it reads by this.
CREATE FUNCTION data_artifact_content_returned_bytes(p_artifact_id uuid)
RETURNS bigint LANGUAGE sql STABLE AS $$
    SELECT octet_length(content::text)::bigint
      FROM kb_data_artifact_content
     WHERE artifact_id = p_artifact_id
$$;

COMMENT ON FUNCTION data_artifact_content_returned_bytes(uuid) IS
    'How many bytes a read of the artifact''s content receives: the length of the stored jsonb''s text, which prints numbers in full and so can be far longer than the committed content_bytes. NULL when the artifact has no content row (committed as null). Raises a program-limit error (SQLSTATE class 54) for content Postgres cannot render as text (20261108000000).';

SELECT declare_migration(
    20261108000000,
    'additive',
    'New tables kb_shape_reconcile_uncheckable (shapes the shape-reconcile drain cannot compile, keyed by shape row) and kb_shape_reconcile_unreadable (artifacts whose content it cannot read, keyed by artifact with the staleness triple), each with a closed reason, and function data_artifact_content_returned_bytes(uuid), the length of an artifact''s content as a read receives it. No existing object is altered or replaced, and no parameter carries a DEFAULT; a deployed binary that does not read them is unaffected.'
);
