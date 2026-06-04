-- ============================================================================
-- Temper — Arc-1 destination schema: SEED (one worked scenario)
-- ----------------------------------------------------------------------------
-- A single coherent scenario that makes every load-bearing access invariant
-- demonstrable by 04_scenarios.sql. Built on the access spec's own worked
-- example (the epd-team-a / epd-team-b intersection bridge).
--
-- Teams DAG (child → parent):
--     temper-system (root)
--        ├── org-common
--        ├── epd-department
--        └── directors
--     epd-team-a  → {epd-department, org-common}
--     epd-team-b  → {epd-department, org-common}
--
-- Cogmaps:
--     system-default  → joined to {temper-system}     (public floor)
--     bridge-map      → joined to {epd-team-a, epd-team-b}   (the intersection)
--     side-map        → joined to {epd-team-a}         (shares a team with bridge-map)
--     directors-map   → joined to {directors}          (homes the private edge)
--     onboarding-cogmap → seeded via cogmap_genesis, joined to {org-common}
--
-- People:  alice∈team-a · bob∈team-b · dave∈org-common · carol∈directors
--          sysadmin (system_access=admin) · nomad (no teams, system_access=none)
-- ============================================================================

SET search_path TO temper_next, public;

-- Event types (the ledger registry). ------------------------------------------
INSERT INTO kb_event_types (name) VALUES
    ('resource_created'), ('resource_updated'), ('resource_deleted'),
    ('relationship_asserted'), ('relationship_retracted'), ('relationship_retyped'),
    ('relationship_reweighted'), ('relationship_folded'),
    ('property_asserted'), ('property_retracted'), ('property_reweighted'), ('property_folded'),
    ('block_created'), ('block_mutated'), ('block_folded'), ('block_provenance_corrected'),
    ('grant_created'), ('grant_revoked'),
    ('cogmap_seeded'), ('region_materialized'), ('delegated_launch');

DO $seed$
DECLARE
    -- profiles
    p_alice uuid; p_bob uuid; p_dave uuid; p_carol uuid; p_sysadmin uuid; p_nomad uuid;
    -- entities (event actors)
    e_alice uuid; e_dave uuid; e_carol uuid; e_agent uuid;
    -- teams
    t_root uuid; t_orgcommon uuid; t_epd uuid; t_directors uuid; t_a uuid; t_b uuid;
    -- cogmaps
    c_sysdefault uuid; c_bridge uuid; c_side uuid; c_directors uuid; c_onboarding uuid;
    -- resources
    r_public_telos uuid; r_common uuid; r_a_private uuid; r_profile_shared uuid;
    r_concept_sprint uuid; r_concept_formal uuid; r_regulation uuid;
    -- events
    ev_assert uuid; ev_region uuid; ev_late uuid;
    et_assert uuid; et_region uuid;
    -- region
    reg uuid;
    v_centroid vector := ('[' || array_to_string(array_fill(0.01::float8, ARRAY[768]), ',') || ']')::vector;
