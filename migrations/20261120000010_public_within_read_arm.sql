-- public_within, Phase 2a: the read arm, layered so that every existing name keeps its signature.
--
-- Design: temper-artifacts specs/2026-11-06-public-within-design.md P6 (the SQL sketch), P7 (the door
-- table), P8, P12. Plan: plans/2026-11-06-public-within.md Phase 2.1 and 2.4. Review: PW-7, PW-8, PW-9.
--
-- THE TWO SETS. One body defines both, switched by a boolean:
--   reading visibility    = membership visibility + arm 5 (contexts_public_within), for a PERSON;
--   membership visibility = arms 1-4, unchanged, for every act on T's material and every machine.
--
-- THE LAYERING (PW-8). A parameter added to an existing name would be a second overload, which makes
-- every two-argument call ambiguous; a DROP is shape-breaking under DEPLOYING.md. So each body moves
-- to a new `_`-prefixed name that takes `p_reading`, and the existing name becomes a body-only
-- wrapper with its signature unchanged:
--
--   _contexts_readable_by_teams(p, teams, p_reading)   arms 1-4 VERBATIM from
--                                                       20261110000100:561 (its latest body), plus
--                                                       arm 5 guarded by p_reading
--   contexts_readable_by_teams(p, teams)                wrapper: p_reading := public_within_reading(p)
--   _resources_visible(p, p_reading)                    resources_visible_to's body VERBATIM from
--                                                       20260807000010:164, its context arm calling
--                                                       _contexts_readable_by_teams(..., p_reading)
--   resources_visible_to(p)                             wrapper: p_reading := public_within_reading(p)
--   resources_visible_by_membership(p)                  wrapper: p_reading := false
--   contexts_readable_by_membership(p)                  the context-set twin, p_reading := false
--   can_read_by_membership(p, table, id)                can(read) narrowed to the membership set
--   resources_public_within(p)                          the resources arm 5 adds: reading minus
--                                                       membership, computed without either set
--
-- The reader fence is evaluated inside the wrappers, per call, so no caller can forget it: a machine,
-- a connection agent or any profile without a human auth link reads exactly what it read before.
-- With the instance switch off, public_within_reading answers false after one settings read per call,
-- and arm 5 does not run at all. Nothing else in the read path changes.
--
-- WHAT IS UNTOUCHED. profile_reachable_teams; every write, authoring and grant predicate; and every
-- caller of the existing names, which now read the reading set through the wrappers. The acts that
-- must use the membership set instead (the audit finding, grant attenuation) are moved in the next
-- migration and in Rust; endpoint checks stay on the reading set by ruling R-j.
--
-- THE ELEMENT TRAIL (PW-7). element_trail_node's latest body (20261007100000) gains one filter: for
-- a resource the caller reads only through arm 5, only the events produced in a context owned by the
-- team that owns the resource's home are shown, so a resource re-homed into T does not show the
-- history it made in the private context it came from.
--
-- Rationale lives in COMMENT ON FUNCTION, never in a new body (PW-9). The bodies copied verbatim
-- carry their original inline comments unchanged.

-- ── 1. The context read set ───────────────────────────────────────────────────────────────────────
CREATE FUNCTION _contexts_readable_by_teams(p_profile uuid, p_teams uuid[], p_reading boolean)
 RETURNS TABLE(context_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    -- 1. personal context
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
      AND c.is_active

    UNION

    -- 2. context OWNED by an enclosing team.
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_teams' AND c.owner_id = ANY(p_teams)
      AND c.is_active

    UNION

    -- 3. context SHARED to an enclosing team. context_is_dead: retired, or owned by a soft-deleted
    --    team, confers nothing through a share (soft-delete design D3).
    SELECT tc.context_id
    FROM kb_team_contexts tc
    WHERE tc.team_id = ANY(p_teams)
      AND NOT context_is_dead(tc.context_id)

    UNION

    -- 4. explicit read-grant on the context (profile-anchored, or team-anchored on a reachable team);
    --    floored by context_is_dead like arm 3.
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_contexts' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams' AND g.principal_id = ANY(p_teams)) )
      AND NOT context_is_dead(g.subject_id)

    UNION

    SELECT pw.context_id
    FROM contexts_public_within(p_profile, p_teams) pw
    WHERE p_reading;
