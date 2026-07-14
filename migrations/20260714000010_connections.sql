-- Connections: temper's authed link to a remote system (S1 of "external systems as
-- subscribed emitters", spec 2026-07-13).
--
-- A PLAIN INFRA TABLE, deliberately NOT event-sourced, on two grounds:
--   1. It follows the shipped `kb_machine_clients` precedent (2026-07-11), which fires no
--      ledger event. There are no admin event tables in `migrations/` at all.
--   2. It follows this goal's own invariant: *the ledger records receipt, never elaboration.*
--      An admin creating a connection is not a receipt of anything external. The ledger's job
--      is the outside world; provisioning is internal infra.
--
-- A connection is a machine principal wearing an integration's clothes, so it reuses the
-- machine-registration gate verbatim (`is_system_admin` OR owner of the owning team; teamless
-- fails closed). No new authz predicate is introduced.
--
-- NOTE the direction of the credential, which is the easiest thing to get wrong here:
-- `kb_machine_clients` answers "who may authenticate TO temper". A connection never does that
-- -- GitHub holds no temper token. A connection is temper authenticating to a REMOTE system.
-- Opposite direction, different table, and a connection has NO machine-client row.
CREATE TABLE kb_connections (
    id                       UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    provider                 TEXT        NOT NULL,
    slug                     TEXT        NOT NULL UNIQUE,
    name                     TEXT        NOT NULL,

    -- Ownership: the machine-registration rule, verbatim.
    owner_team_id            UUID            NULL REFERENCES kb_teams(id),
    registered_by_profile_id UUID        NOT NULL REFERENCES kb_profiles(id),

    -- The emitter. This is what lets a remote system emit into kb_events.
    profile_id               UUID        NOT NULL REFERENCES kb_profiles(id),
    emitter_entity_id        UUID        NOT NULL REFERENCES kb_entities(id),
    home_context_id          UUID        NOT NULL REFERENCES kb_contexts(id),

    -- The BROKER SEAM. NULL means needs_credential.
    credential               JSONB           NULL,

    -- The two capability tiers, separately provisioned, both explicit.
    webhook_events           TEXT[]      NOT NULL DEFAULT '{}',
    tool_manifest            JSONB       NOT NULL DEFAULT '{}',

    -- Declared reach fidelity (the privilege asymmetry, made reviewable).
    reach_granularity        TEXT            NULL,
    reach_covers             TEXT            NULL,

    created                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at               TIMESTAMPTZ     NULL,
    revoked_by_profile_id    UUID            NULL REFERENCES kb_profiles(id)
);

CREATE INDEX idx_kb_connections_team     ON kb_connections(owner_team_id) WHERE owner_team_id IS NOT NULL;
CREATE INDEX idx_kb_connections_profile  ON kb_connections(profile_id);
CREATE INDEX idx_kb_connections_provider ON kb_connections(provider);

COMMENT ON TABLE kb_connections IS
  'temper''s authed link to a remote system (a GitHub App installation, a Linear workspace). Admin-provisioned infra, not event-sourced: an admin creating a connection is not a receipt of anything external.';

COMMENT ON COLUMN kb_connections.provider IS
  'github | linear | ... Deliberately TEXT with no CHECK, matching kb_machine_clients.issuer: provider admissibility is a design decision evidenced by tool_manifest, not a constraint a migration should adjudicate. A new provider must be brokerable (API/MCP/CLI with credentials handed to us) -- proxying is out of scope by rule.';

COMMENT ON COLUMN kb_connections.owner_team_id IS
  'The connection OWNER -- which team a provisioning was performed on behalf of. NEVER consulted for authorization; owning a connection does not confer the right to subscribe to it. Reach is plural and explicitly granted. NULL = teamless = admin-only, and FAILS CLOSED.';

COMMENT ON COLUMN kb_connections.profile_id IS
  'The connection''s dedicated agent profile. It exists ONLY so the connection can own an emitter entity (kb_events.emitter_entity_id is NOT NULL and FKs to kb_entities, which FKs to kb_profiles). It carries NO kb_profile_auth_links row and NO kb_machine_clients row: a connection never authenticates to temper.';

COMMENT ON COLUMN kb_connections.emitter_entity_id IS
  'The entity a remote payload is attributed to, named `<handle>@webhook`. Created DIRECTLY rather than via Surface::ALL -- `webhook` is deliberately not a Surface variant, because adding one would oblige provisioning a webhook emitter onto every human profile (see temper-workflow operations::surface). Intake resolves the emitter from THIS column, never from a surface marker.';

COMMENT ON COLUMN kb_connections.home_context_id IS
  'Where the connection is homed. An event has exactly one producing_anchor, so a payload matching three subscribers cannot be anchored three ways: the anchor is the receipt fact (one place), and the fan-out lives in kb_events.references. Contexts-as-home also means read authz inherits for free.';

COMMENT ON COLUMN kb_connections.credential IS
  'An ABSTRACT credential reference behind the broker seam -- {broker, connector, installation?} -- never a bare Vercel connector id. `broker` names the implementation so a platform swap costs one adapter (the seam is two operations: mint, verifyInbound). Storing the connector id on the row (not in code) is also what lets a self-hosted operator provision their own connectors in their own Vercel team. NULL is the needs_credential state -- a status enum would only drift out of sync with this.';

COMMENT ON COLUMN kb_connections.webhook_events IS
  'Registered remote event types. Non-empty => LEDGER-CAPABLE: events land, facts accrue. Useful on its own.';

COMMENT ON COLUMN kb_connections.tool_manifest IS
  'Declared read-only remote tools. Non-empty => REACH-CAPABLE: agents can read the remote back, so judgment becomes possible. Not decorative -- it is the evidence the provider is admissible at all. A ledger-only connection is legal and useful but INERT FOR JUDGMENT, and says so rather than leaving an agent to mysteriously produce nothing.';

COMMENT ON COLUMN kb_connections.reach_granularity IS
  'org | workspace | installation | repo-set | project. What grain the credential is scoped at, in the PROVIDER''s terms.';

COMMENT ON COLUMN kb_connections.reach_covers IS
  'What the credential can ACTUALLY see, in provider terms (e.g. `acme/temper`, or `acme/*`). Together with reach_granularity this is the declared reach fidelity. Deliberately two honest fields rather than a computed `exceeds_temper_reach` bool: remote scope and temper scope are incommensurable (no predicate can compute "does acme/* exceed team A''s cogmaps?"), and a stored bool would go stale. The grant path raises the warning at the moment it matters. Silence must never encode absence of capability -- and never encode excess of it either.';

COMMENT ON COLUMN kb_connections.revoked_at IS
  'A revoked connection is dead. Reactivation is a new provisioning, never an UPDATE. Rows are never deleted.';
