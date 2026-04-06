-- kb_resource_audits: audit trail linking resource mutations to events with hash snapshots.
-- Used for precedence-based sync conflict resolution (not compliance — CASCADE on delete).

CREATE TABLE kb_resource_audits (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    resource_id UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    event_id    UUID NOT NULL REFERENCES kb_events(id) ON DELETE CASCADE,
    profile_id  UUID NOT NULL REFERENCES kb_profiles(id),
    device_id   TEXT NOT NULL,
    body_hash   TEXT NOT NULL,
    managed_hash TEXT NOT NULL,
    open_hash   TEXT NOT NULL,
    action      TEXT NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(resource_id, event_id)
);

CREATE INDEX idx_resource_audits_resource ON kb_resource_audits(resource_id, created DESC);
CREATE INDEX idx_resource_audits_event ON kb_resource_audits(event_id);
