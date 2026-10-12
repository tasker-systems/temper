-- An internal signature is good once.
--
-- The internal HMAC gates (temper_core::internal_sig) accept a signed request while its timestamp is
-- within MAX_SKEW_SECS of the verifier's clock. Freshness alone let a captured request verify again
-- for the rest of that window, and at /internal/slack/mint a repeat hands back a fresh access token
-- carrying a human's whole reach. The v2 construction signs a per-request nonce; this table is where
-- a verifier records the nonces it has accepted, so the same one is refused the second time.
--
-- The table is shared by every instance of the API, which is what makes "once" hold across a
-- deployment that runs more than one: a nonce claimed by one instance is refused by the next.
--
-- A row is kept until its signature can no longer be fresh. Its signed timestamp is accepted up to
-- MAX_SKEW_SECS after it was signed, so expires_at is the timestamp plus twice that, which also
-- covers clock disagreement between instances. Nothing reads a row after expires_at; the verifier
-- deletes expired rows as it claims new ones, so the table holds about one minute of traffic.
--
-- A row is written only after the signature verifies, so nobody without the door's key can add one.

CREATE TABLE kb_internal_signature_nonces (
    nonce      TEXT PRIMARY KEY,
    door       TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX kb_internal_signature_nonces_expires_at
    ON kb_internal_signature_nonces (expires_at);

COMMENT ON TABLE kb_internal_signature_nonces IS
  'Nonces of accepted v2 internal signatures (temper_core::internal_sig), kept until the signature can '
  'no longer be fresh, so each is accepted once. Written only after the signature verifies.';

SELECT declare_migration(
    20261028100000,
    'additive',
    'Adds kb_internal_signature_nonces, where temper-api records the nonce of each accepted v2 internal '
    'signature until it can no longer be fresh, with an index on expires_at for the prune. Nothing '
    'existing changes. A binary without this migration never reads or writes the table.'
);
