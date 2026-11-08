-- The public_within machine fence asks the one shared person test.
--
-- public_within_reading (20261120000000) answered "is this profile a machine or a connection's
-- agent?" with its own copies of two lookups: NOT EXISTS over kb_machine_clients and NOT EXISTS
-- over kb_connections. profile_is_person (20261113000000) is exactly those two tests, the first by
-- way of is_machine_profile (as redefined in 20261024100000), and it is already the one shared
-- person definition (personal space, private cogmaps, the personal-team and machine-minter
-- checks); is_machine_profile is in turn what the caller classification in Rust and the
-- machine-role triggers read. Two spellings of one fact are two places for a later edit to answer
-- it differently, so the fence now asks profile_is_person. The human-link clause is carried as it
-- was.
--
-- That test is enough because the set it excludes is closed:
--   * a client_credentials token signs in only through a kb_machine_clients row that is not
--     revoked (profile_service::resolve_machine_from_claims): an unregistered client id, or a
--     dynamically registered client (which has no such row), is refused before any profile
--     exists, and a revoked one is refused at the gate though its profile and row remain;
--   * a client row is never deleted and never moved to another profile (20261024100000), so a
--     profile that was a machine stays one, revoked included;
--   * a connection's agent profile carries no auth link and no client row, so it never signs in
--     (kb_connections.profile_id's comment, pinned by connection_service's provisioning test), and
--     its kb_connections row is what marks it.
--
-- What a later edit must not break: the signature, the STABLE SQL body, the human-link clause and
-- the call to profile_is_person. Behaviour is unchanged: profile_is_person is exactly the two
-- lookups it replaces (its NULL guard matches the old body, which is false for a NULL profile).
-- search_path is pinned, as profile_is_person's is, so a table earlier in a caller's path cannot
-- stand in for kb_profile_auth_links. Cost: contexts_public_within still calls the fence only once
-- a candidate row exists. Each call costs about 4 microseconds more than the inline body, paid once
-- per read with a candidate row, once per team view or public_within listing, and once per
-- membership row public_within_audience scans.

CREATE OR REPLACE FUNCTION public_within_reading(p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
               SELECT 1 FROM kb_profile_auth_links l
                WHERE l.profile_id = p_profile
                  AND l.auth_provider <> 'auth0-m2m')
       AND profile_is_person(p_profile);
$$;

COMMENT ON FUNCTION public_within_reading(uuid) IS
$c$The machine fence: machine principals never read through public_within. True only for a
PERSON: a profile with a human auth link (any kb_profile_auth_links row whose provider is not the
machine provider auth0-m2m) that is a person by profile_is_person, the one shared person test
(neither a machine client, revoked or not, nor a connection's agent profile). Everything else is
false, fail-closed: machine clients, connection agents, stewards and auditors (machine clients
themselves), and any profile with no human auth link. The shared person test suffices because
every machine that signs in has a client row, a client row is never deleted or moved, and a
connection's agent holds neither an auth link nor a client row.

Keyed on the profile alone: no GUC, no setting and no request-scoped state. Called by
contexts_public_within (only when a candidate row exists), public_within_audience and the team
service's team view and public_within listing. The fence is the principal, never the token or the
surface, so a person reads with any token that identifies them, including the Slack mention path's
act-as-the-human mint.$c$;

SELECT declare_migration(
    20261123100000,
    'additive',
    'public_within_reading, same signature and same answers: its machine-client and '
    'connection-agent lookups become a call to profile_is_person, the identical pair of tests '
    'already shared as the one person definition, and its search_path is pinned. No '
    'table, column, row or result changes, so an older binary is unaffected.'
);
