-- Inbound connector resolution: the lookup EVERY received webhook performs (S3 of "external
-- systems as subscribed emitters", spec 2026-07-13).
--
-- `CredentialBroker::verify_inbound` returns the connector identity from the SIGNED `trigger`
-- claim (`{provider, connector_uid, connector_id}`) and stops there on purpose -- its doc:
-- "resolving it to a `kb_connections` row is the caller's job (the broker stays DB-free, hence
-- swappable)". That resolution keys on `credential->>'connector'`, because the credential is
-- `{broker, connector, installation?}` (`ConnectionCredential`) and `connector` holds the
-- connector *uid* -- the same value `connect_token_url` percent-encodes for the mint hop, and the
-- same value the attestation's `trigger.uid` carries.
--
-- `kb_connections` carried indexes on `owner_team_id`, `profile_id` and `provider`
-- (`20260714000010_connections.sql:50-52`) and NONE on the credential's connector. Every inbound
-- webhook was a seq scan on the one table lookup that sits in front of the whole intake path.
--
-- PARTIAL on the two predicates the resolution always carries: a NULL credential is the
-- `needs_credential` birth state (it can receive nothing), and a revoked connection must not
-- receive at all. Keeping both out of the index also keeps it small. The resolving query MUST
-- repeat both predicates or the planner cannot use this index.
--
-- NOT unique: two connections may legitimately name the same connector (two teams against one
-- org-wide GitHub App). Ambiguity is therefore possible and is resolved in Rust as a
-- configuration FAILURE (temper cannot know which connection a payload was for), never by
-- picking one -- see `connection_service::resolve_inbound`.
CREATE INDEX idx_kb_connections_connector
    ON kb_connections ((credential->>'connector'))
 WHERE credential IS NOT NULL AND revoked_at IS NULL;

COMMENT ON INDEX idx_kb_connections_connector IS
  'Inbound webhook resolution: the signed trigger.uid from a verified attestation -> the kb_connections row that receives it. Partial on (credential IS NOT NULL AND revoked_at IS NULL) -- an uncredentialed connection is in its needs_credential birth state and a revoked one must not receive, so neither is ever a resolution target. A resolving query must repeat both predicates to use this index.';

SELECT declare_migration(
    20260819000040,
    'additive',
    'Adds one partial expression index on kb_connections ((credential->>''connector'')). Additive: no column, no constraint, no enum, no wire-contract move, and no row is read or written differently by the pre-deploy binary -- an index changes only the plan a query gets, never the answer, so the binary that predates its reader is unaffected. Its reader is `connection_service::resolve_inbound`, landing in the same deploy. Note it is a plain (non-CONCURRENT) CREATE INDEX and so takes a brief ACCESS EXCLUSIVE lock on kb_connections; that table is admin-provisioned infra with a handful of rows, so the lock is bounded by table size rather than by traffic. Additive-only => `main` stays auto-deployable.'
);
