-- Record which key sealed each Slack grant vault row.
--
-- A row that will not open fails the same AEAD tag whether the deployment holds the wrong key, the
-- key was rotated on purpose, or the row itself is bad. Those need opposite answers: a wrong key is
-- healed for everyone by restoring it, so the mint must not offer a re-link that would overwrite the
-- row; a retired key or a bad row is healed only by a re-link. key_fingerprint is what tells them
-- apart (slack_grant_vault_service::mint_access_token holds the decision table).
--
-- THE FINGERPRINT IS A CLASSIFIER, NOT A GATE. The mint opens the row first and reads the fingerprint
-- only to explain a failure; a row that opens is used whatever this column says. So a forged value can
-- only change how a failure is reported.
--
-- Nullable, and NULL for every row this migration finds: the database never holds the key, so it
-- cannot compute one. The mint stamps a NULL row on its first successful open.
--
-- Additive. Namespace-free (no SET search_path).

ALTER TABLE kb_slack_grant_vault
    ADD COLUMN key_fingerprint BYTEA
        CONSTRAINT kb_slack_grant_vault_key_fingerprint_len
        CHECK (key_fingerprint IS NULL OR octet_length(key_fingerprint) = 8);

COMMENT ON COLUMN kb_slack_grant_vault.key_fingerprint IS
    'First 8 bytes of HMAC-SHA256(sealing key, ''temper-slack-vault-key-id/v1''): which key sealed this '
    'row''s rt and at ciphertexts (always one key, written in one statement). Not secret. Stamped by '
    'store_grant and the mint''s re-seal; NULL until a row predating 20261028100000 first opens. Read '
    'only to classify an open failure: current key = row-level fault, a fingerprint listed in '
    'SLACK_VAULT_RETIRED_KEY_FINGERPRINTS = deliberate rotation, anything else = misconfigured key.';

COMMENT ON COLUMN kb_slack_grant_vault.key_version IS
    'Inert: stamped 1 and never read. Superseded by key_fingerprint (20261028100000), which is derived '
    'from the key itself rather than numbered by an operator.';

SELECT declare_migration(
    20261028100000,
    'additive',
    'Adds a nullable key_fingerprint BYTEA column to kb_slack_grant_vault, with a CHECK that a present '
    'value is 8 bytes, and comments it and key_version. No existing row changes. A binary without this '
    'migration never names the column: its INSERT leaves it NULL and its UPDATE leaves it as it was, '
    'and a stale fingerprint only changes how a later open failure is reported, since a row that '
    'opens is restamped.'
);
