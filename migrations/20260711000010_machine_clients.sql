-- Machine-principal registration (spec 2026-07-10, D2/D3/D6).
-- Registration is a GATE, not a ledger: `resolve_machine_from_claims` rejects any client_id
-- absent from this table, even bearing a perfectly valid IdP token.
CREATE TABLE kb_machine_clients (
    id                       UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    client_id                TEXT        NOT NULL UNIQUE,
    issuer                   TEXT        NOT NULL DEFAULT 'auth0-m2m',
    label                    TEXT        NOT NULL,
    profile_id               UUID        NOT NULL REFERENCES kb_profiles(id),
    team_id                  UUID            NULL REFERENCES kb_teams(id),
    registered_by_profile_id UUID        NOT NULL REFERENCES kb_profiles(id),
    created                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at             TIMESTAMPTZ     NULL,
    revoked_at               TIMESTAMPTZ     NULL,
    revoked_by_profile_id    UUID            NULL REFERENCES kb_profiles(id)
);

CREATE INDEX idx_kb_machine_clients_profile ON kb_machine_clients(profile_id);
CREATE INDEX idx_kb_machine_clients_team    ON kb_machine_clients(team_id) WHERE team_id IS NOT NULL;

COMMENT ON TABLE kb_machine_clients IS
  'Allowlist of machine (client_credentials) principals. Fail-closed: an unregistered client_id is rejected at authentication.';
COMMENT ON COLUMN kb_machine_clients.client_id IS
  'The IdP client identifier, matching AuthClaims.external_user_id from normalize_machine. UNIQUE; its constraint index serves the authentication-path lookup.';
COMMENT ON COLUMN kb_machine_clients.issuer IS
  'Who issued this credential. Phase A writes only auth0-m2m. Phase B writes temper. Forward slot.';
COMMENT ON COLUMN kb_machine_clients.team_id IS
  'The machine OWNER -- which team a registration was performed on behalf of. NEVER consulted for authorization. Reach is kb_access_grants plus team membership, both plural; an authorization predicate written against this column would be strictly narrower than resources_visible_to. Never the agent''s own auto-provisioned personal team.';
COMMENT ON COLUMN kb_machine_clients.last_seen_at IS
  'Coarse (five-minute) liveness touch. Deliberately not precise: authentication must not write on the common path.';
COMMENT ON COLUMN kb_machine_clients.revoked_at IS
  'A revoked row is dead. Reactivation is a new registration, never an UPDATE. Rows are never deleted.';
