-- ============================================================================
-- Temper — Arc-1 destination schema: SCENARIO QUERIES
-- ----------------------------------------------------------------------------
-- Exercises the model against 03_seed.sql to make every load-bearing invariant
-- observable. Run after 01→02→03. Each block prints expected behaviour, then the
-- actual rows / verdict. This is the empirical read the strategy is after.
-- ============================================================================

\pset footer off
SET search_path TO temper_next, public;

\echo ''
\echo '════════ S1. CONSUMER AXIS — resources_visible_to(person) ════════'
\echo '-- alice (team-a): sees org-common-policy (DAG), team-a-private, shared-with-alice (profile), 2 public concepts'
SELECT r.title
FROM resources_visible_to((SELECT id FROM kb_profiles WHERE handle='alice')) v
JOIN kb_resources r ON r.id = v.resource_id
ORDER BY r.title;

\echo '-- bob (team-b): sees org-common-policy + public concepts, but NOT team-a-private nor shared-with-alice'
SELECT r.title
FROM resources_visible_to((SELECT id FROM kb_profiles WHERE handle='bob')) v
JOIN kb_resources r ON r.id = v.resource_id
ORDER BY r.title;

\echo '-- nomad (no teams, no system access, owns nothing): sees NOTHING'
SELECT count(*) AS nomad_visible
FROM resources_visible_to((SELECT id FROM kb_profiles WHERE handle='nomad'));

\echo ''
\echo '════════ S2. PRODUCER AXIS — the least-privilege team INTERSECTION ════════'
\echo '-- "more teams = narrower reach": team-a-private is in side-map(team-a) but FALLS OUT of bridge-map(team-a ∩ team-b).'
\echo '-- shared-with-alice (a PROFILE grant) is in NEITHER map — leak-safety: profile grants never enter a vis(T).'
SELECT r.title,
       r.id IN (SELECT resource_id FROM resources_accessible_to_cogmap((SELECT id FROM kb_cogmaps WHERE name='side-map')))   AS in_side_map_a,
       r.id IN (SELECT resource_id FROM resources_accessible_to_cogmap((SELECT id FROM kb_cogmaps WHERE name='bridge-map'))) AS in_bridge_map_ab
FROM kb_resources r
WHERE r.title IN ('doc: org-common-policy', 'doc: team-a-private', 'doc: shared-with-alice')
ORDER BY r.title;

\echo '-- bridge-map producer reach in full (the common ground of team-a and team-b + the public floor):'
SELECT r.title
FROM resources_accessible_to_cogmap((SELECT id FROM kb_cogmaps WHERE name='bridge-map')) a
JOIN kb_resources r ON r.id = a.resource_id
ORDER BY r.title;

\echo ''
\echo '════════ S3. EDGE-HOME PROTECTION — a private edge between two public concepts ════════'
\echo '-- The directors leads_to edge is homed in directors-map. Both endpoints are public.'
\echo '-- Expect: visible to carol (∈directors) — TRUE; to alice & nomad — FALSE (home unreadable), even though'
\echo '-- alice can read both endpoints.'
SELECT p.handle,
       (SELECT e.id FROM kb_edges e WHERE e.label='sprint-rituals→formalization')
         IN (SELECT edge_id FROM edges_visible_to(p.id)) AS sees_directors_edge
FROM kb_profiles p
WHERE p.handle IN ('carol', 'alice', 'nomad')
ORDER BY p.handle;

\echo ''
\echo '════════ S4. DOMAIN-B PROJECTIONS — charter / questions / regulation ════════'
\echo '-- charter body (onboarding) via the generic resource read:'
SELECT r.title, resource_body_text(r.id) AS body_text
FROM kb_resources r
WHERE r.id = cogmap_telos((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'));

\echo '-- guiding questions (onboarding): role=question blocks, with the provenance-attribution signal:'
SELECT seq, body_text, reinforce_count
FROM resource_blocks(
        cogmap_telos((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap')),
        'cogmap', (SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'), 'question')
ORDER BY seq;

\echo '-- cogmap_regulation(onboarding): the express-edged regulation concept(s):'
SELECT title, edge_label, body_text
FROM cogmap_regulation((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'),
                       'cogmap', (SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'));

\echo '-- gating: nomad (cannot read the map) gets ZERO charter blocks:'
SELECT count(*) AS nomad_charter_rows
FROM resource_blocks(
        cogmap_telos((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap')),
        'profile', (SELECT id FROM kb_profiles WHERE handle='nomad'), NULL);

\echo ''
\echo '════════ S5. DELEGATION PRIMING — cogmaps_share_a_team ════════'
\echo '-- bridge-map & side-map share team-a ⇒ TRUE; bridge-map & directors-map share nothing ⇒ FALSE'
SELECT cogmaps_share_a_team((SELECT id FROM kb_cogmaps WHERE name='bridge-map'),
                            (SELECT id FROM kb_cogmaps WHERE name='side-map'))      AS bridge_shares_side,
       cogmaps_share_a_team((SELECT id FROM kb_cogmaps WHERE name='bridge-map'),
                            (SELECT id FROM kb_cogmaps WHERE name='directors-map')) AS bridge_shares_directors;

\echo ''
\echo '════════ S6. SHAPE SURFACE + STALENESS (on-read aggregate, A3-3) ════════'
\echo '-- cogmap_shape(onboarding): the region surface (member identities NOT exposed):'
SELECT label, salience, member_count
FROM cogmap_shape((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'),
                  'cogmap', (SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'));

\echo '-- cogmap_staleness(onboarding): a later edge event touched the map AFTER materialization ⇒ is_stale=TRUE'
SELECT is_stale, (latest_touch > materialized_at) AS touch_after_materialize
FROM cogmap_staleness((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'));

\echo ''
\echo '════════ S7. ENTITY LAUNCH-METADATA (open jsonb, no entity_kind enum) ════════'
\echo '-- the agent-instance entity carries its launch-metadata in the open metadata jsonb:'
SELECT name, metadata
FROM kb_entities
WHERE name = 'onboarding-agent#1';

\echo ''
\echo '════════ S8. DESCRIPTOR COHERENCE CHECK — write|delete|grant ⇒ read ════════'
\echo '-- an attempt to grant can_write without can_read must be REJECTED by the table CHECK:'
DO $check$
BEGIN
    INSERT INTO kb_resource_access (resource_id, anchor_table, anchor_id, can_read, can_write, granted_by_profile_id)
    VALUES ((SELECT id FROM kb_resources LIMIT 1), 'kb_profiles',
            (SELECT id FROM kb_profiles WHERE handle='nomad'), false, true,
            (SELECT id FROM kb_profiles WHERE handle='sysadmin'));
    RAISE NOTICE 'UNEXPECTED: invalid grant (write without read) was accepted';
EXCEPTION WHEN check_violation THEN
    RAISE NOTICE 'OK: invalid grant rejected by coherence CHECK (write|delete|grant ⇒ read)';
END;
$check$;

\echo ''
\echo '════════ scenarios complete ════════'
