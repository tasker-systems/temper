-- Blob file metadata: tracks files uploaded to Vercel Blob.
-- Access control flows through the associated resource, not the file itself.
-- Status tracks the processing lifecycle: pending → processing → processed → failed.

CREATE TABLE blob_files (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id      UUID NOT NULL REFERENCES kb_profiles(id),
    resource_id     UUID REFERENCES resources(id),
    blob_url        TEXT NOT NULL,
    pathname        TEXT NOT NULL,
    content_type    TEXT,
    file_size_bytes BIGINT,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'processing', 'processed', 'failed')),
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_blob_files_profile ON blob_files(profile_id);
CREATE INDEX idx_blob_files_resource ON blob_files(resource_id);
CREATE INDEX idx_blob_files_status ON blob_files(status);
