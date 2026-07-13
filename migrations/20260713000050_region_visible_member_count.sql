-- D5 — "If you can't see it, you can't count it."
--
-- `kb_cogmap_regions.member_count` is a STORED column, computed at materialize time over *all*
-- members with no reader in scope, and returned verbatim by every region read. So a caller who can
-- see three of a region's twelve members is told the region has twelve — a cardinality disclosure
-- about content they have no read on.
--
-- The same row is already careful in the other direction. The label fallback (20260713000020) refuses
-- to name a region after a member the caller cannot read, and its header says why: *"surfacing that
-- resource's title as the region's label would leak it, through a read whose own gate says nothing
-- about members."* Today we decline to name the invisible, and then count it out loud.
--
-- The invariant, stated once (spec §D5,
-- docs/superpowers/specs/2026-07-13-unified-visibility-semantics-design.md):
--
--     Anchor gate for the row. Member gate for anything derived FROM members.
--     No returned value is computed over members the caller cannot see.
--
-- ── This is the outlier, not the convention ─────────────────────────────────────────────────────
--
-- Seven functions in the schema return a `member_count`. Four of them — `graph_context_containers`,
-- `graph_context_residual_counts`, `graph_context_territories`, and the label half of the region
-- reads — already count only what the caller can see, deriving the count from a `resources_visible_to`
-- join rather than a stored column. The three region reads below are the only ones that hand back a
-- number computed over everybody. This migration brings them to the convention the rest of the schema
-- already keeps.
--
-- ── The fix is a no-op for a fully-sighted caller (measured, on prod) ───────────────────────────
--
-- Differential, over all 546 live (non-folded) regions — 267 cogmap-homed, 279 context-homed:
--
--     stored member_count  vs  COUNT(active kb_resources members):  0 of 546 diverge
--     cogmap regions:  421 stored / 421 counted
--     context regions: 1102 stored / 1102 counted
--     all 5698 member rows are member_table = 'kb_resources'
--
-- So for a caller who can see everything, the count below is byte-identical to the stored one, on
-- every live region and both anchor kinds. A visible-count that changes a fully-sighted read would be
-- a bug in this fix; it does not.
--
-- And, honestly: on today's production data *no live principal is currently over-counted*. Both real
-- profiles see every member of every region whose anchor they can read (0 regions over-counted, 0
-- regions fully invisible). The leak is structural — it is what the read is *willing* to say, not
-- what it has yet said — and prod's team DAG simply has no visibility boundary running through a
-- region today. It will. This closes it before it does, and it costs a fully-sighted reader nothing.
--
-- ── Two derivations would be pure waste ─────────────────────────────────────────────────────────
--
-- The visible-member set is now needed twice per region — once to name it, once to count it. It is
-- derived ONCE: a single LATERAL over the region's members yields both the count and the
-- representative title, and `WITH vis AS MATERIALIZED` hoists the visible set out of the per-region
-- loop (measured on prod at 276 regions when the label fallback landed: 100ms → 58ms).
-- `graph_cogmap_territories` and `graph_region_territories` never got that hoist — they re-derive
-- `resources_visible_to` inside the LATERAL for every region. They get it here.
--
-- ── A region with no visible members is not returned ────────────────────────────────────────────
--
-- A region you can see nothing in is not a region you can see. Returning it as an empty husk — a
-- salience, a cohesion, a null label, a zero count — would disclose that a region exists there,
-- which is the same disclosure in a thinner coat.
--
-- ── The `cogmap` principal keeps today's behavior, EXACTLY ──────────────────────────────────────
--
-- `anchor_shape` also serves `p_principal_kind = 'cogmap'` (the map self-read), and
-- `resources_visible_to` takes a PROFILE — a cogmap id yields the empty set. Under the rule above
-- that would drop EVERY region from the map's own read, turning a leak fix into a blackout of an arm
-- no Rust caller exercises yet (it exists for the agent-invocation design).
--
-- A cogmap reading its own map is not a third party, so "count only what you can see" has no
-- purchase there — but deciding what it *should* see is a semantic call this task has no mandate to
-- make. So the cogmap arm is preserved verbatim: stored count, every region, NULL label (the label
-- fallback already degraded it that way, deliberately). The member gate applies to the profile arm,
-- which is the only arm that has a third party in it. When the agent-invocation design lights the
-- cogmap arm up, it decides what a map may count — on purpose, not by inheriting this.
--
-- Additive: no signature change, no wire change, `member_count` was already `integer`. A caller who
-- could see everything sees the same number.

