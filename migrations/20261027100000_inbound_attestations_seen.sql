-- An inbound webhook attestation is good once.
--
-- The webhook intake has no bearer token of temper's own; its one control is the Vercel Connect
-- attestation, a signed JWT valid until its exp (twelve hours, research 019f62e6). Nothing recorded
-- an accepted one, so a captured attestation could be replayed with any body until it expired.
-- Connect's attestation carries no jti, so the key is the SHA-256 of the bearer as presented: a
-- replay presents the same bearer. The bearer itself is never stored.
--
-- A row is kept until the attestation can no longer verify (its exp plus the verifier's clock-skew
-- leeway, and the leeway again for instances whose clocks disagree); nothing reads it after. The
-- intake deletes expired rows as it claims new ones. The row is written in the same transaction as
-- the receipt's event, so a receipt that fails or is cut off never records its attestation, and a
-- redelivery of an event temper never recorded is not a replay.
--
-- Whether a repeat is refused or only reported is VERCEL_CONNECT_ENFORCE_ATTESTATION_BINDING's:
-- the table records either way, which is what lets the log say a repeat happened before anything
-- refuses one.

CREATE TABLE kb_inbound_attestations_seen (
    replay_key TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX kb_inbound_attestations_seen_expires_at
    ON kb_inbound_attestations_seen (expires_at);

COMMENT ON TABLE kb_inbound_attestations_seen IS
  'SHA-256 of each accepted inbound webhook attestation bearer, kept until the attestation expires, '
  'so a repeat is recognised. Never the bearer itself.';

SELECT declare_migration(
    20261027100000,
    'additive',
    'Adds kb_inbound_attestations_seen, where the webhook intake records the SHA-256 of each accepted '
    'Vercel Connect attestation until it expires, with an index on expires_at for the prune. Nothing '
    'existing changes. A binary without this migration never reads or writes the table.'
);