$function$;

COMMENT ON FUNCTION _contexts_readable_by_teams(uuid, uuid[], boolean) IS
$c$THE context read-set body, taking an already-expanded team closure (p_teams) and a switch.

Arms 1-4 are membership visibility, verbatim from 20261110000100: the profile's personal contexts,
contexts owned by a team in the closure, contexts shared to one, and contexts granted to the
profile or to a team in the closure; each floored by is_active or context_is_dead.

Arm 5 is public_within (spec P6): contexts owned by a team T whose effective public_within row is
anchored on a team in the closure (contexts_public_within). It counts only when p_reading is true.
Callers do not pass p_reading themselves: contexts_readable_by_teams passes
public_within_reading(p_profile), so only a person reads through it, and the membership doors pass
false. Any future read-time filter (sensitivity, say) sits at or below this body, so that public
reach never exceeds member reach (P8).$c$;

CREATE OR REPLACE FUNCTION contexts_readable_by_teams(p_profile uuid, p_teams uuid[])
 RETURNS TABLE(context_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    SELECT r.context_id
    FROM _contexts_readable_by_teams(p_profile, p_teams, public_within_reading(p_profile)) r;
$function$;

COMMENT ON FUNCTION contexts_readable_by_teams(uuid, uuid[]) IS
$c$The context read-set for a profile, taking an already-expanded team closure: READING visibility.
A body-only wrapper over _contexts_readable_by_teams, which holds the arms, passing
public_within_reading(p_profile), so arm 5 (public_within) counts for a person and never for a
machine, a connection agent or a profile without a human auth link. Signature unchanged since
20260807000010; contexts_readable_by(uuid) still expands the closure and delegates here. A NULL or
empty p_teams reaches nothing through the team arms, and nothing through arm 5.$c$;

CREATE FUNCTION contexts_readable_by_membership(p_profile uuid)
 RETURNS TABLE(context_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    SELECT r.context_id
    FROM _contexts_readable_by_teams(
             p_profile,
             coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams(p_profile)),
                      '{}'::uuid[]),
             false
         ) r;
$function$;

COMMENT ON FUNCTION contexts_readable_by_membership(uuid) IS
$c$The context read-set by MEMBERSHIP alone: arms 1-4, never public_within. The twin of
contexts_readable_by for the doors that act on a context rather than read it (spec P7): grant
attenuation's read check (can_read_by_membership).$c$;

-- ── 2. The resource read set ──────────────────────────────────────────────────────────────────────
CREATE FUNCTION _resources_visible(p_profile uuid, p_reading boolean)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql
STABLE
AS $$
    -- MATERIALIZED is load-bearing and cheap: a handful of team ids. It makes "computed once" a
    -- property of the plan rather than a hope — this CTE now has four references (three joins plus
    -- the aggregate below), and an inlined CTE could be expanded at each, which is the very defect
    -- this migration removes, multiplied.
    WITH reachable_teams AS MATERIALIZED (
        SELECT team_id FROM profile_reachable_teams(p_profile)
    )
    SELECT v.resource_id
    FROM (
        -- owned (the home confers access to its OWNER; originator is provenance only, not access)
        SELECT h.resource_id FROM kb_resource_homes h
         WHERE h.owner_profile_id = p_profile
        UNION
        -- direct profile-anchored grant (consumer-axis ONLY -- never enters a vis(T))
        SELECT g.subject_id FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_profiles'
           AND g.principal_id = p_profile AND g.can_read
        UNION
        -- team-anchored grant on a reachable (self-or-ancestor) team
        SELECT g.subject_id FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
        UNION
        -- resources homed in a context the profile can READ.
        --
        -- THE ONLY ARM THIS MIGRATION CHANGES. It called contexts_readable_by(p_profile), which
        -- recomputed the closure this function already has; it now passes it. The aggregate is an
        -- InitPlan over the materialized CTE — one evaluation, no second walk. Every other arm
        -- above and below is carried over verbatim, which is what makes the row set trivially
        -- unchanged rather than merely believed to be.
        SELECT h.resource_id
        FROM _contexts_readable_by_teams(
                 p_profile,
                 coalesce((SELECT array_agg(team_id) FROM reachable_teams), '{}'::uuid[]),
                 p_reading
             ) rc
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = rc.context_id
        UNION
        -- cogmap membership: resources homed in a cognitive map joined to a REACHABLE team
        SELECT h.resource_id
        FROM kb_team_cogmaps tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
        UNION
        -- explicit read-grant on a COGMAP home
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN kb_access_grants g
          ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
        WHERE h.anchor_table = 'kb_cogmaps' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ) v
    -- soft-delete READ floor: a deleted resource is invisible on every axis.
    JOIN kb_resources r ON r.id = v.resource_id AND r.is_active;
