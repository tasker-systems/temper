-- SAML Phase 2: role + team provisioning (reconcile-on-login).
-- Additive-only. See docs/superpowers/specs/2026-07-01-saml-phase2-role-team-provisioning-design.md §5.

-- 1. Provenance on team membership. Existing rows are native by definition (the DEFAULT backfills them).
CREATE TYPE team_member_source AS ENUM ('native', 'idp');
ALTER TABLE kb_team_members
    ADD COLUMN source team_member_source NOT NULL DEFAULT 'native';

-- 2. The group -> (team, role) mapping, per-IdP. Operator-maintained via SQL in v1.
CREATE TABLE kb_saml_group_mappings (
    idp_key      TEXT      NOT NULL REFERENCES kb_saml_idp(idp_key) ON DELETE CASCADE,
    group_value  TEXT      NOT NULL,
    team_id      UUID      NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    role         team_role NOT NULL,
    created      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (idp_key, group_value, team_id)
);
CREATE INDEX idx_kb_saml_group_mappings_idp ON kb_saml_group_mappings(idp_key);

-- 3. Which assertion attribute carries the group list. NULL => pure authn (no reconcile).
ALTER TABLE kb_saml_idp ADD COLUMN groups_attr TEXT;

-- 4. Discovery capture: every asserted group value (mapped or NOT) is upserted here on each
--    reconcile, so operators can see what the IdP actually sends and add mappings reactively
--    (the mapping table need not be pre-populated). Never read by the reconcile diff itself.
CREATE TABLE kb_saml_seen_groups (
    idp_key     TEXT        NOT NULL REFERENCES kb_saml_idp(idp_key) ON DELETE CASCADE,
    group_value TEXT        NOT NULL,
    first_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (idp_key, group_value)
);