CREATE OR REPLACE FUNCTION anchor_shape(
    p_anchor_table  text,
    p_anchor_id     uuid,
    p_principal_kind text,
    p_principal_id  uuid,
    p_lens          uuid DEFAULT NULL
)
RETURNS TABLE(
    region_id        uuid,
    lens_id          uuid,
    salience         double precision,
    content_cohesion double precision,
    label            text,
    member_count     integer
)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        -- Computed once, not once per region. Empty for a non-profile principal — see the cogmap arm.
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    )
    SELECT reg.id, reg.lens_id, reg.salience, reg.content_cohesion,
           COALESCE(reg.label, seen.rep_title) AS label,
           CASE
               -- The map self-read is not a third party: preserve today's stored count exactly.
               WHEN p_principal_kind = 'cogmap' THEN reg.member_count
               ELSE seen.visible_members
           END AS member_count
    FROM kb_cogmap_regions reg
    CROSS JOIN LATERAL (
        -- ONE pass over this region's members, gated on the caller's visible set. It yields both the
        -- count we report and the title we name the region after: the same visible set answers both
        -- questions, so it is derived once. Always returns exactly one row (an ungrouped aggregate),
        -- so CROSS JOIN never drops a region — the WHERE below decides that, explicitly.
        SELECT count(*)::int AS visible_members,
               (array_agg(r.title ORDER BY m.affinity DESC NULLS LAST))[1] AS rep_title
        FROM kb_cogmap_region_members m
        JOIN vis v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
    ) seen
    WHERE reg.home_anchor_table = p_anchor_table
      AND reg.home_anchor_id    = p_anchor_id
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)
      -- A region you can see nothing in is not a region you can see. (The cogmap arm is exempt: its
      -- visible set is empty by construction, and dropping every region would be a blackout, not a fix.)
      AND (p_principal_kind = 'cogmap' OR seen.visible_members > 0)
      AND (
        (p_principal_kind = 'profile'
             AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
        OR (p_principal_kind = 'cogmap'
             AND p_anchor_table = 'kb_cogmaps'
             AND p_principal_id = p_anchor_id)
      )
    ORDER BY reg.salience DESC NULLS LAST, reg.id;
$$;

COMMENT ON FUNCTION anchor_shape(text, uuid, text, uuid, uuid) IS
'Surface-tier read of an anchor''s materialized regions, for EITHER anchor kind (spec §3.7, T8). Keyed on the anchor pair (home_anchor_table, home_anchor_id), not the vestigial cogmap_id. Gate is inside the SQL (deny => zero rows, never an error). Both values derived from members honor the member gate (spec §D5): `label` falls back to the most-affine VISIBLE member''s title, and `member_count` counts ONLY visible members — a caller is never told how many resources they cannot read. A region with no visible members is not returned. The cogmap self-read arm keeps the stored count (a map reading its own map is not a third party). cogmap_shape is a wrapper over this.';

-- ── The Atlas cogmap panorama: same leak, same fix ──────────────────────────────────────────────
-- Still its own function because D1 (retiring it onto `anchor_shape`) has not landed. Fixing one read
-- and leaving the other lying is exactly the drift D1 exists to end — so both move together, and this
-- one also picks up the MATERIALIZED hoist it never had.

CREATE OR REPLACE FUNCTION graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid)
RETURNS TABLE(
    region_id    uuid,
    cogmap_id    uuid,
    label        text,
    member_count integer,
    salience     double precision,
    coherence    double precision
)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        SELECT v.resource_id FROM resources_visible_to(p_profile) v
    )
    SELECT reg.id, reg.cogmap_id,
           COALESCE(reg.label, seen.rep_title) AS label,
           seen.visible_members, reg.salience, reg.content_cohesion
    FROM kb_cogmap_regions reg
    CROSS JOIN LATERAL (
        SELECT count(*)::int AS visible_members,
               (array_agg(r.title ORDER BY m.affinity DESC NULLS LAST))[1] AS rep_title
        FROM kb_cogmap_region_members m
        JOIN vis v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
    ) seen
    WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded AND reg.lens_id = p_lens
      AND seen.visible_members > 0
      AND cogmap_readable_by_profile(p_profile, p_cogmap);
$$;

COMMENT ON FUNCTION graph_cogmap_territories(uuid, uuid, uuid) IS
'Atlas panorama read of a cogmap''s regions. Label and member_count are both member-gated (spec §D5): only members in resources_visible_to(p_profile) are named or counted, and a region with no visible members is not returned. Retires onto anchor_shape under D1.';

-- ── The team panorama: no Rust caller, same leak, fixed anyway ──────────────────────────────────
-- `graph_region_territories` has zero callers today (grep: docs only). It is left in the schema, so
-- it is left correct: a fix that lands in the two live reads and leaves an identical leak sitting in
-- a third, waiting for the first caller to wire it up, has not closed anything. It is a drop
-- candidate for D1 alongside `graph_cogmap_territories` — but dropping it is D1's call, not this
-- task's, and a dead function is cheaper to fix than to argue about.