$$;

COMMENT ON FUNCTION _resources_visible(uuid, boolean) IS
$c$THE consumer-axis resource read body: every resource a profile may see, on any axis. Holds
resources_visible_to's six arms verbatim from 20260807000010, expanding the team closure ONCE
(the fold site reachable_teams_one_definition_test probes) and threading it through every arm.
The context arm calls _contexts_readable_by_teams with p_reading, so p_reading = true is reading
visibility (public_within included) and false is membership visibility. Never called with a
caller-chosen p_reading: resources_visible_to passes public_within_reading(p_profile), and
resources_visible_by_membership passes false.$c$;

CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql
STABLE
AS $$
    SELECT v.resource_id
    FROM _resources_visible(p_profile, public_within_reading(p_profile)) v;
$$;

COMMENT ON FUNCTION resources_visible_to(uuid) IS
$c$THE consumer-axis read predicate: every resource a profile may see, on any axis. READING
visibility: a body-only wrapper over _resources_visible, passing public_within_reading(p_profile),
so a person also reads what an effective public_within row opens to them and a machine never does
(spec P6). Signature unchanged. Still computed per call from base tables; there is no cache.$c$;

CREATE FUNCTION resources_visible_by_membership(p_profile uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql
STABLE
AS $$
    SELECT v.resource_id
    FROM _resources_visible(p_profile, false) v;
$$;

COMMENT ON FUNCTION resources_visible_by_membership(uuid) IS
$c$MEMBERSHIP visibility: every resource a profile sees through arms 1-4 and the resource, cogmap and
grant arms, never through public_within. The set every act ON a resource is authorized against
(spec P7, the organizing rule: public reach can be read and referenced, never acted on): the audit
finding probe, in the gate and in the queue (R-m, R-o). For a machine it equals resources_visible_to.$c$;

CREATE FUNCTION can_read_by_membership(p_profile uuid, p_subject_table text, p_subject_id uuid)
RETURNS boolean
LANGUAGE sql
STABLE
AS $$
    SELECT can('kb_profiles', p_profile, 'read', p_subject_table, p_subject_id)
       AND CASE p_subject_table
               WHEN 'kb_resources' THEN EXISTS (
                   SELECT 1 FROM resources_visible_by_membership(p_profile) v
                    WHERE v.resource_id = p_subject_id)
               WHEN 'kb_contexts' THEN EXISTS (
                   SELECT 1 FROM contexts_readable_by_membership(p_profile) c
                    WHERE c.context_id = p_subject_id)
               ELSE true
           END;
$$;

COMMENT ON FUNCTION can_read_by_membership(uuid, text, uuid) IS
$c$can(read) for a profile, narrowed to MEMBERSHIP visibility. Used by grant attenuation
(access_service::authorize_capability_grant): a delegated granter may confer read only on what it
reads by membership, so a can_grant-without-can_read holder under A cannot confer read on T's
material to anyone (spec P7, PW-5/F13). public_within opens only contexts and what they home, so
other subject kinds (cogmaps) answer exactly can(read). Never wider than can(read).$c$;

CREATE FUNCTION resources_public_within(p_profile uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql
STABLE
AS $$
    SELECT h.resource_id
    FROM contexts_public_within(
             p_profile,
             coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams(p_profile)),
                      '{}'::uuid[])
         ) pw
    JOIN kb_resource_homes h ON h.anchor_table = 'kb_contexts' AND h.anchor_id = pw.context_id
    JOIN kb_resources r ON r.id = h.resource_id AND r.is_active
    WHERE public_within_reading(p_profile);
