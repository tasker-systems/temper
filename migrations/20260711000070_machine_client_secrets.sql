-- Machine-principal Phase B1: temper as a client_credentials issuer.
-- Spec 2026-07-10-machine-principal-phase-b1-issuer-grant-design.md (D1, D5, D6).
--
-- Additive and all-nullable: auth0-m2m rows are untouched (secret_hash NULL; they keep
-- verifying against Auth0's JWKS). issuer='temper' rows carry a SHA-256 hex of a
-- temper-minted secret. Two verification paths, keyed on `issuer`. No plaintext is ever stored.
ALTER TABLE kb_machine_clients
  ADD COLUMN secret_hash                TEXT        NULL,
  ADD COLUMN secret_hash_previous       TEXT        NULL,
  ADD COLUMN secret_previous_expires_at TIMESTAMPTZ NULL,
  ADD COLUMN secret_rotated_at          TIMESTAMPTZ NULL;

COMMENT ON COLUMN kb_machine_clients.secret_hash IS
  'SHA-256 hex of the current client secret, for issuer=temper rows only. NULL for auth0-m2m rows, which verify against Auth0 JWKS. No plaintext is ever stored (D1).';
COMMENT ON COLUMN kb_machine_clients.secret_hash_previous IS
  'The second live secret during rotation; accepted only until secret_previous_expires_at. Zero-downtime secret rotation, capped at two live secrets (D6).';
COMMENT ON COLUMN kb_machine_clients.secret_previous_expires_at IS
  'Expiry of secret_hash_previous. Past this instant, only secret_hash is accepted.';
COMMENT ON COLUMN kb_machine_clients.secret_rotated_at IS
  'Audit stamp of the last secret rotation.';
