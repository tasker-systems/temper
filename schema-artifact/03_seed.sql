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
    ('resource_created'), ('resource_updated'), ('resource_deleted'), ('resource_rehomed'),
    ('relationship_asserted'), ('relationship_retracted'), ('relationship_retyped'),
    ('relationship_reweighted'), ('relationship_folded'),
    ('relationship_decayed'), ('relationship_corrected'),
    ('property_asserted'), ('property_set'), ('property_retracted'), ('property_reweighted'), ('property_folded'),
    ('block_created'), ('block_mutated'), ('block_folded'), ('block_provenance_corrected'),
    ('grant_created'), ('grant_revoked'),
    ('cogmap_seeded'), ('region_materialized'), ('delegated_launch'), ('invocation_closed'), ('lens_created');

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
    ev_assert uuid; ev_lens1 uuid; ev_lens2 uuid; ev_late uuid;
    et_assert uuid; et_lens uuid; et_reweight uuid;
    -- edges that get payload-referenced (identity-as-input — payloads carry ids)
    v_dir_edge uuid; v_express_edge uuid;
    -- lenses (regions are produced by the temper-next harness, not hand-seeded)
    v_lens uuid; v_lens2 uuid;
BEGIN
    SELECT id INTO et_assert   FROM kb_event_types WHERE name = 'relationship_asserted';
    SELECT id INTO et_lens     FROM kb_event_types WHERE name = 'lens_created';
    SELECT id INTO et_reweight FROM kb_event_types WHERE name = 'relationship_reweighted';

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
    -- Payload-first: the event carries the conformant RelationshipAsserted payload (edge id
    -- pre-generated — identity-as-input); root convention: correlation_id = own id.
    v_dir_edge := uuid_generate_v7();
    ev_assert  := uuid_generate_v7();
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table,
                           producing_anchor_id, correlation_id, payload)
        VALUES (ev_assert, et_assert, e_carol, 'kb_cogmaps', c_directors, ev_assert,
                jsonb_build_object(
                    'edge_id', v_dir_edge,
                    'source', jsonb_build_object('table','kb_resources','id',r_concept_sprint),
                    'target', jsonb_build_object('table','kb_resources','id',r_concept_formal),
                    'edge_kind','leads_to','label','sprint-rituals→formalization','weight',1.0,
                    'home', jsonb_build_object('table','kb_cogmaps','id',c_directors)));
    INSERT INTO kb_edges (id, source_table, source_id, target_table, target_id, edge_kind, label,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
        VALUES (v_dir_edge, 'kb_resources', r_concept_sprint, 'kb_resources', r_concept_formal, 'leads_to',
                'sprint-rituals→formalization', 'kb_cogmaps', c_directors, ev_assert, ev_assert);

    -- ── Genesis: seed onboarding-cogmap via the single-txn function ───────
    -- Payload-first: this pure-SQL seed builds the BlockManifest payload (pre-generated ids + sha256
    -- hashes, NO prose — identity-as-input) and the content sidecar (prose, NULL embedding —
    -- embed_chunks backfills later, same as the concept resources below) from the same rows. Region
    -- membership keys on declared affinity, not chunk hashes/embeddings, so this stays byte-equivalent
    -- to the YAML path's regions (the cross-path proof). Prose is verbatim shared with
    -- schema-artifact/scenarios/onboarding-cogmap.yaml.
    DECLARE v_manifests jsonb; v_content jsonb;
            v_cg uuid := uuid_generate_v7(); v_telos uuid := uuid_generate_v7();
    BEGIN
        WITH rows AS (
            SELECT ord, txt, uuid_generate_v7() AS block_id, uuid_generate_v7() AS chunk_id
            FROM (VALUES
                (0, 'Help a new EPD engineer reach first-merge confidence in week one.'),
                (1, 'What does this person already know that transfers?'),
                (2, 'What is the smallest real change that builds confidence?'),
                (3, 'Where are the sharp edges that scar newcomers?')
            ) AS t(ord, txt)
        )
        SELECT
            jsonb_agg(jsonb_build_object(
                'block_id', block_id,
                'seq', ord,
                'role', CASE WHEN ord = 0 THEN 'statement' ELSE 'question' END,
                'chunks', jsonb_build_array(jsonb_build_object(
                    'chunk_id', chunk_id,
                    'chunk_index', 0,
                    'content_hash', encode(sha256(convert_to(txt, 'UTF8')), 'hex')
                ))
            ) ORDER BY ord),
            jsonb_object_agg(chunk_id::text, jsonb_build_object('content', txt, 'embedding', NULL))
        INTO v_manifests, v_content
        FROM rows;

        SELECT g.cogmap_id INTO c_onboarding FROM cogmap_genesis(
            jsonb_build_object(
                'cogmap_id', v_cg,
                'name', 'onboarding-cogmap',
                'owner_profile_id', p_dave,
                'telos', jsonb_build_object(
                    'resource_id', v_telos,
                    'title', 'Onboarding charter',
                    'origin_uri', 'temper://genesis',
                    'blocks', v_manifests)),
            v_content, e_agent) g;
    END;
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
                'kb_cogmaps', c_onboarding, ev_assert, ev_assert)
        RETURNING id INTO v_express_edge;  -- referenced by the late edge-touch payload below

    -- ── The two lenses (each its own honest lens_created event, payload-first) ────
    -- telos-default (spec §5c, concrete starting defaults; tunable) and the S6f prop-heavy split
    -- (property-dominant + sequence-discounted: the leads_to-only setup→first-build pair merges
    -- under telos-default, splits under prop-heavy). Lens ids pre-generated; payloads conformant
    -- with the LensCreated schema (payload spec §3).
    v_lens   := uuid_generate_v7();
    ev_lens1 := uuid_generate_v7();
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table,
                           producing_anchor_id, correlation_id, payload)
        VALUES (ev_lens1, et_lens, e_agent, 'kb_cogmaps', c_onboarding, ev_lens1,
                jsonb_build_object(
                    'lens_id', v_lens, 'cogmap_id', c_onboarding,
                    'name','telos-default','selection_kind','homed',
                    'weights', jsonb_build_object('express',1.0,'contains',1.0,'leads_to',0.6,'near',0.3,'prop',0.4),
                    'salience', jsonb_build_object('telos',0.5,'ref',0.3,'central',0.2),
                    'resolution', 0.5));
    INSERT INTO kb_cogmap_lenses
        (id, cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near,
         w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id)
    VALUES (v_lens, c_onboarding, 'telos-default', 'homed', 1.0, 1.0, 0.6, 0.3,
            0.4, 0.5, 0.3, 0.2, 0.5, ev_lens1);

    v_lens2  := uuid_generate_v7();
    ev_lens2 := uuid_generate_v7();
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table,
                           producing_anchor_id, correlation_id, payload)
        VALUES (ev_lens2, et_lens, e_agent, 'kb_cogmaps', c_onboarding, ev_lens2,
                jsonb_build_object(
                    'lens_id', v_lens2, 'cogmap_id', c_onboarding,
                    'name','telos-default-propheavy','selection_kind','homed',
                    'weights', jsonb_build_object('express',1.0,'contains',1.0,'leads_to',0.1,'near',0.3,'prop',1.2),
                    'salience', jsonb_build_object('telos',0.5,'ref',0.3,'central',0.2),
                    'resolution', 0.5));
    INSERT INTO kb_cogmap_lenses
        (id, cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near,
         w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id)
    VALUES (v_lens2, c_onboarding, 'telos-default-propheavy', 'homed', 1.0, 1.0, 0.1, 0.3,
            1.2, 0.5, 0.3, 0.2, 0.5, ev_lens2);

    -- ════════════════════════════════════════════════════════════════════════
    -- THE FALSIFICATION CAST (spec §5a/§5b). Content is the INDEPENDENT VARIABLE:
    -- α genuinely similar; β genuinely divergent; solo near-α in content yet declared-standalone.
    -- Regions are produced by the temper-next harness; this seeds the DECLARED substrate only.
    --
    -- Clustering note (average-link, resolution 0.5; affinity = Σ w_kind·weight + w_prop·facet_overlap):
    --   • a node joins a cluster only if its AVERAGE affinity to all members ≥ resolution, so a single
    --     edge cannot pull a node into a large cluster — a SHARED FACET (links every pair) is what binds
    --     the deployment group, and the edgeless bridge joins via that facet alone (S6e).
    --   • the deployment facet is authored at weight 1.5 so facet-overlap alone (w_prop 0.4 · 1.5 = 0.6)
    --     clears resolution. This is declared substrate, not cosine — the falsification stands (§5b).
    --   • the setup→first-build pair is leads_to-only (no facet): 0.6 under telos-default (merges),
    --     0.1 under prop-heavy (splits) — the S6f membership delta.
    -- Helper var bag (declared per concept inside nested blocks, like b_reg/ch_reg above).
    -- ════════════════════════════════════════════════════════════════════════

    -- ── α: first-week confidence (content authored to be GENUINELY SIMILAR) ──
    DECLARE r_pair uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: pair-on-first-PR','temper://c/pair') RETURNING id INTO r_pair;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_pair,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_pair,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_pair, 0, md5('temper://c/pair')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Pair on the first pull request. A new engineer''s earliest change should be small and made '
         || 'alongside someone who knows the code, so the first contribution builds confidence safely '
         || 'rather than risking a large unfamiliar change. Small, paired, early — that is how confidence starts.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_pair, 'facet', '{"phase":"first-week"}'::jsonb, ev_assert, ev_assert);
    END;
    DECLARE r_small uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: smallest-real-change','temper://c/smallest') RETURNING id INTO r_small;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_small,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_small,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_small, 0, md5('temper://c/smallest')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Choose the smallest real change for a newcomer''s first contribution. A tiny, safe, early '
         || 'pull request builds confidence faster than an ambitious one. Keep the first change small and '
         || 'paired so confidence grows safely in the first week.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_small, 'facet', '{"phase":"first-week"}'::jsonb, ev_assert, ev_assert);
    END;
    DECLARE r_conf uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: early-confidence-signal','temper://c/confidence') RETURNING id INTO r_conf;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_conf,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_conf,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_conf, 0, md5('temper://c/confidence')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Read the early signals of a new engineer''s confidence. In the first week a small, paired '
         || 'contribution that merges safely is the clearest sign confidence is building. Watch for the '
         || 'early, small, safe wins that tell you the newcomer is gaining footing.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_conf, 'facet', '{"phase":"first-week"}'::jsonb, ev_assert, ev_assert);
    END;

    -- ── β: deployment (content authored to be GENUINELY DIVERGENT subtopics) ──
    -- facet {topic:deployment} at weight 1.5 binds the group; content diverges hard.
    DECLARE r_stage uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: staging-rollout','temper://c/staging') RETURNING id INTO r_stage;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_stage,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_stage,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_stage, 0, md5('temper://c/staging')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Promote a release through staging environments before production. Each environment — '
         || 'development, staging, pre-prod — gates the next, and a build is promoted only after it '
         || 'passes that stage. Staging mirrors production so problems surface before the final promotion.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_stage, 'facet', '{"topic":"deployment"}'::jsonb, 1.5, ev_assert, ev_assert);
    END;
    DECLARE r_flags uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: feature-flags','temper://c/flags') RETURNING id INTO r_flags;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_flags,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_flags,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_flags, 0, md5('temper://c/flags')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Gate new functionality behind feature flags. A toggle enables a code path for a percentage '
         || 'of users, dark-launches it, or turns it off instantly without a redeploy. Flags decouple '
         || 'deploy from release and let you ramp exposure gradually.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_flags, 'facet', '{"topic":"deployment"}'::jsonb, 1.5, ev_assert, ev_assert);
    END;
    DECLARE r_roll uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: rollback-runbook','temper://c/rollback') RETURNING id INTO r_roll;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_roll,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_roll,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_roll, 0, md5('temper://c/rollback')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'When an incident starts, follow the rollback runbook. Revert to the last known-good version, '
         || 'page the on-call owner, and record the timeline. The runbook lists the exact commands to undo '
         || 'a bad change and restore the previous release.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_roll, 'facet', '{"topic":"deployment"}'::jsonb, 1.5, ev_assert, ev_assert);
    END;
    DECLARE r_onc uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: oncall-handoff','temper://c/oncall') RETURNING id INTO r_onc;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_onc,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_onc,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_onc, 0, md5('temper://c/oncall')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Hand off the on-call pager cleanly at the end of each shift. Summarize open incidents, ongoing '
         || 'alerts, and escalation contacts so the next responder has full context. A good handoff covers '
         || 'who to page and what is still unresolved.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_onc, 'facet', '{"topic":"deployment"}'::jsonb, 1.5, ev_assert, ev_assert);
    END;

    -- ── bridge: facet-only member of β (NO edge — joins via facet_overlap, S6e) ──
    DECLARE r_check uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: deploy-confidence-checklist','temper://c/checklist') RETURNING id INTO r_check;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_check,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_check,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_check, 0, md5('temper://c/checklist')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'A deployment readiness checklist gathers the pre-release checks into one list: tests green, '
         || 'migrations reviewed, dashboards ready, rollback plan written. Tick every box before you ship '
         || 'so the team can deploy with confidence.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_check, 'facet', '{"topic":"deployment"}'::jsonb, 1.5, ev_assert, ev_assert);
    END;

    -- ── tension: blue-green vs big-bang (near + label 'contradicts' → internal_tension, S6g) ──
    DECLARE r_bg uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: blue-green','temper://c/bluegreen') RETURNING id INTO r_bg;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_bg,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_bg,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_bg, 0, md5('temper://c/bluegreen')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Blue-green deployment runs two identical production environments. Traffic flows to blue while '
         || 'green holds the new version; you cut over by switching the router, and switch back instantly '
         || 'if anything looks wrong. Two environments, one live at a time.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_bg, 'facet', '{"topic":"deployment"}'::jsonb, 1.5, ev_assert, ev_assert);
    END;
    DECLARE r_bb uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: big-bang-cutover','temper://c/bigbang') RETURNING id INTO r_bb;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_bb,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_bb,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_bb, 0, md5('temper://c/bigbang')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'A big-bang cutover switches every user to the new system at once. There is a single '
         || 'coordinated moment when the old version is retired and the new one takes all traffic — no '
         || 'gradual ramp, no parallel environments, just one decisive switch.');
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_bb, 'facet', '{"topic":"deployment"}'::jsonb, 1.5, ev_assert, ev_assert);
    END;

    -- ── isolate: content reads like α, but NO facet + NO edge ⇒ cosine WOULD merge, declared does NOT (S6d) ──
    DECLARE r_solo uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: solo-retro-note','temper://c/solo') RETURNING id INTO r_solo;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_solo,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_solo,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_solo, 0, md5('temper://c/solo')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'A retro note on building confidence: the small, early, paired wins in a newcomer''s first '
         || 'week are what make them feel safe to contribute. Confidence comes from safe, small, early steps.');
        -- DELIBERATELY no facet, no edge.
    END;

    -- ── S6f delta pair: setup → first-build, leads_to ONLY (no facet) ──
    -- telos-default (w_leads_to 0.6): co-region. prop-heavy (w_leads_to 0.1): two singletons.
    DECLARE r_setup uuid; r_build uuid; b_tmp uuid; ch_tmp uuid; BEGIN
        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: first-day-setup','temper://c/setup') RETURNING id INTO r_setup;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_setup,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_setup,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_setup, 0, md5('temper://c/setup')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'On day one, set up your laptop: install the toolchain, configure access, and get the '
         || 'development environment running so you can build the project locally.');

        INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: first-build-green','temper://c/firstbuild') RETURNING id INTO r_build;
        INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
            VALUES (r_build,'kb_cogmaps',c_onboarding,p_dave,p_dave);
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (r_build,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
            VALUES (b_tmp, r_build, 0, md5('temper://c/firstbuild')) RETURNING id INTO ch_tmp;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (ch_tmp,
            'Once setup is done, get a green build: pull the main branch, run the test suite, and confirm '
         || 'everything passes locally before you start changing code.');

        INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label,
                              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', r_setup, 'kb_resources', r_build, 'leads_to', 'then',
                    'kb_cogmaps', c_onboarding, ev_assert, ev_assert);
    END;

    -- ── Declared edges among the cast (the ONLY structure that forms regions) ──
    INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
    SELECT 'kb_resources', s.id, 'kb_resources', t.id, k::edge_kind, lbl,
           'kb_cogmaps', c_onboarding, ev_assert, ev_assert
    FROM (VALUES
        ('temper://c/pair','temper://c/smallest','near', NULL),       -- α1 ~ α2
        ('temper://c/pair','temper://c/confidence','near', NULL),     -- α1 ~ α3
        ('temper://c/smallest','temper://c/pair','near', NULL),       -- α2 ~ α1 (symmetric reinforcement)
        ('temper://c/confidence','temper://c/pair','express', NULL),  -- α3 → α1
        ('temper://c/staging','temper://c/flags','leads_to', NULL),   -- β1 → β2
        ('temper://c/flags','temper://c/rollback','leads_to', NULL),  -- β2 → β3
        ('temper://c/rollback','temper://c/oncall','leads_to', NULL), -- β3 → β4
        ('temper://c/bluegreen','temper://c/bigbang','near','contradicts')  -- tension (binds, S6g)
    ) AS e(src, tgt, k, lbl)
    JOIN kb_resources s ON s.origin_uri = e.src
    JOIN kb_resources t ON t.origin_uri = e.tgt;

    -- A late edge-touch event on the express edge, at seed time (now()). Regions are produced by the
    -- temper-next harness AFTER the seed, so the materialization watermark is naturally LATER than this
    -- seed-time touch ⇒ a fresh map right after materialize. A genuinely-later touch (the S6h edge in
    -- run_eval) then drives the fresh→stale transition. (Previously future-dated now()+1min to fake
    -- "after a seed-time materialization", which no longer exists — that dating dominated latest_touch
    -- and made the staleness signal untestable.)
    -- Semantically a TOUCH, not an assert — typed relationship_reweighted with a conformant
    -- payload referencing the regulation express edge (identity-as-input).
    ev_late := uuid_generate_v7();
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table,
                           producing_anchor_id, correlation_id, occurred_at, payload)
        VALUES (ev_late, et_reweight, e_agent, 'kb_cogmaps', c_onboarding, ev_late, now(),
                jsonb_build_object('edge_id', v_express_edge, 'weight', 1.0));
    UPDATE kb_edges SET last_event_id = ev_late
        WHERE home_anchor_table = 'kb_cogmaps' AND home_anchor_id = c_onboarding AND edge_kind = 'express';
END;
$seed$;

-- ============================================================================
-- End of 03_seed.sql. Scenarios → 04_scenarios.sql.
-- ============================================================================
