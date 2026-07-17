-- Slack account-link flow intents (T2; spec 2026-07-16-slack-account-link-design.md).
--
-- CLIENT-side OAuth state, deliberately distinct from `kb_oauth_flow`. That table is the
-- Authorization Server's own bookkeeping for flows IT authorizes (pending_saml -> code_issued
-- -> consumed). This one holds the PKCE verifier temper carries across a redirect while acting
-- as an OAuth *client* — of Auth0 on temperkb.io, of the local AS on an enterprise install.
-- Same protocol, opposite ends; one table each.
--
-- Additive. Namespace-free (no SET search_path).
--
-- `state_nonce` is opaque random, NOT a signed blob (spec D6): signed-and-stateless cannot be
-- single-use, and burning a state needs a store regardless. One mechanism therefore delivers
-- single-use, TTL and unguessability. Consume is an atomic conditional UPDATE ... RETURNING
-- (the `bindCodeToFlow` pattern, oauth/flow.ts:56-77): zero rows means unknown, expired or
-- replayed -- indistinguishably, and safely.

CREATE TABLE kb_slack_link_intents (
    id                 UUID PRIMARY KEY,
    -- The opaque `state` handed to the IdP and echoed back to the callback.
    state_nonce        TEXT NOT NULL UNIQUE,
    -- The PKCE verifier, held across the redirect. Paired with the challenge sent to the IdP.
    code_verifier      TEXT NOT NULL,
    -- The WHOLE opaque principal (`slack:<team>:<user>`). 2-4 segments; never split on ':'.
    slack_principal_id TEXT NOT NULL,
    expires_at         TIMESTAMPTZ NOT NULL,
    -- NULL until burned. The single-use marker.
    consumed_at        TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The consume path filters unburned + unexpired rows by nonce; UNIQUE(state_nonce) already
-- indexes the lookup. This partial index serves reaping of abandoned intents.
CREATE INDEX idx_slack_link_intents_unconsumed
    ON kb_slack_link_intents (expires_at)
    WHERE consumed_at IS NULL;
