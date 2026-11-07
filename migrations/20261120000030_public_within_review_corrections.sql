-- public_within: corrections from review, applied on top of 20261120000000..20261120000020.
--
-- Those three migrations shipped to preview databases, so they are not edited; this one carries
-- every change to their objects. What changes, and why:
--
--   * kb_team_public_within.opened_by becomes nullable, ON DELETE SET NULL: deleting the profile that
--     opened a row keeps the row (the ledger keeps the act). The shape guard still requires it on a
--     fresh insert, judges only an insert or a change of team or anchor (so the SET NULL cascade and
--     a flag change pass), and stands aside under replay (temper.replaying), which restores rows as
--     they stand.
--   * team_is_root_or_gating: the one root-or-gating predicate, used by the shape guard at write time
--     and by public_within_row_effective at read time, so a later gating-slug change cannot make a
--     row on the gating team confer.
--   * public_within_row_effective evaluates policy first inside a CASE, so a switched-off instance
--     never walks the tree; public_within_reading reads the switch first for the same reason.
--   * contexts_public_within drops a context_is_dead probe that cannot change its answer.
--   * resources_public_within: the resources arm 5 adds. The audit queue and element_trail_node
--     derive reading from membership plus this delta instead of a second full scan.
--
-- Object-level rationale is in each COMMENT ON. Answers are unchanged while public_within_enabled is
-- false, its default, and nothing opens a row yet.

ALTER TABLE kb_team_public_within ALTER COLUMN opened_by DROP NOT NULL;
ALTER TABLE kb_team_public_within DROP CONSTRAINT kb_team_public_within_opened_by_fkey;
ALTER TABLE kb_team_public_within
    ADD CONSTRAINT kb_team_public_within_opened_by_fkey
    FOREIGN KEY (opened_by) REFERENCES kb_profiles(id) ON DELETE SET NULL;
COMMENT ON COLUMN kb_team_public_within.opened_by IS
    'Who opened the row. Required on insert (the shape guard); NULL only once that profile is '
    'deleted, when the ledger record keeps the act.';

CREATE FUNCTION team_is_root_or_gating(p_team uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_teams t
         WHERE t.id = p_team
           AND ( t.slug = 'temper-system'
              OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)));
$$;

COMMENT ON FUNCTION team_is_root_or_gating(uuid) IS
$c$The root (temper-system) or the instance's current gating team: the SQL twin of
access_service::is_root_or_gating_team. public_within's shape guard refuses such a team as T at
write time, and public_within_row_effective refuses it at read time, through this one predicate.$c$;

