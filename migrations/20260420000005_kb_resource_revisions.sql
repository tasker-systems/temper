-- kb_resource_revisions: content-version anchor for kb_chunks.
--
-- A revision is produced for every chunk-producing action on a resource
-- (kb_resource_audits.action IN ('create', 'update_body')). Metadata-only
-- updates (action='update_meta') produce audits but NOT revisions — chunks
-- are not re-written for meta edits.
--
-- audit_id is ON DELETE SET NULL so revisions outlive their audits and the
-- retention sweep for kb_resource_audits does not cascade into chunk loss.

CREATE TABLE kb_resource_revisions (
    id          UUID PRIMARY KEY,
    resource_id UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    audit_id    UUID REFERENCES kb_resource_audits(id) ON DELETE SET NULL,
    body_hash   TEXT NOT NULL,
    chunk_count INT NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_resource_revisions_resource_created
    ON kb_resource_revisions(resource_id, created DESC);
CREATE INDEX idx_resource_revisions_audit
    ON kb_resource_revisions(audit_id);
CREATE INDEX idx_resource_revisions_body_hash
    ON kb_resource_revisions(body_hash);

-- kb_chunks revision linkage.
-- first_revision_id nullable for now; Task 8 tightens to NOT NULL after backfill.
-- Both columns ON DELETE RESTRICT — a revision referenced by any chunk
-- cannot be deleted (retention sweep must skip pinned revisions).

ALTER TABLE kb_chunks
    ADD COLUMN first_revision_id      UUID REFERENCES kb_resource_revisions(id) ON DELETE RESTRICT,
    ADD COLUMN superseded_revision_id UUID REFERENCES kb_resource_revisions(id) ON DELETE RESTRICT;

CREATE INDEX idx_kb_chunks_first_revision
    ON kb_chunks(first_revision_id);
CREATE INDEX idx_kb_chunks_superseded_revision
    ON kb_chunks(superseded_revision_id)
    WHERE superseded_revision_id IS NOT NULL;