CREATE OR REPLACE FUNCTION graph_region_territories(p_profile uuid, p_team uuid, p_lens uuid)
RETURNS TABLE(
    region_id    uuid,
    cogmap_id    uuid,
    label        text,
    member_count integer,
    salience     double precision
)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        SELECT v.resource_id FROM resources_visible_to(p_profile) v
    )
    SELECT reg.id, reg.cogmap_id,
           COALESCE(reg.label, seen.rep_title) AS label,
           seen.visible_members, reg.salience
    FROM kb_cogmap_regions reg
    JOIN kb_team_cogmaps tc ON tc.cogmap_id = reg.cogmap_id
    JOIN team_ancestors(p_team) a ON a.team_id = tc.team_id
    CROSS JOIN LATERAL (
        SELECT count(*)::int AS visible_members,
               (array_agg(r.title ORDER BY m.affinity DESC NULLS LAST))[1] AS rep_title
        FROM kb_cogmap_region_members m
        JOIN vis v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
    ) seen
    WHERE NOT reg.is_folded
      AND reg.lens_id = p_lens
      AND seen.visible_members > 0
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;

COMMENT ON FUNCTION graph_region_territories(uuid, uuid, uuid) IS
'Team-scoped panorama read of regions across a team''s cogmaps. Label and member_count are both member-gated (spec §D5). Has no Rust caller today; a drop candidate for D1.';

-- ── The metrics door must agree about which regions EXIST ───────────────────────────────────────
--
-- `anchor_region_metrics` enumerates the same regions off the same anchor with no member gate at all.
-- Left alone, it would answer for a region the shape read now refuses to show — handing back the
-- region's id, centrality and cohesion for a region the caller can see nothing in. That is a strictly
-- worse disclosure than the count we just fixed, and the fix above is what would have created it: the
-- two doors returned the same region set before this migration, and would not after.
--
-- So the drop rule is applied here too. **This is not per-caller metric recomputation** — the thing
-- the task explicitly rules out, and rightly. The stored metrics still ride through exactly as stored
-- (an aggregate over all members, reader-independent, accepted as a bounded disclosure — spec §D5,
-- "Trade-offs accepted"). All that changes is WHICH regions are enumerated: one you can see nothing
-- in is not enumerated, at either door. `cogmap_region_metrics` is a wrapper over this and inherits it.

CREATE OR REPLACE FUNCTION anchor_region_metrics(
    p_anchor_table  text,
    p_anchor_id     uuid,
    p_principal_kind text,
    p_principal_id  uuid,
    p_lens          uuid DEFAULT NULL
)
RETURNS TABLE(
    region_id          uuid,
    lens_id            uuid,
    centrality         double precision,
    content_cohesion   double precision,
    internal_tension   double precision,
    reference_standing double precision,
    telos_alignment    double precision
)
LANGUAGE sql STABLE AS $$
    WITH vis AS MATERIALIZED (
        SELECT v.resource_id FROM resources_visible_to(p_principal_id) v
    )
    SELECT reg.id, reg.lens_id, reg.centrality, reg.content_cohesion,
           reg.internal_tension, reg.reference_standing, reg.telos_alignment
    FROM kb_cogmap_regions reg
    WHERE reg.home_anchor_table = p_anchor_table
      AND reg.home_anchor_id    = p_anchor_id
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)
      -- Same rule, same words, same reason as anchor_shape: a region you can see nothing in is not a
      -- region you can see. The cogmap self-read arm is exempt for the same reason it is there.
      AND (p_principal_kind = 'cogmap' OR EXISTS (
            SELECT 1
            FROM kb_cogmap_region_members m
            JOIN vis v ON v.resource_id = m.member_id
            JOIN kb_resources r ON r.id = m.member_id AND r.is_active
            WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
      ))
      AND (
        (p_principal_kind = 'profile'
             AND anchor_readable_by_profile(p_principal_id, p_anchor_table, p_anchor_id))
        OR (p_principal_kind = 'cogmap'
             AND p_anchor_table = 'kb_cogmaps'
             AND p_principal_id = p_anchor_id)
      )
    ORDER BY reg.centrality DESC NULLS LAST, reg.id;  -- centrality IS nullable: NULLS LAST is load-bearing here
$$;

COMMENT ON FUNCTION anchor_region_metrics(text, uuid, text, uuid, uuid) IS
'Analytics-tier read of an anchor''s materialized regions, for EITHER anchor kind (spec §3.7, T8). Same gate as anchor_shape, and the same member rule (spec §D5): a region with no VISIBLE members is not enumerated, so this door cannot answer for a region the shape door hides. The metrics themselves are stored, computed over all members at materialize time, and ride through as-is — an accepted bounded disclosure (an aggregate, never an identity). cogmap_region_metrics is a wrapper over this.';