CREATE OR REPLACE FUNCTION kb_team_public_within_shape_guard() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    -- Replay restores rows verbatim, as they stand: a row whose opener was since deleted, or whose
    -- team has since become the gating team, is restored, not re-judged (the kb_events precedent).
    IF current_setting('temper.replaying', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'INSERT' AND NEW.opened_by IS NULL THEN
        RAISE EXCEPTION 'public_within: a row records who opened it'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_opened_by';
    END IF;
    IF EXISTS (SELECT 1 FROM kb_teams t WHERE t.id = NEW.team_id AND t.personal_of IS NOT NULL) THEN
        RAISE EXCEPTION 'public_within: the team is a personal team'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_team_personal';
    END IF;
    IF team_is_root_or_gating(NEW.team_id) THEN
        RAISE EXCEPTION 'public_within: the team is the root or the gating team'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_team_root_or_gating';
    END IF;
    IF EXISTS (SELECT 1 FROM kb_teams t WHERE t.id = NEW.anchor_id AND t.personal_of IS NOT NULL) THEN
        RAISE EXCEPTION 'public_within: the anchor is a personal team'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_anchor_personal';
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION kb_team_public_within_shape_guard() IS
$c$Refuses a public_within row whose team is personal, the root (temper-system) or the gating team,
or whose anchor is personal (spec P1, P2 rule 8, PW-13), and an insert that names no opener. The
guard runs on an insert and on a change of team or anchor, never on any other update (so the
opened_by SET NULL cascade and a later flag change pass), and not under replay (temper.replaying),
which restores rows as they stand. The constraint name on each refusal is
what Rust renders the reason from. The anchor may be the root or the gating team: that is an
admin's open (P2 rule 4), decided by the door, not here.$c$;

DROP TRIGGER kb_team_public_within_shape ON kb_team_public_within;
CREATE TRIGGER kb_team_public_within_shape
    BEFORE INSERT OR UPDATE OF team_id, anchor_id ON kb_team_public_within
    FOR EACH ROW EXECUTE FUNCTION kb_team_public_within_shape_guard();

CREATE OR REPLACE FUNCTION public_within_row_effective(p_team uuid, p_anchor uuid, p_instance_wide boolean)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN public_within_allowed_over(p_team) THEN
                p_team <> p_anchor
            AND NOT team_is_root_or_gating(p_team)
            AND EXISTS (SELECT 1 FROM team_ancestors(p_team) a WHERE a.team_id = p_anchor)
            AND (p_instance_wide OR NOT anchor_subtree_holds_pool(p_anchor))
        ELSE false
    END;
$$;

COMMENT ON FUNCTION public_within_row_effective(uuid, uuid, boolean) IS
$c$Does one public_within row confer right now (spec P5, N9)? All of:
  * policy allows it over the team (public_within_allowed_over: the switch, then no forbid above T),
    evaluated first, inside a CASE, so a switched-off instance never walks the tree;
  * the team is not the root or the gating team: the shape trigger refuses that at write time, and
    this keeps a later gating-slug change from making such a row confer;
  * the anchor is a strict ancestor of the team along a LIVE chain: team_ancestors returns T only if
    T is live and walks only through live teams, so T dead, A dead, or every path between them dead
    each makes the row dormant (liveness);
  * the anchor's live subtree holds no auto-join or gating team, unless the row is an admin's
    instance_wide row (PW-1).
Every condition is read-time, so an act that breaks one makes the row inert at once, whatever the
open-time checks said.$c$;

CREATE OR REPLACE FUNCTION contexts_public_within(p_profile uuid, p_teams uuid[])
RETURNS TABLE(context_id uuid) LANGUAGE sql STABLE AS $$
    WITH candidates AS MATERIALIZED (
        SELECT pw.team_id
          FROM kb_team_public_within pw
         WHERE pw.anchor_id = ANY(p_teams)
           AND public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
    )
    SELECT c.id
      FROM candidates cand
      JOIN kb_contexts c
        ON c.owner_table = 'kb_teams' AND c.owner_id = cand.team_id AND c.is_active;
$$;

COMMENT ON FUNCTION contexts_public_within(uuid, uuid[]) IS
$c$Arm 5 of the context read set (spec P6): the contexts OWNED by every team T whose public_within row
is anchored on a team in p_teams (the caller's reachable closure) and is effective. Owned contexts
only (R-b): never a context shared to T, a grant held by T or a map bound to T (N8). Not inherited
(R-a): T's children stay private. A retired context is floored by is_active; a context of a dead
team never appears, because public_within_row_effective requires T live.

The candidate rows are hoisted into a MATERIALIZED CTE so the per-row effect checks run once per
row, never per context (PW-19). p_profile is carried for the read-set signature. It applies NO reader
fence itself: every caller must, and both do (_contexts_readable_by_teams through p_reading,
resources_public_within through public_within_reading). Do not call it from anywhere else.$c$;

CREATE OR REPLACE FUNCTION public_within_reading(p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN COALESCE((SELECT s.public_within_enabled FROM kb_system_settings s WHERE s.id = 1), false)
        THEN EXISTS (
                 SELECT 1 FROM kb_profile_auth_links l
                  WHERE l.profile_id = p_profile
                    AND l.auth_provider <> 'auth0-m2m')
         AND NOT EXISTS (SELECT 1 FROM kb_machine_clients m WHERE m.profile_id = p_profile)
         AND NOT EXISTS (SELECT 1 FROM kb_connections k WHERE k.profile_id = p_profile)
        ELSE false
    END;
$$;

COMMENT ON FUNCTION public_within_reading(uuid) IS
$c$The machine fence (spec P6, PW-4, R-n). False whenever the instance switch is off, read first so
that every read-set call on a born-denied instance pays one settings read and nothing more. With it
on, true only for a PERSON: a profile with a human auth link
(any kb_profile_auth_links row whose provider is not the machine provider auth0-m2m) and with no
kb_machine_clients row (revoked or not; the same test as is_machine_profile) and no kb_connections
row (a connection's agent profile). Everything else is false, fail-closed: machine clients,
connection agents, stewards and auditors (machine clients themselves), and any profile with no
human auth link.

Keyed on the profile and the instance setting alone (R-n): no GUC and no request-scoped state. The fence is the principal,
never the token or the surface, so a person reads with any token that identifies them, including
the Slack mention path's act-as-the-human mint.$c$;

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

CREATE OR REPLACE FUNCTION resource_stale_citations_multi(p_findings uuid[], p_principal uuid)
RETURNS TABLE(finding_id uuid, block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH member AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_by_membership(p_principal)
    ),
    readable AS MATERIALIZED (
        SELECT resource_id FROM member
        UNION
        SELECT resource_id FROM resources_public_within(p_principal)
    ),
    gated AS (
        SELECT f.fid
          FROM unnest(p_findings) AS f(fid)
         WHERE f.fid IN (SELECT resource_id FROM member)
    )
    SELECT g.fid, lc.block_id, lc.source_id
      FROM gated g
      -- THE SOURCE GATE, applied to the citations before any arm below reads them: a citation is
      -- stale for a principal only when it can read the cited source.
      CROSS JOIN LATERAL (
          SELECT l.block_id, l.source_id
            FROM resource_live_citations(g.fid) l
           WHERE l.source_id IN (SELECT resource_id FROM readable)
      ) lc
      JOIN kb_content_blocks b  ON b.id  = lc.block_id
      JOIN kb_events         be ON be.id = b.last_event_id
      -- `ab.resource_id = g.fid` keeps each finding's watermark its own; see `20260828000020`.
      CROSS JOIN LATERAL (
          SELECT
              max(a.created)                                         AS finding_wm,
              max(a.created) FILTER (WHERE a.block_id = lc.block_id) AS block_wm
            FROM kb_citation_audits a
            JOIN kb_content_blocks ab
              ON ab.id = a.block_id AND ab.resource_id = g.fid
           WHERE a.source_kind = 'resource'
             AND a.source_id   = lc.source_id
             AND a.audited_by_profile_id = p_principal
      ) w
     WHERE w.finding_wm IS NOT NULL
       AND ( w.block_wm IS NULL
          OR be.occurred_at > w.block_wm
          OR EXISTS (SELECT 1 FROM kb_content_blocks sb
                       JOIN kb_events se ON se.id = sb.last_event_id
                      WHERE sb.resource_id = lc.source_id
                        AND se.occurred_at > w.block_wm) );
$$;

CREATE OR REPLACE FUNCTION resource_auditable_citations(p_finding uuid, p_principal uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH member AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_by_membership(p_principal)
    ),
    readable AS MATERIALIZED (
        SELECT resource_id FROM member
        UNION
        SELECT resource_id FROM resources_public_within(p_principal)
    ),
    gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM member)
    )
    SELECT lc.block_id, lc.source_id
      FROM gated g
      CROSS JOIN LATERAL resource_live_citations(g.fid) lc
     WHERE lc.source_id IN (SELECT resource_id FROM readable)
       AND NOT EXISTS (
           SELECT 1
             FROM kb_citation_audits a
            WHERE a.block_id              = lc.block_id
              AND a.source_kind           = 'resource'
              AND a.source_id             = lc.source_id
              AND a.audited_by_profile_id = p_principal)
    UNION
    -- Source-gated inside the staleness set itself.
    SELECT s.block_id, s.source_id
      FROM resource_stale_citations(p_finding, p_principal) s;
$$;

CREATE OR REPLACE FUNCTION audit_drift_sweep(p_principal uuid, p_limit int)
RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)
LANGUAGE sql STABLE AS $$
    WITH member AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_by_membership(p_principal)
    ),
    readable AS MATERIALIZED (
        SELECT resource_id FROM member
        UNION
        SELECT resource_id FROM resources_public_within(p_principal)
    ),
    candidates AS (
        SELECT h.anchor_id AS cogmap_id, r.id AS finding_id
          FROM steward_candidate_cogmaps(p_principal) m
          JOIN kb_resource_homes h
            ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = m.cogmap_id
          JOIN kb_resources r ON r.id = h.resource_id
         WHERE r.is_active
           AND r.ingest_state = 'complete'
           -- The same predicate Task 6's gate runs, so the queue cannot offer what the gate refuses.
           AND r.id IN (SELECT resource_id FROM member)
    ),
    -- MATERIALIZED IS LOAD-BEARING, NOT STYLE (`20260727000010`). `magnitude` and `coverage` here
    -- are READABLE: distinct sources this principal can read, and those of them anyone has audited at
    -- this citation. One pass over `resource_live_citations` yields both.
    scored AS MATERIALIZED (
        SELECT c.cogmap_id, c.finding_id, rc.magnitude, rc.coverage
          FROM candidates c
          CROSS JOIN LATERAL (
              SELECT count(DISTINCT lc.source_id)::int AS magnitude,
                     count(DISTINCT lc.source_id) FILTER (WHERE EXISTS (
                         SELECT 1 FROM kb_citation_audits a
                          WHERE a.block_id    = lc.block_id
                            AND a.source_kind = 'resource'
                            AND a.source_id   = lc.source_id))::int AS coverage
                FROM resource_live_citations(c.finding_id) lc
               WHERE lc.source_id IN (SELECT resource_id FROM readable)
          ) rc
    ),
    -- THE SHORT-CIRCUIT, set-wise (`20260828000020`): only findings the uncovered arm did not admit.
    stale AS (
        SELECT DISTINCT m.finding_id
          FROM resource_stale_citations_multi(
                   ARRAY(SELECT s.finding_id FROM scored s
                          WHERE s.magnitude > 0 AND s.coverage >= s.magnitude),
                   p_principal) m
    ),
    -- THE ORDERING KEY is unchanged from `20260726000010`; read its rationale there.
    ranked AS (
        SELECT s.cogmap_id, s.finding_id, s.magnitude, s.coverage,
               (s.coverage < s.magnitude) AS is_uncovered,
               row_number() OVER (PARTITION BY (s.coverage < s.magnitude)
                                  ORDER BY (s.magnitude - s.coverage) DESC, s.finding_id) AS rn
          FROM scored s
         WHERE s.magnitude > 0
           AND ( s.coverage < s.magnitude
              OR s.finding_id IN (SELECT finding_id FROM stale) )
    )
    SELECT k.cogmap_id, k.finding_id, (k.magnitude - k.coverage) AS uncovered
      FROM ranked k
     ORDER BY k.rn, k.is_uncovered DESC, (k.magnitude - k.coverage) DESC, k.finding_id
     LIMIT p_limit;
$$;

SELECT declare_migration(
    20261120000030,
    'additive',
    'public_within review corrections: kb_team_public_within.opened_by becomes nullable with ON DELETE '
    'SET NULL; new functions team_is_root_or_gating and resources_public_within; body-only CREATE OR '
    'REPLACE, signatures unchanged, of kb_team_public_within_shape_guard (and its trigger, now on '
    'INSERT or UPDATE OF team_id, anchor_id), public_within_row_effective, contexts_public_within, '
    'public_within_reading, element_trail_node and the three audit queue functions. No answer changes '
    'while public_within_enabled is false, its default; no binary reads opened_by.'
);
