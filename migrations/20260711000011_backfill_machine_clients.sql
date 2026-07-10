-- Backfill: every pre-existing auth0-m2m auth link becomes a registered client.
-- Verified against temper-cloud/main on 2026-07-10: this set is exactly the steward.
-- ONE statement, idempotent. Kept in its own file so a test can execute it standalone.
INSERT INTO kb_machine_clients (client_id, issuer, label, profile_id, registered_by_profile_id)
SELECT l.auth_provider_user_id,
       'auth0-m2m',
       'backfilled: ' || p.handle,
       l.profile_id,
       l.profile_id
  FROM kb_profile_auth_links l
  JOIN kb_profiles p ON p.id = l.profile_id
 WHERE l.auth_provider = 'auth0-m2m'
ON CONFLICT (client_id) DO NOTHING;
