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

-- NEW (WS6 adjudication §2): the default personal team — a loopback self-reference
-- so a solo profile's maps read their own contexts through the SAME intersection
-- mechanics (share context → personal team; join map → personal team). No
-- visibility-model special case. Idempotent by slug: replay restores kb_teams
-- BEFORE kb_profiles, so the trigger's insert no-ops against restored rows and
-- the original team ids survive (mirrors the kb_team_members tolerance).
CREATE FUNCTION sync_personal_team()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    v_team uuid;
    v_root uuid;
BEGIN
    INSERT INTO kb_teams (slug, name)
    VALUES ('personal-' || NEW.handle, NEW.display_name || ' (personal)')
    ON CONFLICT (slug) DO NOTHING;
    SELECT id INTO v_team FROM kb_teams WHERE slug = 'personal-' || NEW.handle;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    VALUES (v_team, NEW.id, 'owner'::team_role)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
    SELECT id INTO v_root FROM kb_teams WHERE slug = 'temper-system';
    IF v_root IS NOT NULL THEN
        INSERT INTO kb_teams_parents (child_id, parent_id)
        VALUES (v_team, v_root)
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER trg_sync_personal_team
    AFTER INSERT ON kb_profiles
    FOR EACH ROW EXECUTE FUNCTION sync_personal_team();

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

