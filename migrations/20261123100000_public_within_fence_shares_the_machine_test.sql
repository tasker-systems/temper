-- The public_within machine fence asks the one shared machine test.
--
-- public_within_reading (20261120000000) answered "is this profile a machine client?" with its own
-- copy of the lookup: NOT EXISTS (SELECT 1 FROM kb_machine_clients ...). The same question is
-- answered everywhere else by is_machine_profile (as redefined in 20261024100000): the caller
-- classification in Rust, the machine role ceiling, machine_never_governs and the personal-space
-- guard. Two spellings of one fact are two places for a later edit to answer it differently, so the
-- fence now calls the shared function.
--
-- That one test is enough on the machine side because the set it reads is closed:
--   * a client_credentials token signs in only through a kb_machine_clients row that is not
--     revoked (profile_service::resolve_machine_from_claims); an unregistered client id, a revoked
--     one, and a dynamically registered client (which has no such row) are refused before any
--     profile exists, so every machine that reaches a door has a row;
--   * a client row is never deleted and never moved to another profile (20261024100000), so a
--     profile that was a machine stays one, revoked included.
--
-- What a later edit must not break: the signature, the STABLE SQL body, the human-link clause and
-- the connection-agent exclusion. Behaviour is unchanged: is_machine_profile is exactly the lookup
-- it replaces. search_path is pinned, as is_machine_profile's is, so a table earlier in a caller's
-- path cannot stand in for kb_machine_clients or kb_connections. Cost: contexts_public_within still
-- calls the fence only once a candidate row exists; each call now runs the shared function, about
-- 2 microseconds more per call (50,000-call loop: 5.8 to 7.8 us), paid once per read with a
-- candidate row, once per team view, and once per member counted by public_within_audience.

CREATE OR REPLACE FUNCTION public_within_reading(p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
               SELECT 1 FROM kb_profile_auth_links l
                WHERE l.profile_id = p_profile
                  AND l.auth_provider <> 'auth0-m2m')
       AND NOT is_machine_profile(p_profile)
       AND NOT EXISTS (SELECT 1 FROM kb_connections k WHERE k.profile_id = p_profile);
$$;

COMMENT ON FUNCTION public_within_reading(uuid) IS
$c$The machine fence: machine principals never read through public_within. True only for a
PERSON: a profile with a human auth link (any kb_profile_auth_links row whose provider is not the
machine provider auth0-m2m), that is not a machine (is_machine_profile, the one machine test the
caller classification and the machine-role triggers share), and that is no connection's agent
profile (kb_connections). Everything else is false, fail-closed: machine clients revoked or not,
connection agents, stewards and auditors (machine clients themselves), and any profile with no
human auth link. The shared machine test suffices because every machine that signs in has a client
row, and a client row is never deleted or moved.

Keyed on the profile alone: no GUC, no setting and no request-scoped state. Called by
contexts_public_within (only when a candidate row exists), public_within_audience and the team
service's team view and public_within listing. The fence is the principal, never the token or the
surface, so a person reads with any token that identifies them, including the Slack mention path's
act-as-the-human mint.$c$;

SELECT declare_migration(
    20261123100000,
    'additive',
    'public_within_reading, same signature and same answers: its machine-client lookup becomes a '
    'call to is_machine_profile, the identical lookup shared by the caller classification and the '
    'machine-role triggers, and its search_path is pinned. No table, column, row or result '
    'changes, so an older binary is unaffected.'
);
