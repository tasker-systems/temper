-- Relax kb_team_invitations uniqueness from a full UNIQUE(team_id, invited_email)
-- to a PARTIAL unique index scoped to pending rows, mirroring
-- idx_join_requests_one_pending. This lets declined/expired/accepted history
-- rows coexist while still enforcing "one pending invite per email per team".
-- Safe: the table is inert (zero rows in every environment), and relaxing a
-- uniqueness constraint cannot break existing data.

ALTER TABLE kb_team_invitations
    DROP CONSTRAINT kb_team_invitations_team_id_invited_email_key;

CREATE UNIQUE INDEX idx_invitations_one_pending
    ON kb_team_invitations (team_id, invited_email)
    WHERE status = 'pending';