-- Generic per-resource block projection (D3): a resource's non-folded blocks with assembled body
-- text, their block_role, and the provenance-attribution signal. Access-gated via resources_readable_by.
-- p_role NULL ⇒ all blocks; otherwise only blocks whose block_role property equals p_role. The
-- questions / framing / statement reads are all THIS function with a role filter — "kind" is not a
-- per-resource-type concept, but a property-filtered block read is universal (design §2, §4).
-- reinforce_count is a provenance-ATTRIBUTION accretion count (a reinforcement proxy), not a modeled
-- block-level trajectory (design §5).
-- ASSUMES single-label roles (design §3.2): exactly one non-folded block_role row per block, so the
-- role LEFT JOIN is 1:1 and reinforce_count's provenance fan-out is accurate. The weighted-multi-role
-- seam (design §3.4) would emit a row per role AND multiply reinforce_count — when that opens, pre-
-- aggregate kb_block_provenance in a subquery before the role join.
CREATE FUNCTION resource_blocks(
    p_resource uuid, p_principal_kind text, p_principal_id uuid, p_role text DEFAULT NULL
) RETURNS TABLE(seq int, block_id uuid, body_text text, role text,
                reinforce_count bigint, last_reinforced_at timestamptz) LANGUAGE sql STABLE AS $$
    SELECT b.seq, b.id, block_body_text(b.id),
           rp.property_value #>> '{}',
           count(pr.id) FILTER (WHERE NOT pr.is_corrected),
           max(pr.created) FILTER (WHERE NOT pr.is_corrected)
    FROM kb_content_blocks b
    LEFT JOIN kb_properties rp
           ON rp.owner_table = 'kb_content_blocks' AND rp.owner_id = b.id
          AND rp.property_key = 'block_role' AND NOT rp.is_folded
    LEFT JOIN kb_block_provenance pr ON pr.block_id = b.id
    WHERE b.resource_id = p_resource AND NOT b.is_folded
      AND p_resource IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
      AND (p_role IS NULL OR rp.property_value #>> '{}' = p_role)
    GROUP BY b.seq, b.id, rp.property_value
    ORDER BY b.seq;
$$;

-- The one genuinely cogmap-specific read (D3): resolve a cogmap to its telos-charter resource id (the
-- kb_cogmaps.telos_resource_id FK). Everything else is generic resource-level — resource_body_text for
-- the charter body, resource_blocks(telos, …, p_role) for questions/framing. Retires
-- cogmap_charter/cogmap_questions. (cogmap_regulation is a graph-edge read — left untouched; it may be
-- demoted when the regulation/edge-semantics deliverable lands.)
CREATE FUNCTION cogmap_telos(p_cogmap uuid)
RETURNS uuid LANGUAGE sql STABLE AS $$
    SELECT telos_resource_id FROM kb_cogmaps WHERE id = p_cogmap;
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
-- Shared chunk-row writer: one persisted chunk (kb_chunks row + its kb_chunk_content prose), used by
-- BOTH the create projector (_project_blocks) and the revise projector (_project_block_mutated) so the
-- fragile fire-vs-replay embedding CASE lives in exactly one place. p_emb is the sidecar's
-- `embedding` jsonb: SQL/JSON-null ⇒ NULL vector; a JSON string ⇒ pgvector text (replay path); a JSON
-- array ⇒ fire path. version / is_current are explicit so the create path (v1, current) and the revise
-- path (next version, current) share the writer without relying on column defaults.
CREATE FUNCTION _insert_chunk(p_chunk uuid, p_block uuid, p_resource uuid, p_chunk_index int,
                              p_version int, p_content_hash text, p_emb jsonb, p_is_current boolean,
                              p_content text, p_occurred timestamptz)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, content_hash,
                           embedding, is_current, created)
        VALUES (p_chunk, p_block, p_resource, p_chunk_index, p_version, p_content_hash,
                CASE
                    WHEN p_emb IS NULL OR jsonb_typeof(p_emb) = 'null' THEN NULL
                    WHEN jsonb_typeof(p_emb) = 'string' THEN (p_emb #>> '{}')::vector  -- replay: pgvector text
                    ELSE (p_emb::text)::vector                                          -- fire: JSON array
                END,
                p_is_current, p_occurred);
    INSERT INTO kb_chunk_content (chunk_id, content) VALUES (p_chunk, p_content);
END;
$$;

-- Shared resource body_hash recompute: sha256 merkle over each non-folded block's (sha256 of its
-- is_current chunk hashes in chunk_index order), blocks in seq order — a pure function of the resource's
-- CURRENT visible state. Both projectors call this after writing their chunks: at create every block is
-- fresh + current (so it equals the array-order concatenation), at revise it reflects the superseded
-- chunks. Replay-stable: identical visible state ⇒ identical hash, with `updated` carried from the event.
CREATE FUNCTION _recompute_resource_body_hash(p_resource uuid, p_occurred timestamptz)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_resource_hashes text;
BEGIN
    SELECT string_agg(bh, '' ORDER BY seq) INTO v_resource_hashes FROM (
        SELECT b.seq,
               encode(sha256(convert_to(string_agg(ch.content_hash, '' ORDER BY ch.chunk_index), 'UTF8')),
                      'hex') AS bh
        FROM kb_content_blocks b
        JOIN kb_chunks ch ON ch.block_id = b.id AND ch.is_current
        WHERE b.resource_id = p_resource AND NOT b.is_folded
        GROUP BY b.seq
    ) per_block;
    UPDATE kb_resources
        SET body_hash = encode(sha256(convert_to(coalesce(v_resource_hashes, ''), 'UTF8')), 'hex'),
            updated = p_occurred
        WHERE id = p_resource;
END;
$$;

