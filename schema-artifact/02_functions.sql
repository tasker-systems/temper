-- ============================================================================
-- Temper — Arc-1 destination schema: FUNCTIONS
-- ----------------------------------------------------------------------------
-- The access-gating functions (the two-principal sum type), the delegation
-- predicate, the single-transaction cogmap seeding, and the Domain-B read
-- projections. Load after 01_schema.sql.
--
-- Principal sum type (access §4, CS-1): a substrate read carries ONE principal.
--   Profile(uuid) — a person reading        → resources_visible_to        (consumer axis)
--   Cogmap(uuid)  — an agent producing in M  → resources_accessible_to_cogmap (producer axis)
-- An agent cannot pass a profile: "never who is at the keyboard" is structural.
-- In SQL the sum is modeled as (p_principal_kind text IN ('profile','cogmap'),
-- p_principal_id uuid), dispatched by resources_readable_by below.
-- ============================================================================

SET search_path TO temper_next, public;

-- ============================================================================
-- TEAMS DAG (kb_teams_parents inherits DOWN-only, access §4)
-- ============================================================================

-- A team plus all its ancestors (up the DAG). Grants on an ancestor are
-- inherited DOWN to this team, so the resources this team can reach via grants
-- are those granted on any team in {self} ∪ ancestors.
CREATE FUNCTION team_ancestors(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE up AS (
        SELECT p_team AS team_id
        UNION
        SELECT tp.parent_id
        FROM kb_teams_parents tp
        JOIN up ON tp.child_id = up.team_id
    )
    SELECT team_id FROM up;
$$;

-- A person's effective team set is just their memberships — uniform, no special
-- cases. The root-team membership is NOT derived at read time from a slug + a
-- system_access check; it is a REAL kb_team_members row that falls out of enabling
-- the profile (see sync_system_membership below). Approval auto-joins the
-- temper-system root, whose full overlap with the system-default cogmap is then
-- structural, not re-derived in every read function.
--   [REVISES access §6 OQ-3: "virtual root-membership, no stored row" → a real
--    membership maintained at approval time. The access mechanics read plainly.]
CREATE FUNCTION profile_effective_teams(p_profile uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    SELECT tm.team_id FROM kb_team_members tm WHERE tm.profile_id = p_profile;
$$;

-- Enabling a profile is what joins it to the teams DAG root. system_access is the
-- profile STATUS; its access consequence is a maintained membership, not a
-- read-time branch. Role encodes the §6 tier: approved → watcher (read-only
-- ceiling); admin → owner (management tier). Disabling ('none') removes the join.
CREATE FUNCTION sync_system_membership()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    v_root uuid;
BEGIN
    SELECT id INTO v_root FROM kb_teams WHERE slug = 'temper-system';
    IF v_root IS NULL THEN
        RETURN NEW;  -- root not yet seeded; nothing to maintain
    END IF;
    IF NEW.system_access = 'none' THEN
        DELETE FROM kb_team_members WHERE team_id = v_root AND profile_id = NEW.id;
    ELSE
        INSERT INTO kb_team_members (team_id, profile_id, role)
        VALUES (v_root, NEW.id,
                CASE NEW.system_access WHEN 'admin' THEN 'owner'::team_role
                                       ELSE 'watcher'::team_role END)
        ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER trg_sync_system_membership
    AFTER INSERT OR UPDATE OF system_access ON kb_profiles
    FOR EACH ROW EXECUTE FUNCTION sync_system_membership();

-- ============================================================================
-- CONSUMER AXIS — resources_visible_to(profile)
-- ============================================================================

-- A person reads a resource if they own/originated it, hold a direct
-- profile-anchored grant, or hold a team-anchored grant on any team they reach
-- (an effective team or one of its ancestors — grants inherit down). The
-- temper-system root floor falls out: every team descends from root, so a root
-- grant lands in every person's ancestor set.
CREATE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    -- owned / originated (the home confers access to its principals)
    SELECT h.resource_id FROM kb_resource_homes h
     WHERE h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile
    UNION
    -- direct profile-anchored grant (consumer-axis ONLY — never enters a vis(T))
    SELECT ra.resource_id FROM kb_resource_access ra
     WHERE ra.anchor_table = 'kb_profiles' AND ra.anchor_id = p_profile AND ra.can_read
    UNION
    -- team-anchored grant on a reachable (self-or-ancestor) team
    SELECT ra.resource_id FROM kb_resource_access ra
     JOIN reachable_teams rt ON ra.anchor_id = rt.team_id
     WHERE ra.anchor_table = 'kb_teams' AND ra.can_read;
$$;

-- ============================================================================
-- PRODUCER AXIS — resources_accessible_to_cogmap(M) = ⋂ vis(T) over teams(M)
-- ============================================================================

-- vis(T): team T's visibility. TEAM-anchored grants on T or its ancestors only.
-- Profile-anchored grants NEVER enter vis(T) — the A2 leak-safety invariant
-- (access §4): a person-grant cannot be referenced into a cogmap or leak
-- cross-team, which is what makes admitting kb_profiles as a grantee safe.
CREATE FUNCTION vis_team(p_team uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT ra.resource_id
    FROM team_ancestors(p_team) a
    JOIN kb_resource_access ra
      ON ra.anchor_table = 'kb_teams' AND ra.anchor_id = a.team_id AND ra.can_read;
$$;

-- The least-privilege team-INTERSECTION (access §4). An agent producing in M may
-- read only the common ground of M's joined teams — the only bound that closes
-- the cross-team leak (every joined team can read M's shape). Plus M's own
-- interior (resources homed in M), conferred unconditionally by map-home.
--   • empty teams(M) ⇒ NO shared rows (⋂ over ∅ would be the universe — backwards;
--     default-closed by construction). Own-home still readable.
--   • more teams ⇒ NARROWER reach (the overlap). Deliberate.
--   • the launching person's memberships never enter.
CREATE FUNCTION resources_accessible_to_cogmap(p_cogmap uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    -- own interior (map-home-confers, access §1) — always readable by the map's agent
    SELECT h.resource_id FROM kb_resource_homes h
     WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = p_cogmap
    UNION
    -- shared reach: a resource in EVERY joined team's vis(T); empty join ⇒ none
    SELECT v.resource_id
    FROM (
        SELECT tc.team_id, vt.resource_id
        FROM kb_team_cogmaps tc
        CROSS JOIN LATERAL vis_team(tc.team_id) vt
        WHERE tc.cogmap_id = p_cogmap
    ) v
    GROUP BY v.resource_id
    HAVING count(DISTINCT v.team_id) = (
        SELECT count(*) FROM kb_team_cogmaps tc WHERE tc.cogmap_id = p_cogmap
    )
    AND (SELECT count(*) FROM kb_team_cogmaps tc WHERE tc.cogmap_id = p_cogmap) > 0;
$$;

-- Dispatch the principal sum type to the correct axis.
CREATE FUNCTION resources_readable_by(p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT resource_id FROM resources_visible_to(p_principal_id)        WHERE p_principal_kind = 'profile'
    UNION
    SELECT resource_id FROM resources_accessible_to_cogmap(p_principal_id) WHERE p_principal_kind = 'cogmap';
$$;

-- ============================================================================
-- COGMAP READABILITY & EDGE-HOME (access §3)
-- ============================================================================

-- A person reads a cogmap's SHAPE iff they are a member of a team joined to it
-- (any joined-team member, access §3 / map-regions §4). The root-joined
-- system-default cogmap is readable by anyone with approved+ (effective teams
-- include root). Membership-based — distinct from the down-only GRANT inheritance.
CREATE FUNCTION cogmap_readable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_team_cogmaps tc
        JOIN profile_effective_teams(p_profile) e ON e.team_id = tc.team_id
        WHERE tc.cogmap_id = p_cogmap
    );
$$;

-- Can a Profile read a polymorphic anchor (an edge/region home)?
--   cogmap  → cogmap_readable_by_profile
--   context → treated as readable (Domain-A workspaces; the gating scenario that
--             matters is the cogmap-homed private edge). A real impl would gate
--             context-home by context membership; simplified here for the artifact.
CREATE FUNCTION anchor_readable_by_profile(p_profile uuid, p_anchor_table text, p_anchor_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps'  THEN cogmap_readable_by_profile(p_profile, p_anchor_id)
        WHEN 'kb_contexts' THEN true
        ELSE false
    END;
$$;

-- Is a polymorphic endpoint readable by a Profile? (resource → visible-set;
-- cogmap → shape-readable). The "endpoint integrity" gate (access §3): you may
-- not traverse to a node you cannot see.
CREATE FUNCTION endpoint_readable_by_profile(p_profile uuid, p_endpoint_table text, p_endpoint_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_endpoint_table
        WHEN 'kb_resources' THEN p_endpoint_id IN (SELECT resource_id FROM resources_visible_to(p_profile))
        WHEN 'kb_cogmaps'   THEN cogmap_readable_by_profile(p_profile, p_endpoint_id)
        ELSE false
    END;
$$;

-- Edges TRAVERSABLE by a Profile: edge-home visible AND both endpoints
-- independently readable (access §3, A2-4 — both gates AND for traversal). The
-- directors' private edge between two public concepts is invisible because its
-- HOME (the directors' cogmap) is unreadable, even though both endpoints are public.
CREATE FUNCTION edges_visible_to(p_profile uuid)
RETURNS TABLE(edge_id uuid) LANGUAGE sql STABLE AS $$
    SELECT e.id
    FROM kb_edges e
    WHERE NOT e.is_folded
      AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, e.source_table, e.source_id)
      AND endpoint_readable_by_profile(p_profile, e.target_table, e.target_id);
$$;

-- ============================================================================
-- DELEGATION — cogmaps_share_a_team (map-to-map spec)
-- ============================================================================

-- The priming authz-prior: true iff the two cogmaps share ≥1 joined team. A LIVE
-- predicate (never materialized). Gates frame-injection (borrowing a target's
-- telos + blurred shape on a single bridge), NOT material reads — those stay
-- bound to resources_accessible_to_cogmap(originating), strictly stronger.
CREATE FUNCTION cogmaps_share_a_team(p_cogmap_a uuid, p_cogmap_b uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_team_cogmaps a
        JOIN kb_team_cogmaps b ON a.team_id = b.team_id
        WHERE a.cogmap_id = p_cogmap_a AND b.cogmap_id = p_cogmap_b
    );
$$;

-- ============================================================================
-- BODY-TEXT PROJECTION (β: text is emergent from chunks, not stored on blocks)
-- ============================================================================

-- Assemble a resource's body from its current chunks of non-folded blocks
-- (is_current AND NOT is_folded — the orthogonal gates).
CREATE FUNCTION resource_body_text(p_resource uuid)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT string_agg(cc.content, E'\n\n' ORDER BY b.seq, ch.chunk_index)
    FROM kb_content_blocks b
    JOIN kb_chunks ch        ON ch.block_id = b.id AND ch.is_current
    JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
    WHERE b.resource_id = p_resource AND NOT b.is_folded;
$$;

CREATE FUNCTION block_body_text(p_block uuid)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT string_agg(cc.content, E'\n\n' ORDER BY ch.chunk_index)
    FROM kb_chunks ch
    JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
    WHERE ch.block_id = p_block AND ch.is_current;
$$;

-- ============================================================================
-- DOMAIN-B READ PROJECTIONS (conventions over kernel; access-gated reads)
-- ============================================================================

-- The telos-charter resource of a cogmap, IF readable by the principal.
CREATE FUNCTION cogmap_charter(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(resource_id uuid, title text, body_text text) LANGUAGE sql STABLE AS $$
    SELECT r.id, r.title, resource_body_text(r.id)
    FROM kb_cogmaps c
    JOIN kb_resources r ON r.id = c.telos_resource_id
    WHERE c.id = p_cogmap
      AND r.id IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id));
$$;

-- The charter's guiding questions: blocks seq>=1 (block-0 is the telos statement).
-- Reinforcement signal (Domain-B PQ-1): the artifact's proxy is the count + recency
-- of {kind:block} provenance accretions into the question-block — "the substrate
-- exposes the reference stream; narrowing to confirming-acts is black-box tuning."
CREATE FUNCTION cogmap_questions(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(seq int, block_id uuid, body_text text,
              reinforce_count bigint, last_reinforced_at timestamptz) LANGUAGE sql STABLE AS $$
    SELECT b.seq, b.id, block_body_text(b.id),
           count(p.id) FILTER (WHERE NOT p.is_corrected),
           max(p.created) FILTER (WHERE NOT p.is_corrected)
    FROM kb_cogmaps c
    JOIN kb_resources r          ON r.id = c.telos_resource_id
    JOIN kb_content_blocks b     ON b.resource_id = r.id AND b.seq >= 1 AND NOT b.is_folded
    LEFT JOIN kb_block_provenance p ON p.block_id = b.id
    WHERE c.id = p_cogmap
      AND r.id IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    GROUP BY b.seq, b.id
    ORDER BY b.seq;
$$;

-- Regulation: the open set of concept-resources the charter `express`-edges to
-- (label 'operationalized_by'), filtered to those the principal can read.
CREATE FUNCTION cogmap_regulation(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(resource_id uuid, title text, body_text text, edge_label text) LANGUAGE sql STABLE AS $$
    SELECT r.id, r.title, resource_body_text(r.id), e.label
    FROM kb_cogmaps c
    JOIN kb_edges e ON e.source_table = 'kb_resources'
                   AND e.source_id = c.telos_resource_id
                   AND e.edge_kind = 'express'
                   AND NOT e.is_folded
    JOIN kb_resources r ON r.id = e.target_id AND e.target_table = 'kb_resources'
    WHERE c.id = p_cogmap
      AND r.id IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id));
$$;

-- ============================================================================
-- COGMAP SHAPE SURFACE + STALENESS (map-regions §3/§6)
-- ============================================================================

-- The surface tier (centroid, salience, label, member_count) — readable by any
-- principal who can read the map. Member identities are NEVER returned here
-- (interior is dereferenced per-member through resources_visible_to).
CREATE OR REPLACE FUNCTION cogmap_shape(
    p_cogmap uuid, p_principal_kind text, p_principal_id uuid, p_lens uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, salience double precision,
              content_cohesion double precision, label text, member_count int)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.lens_id, reg.salience, reg.content_cohesion, reg.label, reg.member_count
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)   -- default = all lenses; Plan 3 may default to telos-default
      AND (
        (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
        OR (p_principal_kind = 'cogmap' AND p_principal_id = p_cogmap)
      );
$$;

-- Content cohesion (spec §2c): mean member-to-centroid cosine. A DOWNSTREAM readout over a
-- formed region (cosine never enters FORMATION — that is Plan 2's declared-only affinity).
-- Per-concept pooling: each member resource's current chunk embeddings are mean-pooled to one
-- vector first (pool-per-concept-then-mean, map-regions OQ-1); the region centroid is the mean
-- of those; cohesion is the mean cosine of each member-vector to the centroid.
CREATE FUNCTION cogmap_region_content_cohesion(p_region uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH member_vec AS (   -- one pooled vector per member resource
        SELECT m.member_id, avg(ch.embedding) AS v
        FROM kb_cogmap_region_members m
        JOIN kb_chunks ch        ON ch.resource_id = m.member_id AND ch.is_current
        JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded  -- vector gate mirrors embed + body-text
        WHERE m.region_id = p_region AND m.member_table = 'kb_resources'
        GROUP BY m.member_id
    ),
    ctr AS (SELECT avg(v) AS c FROM member_vec)
    SELECT avg(1 - (mv.v <=> ctr.c)) FROM member_vec mv, ctr;
$$;

-- Telos alignment (spec §2c, salience part): cosine of the region centroid to the cogmap's
-- telos-resource embedding (kb_cogmaps.telos_resource_id). "Importance under the map's telos,"
-- literal because the telos IS a resource with chunks. NULL iff the telos has no current chunks.
CREATE FUNCTION cogmap_region_telos_alignment(p_region uuid, p_cogmap uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH telos AS (
        SELECT avg(ch.embedding) AS v
        FROM kb_cogmaps c
        JOIN kb_chunks ch        ON ch.resource_id = c.telos_resource_id AND ch.is_current
        JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded  -- vector gate mirrors embed + body-text
        WHERE c.id = p_cogmap
    ),
    reg AS (SELECT centroid AS v FROM kb_cogmap_regions WHERE id = p_region)
    SELECT 1 - (reg.v <=> telos.v) FROM reg, telos WHERE telos.v IS NOT NULL;
$$;

-- Reference standing (spec §2c): summed reinforce_count over the member resources' blocks
-- (a count() over kb_block_provenance, page-04 — derived, never stored). is_corrected excluded.
CREATE FUNCTION cogmap_region_reference_standing(p_region uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT coalesce(count(p.*), 0)::double precision
    FROM kb_cogmap_region_members m
    JOIN kb_content_blocks b ON b.resource_id = m.member_id AND NOT b.is_folded
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE m.region_id = p_region AND m.member_table = 'kb_resources';
$$;

-- Centrality (spec §2c): internal declared-affinity mass × size. Sum of declared edge weights
-- BOTH of whose endpoints are members of the region, times member_count. Raw (un-lens-weighted);
-- Plan 2 scales by the lens at materialization. Cosine never enters.
CREATE FUNCTION cogmap_region_centrality(p_region uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH mem AS (
        SELECT member_id FROM kb_cogmap_region_members
        WHERE region_id = p_region AND member_table = 'kb_resources'
    ),
    internal AS (
        SELECT coalesce(sum(e.weight), 0) AS mass
        FROM kb_edges e
        WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
          AND e.source_id IN (SELECT member_id FROM mem)
          AND e.target_id IN (SELECT member_id FROM mem)
          AND NOT e.is_folded
    )
    SELECT internal.mass * (SELECT count(*) FROM mem) FROM internal;
$$;

-- Internal tension (spec §2a/§2c): declared opposition among members — a FEATURE of the region,
-- never a fracture. Matches a caller-supplied label set (default {'contradicts'}); semantics are
-- NOT reserved at the kernel — the caller (lens) decides what counts as opposed.
CREATE FUNCTION cogmap_region_internal_tension(p_region uuid, p_opposed_labels text[])
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH mem AS (
        SELECT member_id FROM kb_cogmap_region_members
        WHERE region_id = p_region AND member_table = 'kb_resources'
    )
    SELECT coalesce(sum(e.weight), 0)::double precision
    FROM kb_edges e
    WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
      AND e.source_id IN (SELECT member_id FROM mem)
      AND e.target_id IN (SELECT member_id FROM mem)
      AND NOT e.is_folded
      AND e.label = ANY(p_opposed_labels);
$$;

-- Staleness (A3-3): ON-READ aggregate, not a denormalized watermark. Compares the
-- stored materialization watermark (kb_cogmaps.shape_materialized_event_id) against
-- the latest event touching the map's homed regions/edges. Stale reads are allowed
-- and LEGIBLE — this reports staleness, never blocks on it.
CREATE FUNCTION cogmap_staleness(p_cogmap uuid)
RETURNS TABLE(materialized_at timestamptz, latest_touch timestamptz, is_stale boolean)
LANGUAGE sql STABLE AS $$
    WITH mat AS (
        SELECT ev.occurred_at AS materialized_at
        FROM kb_cogmaps c
        LEFT JOIN kb_events ev ON ev.id = c.shape_materialized_event_id
        WHERE c.id = p_cogmap
    ),
    touch AS (
        SELECT max(occurred_at) AS latest_touch FROM (
            SELECT ev.occurred_at FROM kb_cogmap_regions reg
              JOIN kb_events ev ON ev.id = reg.last_event_id
             WHERE reg.cogmap_id = p_cogmap
            UNION ALL
            SELECT ev.occurred_at FROM kb_edges e
              JOIN kb_events ev ON ev.id = e.last_event_id
             WHERE e.home_anchor_table = 'kb_cogmaps' AND e.home_anchor_id = p_cogmap
        ) t
    )
    SELECT mat.materialized_at, touch.latest_touch,
           COALESCE(touch.latest_touch > mat.materialized_at, mat.materialized_at IS NULL)
    FROM mat, touch;
$$;

-- ============================================================================
-- COGMAP GENESIS — single-transaction, resource-first seeding (Domain-B §5)
-- ============================================================================

-- Seeds a new cogmap atomically. Resource-first ordering: the telos resource +
-- its blocks are created BEFORE the kb_cogmaps row, so telos_resource_id NOT NULL
-- holds at insert with no deferred FK (map-regions OQ-4, superseded by spine #2).
--   block-0   = the telos statement (purpose + thin-who)
--   block-1.. = the guiding questions
-- Emits a single `cogmap_seeded` event (the genesis correlation root).
-- doc_type property 'cogmap_charter' is stamped on the telos resource.
-- Returns the new cogmap id.
CREATE FUNCTION cogmap_genesis(
    p_name            text,
    p_telos_title     text,
    p_telos_statement text,
    p_questions       text[],
    p_owner_profile   uuid,
    p_emitter_entity  uuid,
    p_origin_uri      text DEFAULT 'temper://genesis'
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_event_type   uuid;
    v_event        uuid;
    v_resource     uuid;
    v_cogmap       uuid;
    v_block        uuid;
    v_chunk        uuid;
    v_seq          int := 0;
    v_q            text;
    v_block_hash   text;
    v_hashes       text := '';
BEGIN
    SELECT id INTO v_event_type FROM kb_event_types WHERE name = 'cogmap_seeded';
    IF v_event_type IS NULL THEN
        RAISE EXCEPTION 'event_type cogmap_seeded not seeded';
    END IF;

    -- 1. genesis event (the correlation root). producing anchor unknown until the
    --    cogmap exists, so it is backfilled after the cogmap row is created.
    INSERT INTO kb_events (event_type_id, emitter_entity_id, correlation_id, metadata)
    VALUES (v_event_type, p_emitter_entity, uuid_generate_v7(),
            jsonb_build_object('genesis', 'cogmap', 'name', p_name))
    RETURNING id INTO v_event;

    -- 2. telos resource (resource-FIRST — exists before the cogmap row)
    INSERT INTO kb_resources (title, origin_uri)
    VALUES (p_telos_title, p_origin_uri)
    RETURNING id INTO v_resource;

    -- 2a. block-0 = telos statement
    INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
    VALUES (v_resource, 0, v_event, v_event) RETURNING id INTO v_block;
    INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
    VALUES (v_block, v_resource, 0, md5(p_telos_statement)) RETURNING id INTO v_chunk;
    INSERT INTO kb_chunk_content (chunk_id, content) VALUES (v_chunk, p_telos_statement);
    v_block_hash := md5(p_telos_statement);
    INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count)
    VALUES (v_block, v_block_hash, 1);
    v_hashes := v_hashes || v_block_hash;

    -- 2b. blocks 1..n = guiding questions
    FOREACH v_q IN ARRAY COALESCE(p_questions, ARRAY[]::text[]) LOOP
        v_seq := v_seq + 1;
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
        VALUES (v_resource, v_seq, v_event, v_event) RETURNING id INTO v_block;
        INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
        VALUES (v_block, v_resource, 0, md5(v_q)) RETURNING id INTO v_chunk;
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (v_chunk, v_q);
        v_block_hash := md5(v_q);
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count)
        VALUES (v_block, v_block_hash, 1);
        v_hashes := v_hashes || v_block_hash;
    END LOOP;

    -- 2c. denormalized resource body_hash = merkle over ordered block hashes
    UPDATE kb_resources SET body_hash = md5(v_hashes) WHERE id = v_resource;

    -- 3. the cogmap row (telos_resource_id NOT NULL satisfied — resource exists)
    INSERT INTO kb_cogmaps (name, telos_resource_id)
    VALUES (p_name, v_resource) RETURNING id INTO v_cogmap;

    -- 4. home the telos resource IN the cogmap (map-home; charter is the map's hub)
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                   originator_profile_id, owner_profile_id)
    VALUES (v_resource, 'kb_cogmaps', v_cogmap, p_owner_profile, p_owner_profile);

    -- 5. doc_type = cogmap_charter (demoted doctype-as-property)
    INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                              asserted_by_event_id, last_event_id)
    VALUES ('kb_resources', v_resource, 'doc_type', '"cogmap_charter"'::jsonb, v_event, v_event);

    -- 6. backfill the event's producing anchor now that the cogmap exists
    UPDATE kb_events
       SET producing_anchor_table = 'kb_cogmaps', producing_anchor_id = v_cogmap
     WHERE id = v_event;

    RETURN v_cogmap;
END;
$$;

-- ============================================================================
-- Reusable mutation mechanics (the cogmap_genesis mold): each emits its event AND projects in one
-- txn. The scenario loader / temper-cogmap lift call these; YAML is the event input source.
-- ============================================================================

-- Create a resource, home it in a cogmap, give it its content as the real nesting (blocks ⊃ chunks ⊃
-- chunk_content), optionally stamp a doc_type property. Emits one `resource_created` event (the
-- projection root). Returns the resource id.
--
-- p_blocks is the Rust-prepared content (crates/temper-next/src/content.rs `Vec<PreparedBlock>`,
-- serialized): an ordered array of
--     { "seq": int, "chunks": [ { "chunk_index": int, "content_hash": text, "content": text,
--                                 "embedding": [f32; 768] | null } ] }
-- Chunking + sha256 hashing + bge-768 embedding all happen Rust-side (borrowing temper-ingest), exactly
-- as production chunks in code and persists a chunk-set in SQL. This function ONLY persists: it iterates
-- blocks→chunks, writes kb_content_blocks / kb_chunks (with the inline embedding) / kb_chunk_content,
-- derives each block_body_hash as the sha256 merkle over its ordered chunk hashes, and the resource
-- body_hash as the sha256 merkle over the ordered block hashes. Blocks carry NO prose (β) — text lives
-- only in kb_chunk_content. A block whose prose exceeded one 510-token window arrives as >1 chunk.
CREATE FUNCTION resource_create(
    p_title text, p_origin_uri text, p_home_cogmap uuid, p_owner uuid,
    p_blocks jsonb, p_doc_type text, p_emitter uuid
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_et uuid; v_ev uuid; v_resource uuid; v_block uuid; v_chunk uuid;
    v_block_json jsonb; v_chunk_json jsonb; v_emb jsonb;
    v_block_hash text; v_chunk_hashes text; v_chunk_count int;
    v_resource_hashes text := '';
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='resource_created';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type resource_created not seeded'; END IF;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, 'kb_cogmaps', p_home_cogmap) RETURNING id INTO v_ev;
    INSERT INTO kb_resources (title, origin_uri) VALUES (p_title, p_origin_uri) RETURNING id INTO v_resource;
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
        VALUES (v_resource, 'kb_cogmaps', p_home_cogmap, p_owner, p_owner);

    -- blocks ⊃ chunks, in array order (already seq-ordered Rust-side; the block's own seq is authoritative).
    FOR v_block_json IN SELECT jsonb_array_elements(p_blocks) LOOP
        INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (v_resource, (v_block_json->>'seq')::int, v_ev, v_ev) RETURNING id INTO v_block;
        v_chunk_hashes := '';
        v_chunk_count := 0;
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_emb := v_chunk_json->'embedding';
            INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding)
                VALUES (v_block, v_resource, (v_chunk_json->>'chunk_index')::int,
                        v_chunk_json->>'content_hash',
                        CASE WHEN v_emb IS NULL OR jsonb_typeof(v_emb) = 'null'
                             THEN NULL ELSE (v_emb::text)::vector END)
                RETURNING id INTO v_chunk;
            INSERT INTO kb_chunk_content (chunk_id, content)
                VALUES (v_chunk, v_chunk_json->>'content');
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        -- block_body_hash = sha256 merkle over the ordered chunk hashes (retires md5).
        v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count)
            VALUES (v_block, v_block_hash, v_chunk_count);
        v_resource_hashes := v_resource_hashes || v_block_hash;
    END LOOP;

    -- resource body_hash = sha256 merkle over the ordered block hashes.
    UPDATE kb_resources SET body_hash = encode(sha256(convert_to(v_resource_hashes, 'UTF8')), 'hex')
        WHERE id = v_resource;
    IF p_doc_type IS NOT NULL THEN
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', v_resource, 'doc_type', to_jsonb(p_doc_type), v_ev, v_ev);
    END IF;
    RETURN v_resource;
END;
$$;

-- Assert a typed edge between two resources, homed in a cogmap. Emits `relationship_asserted`.
CREATE FUNCTION relationship_assert(
    p_src uuid, p_tgt uuid, p_kind edge_kind, p_label text, p_weight double precision,
    p_home_cogmap uuid, p_emitter uuid
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid; v_edge uuid;
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='relationship_asserted';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type relationship_asserted not seeded'; END IF;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, 'kb_cogmaps', p_home_cogmap) RETURNING id INTO v_ev;
    INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label, weight,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
        VALUES ('kb_resources', p_src, 'kb_resources', p_tgt, p_kind, p_label, p_weight,
                'kb_cogmaps', p_home_cogmap, v_ev, v_ev) RETURNING id INTO v_edge;
    RETURN v_edge;
END;
$$;

-- Set a resource's facet property as ONE coherent kb_properties row. Emits `property_asserted`.
-- The event anchors to the resource's home cogmap (producing_anchor CHECK forbids kb_resources).
CREATE FUNCTION facet_set(p_resource uuid, p_values jsonb, p_weight double precision, p_emitter uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid; v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='property_asserted';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type property_asserted not seeded'; END IF;
    -- Anchor the event to the resource's home (cogmap OR context — both are in the kb_events CHECK set),
    -- preferring a cogmap home. NEVER hardcode 'kb_cogmaps' with a possibly-NULL id: a context-homed
    -- resource would violate the (table IS NULL)=(id IS NULL) CHECK and abort the txn.
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id=p_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'facet_set: resource % has no home to anchor the property event', p_resource;
    END IF;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, v_anchor_tbl, v_anchor) RETURNING id INTO v_ev;
    INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
        VALUES ('kb_resources', p_resource, 'facet', p_values, p_weight, v_ev, v_ev);
END;
$$;

-- Create a lens (global when p_cogmap IS NULL). Emits `lens_created`; a global lens is a system event
-- with no producing anchor (both NULL — satisfies the both-null-or-both-set CHECK). Returns lens id.
CREATE FUNCTION lens_create(
    p_cogmap uuid, p_name text,
    p_w_express double precision, p_w_contains double precision, p_w_leads_to double precision,
    p_w_near double precision, p_w_prop double precision,
    p_s_telos double precision, p_s_ref double precision, p_s_central double precision,
    p_resolution double precision, p_emitter uuid
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid; v_lens uuid; v_anchor_tbl text;
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='lens_created';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type lens_created not seeded'; END IF;
    v_anchor_tbl := CASE WHEN p_cogmap IS NULL THEN NULL ELSE 'kb_cogmaps' END;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, v_anchor_tbl, p_cogmap) RETURNING id INTO v_ev;
    INSERT INTO kb_cogmap_lenses
        (cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near, w_prop,
         s_telos, s_ref, s_central, resolution, asserted_by_event_id)
    VALUES (p_cogmap, p_name, 'homed', p_w_express, p_w_contains, p_w_leads_to, p_w_near, p_w_prop,
            p_s_telos, p_s_ref, p_s_central, p_resolution, v_ev)
    RETURNING id INTO v_lens;
    RETURN v_lens;
END;
$$;

-- ============================================================================
-- End of 02_functions.sql. Seed → 03_seed.sql; scenarios → 04_scenarios.sql.
-- ============================================================================
