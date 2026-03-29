-- R2 file metadata: tracks files uploaded to Cloudflare R2.
-- Access control flows through the associated resource, not the file itself.

CREATE TABLE r2_files (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id      UUID NOT NULL REFERENCES kb_profiles(id),
    resource_id     UUID REFERENCES resources(id),
    object_key      TEXT NOT NULL UNIQUE,
    file_url        TEXT NOT NULL,
    content_type    TEXT,
    file_size_bytes BIGINT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_r2_files_profile ON r2_files(profile_id);
CREATE INDEX idx_r2_files_resource ON r2_files(resource_id);