-- ============================================================================
-- Shared block→chunk PROJECTOR (the content-block write path, payload-first). p_manifests is the
-- payload's BlockManifest array — pre-generated ids + seqs + roles + sha256 content hashes, NO prose
-- (the CAS rule: prose lives once in kb_chunk_content). p_content is the sidecar
--   { "<chunk_id>": { "content": text, "embedding": [f32;768] | "[...]" | null } }
-- persisted to kb_chunk_content / kb_chunks.embedding, never written to the ledger. Structural truth
-- comes ONLY from the manifests: a manifest chunk missing from the sidecar is an exception; sidecar
-- extras are ignored. Projected timestamps come from the owning event's occurred_at (replay-stable).
-- block_body_hash / resource body_hash stay DERIVED (sha256 merkles over the carried chunk hashes).
-- Blocks carry NO prose (β); a block whose prose exceeded one 510-token window arrives as >1 chunk.
-- The block_role property pair (owner_table='kb_content_blocks', property_key='block_role')
-- double-segregates roles from the resource-facet lens math (D3 guard-rail).
CREATE FUNCTION _project_blocks(p_resource uuid, p_event uuid, p_manifests jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_block uuid; v_chunk uuid;
    v_block_json jsonb; v_chunk_json jsonb; v_side jsonb;
    v_block_hash text; v_chunk_hashes text; v_chunk_count int;
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    -- blocks ⊃ chunks, in array order (already seq-ordered Rust-side; the block's own seq is authoritative).
    FOR v_block_json IN SELECT jsonb_array_elements(p_manifests) LOOP
        v_block := (v_block_json->>'block_id')::uuid;
        INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id, created)
            VALUES (v_block, p_resource, (v_block_json->>'seq')::int, p_event, p_event, v_occurred);
        IF v_block_json ? 'role' AND jsonb_typeof(v_block_json->'role') = 'string' THEN
            INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                       asserted_by_event_id, last_event_id, created)
            VALUES ('kb_content_blocks', v_block, 'block_role', v_block_json->'role',
                    p_event, p_event, v_occurred);
        END IF;
        v_chunk_hashes := '';
        v_chunk_count := 0;
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_chunk := (v_chunk_json->>'chunk_id')::uuid;
            v_side := p_content->(v_chunk_json->>'chunk_id');
            IF v_side IS NULL THEN
                RAISE EXCEPTION '_project_blocks: content sidecar missing chunk %', v_chunk;
            END IF;
            -- create path: every chunk is version 1 + current (the column defaults, stated explicitly here).
            PERFORM _insert_chunk(v_chunk, v_block, p_resource, (v_chunk_json->>'chunk_index')::int,
                                  1, v_chunk_json->>'content_hash', v_side->'embedding', true,
                                  v_side->>'content', v_occurred);
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        -- block_body_hash = sha256 merkle over the ordered chunk hashes (derived, never payload).
        v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
            VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    END LOOP;
    -- resource body_hash = merkle over current visible state (= the array-order concatenation at create).
    PERFORM _recompute_resource_body_hash(p_resource, v_occurred);
END;
$$;

-- ============================================================================
-- COGMAP GENESIS — single-transaction, resource-first seeding (Domain-B §5)
-- ============================================================================