$$;

COMMENT ON FUNCTION resources_public_within(uuid) IS
$c$The resources arm 5 alone opens to a profile: those homed in a context contexts_public_within opens
to its closure, live, and only for a person (public_within_reading). Reading visibility is exactly
membership visibility UNION this set (the visibility harness holds the equality), so a caller that
needs both can derive reading from membership without a second full scan. On a switched-off instance
it is one settings read: public_within_reading is a one-time filter, so neither the closure nor
contexts_public_within runs.$c$;

-- ── 3. The element trail (PW-7) ───────────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION public.element_trail_node(p_profile uuid, p_resource uuid)
 RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamp with time zone, metadata jsonb, payload jsonb, actor_name text, correlation_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    WITH ev_ids AS (
        -- `producing_anchor_table IS NOT NULL` on every arm, not once at the end: it prunes inside
        -- the index scans rather than after the UNION.
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_resource
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
        WHERE b.resource_id = p_resource
          AND ev.producing_anchor_table IS NOT NULL
    ),
    gate AS MATERIALIZED (
        SELECT r.reads,
               CASE WHEN r.reads
                     AND EXISTS (SELECT 1 FROM resources_public_within(p_profile) a
                                  WHERE a.resource_id = p_resource)
                    THEN NOT EXISTS (
                        SELECT 1 FROM resources_visible_by_membership(p_profile) m
                         WHERE m.resource_id = p_resource)
                    ELSE false
               END AS public_only
          FROM (SELECT EXISTS (
                    SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
                ) AS reads) r
    ),
    home_team AS MATERIALIZED (
        SELECT c.owner_id AS team_id
          FROM kb_resource_homes h
          JOIN kb_contexts c
            ON h.anchor_table = 'kb_contexts' AND c.id = h.anchor_id AND c.owner_table = 'kb_teams'
         WHERE h.resource_id = p_resource
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name,
           ev.correlation_id
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE et.category = 'domain'
      AND et.name <> 'webhook_received'
      AND (SELECT g.reads FROM gate g)
      AND ( NOT (SELECT g.public_only FROM gate g)
         OR ( ev.producing_anchor_table = 'kb_contexts'
          AND EXISTS (SELECT 1
                        FROM kb_contexts pc
                        JOIN home_team ht ON pc.owner_table = 'kb_teams' AND pc.owner_id = ht.team_id
                       WHERE pc.id = ev.producing_anchor_id) ) )
    ORDER BY ev.id;
$function$;

COMMENT ON FUNCTION element_trail_node(uuid, uuid) IS
$c$The domain events about one resource, for a profile that can read it (resources_visible_to);
admin and received-webhook events never appear. For a resource the profile reads ONLY through
public_within (in the reading set, not in the membership set), the trail is filtered to the events
produced in a context owned by the team that owns the resource's home (spec P7, PW-7): a resource
re-homed into T does not show the history, the text or the actors of the private context it came
from. A member, a grantee or a machine sees the whole trail, as before. The membership set is scanned
only when the resource is one arm 5 opens to the reader (resources_public_within), so every other
trail read costs what it did.$c$;

SELECT declare_migration(
    20261120000010,
    'additive',
    'public_within Phase 2: adds _contexts_readable_by_teams(uuid,uuid[],boolean) holding arms 1-4 '
    'verbatim plus arm 5 (contexts_public_within, guarded by p_reading), _resources_visible(uuid,'
    'boolean) holding resources_visible_to''s body verbatim, and the membership entry points '
    'resources_visible_by_membership, contexts_readable_by_membership, can_read_by_membership and '
    'resources_public_within. '
    'contexts_readable_by_teams and resources_visible_to become body-only wrappers at unchanged '
    'signatures passing public_within_reading(p); no DROP and no overload. element_trail_node keeps '
    'its signature and filters an arm-5-only reader''s trail to T''s contexts. Answers change only '
    'for a person under the anchor of an effective public_within row, and no row is effective '
    'while public_within_enabled is false, its default; an older binary reads the same sets.'
);