BEGIN
    SELECT id INTO et_assert FROM kb_event_types WHERE name = 'relationship_asserted';
    SELECT id INTO et_region FROM kb_event_types WHERE name = 'region_materialized';

    -- ── Teams + DAG (created FIRST so the root exists when profiles enable) ──
    INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System') RETURNING id INTO t_root;
    INSERT INTO kb_teams (slug, name) VALUES ('org-common', 'Org Common')       RETURNING id INTO t_orgcommon;
    INSERT INTO kb_teams (slug, name) VALUES ('epd-department', 'EPD Department') RETURNING id INTO t_epd;
    INSERT INTO kb_teams (slug, name) VALUES ('directors', 'Directors')          RETURNING id INTO t_directors;
    INSERT INTO kb_teams (slug, name) VALUES ('epd-team-a', 'EPD Team A')         RETURNING id INTO t_a;
    INSERT INTO kb_teams (slug, name) VALUES ('epd-team-b', 'EPD Team B')         RETURNING id INTO t_b;

    INSERT INTO kb_teams_parents (child_id, parent_id) VALUES
        (t_orgcommon, t_root), (t_epd, t_root), (t_directors, t_root),
        (t_a, t_epd), (t_a, t_orgcommon),   -- team-a descends from both
        (t_b, t_epd), (t_b, t_orgcommon);   -- team-b descends from both

    -- ── Profiles. Enabling a profile (system_access <> 'none') AUTO-JOINS the
    --    temper-system root via the sync_system_membership trigger — no read-time
    --    derivation. sysadmin(admin) → root owner; nomad/none → no root join. ──
    INSERT INTO kb_profiles (handle, display_name, system_access) VALUES
        ('alice', 'Alice', 'approved')     RETURNING id INTO p_alice;
    INSERT INTO kb_profiles (handle, display_name, system_access) VALUES
        ('bob', 'Bob', 'approved')         RETURNING id INTO p_bob;
    INSERT INTO kb_profiles (handle, display_name, system_access) VALUES
        ('dave', 'Dave', 'approved')       RETURNING id INTO p_dave;
    INSERT INTO kb_profiles (handle, display_name, system_access) VALUES
        ('carol', 'Carol', 'approved')     RETURNING id INTO p_carol;
    INSERT INTO kb_profiles (handle, display_name, system_access) VALUES
        ('sysadmin', 'Sys Admin', 'admin') RETURNING id INTO p_sysadmin;
    INSERT INTO kb_profiles (handle, display_name, system_access) VALUES
        ('nomad', 'Nomad', 'none')     RETURNING id INTO p_nomad;

    -- ── Entities (the actors that emit events) ────────────────────────────
    INSERT INTO kb_entities (profile_id, name, metadata) VALUES
        (p_alice, 'alice@cli', '{}'::jsonb) RETURNING id INTO e_alice;
    INSERT INTO kb_entities (profile_id, name, metadata) VALUES
        (p_dave, 'dave@cli', '{}'::jsonb) RETURNING id INTO e_dave;
    INSERT INTO kb_entities (profile_id, name, metadata) VALUES
        (p_carol, 'carol@cli', '{}'::jsonb) RETURNING id INTO e_carol;
    -- an agent-instance entity carrying launch-metadata in the open jsonb
    -- ([LEAN→DECISION] domain-b PQ-7 — no entity_kind enum)
    INSERT INTO kb_entities (profile_id, name, metadata) VALUES
        (p_dave, 'onboarding-agent#1',
         jsonb_build_object('model', 'claude-opus-4-8', 'platform', 'cli',
                            'persona', 'steward', 'bound_cogmap', 'onboarding-cogmap'))
        RETURNING id INTO e_agent;

    -- ── Sub-team memberships (the root joins are already maintained above) ──
    INSERT INTO kb_team_members (team_id, profile_id, role) VALUES
        (t_a, p_alice, 'member'),
        (t_b, p_bob, 'member'),
        (t_orgcommon, p_dave, 'maintainer'),
        (t_directors, p_carol, 'owner');

    -- ── Public floor: two public concepts, granted-read to the root team ──
    -- (root grants land in every vis(T) via down-only inheritance ⇒ universal read)
    INSERT INTO kb_resources (title, origin_uri) VALUES
        ('public-telos', 'temper://public') RETURNING id INTO r_public_telos;
    INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES
        ('system-default', r_public_telos) RETURNING id INTO c_sysdefault;
    INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES (c_sysdefault, t_root);
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
        VALUES (r_public_telos, 'kb_cogmaps', c_sysdefault, p_sysadmin, p_sysadmin);

    INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: sprint-rituals', 'temper://c/sprint')
        RETURNING id INTO r_concept_sprint;
    INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: formalization-mandate', 'temper://c/formal')
        RETURNING id INTO r_concept_formal;
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) VALUES
        (r_concept_sprint, 'kb_cogmaps', c_sysdefault, p_sysadmin, p_sysadmin),
        (r_concept_formal, 'kb_cogmaps', c_sysdefault, p_sysadmin, p_sysadmin);
    INSERT INTO kb_resource_access (resource_id, anchor_table, anchor_id, can_read, granted_by_profile_id) VALUES
        (r_concept_sprint, 'kb_teams', t_root, true, p_sysadmin),
        (r_concept_formal, 'kb_teams', t_root, true, p_sysadmin);

    -- ── The intersection demo: three resources, three reach profiles ─────
    -- R_common: granted to org-common ⇒ in vis(a) AND vis(b) (both descend) ⇒ in bridge intersection
    INSERT INTO kb_resources (title, origin_uri) VALUES ('doc: org-common-policy', 'temper://d/common')
        RETURNING id INTO r_common;
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
        VALUES (r_common, 'kb_contexts', uuid_generate_v7(), p_dave, p_dave);
    INSERT INTO kb_resource_access (resource_id, anchor_table, anchor_id, can_read, can_write, granted_by_profile_id)
        VALUES (r_common, 'kb_teams', t_orgcommon, true, true, p_dave);

    -- R_a_private: granted to team-a only ⇒ in vis(a) but NOT vis(b) ⇒ OUT of intersection
    INSERT INTO kb_resources (title, origin_uri) VALUES ('doc: team-a-private', 'temper://d/aprivate')
        RETURNING id INTO r_a_private;
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
        VALUES (r_a_private, 'kb_contexts', uuid_generate_v7(), p_alice, p_alice);
    INSERT INTO kb_resource_access (resource_id, anchor_table, anchor_id, can_read, granted_by_profile_id)
        VALUES (r_a_private, 'kb_teams', t_a, true, p_alice);

    -- R_profile_shared: profile-grant to alice ⇒ visible to alice (consumer) but
    -- NEVER in any vis(T) ⇒ not producer-readable by bridge-map (leak-safety)
    INSERT INTO kb_resources (title, origin_uri) VALUES ('doc: shared-with-alice', 'temper://d/pshared')
        RETURNING id INTO r_profile_shared;
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
        VALUES (r_profile_shared, 'kb_contexts', uuid_generate_v7(), p_dave, p_dave);
    INSERT INTO kb_resource_access (resource_id, anchor_table, anchor_id, can_read, granted_by_profile_id)
        VALUES (r_profile_shared, 'kb_profiles', p_alice, true, p_dave);

    -- ── Cogmaps for the intersection + delegation scenarios ──────────────
    INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('bridge-map', r_public_telos)
        RETURNING id INTO c_bridge;
    INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES (c_bridge, t_a), (c_bridge, t_b);

    INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('side-map', r_public_telos)
        RETURNING id INTO c_side;
    INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES (c_side, t_a);   -- shares team-a with bridge-map

    INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('directors-map', r_public_telos)
        RETURNING id INTO c_directors;
    INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES (c_directors, t_directors);

    -- ── The directors' PRIVATE edge between two PUBLIC concepts ───────────
    -- Homed in directors-map ⇒ invisible to anyone who can't read that map,
    -- even though both endpoints are public (the edge-home protection, access §3).
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, correlation_id)
        VALUES (et_assert, e_carol, 'kb_cogmaps', c_directors, uuid_generate_v7()) RETURNING id INTO ev_assert;
    INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
        VALUES ('kb_resources', r_concept_sprint, 'kb_resources', r_concept_formal, 'leads_to',
                'sprint-rituals→formalization', 'kb_cogmaps', c_directors, ev_assert, ev_assert);

    -- ── Genesis: seed onboarding-cogmap via the single-txn function ───────
    c_onboarding := cogmap_genesis(
        p_name            => 'onboarding-cogmap',
        p_telos_title     => 'Onboarding charter',
        p_telos_statement => 'Help a new EPD engineer reach first-merge confidence in week one.',
        p_questions       => ARRAY[
            'What does this person already know that transfers?',
            'What is the smallest real change that builds confidence?',
            'Where are the sharp edges that scar newcomers?'
        ],
        p_owner_profile   => p_dave,
        p_emitter_entity  => e_agent);
    INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES (c_onboarding, t_orgcommon);

    -- ── Regulation: an express-edged concept-resource off the charter ─────
    INSERT INTO kb_resources (title, origin_uri) VALUES
        ('regulation: pair on the first PR', 'temper://reg/pair') RETURNING id INTO r_regulation;
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
        VALUES (r_regulation, 'kb_cogmaps', c_onboarding, p_dave, p_dave);
    INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
        VALUES ('kb_resources', r_regulation, 'doc_type', '"cogmap_regulation"'::jsonb, ev_assert, ev_assert);
    -- give the regulation a body so the projection returns prose
    DECLARE b_reg uuid; ch_reg uuid; BEGIN
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_regulation, 0, ev_assert, ev_assert) RETURNING id INTO b_reg;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_reg, r_regulation, 0, md5('pair')) RETURNING id INTO ch_reg;
        INSERT INTO kb_chunk_content (chunk_id, content)
            VALUES (ch_reg, 'Always pair a newcomer with a maintainer on their first PR.');
    END;
    INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
        VALUES ('kb_resources', (SELECT telos_resource_id FROM kb_cogmaps WHERE id = c_onboarding),
                'kb_resources', r_regulation, 'express', 'operationalized_by',
                'kb_cogmaps', c_onboarding, ev_assert, ev_assert);

    -- ── A materialized region on onboarding-cogmap (shape + staleness) ────
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (et_region, e_agent, 'kb_cogmaps', c_onboarding) RETURNING id INTO ev_region;
    INSERT INTO kb_cogmap_regions (cogmap_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id)
        VALUES (c_onboarding, v_centroid, 0.9, 'first-week confidence', 1, ev_region, ev_region)
        RETURNING id INTO reg;
    INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity)
        VALUES (reg, 'kb_resources', r_regulation, 0.95);
    UPDATE kb_cogmaps SET shape_materialized_event_id = ev_region WHERE id = c_onboarding;

    -- a LATER edge event touching the map AFTER materialization ⇒ shape is now stale.
    -- occurred_at is set explicitly: now() is transaction-stable, so all seed events
    -- otherwise share one timestamp; real life spreads them across transactions.
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, occurred_at)
        VALUES (et_assert, e_agent, 'kb_cogmaps', c_onboarding, now() + interval '1 minute') RETURNING id INTO ev_late;
    UPDATE kb_edges SET last_event_id = ev_late
        WHERE home_anchor_table = 'kb_cogmaps' AND home_anchor_id = c_onboarding AND edge_kind = 'express';
END;
$seed$;

-- ============================================================================
-- End of 03_seed.sql. Scenarios → 04_scenarios.sql.
-- ============================================================================