-- Seeds a new cogmap atomically, payload-first. Identity-as-input: cogmap/resource/block/chunk ids
-- all arrive in the payload (CogmapSeeded, payloads.rs), so the producing anchor is known UP FRONT —
-- the old post-hoc backfill UPDATE on kb_events is gone (the ledger is append-only). Resource-first
-- ordering preserved: the telos resource + its blocks are created BEFORE the kb_cogmaps row, so
-- telos_resource_id NOT NULL holds at insert with no deferred FK.
-- The telos charter is real content-blocks (block-0 statement, blocks 1..n questions-with-context,
-- then framing), projected via the shared _project_blocks path with the p_content sidecar.
-- Emits a single `cogmap_seeded` event (the genesis correlation root; correlation_id = its own id).
-- doc_type property 'cogmap_charter' is stamped on the telos resource.
CREATE FUNCTION _project_cogmap_seeded(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_cogmap   uuid := (p_payload->>'cogmap_id')::uuid;
        v_resource uuid := (p_payload#>>'{telos,resource_id}')::uuid;
        v_owner    uuid := (p_payload->>'owner_profile_id')::uuid;
BEGIN
    -- telos resource FIRST (telos_resource_id NOT NULL holds at the cogmap insert)
    INSERT INTO kb_resources (id, title, origin_uri, created, updated)
        VALUES (v_resource, p_payload#>>'{telos,title}', p_payload#>>'{telos,origin_uri}',
                v_occurred, v_occurred);
    PERFORM _project_blocks(v_resource, p_event, p_payload#>'{telos,blocks}', p_content);
    INSERT INTO kb_cogmaps (id, name, telos_resource_id, created)
        VALUES (v_cogmap, p_payload->>'name', v_resource, v_occurred);
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                   originator_profile_id, owner_profile_id, created)
        VALUES (v_resource, 'kb_cogmaps', v_cogmap, v_owner, v_owner, v_occurred);
    INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                               asserted_by_event_id, last_event_id, created)
        VALUES ('kb_resources', v_resource, 'doc_type', '"cogmap_charter"'::jsonb,
                p_event, p_event, v_occurred);
END;
$$;

CREATE FUNCTION cogmap_genesis(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS TABLE(cogmap_id uuid, telos_resource_id uuid) LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('cogmap_seeded', p_emitter,
                          'kb_cogmaps', (p_payload->>'cogmap_id')::uuid, p_payload);
    PERFORM _project_cogmap_seeded(v_ev, p_payload, p_content);
    cogmap_id := (p_payload->>'cogmap_id')::uuid;
    telos_resource_id := (p_payload#>>'{telos,resource_id}')::uuid;
    RETURN NEXT;
END;
$$;

-- ============================================================================
-- Reusable mutation mechanics (the cogmap_genesis mold): each emits its event AND projects in one
-- txn. The scenario loader / temper-cogmap lift call these; YAML is the event input source.
-- ============================================================================

-- Create a resource (payload-first; ResourceCreated, payloads.rs): identity from the payload, home
-- per the payload's polymorphic anchor, content as the real nesting (blocks ⊃ chunks ⊃ chunk_content)
-- via the shared _project_blocks path with the p_content sidecar, optional doc_type property.
-- Emits one `resource_created` event (the projection root). Returns the resource id.
CREATE FUNCTION _project_resource_created(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_owner    uuid := (p_payload->>'owner_profile_id')::uuid;
BEGIN
    INSERT INTO kb_resources (id, title, origin_uri, created, updated)
        VALUES (v_resource, p_payload->>'title', p_payload->>'origin_uri', v_occurred, v_occurred);
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                   originator_profile_id, owner_profile_id, created)
        VALUES (v_resource, p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
                v_owner, v_owner, v_occurred);
    PERFORM _project_blocks(v_resource, p_event, p_payload->'blocks', p_content);
    IF p_payload->>'doc_type' IS NOT NULL THEN
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                   asserted_by_event_id, last_event_id, created)
            VALUES ('kb_resources', v_resource, 'doc_type', p_payload->'doc_type',
                    p_event, p_event, v_occurred);
    END IF;
    RETURN v_resource;
END;
$$;

CREATE FUNCTION resource_create(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_created', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload);
    RETURN _project_resource_created(v_ev, p_payload, p_content);
END;
$$;

-- ============================================================================
-- THE ONE EVENT WRITER (payload-first design §5). Every mutation function appends through here;
-- it is also the foreign-event door: an external/webhook event is _event_append with no projection
-- half. Root-event convention: correlation_id = the event's own id when no correlation is supplied
-- (computed up front — the ledger is append-only, no post-hoc UPDATE).
-- ============================================================================
CREATE FUNCTION _event_append(
    p_type_name text, p_emitter uuid, p_anchor_table text, p_anchor_id uuid,
    p_payload jsonb,
    p_references jsonb DEFAULT '[]'::jsonb,
    p_correlation uuid DEFAULT NULL,
    p_payload_version int DEFAULT 1
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid := uuid_generate_v7();
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name = p_type_name;
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type % not seeded', p_type_name; END IF;
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id,
                           producing_anchor_table, producing_anchor_id,
                           payload, "references", payload_version, correlation_id)
    VALUES (v_ev, v_et, p_emitter, p_anchor_table, p_anchor_id,
            p_payload, p_references, p_payload_version, COALESCE(p_correlation, v_ev));
    RETURN v_ev;
END;
$$;

-- ── relationship_asserted ────────────────────────────────────────────────────
-- Projection half: reads ONLY the payload (RelationshipAsserted, payloads.rs). Projected timestamps
-- come from the event's occurred_at, never now() (replay-stable by construction).
CREATE FUNCTION _project_relationship_asserted(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_edges (id, source_table, source_id, target_table, target_id,
                          edge_kind, polarity, label, weight,
                          home_anchor_table, home_anchor_id,
                          asserted_by_event_id, last_event_id, created)
    VALUES (v_edge,
            p_payload#>>'{source,table}', (p_payload#>>'{source,id}')::uuid,
            p_payload#>>'{target,table}', (p_payload#>>'{target,id}')::uuid,
            (p_payload->>'edge_kind')::edge_kind,
            COALESCE(p_payload->>'polarity', 'forward')::edge_polarity,
            p_payload->>'label',
            (p_payload->>'weight')::double precision,
            p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
            p_event, p_event, v_occurred);
    RETURN v_edge;
END;
$$;

-- Assert a typed edge, homed per the payload. Emits `relationship_asserted` + projects, one txn.
CREATE FUNCTION relationship_assert(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('relationship_asserted', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload);
    RETURN _project_relationship_asserted(v_ev, p_payload);
END;
$$;

-- ── relationship_folded ──────────────────────────────────────────────────────
-- Projection half: flips an edge's visibility (is_folded), reads ONLY the payload
-- (RelationshipFolded, payloads.rs). is_folded is the read gate every shape read honors.
CREATE FUNCTION _project_relationship_folded(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
BEGIN
    UPDATE kb_edges SET is_folded = true, last_event_id = p_event WHERE id = v_edge;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    RETURN v_edge;
END;
$$;

-- Fold a declared edge (retire the relationship). The producing anchor is an ENVELOPE concern read
-- from the edge's own home (never payload data) — the same discipline as facet_set.
CREATE FUNCTION relationship_fold(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home
        FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    v_ev := _event_append('relationship_folded', p_emitter, v_home_tbl, v_home, p_payload);
    RETURN _project_relationship_folded(v_ev, p_payload);
END;
$$;

-- ── property_asserted ────────────────────────────────────────────────────────
CREATE FUNCTION _project_property_asserted(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                               asserted_by_event_id, last_event_id, created)
    VALUES (v_prop,
            p_payload#>>'{owner,table}', (p_payload#>>'{owner,id}')::uuid,
            p_payload->>'property_key', p_payload->'value',
            (p_payload->>'weight')::double precision,
            p_event, p_event, v_occurred);
    RETURN v_prop;
END;
$$;

-- Set a property per the payload. Emits `property_asserted`. The producing anchor is an ENVELOPE
-- concern derived from the owner resource's home (cogmap OR context — both in the kb_events CHECK
-- set), preferring a cogmap home — never payload data. A homeless resource is an error.
CREATE FUNCTION facet_set(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_owner ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'facet_set: resource % has no home to anchor the property event', v_owner;
    END IF;
    v_ev := _event_append('property_asserted', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_property_asserted(v_ev, p_payload);
END;
$$;

-- ── block_mutated (content revision) ─────────────────────────────────────────
-- Projection half (BlockMutated, payloads.rs): supersede the block's current chunks, insert the new
-- revision's chunks (re-embedded inline, carried in the sidecar like resource_created), record the
-- revision, bump the block's last_event_id, recompute the resource body_hash merkle. Block-body content
-- is NOT a region-formation input (affinity is declared-only) — this moves a member's embedding, which
-- the downstream SQL readouts (centroid → content_cohesion/telos_alignment) read, without touching any
-- component's membership inputs. Resource body_hash is recomputed from CURRENT visible chunk hashes
-- (ordered by block seq then chunk_index) — a pure function of visible state, so replay reproduces it.
CREATE FUNCTION _project_block_mutated(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_block    uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid;
        v_next_ver int;
        v_chunk_json jsonb; v_chunk uuid; v_side jsonb;
        v_chunk_hashes text := ''; v_chunk_count int := 0; v_block_hash text;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION '_project_block_mutated: block % not found', v_block;
    END IF;
    -- supersede the prior revision's chunks (is_current is the chunk-currency flag; the rows stay for CAS)
    UPDATE kb_chunks SET is_current = false WHERE block_id = v_block AND is_current;
    SELECT coalesce(max(version), 0) + 1 INTO v_next_ver FROM kb_chunks WHERE block_id = v_block;
    FOR v_chunk_json IN SELECT jsonb_array_elements(p_payload->'chunks') LOOP
        v_chunk := (v_chunk_json->>'chunk_id')::uuid;
        v_side  := p_content->(v_chunk_json->>'chunk_id');
        IF v_side IS NULL THEN
            RAISE EXCEPTION '_project_block_mutated: content sidecar missing chunk %', v_chunk;
        END IF;
        -- revise path: supersedes the prior chunks, so the new run carries v_next_ver + is_current.
        PERFORM _insert_chunk(v_chunk, v_block, v_resource, (v_chunk_json->>'chunk_index')::int,
                              v_next_ver, v_chunk_json->>'content_hash', v_side->'embedding', true,
                              v_side->>'content', v_occurred);
        v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
        v_chunk_count := v_chunk_count + 1;
    END LOOP;
    v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
    INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
        VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    UPDATE kb_content_blocks SET last_event_id = p_event WHERE id = v_block;
    -- resource body_hash recomputed from current visible state (the now-superseded chunks excluded).
    PERFORM _recompute_resource_body_hash(v_resource, v_occurred);
    RETURN v_block;
END;
$$;

-- Mutate a block's content (revise its prose). The producing anchor is an ENVELOPE concern derived
-- from the block's resource home (prefer a cogmap home — same discipline as facet_set), never payload
-- data. Emits `block_mutated` + projects, one txn.
CREATE FUNCTION block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION 'block_mutate: block % not found', v_block;
    END IF;
    -- An empty chunk set would supersede the block's current chunks and insert none, silently dropping
    -- the member from its region centroid and diverging body_hash from create-path semantics (which has
    -- no empty-body block). Reject before appending an event — a revise must carry content.
    IF p_payload->'chunks' IS NULL OR jsonb_array_length(p_payload->'chunks') = 0 THEN
        RAISE EXCEPTION 'block_mutate: empty chunk set for block % (a revise with no content would drop the block)', v_block;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'block_mutate: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('block_mutated', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_block_mutated(v_ev, p_payload, p_content);
END;
$$;

-- ── lens_created ─────────────────────────────────────────────────────────────
CREATE FUNCTION _project_lens_created(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_lens uuid := (p_payload->>'lens_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_cogmap_lenses
        (id, cogmap_id, name, selection_kind,
         w_express, w_contains, w_leads_to, w_near, w_prop,
         s_telos, s_ref, s_central, resolution, asserted_by_event_id, created)
    VALUES (v_lens,
            (p_payload->>'cogmap_id')::uuid,             -- NULL for a global lens
            p_payload->>'name', p_payload->>'selection_kind',
            (p_payload#>>'{weights,express}')::double precision,
            (p_payload#>>'{weights,contains}')::double precision,
            (p_payload#>>'{weights,leads_to}')::double precision,
            (p_payload#>>'{weights,near}')::double precision,
            (p_payload#>>'{weights,prop}')::double precision,
            (p_payload#>>'{salience,telos}')::double precision,
            (p_payload#>>'{salience,ref}')::double precision,
            (p_payload#>>'{salience,central}')::double precision,
            (p_payload->>'resolution')::double precision,
            p_event, v_occurred);
    RETURN v_lens;
END;
$$;

-- Create a lens (global when payload.cogmap_id is absent/null — a system event with no producing
-- anchor, both NULL satisfying the both-null-or-both-set CHECK). Returns the lens id.
CREATE FUNCTION lens_create(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text;
BEGIN
    v_anchor_tbl := CASE WHEN p_payload->>'cogmap_id' IS NULL THEN NULL ELSE 'kb_cogmaps' END;
    v_ev := _event_append('lens_created', p_emitter, v_anchor_tbl, (p_payload->>'cogmap_id')::uuid, p_payload);
    RETURN _project_lens_created(v_ev, p_payload);
END;
$$;

-- ── region_materialized ──────────────────────────────────────────────────────
-- Region ROWS are second-order derived compute (clustering output) and stay Rust-side; the
-- projection half records only the act's bookkeeping: the materialization watermark on the cogmap.
-- Replay proof for regions = replay substrate → re-run materialize → fingerprint matches the payload.
CREATE FUNCTION _project_region_materialized(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_cogmaps SET shape_materialized_event_id = p_event
     WHERE id = (p_payload->>'cogmap_id')::uuid;
END;
$$;

CREATE FUNCTION region_materialize(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('region_materialized', p_emitter,
                          'kb_cogmaps', (p_payload->>'cogmap_id')::uuid, p_payload);
    PERFORM _project_region_materialized(v_ev, p_payload);
    RETURN v_ev;
END;
$$;

-- ============================================================================
-- End of 02_functions.sql. Seed → 03_seed.sql; scenarios → 04_scenarios.sql.
-- ============================================================================
