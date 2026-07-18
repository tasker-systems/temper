-- The per-user Slack grant vault (T3; spec 2026-07-16-slack-account-link-design.md).
--
-- T2 obtains the grant (a real Authorization Code + PKCE exchange requesting `offline_access`,
-- so the token response carries a refresh token in its OWN independent grant family) and hands
-- it to this vault behind the `T3 SEAM` in `slack_link.rs::run_callback`. Identity and secret
-- stay in SEPARATE tables: `kb_profile_auth_links` is the directory row and has NO secret column
-- and must never grow one; the secret lives HERE, encrypted at rest.
--
-- Additive. Namespace-free (no SET search_path).
--
-- KEYED BY THE WHOLE OPAQUE PRINCIPAL. `slack_principal_id` is `slack:<team>:<user>` — 2 to 4
-- segments depending on team presence and bot-ness — and is NEVER split on ':'. The mention
-- agent knows the principal (not the profile id), so the principal is the natural lookup key,
-- and T2 already guarantees one principal binds to exactly one profile. `profile_id` rides along
-- so a profile delete cascades the grant away with it.
--
-- ENCRYPTION. The refresh token (and a cached access token) are stored as XChaCha20-Poly1305
-- ciphertext with a per-row 24-byte random nonce, and each ciphertext is bound to its principal +
-- field (`rt`/`at`) via AEAD associated data — so a valid ciphertext transplanted into another
-- row or the other column fails to open. The AEAD key is the instance's `SLACK_VAULT_ENC_KEY`
-- (32 bytes, base64). The database never sees plaintext and never sees the key.
--
-- ROTATION, honestly: today there is ONE key, and `key_version` is stamped `1` and not yet read.
-- Rotating `SLACK_VAULT_ENC_KEY` today is therefore a FLAG-DAY — old ciphertext no longer opens and
-- affected users must re-link. `key_version` reserves the seam for a future keyring (current +
-- previous keys, decrypt-by-version, re-encrypt-to-current lazily on the next refresh); it is
-- deliberately a column now so that upgrade is additive rather than a schema change. Do NOT read
-- the "lazy rotation" as already implemented.

CREATE TABLE kb_slack_grant_vault (
    id                 UUID PRIMARY KEY,
    profile_id         UUID NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    -- The WHOLE opaque principal. Unique: a principal has exactly one grant.
    slack_principal_id TEXT NOT NULL UNIQUE,
    -- Reserved for a future keyring: which key sealed this row. Stamped `1` and not yet read
    -- (see the ROTATION note above — rotation is flag-day today).
    key_version        SMALLINT NOT NULL DEFAULT 1,
    -- The refresh token — the durable grant. Encrypted; nonce is per-row random.
    rt_nonce           BYTEA NOT NULL,
    rt_ciphertext      BYTEA NOT NULL,
    -- A cached access token, so an ordinary mention need not spend a refresh (each refresh
    -- rotates the RT, and rotating on every mention both adds latency and courts the
    -- reuse-detection race). NULL until the first mint caches one. Encrypted, per-row nonce.
    at_nonce           BYTEA,
    at_ciphertext      BYTEA,
    -- When the cached access token expires. `mint` refreshes only within a skew of this.
    access_expires_at  TIMESTAMPTZ,
    -- Set by `revoke`. A revoked row mints nothing further. HONEST SEMANTICS: this stops FUTURE
    -- mints only — an access token already handed out survives to its own `exp`, because JWKS
    -- validation consults no revocation list. Revocation is not instant cutoff.
    revoked_at         TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_slack_grant_vault_profile ON kb_slack_grant_vault(profile_id);
