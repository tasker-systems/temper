-- Person erasure retires the subject's contexts with events, refuses their restore, rewrites their
-- names and slugs on the ledger, and names by count what it leaves outside the estate.
--
-- Task 01a125a1-d1a4-7509-867f-34900bd5a51c, under goal 01a04aee-81ec-7f42-a8e9-e5a0c1a3b918.
-- Rulings R7 and R8 of temper-artifacts reviews/2026-10-10-erasure-inventory.md, and the event shape
-- and ledger-copy rulings recorded on the task (2026-10-10).
--
-- Until now the act retired the estate contexts with an un-evented UPDATE (arm 13 of
-- _erasure_apply_redaction), which replay reproduced only because kb_contexts is restored verbatim.
-- It left the contexts' names and slugs as they were, in the projection and on the ledger, and
-- context_restore let any owner or maintainer of the owning team bring a retired one back,
-- a co-admin of the erased person's personal team included.
--
--   0. A guard: principal_erased gains a required key. No principal has been erased on any
--      deployment (Q5), so this raises if a principal_erased event exists.
--   1. context_erased: a new domain event, one per estate context, payload {context_id, to_name,
--      to_slug} carrying only the sentinels (`erased`, `erased-<context id>`). Its projector
--      retires the context and writes them. No from_* field, so no real name reaches the ledger.
--   2. context_retire, context_restore, context_rename and context_reassign lock the row before
--      reading it, the lock the act holds on every estate context, so a racing verb waits and then
--      finds the context erased. context_restore and context_reassign refuse an erased context
--      (SQLSTATE TE001), whoever asks, after their authorization gate.
--   3. The ledger exception learns a third authority, `principal`: the person act's own
--      principal_erased, admitted only for a tombstoned subject and only for the context events
--      (context_renamed, context_retired, context_restored) of the estate it recorded, at the
--      name and slug paths of its own allowlist, to the same sentinels the live row takes. The
--      resource acts' allowlist, classes and trail are unchanged, and no authority reaches
--      another's classes.
--   4. principal_erasure_survey_plan: names the contexts to erase and the context events to
--      rewrite (redacted_fields), and counts by carrier class the subject's ledger text outside
--      the estate (R8): titles, property values, edge labels and citation-audit reasons.
--   5. principal_erasure_execute: after the resource erasures, locks the estate contexts and reads
--      what to erase and rewrite under the lock, records redacted_fields, projects the redaction
--      rows, runs the redaction, appends context_erased per context, then rewrites the context
--      events in one statement.
--   6. _erasure_apply_redaction: arm 13 (the un-evented retirement) is gone; context_erased does it.
--   7. The principal_erased payload_schema gains the required key redacted_fields.
--
-- Bodies (4), (5) and (6) are their latest definitions (20261021100000) except for the lines named
-- above and their comments; the context verbs in (2) are theirs (20260826000120, 20260826000130)
-- except for the row lock and the refusal; the verifier functions in (3) are their latest
-- (kb_events_redaction_in_trail 20261018100040's, the rest 20261018100010's or 20261009100000's)
-- except for the `principal` arms.
--
-- Replay: the PrincipalErased arm also projects the act's redaction rows, and a ContextErased arm
-- calls _project_context_erased at the event's position. The rewritten context events replay to the
-- sentinels, so the walk no longer drives an erased context's name back from an earlier rename.

-- Section 0. The guard (Q5).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id
                WHERE et.name = 'principal_erased') THEN
        RAISE EXCEPTION '20261022100000: a principal_erased event exists; its payload has no redacted_fields and its estate contexts were retired without context_erased. Ruling Q5 (2026-10-10) assumed none exists: stop and re-rule before deploying.';
    END IF;
END;
$$;

-- Section 1. context_erased.
-- 'domain', like its siblings context_retired and context_restored: anchored to the context, in its
-- element trail. NULL payload_schema keeps it out of TYPED_EVENT_NAMES, as its siblings are.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('context_erased', NULL, 1, 'domain')
ON CONFLICT (name) DO NOTHING;

-- Pure re-apply, the _project_context_retired shape: writes the payload's sentinels and retires the
-- row. Never authorizes; only principal_erasure_execute appends the event.
CREATE FUNCTION _project_context_erased(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET is_active = false,
           name = p_payload->>'to_name',
           slug = p_payload->>'to_slug'
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_erase: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;

-- Section 2. The context verbs lock before they read; restore refuses an erased context.

CREATE OR REPLACE FUNCTION context_retire(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb,
                               p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    -- Existence + current owner + current activation, in one read. The owner drives the
    -- "administers the context" gate; the activation flag drives the no-op refusal below.
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_retire: context % not found', v_context;
    END IF;

    IF NOT v_is_active THEN
        RAISE EXCEPTION 'context_retire: context % is already retired', v_context
              USING ERRCODE = 'P0002';
    END IF;

    -- The acting principal IS the emitter; kb_entities.profile_id (NOT NULL) is the human/machine
    -- behind the actor. Authorize that profile, not the emitter entity.
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_retire: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        -- Context side: the actor must administer the owner (own it directly, or owner/maintainer
        -- on the owning team — matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_retire: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_retire: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_retire: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    v_ev := _event_append('context_retired', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_retired(v_ev, p_payload);
END;
$$;

CREATE OR REPLACE FUNCTION context_restore(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb,
                                p_invocation uuid DEFAULT NULL,
                                p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_restore: context % not found', v_context;
    END IF;


    IF v_is_active THEN
        RAISE EXCEPTION 'context_restore: context % is already active', v_context
              USING ERRCODE = 'P0002';
    END IF;

    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_restore: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_restore: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_restore: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_restore: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    -- An erased context never comes back (R7). Refused whoever asks, an instance admin included.
    -- After the authorization gate, so only a caller who administers the context learns it was
    -- erased. Keyed by the context's own context_erased event, not by its owner, so a co-admin of
    -- an erased person's personal team is refused too. SQLSTATE TE001, which
    -- map_context_write_err renders as 410.
    IF EXISTS (SELECT 1 FROM kb_events e
                 JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE t.name = 'context_erased'
                  AND e.producing_anchor_table = 'kb_contexts'
                  AND e.producing_anchor_id = v_context) THEN
        RAISE EXCEPTION 'context_restore: context % was erased', v_context
              USING ERRCODE = 'TE001';
    END IF;

    v_ev := _event_append('context_restored', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_restored(v_ev, p_payload);
END;
$$;

CREATE OR REPLACE FUNCTION context_rename(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb,
                               p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    -- Existence + current owner + current activation. The owner drives the "administers the
    -- context" gate; the activation flag drives the state refusal directly below. One read, the
    -- shape `context_retire` uses (20260826000120:124-128).
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_rename: context % not found', v_context;
    END IF;

    -- The floor this migration exists for. Before the authorization gate deliberately: whether the
    -- row is renameable at all does not depend on who is asking, and a caller who does not
    -- administer it must still get 42501 rather than learning its retirement state.
    IF NOT v_is_active THEN
        RAISE EXCEPTION 'context_rename: context % is retired', v_context
              USING ERRCODE = 'P0002';
    END IF;

    -- The acting principal IS the emitter; kb_entities.profile_id (NOT NULL) is the human/machine
    -- behind the actor. Authorize that profile, not the emitter entity.
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_rename: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        -- Context side: the actor must administer the owner (own it directly, or owner/maintainer
        -- on the owning team -- matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_rename: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_rename: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_rename: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    v_ev := _event_append('context_renamed', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_renamed(v_ev, p_payload);
END;
$$;

CREATE OR REPLACE FUNCTION context_reassign(p_payload jsonb, p_emitter uuid,
                                 p_metadata jsonb DEFAULT '{}'::jsonb,
                                 p_invocation uuid DEFAULT NULL,
                                 p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_to_table text := p_payload->>'to_owner_table';
    v_to_id uuid := (p_payload->>'to_owner_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_actor uuid;
BEGIN
    -- Existence + current owner (the owner drives the "administers the context" half of the gate).
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id INTO v_owner_table, v_owner_id
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_reassign: context % not found', v_context;
    END IF;

    -- The acting principal IS the emitter; kb_entities.profile_id (NOT NULL) is the human/machine
    -- behind the actor. Authorize that profile, not the emitter entity.
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_reassign: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        -- Target-team side: must be a team, non-gating, and the actor owner/maintainer on it.
        IF v_to_table IS DISTINCT FROM 'kb_teams' THEN
            RAISE EXCEPTION 'context_reassign: transfer target must be a team'
                  USING ERRCODE = '42501';
        END IF;
        IF EXISTS (SELECT 1 FROM kb_teams t
                     JOIN kb_system_settings s ON t.slug = s.gating_team_slug
                    WHERE t.id = v_to_id) THEN
            RAISE EXCEPTION 'context_reassign: cannot transfer into the gating team'
                  USING ERRCODE = '42501';
        END IF;
        IF NOT EXISTS (SELECT 1 FROM kb_team_members
                        WHERE team_id = v_to_id AND profile_id = v_actor
                          AND role IN ('owner', 'maintainer')) THEN
            RAISE EXCEPTION 'context_reassign: actor lacks owner/maintainer on the target team'
                  USING ERRCODE = '42501';
        END IF;

        -- Context side: the actor must administer the CURRENT owner (own it directly, or
        -- owner/maintainer on the owning team — matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_reassign: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_reassign: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_reassign: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    -- An erased context stays where the erasure left it (R7): moving it out of its erased owner's
    -- estate would take it out of the scope a later run of the act rewrites. After the gate, so
    -- only a caller who could otherwise move it learns why not. SQLSTATE TE001, rendered 410.
    IF EXISTS (SELECT 1 FROM kb_events e
                 JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE t.name = 'context_erased'
                  AND e.producing_anchor_table = 'kb_contexts'
                  AND e.producing_anchor_id = v_context) THEN
        RAISE EXCEPTION 'context_reassign: context % was erased', v_context
              USING ERRCODE = 'TE001';
    END IF;

    v_ev := _event_append('context_reassigned', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_reassigned(v_ev, p_payload);
END;
$$;

-- Section 3. The ledger exception's third authority: the person act.
ALTER TABLE kb_event_field_redactions
    DROP CONSTRAINT kb_event_field_redactions_authority_check,
    ADD CONSTRAINT kb_event_field_redactions_authority_check
        CHECK (authority IN ('erasure', 'scrub', 'principal'));

COMMENT ON COLUMN kb_event_field_redactions.authority IS
$c$The authorising event's type: `erasure` (resource_erased, whose subject must be erased), `scrub`
(resource_scrubbed, whose subject must not be) or `principal` (principal_erased, whose subject
profile must be tombstoned; context names and slugs only). Written by that type's projector. A
structural token, never a value.$c$;

-- The person act's allowlist: the paths of the context events that carry a context's name or slug.
-- Disjoint from _erasure_redact_paths by event type. A function, so widening it takes DDL.
CREATE FUNCTION _principal_erasure_redact_paths()
RETURNS TABLE(event_type text, path text, class text)
LANGUAGE sql IMMUTABLE AS $$
    VALUES
        ('context_renamed',  'from_name', 'context-name'),
        ('context_renamed',  'to_name',   'context-name'),
        ('context_renamed',  'from_slug', 'context-slug'),
        ('context_renamed',  'to_slug',   'context-slug'),
        ('context_retired',  'from_slug', 'context-slug'),
        ('context_retired',  'to_slug',   'context-slug'),
        ('context_restored', 'from_slug', 'context-slug'),
        ('context_restored', 'to_slug',   'context-slug')
$$;

-- The person act's scope: the context events of the given estate contexts that are the subject's
-- own estate (_erasure_estate_contexts), of a type its allowlist holds. The intersection ties the
-- authority to the subject: a record naming another principal's context as its estate reaches
-- nothing there. Anchored to the context and naming it in the payload, both: the slug sentinel is
-- derived from the payload's context_id, so an event whose two disagreed would be given another
-- context's sentinel. The one definition: the plan, the projector and the verifier call it.
CREATE FUNCTION _principal_erasure_context_events(p_subject uuid, p_estate uuid[])
RETURNS TABLE(event_id uuid, event_type text)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT e.id, t.name
      FROM kb_events e
      JOIN kb_event_types t ON t.id = e.event_type_id
     WHERE e.producing_anchor_table = 'kb_contexts'
       AND e.producing_anchor_id = ANY(p_estate)
       AND e.producing_anchor_id IN (SELECT _erasure_estate_contexts(p_subject))
       AND e.payload ->> 'context_id' = e.producing_anchor_id::text
       AND t.name IN (SELECT DISTINCT p.event_type FROM _principal_erasure_redact_paths() p);
$$;

-- The class a path takes: the most specific line of either allowlist. The two are disjoint by event
-- type, so a path has at most one class; the authority check is the projector's and the verifier's.
CREATE OR REPLACE FUNCTION _erasure_path_class(p_event_type text, p_path text, p_payload jsonb)
RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT r.class
      FROM (SELECT a.event_type, a.path, a.key_qualifier, a.owner_qualifier, a.class
              FROM _erasure_redact_paths() a
            UNION ALL
            SELECT p.event_type, p.path, NULL, NULL, p.class
              FROM _principal_erasure_redact_paths() p) r
     WHERE r.path = p_path
       AND (r.event_type = p_event_type OR (r.event_type IS NULL AND p_path LIKE 'metadata.%'))
       AND (r.key_qualifier IS NULL OR r.key_qualifier = p_payload ->> 'property_key')
       AND (r.owner_qualifier IS NULL OR r.owner_qualifier = p_payload #>> '{owner,table}')
     ORDER BY (r.key_qualifier IS NOT NULL) DESC, (r.owner_qualifier IS NOT NULL) DESC
     LIMIT 1;
$$;

-- The context sentinels are the live row's: name `erased`, slug `erased-<context id>`.
CREATE OR REPLACE FUNCTION _erasure_sentinel_exact(p_class text, p_event uuid, p_payload jsonb)
RETURNS jsonb
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE p_class
        WHEN 'title'           THEN to_jsonb('erased-' || (p_payload ->> 'resource_id'))
        WHEN 'origin-uri'      THEN to_jsonb('erased:' || (p_payload ->> 'resource_id'))
        WHEN 'property-value'  THEN to_jsonb('erased:' || p_event::text)
        WHEN 'doc-type'        THEN to_jsonb('erased:' || p_event::text)
        WHEN 'artifact-family' THEN to_jsonb('erased:' || p_event::text)
        WHEN 'reason'          THEN 'null'::jsonb
        WHEN 'scar'            THEN '"erased"'::jsonb
        WHEN 'authorship'      THEN '"erased"'::jsonb
        WHEN 'context-name'    THEN '"erased"'::jsonb
        WHEN 'context-slug'    THEN to_jsonb('erased-' || (p_payload ->> 'context_id'))
    END;
$$;

-- The context events of an estate whose name or slug paths still hold something other than the
-- sentinel, as redacted_fields: [{event, paths}] in event order. A path holding no string carries
-- nothing and is left. A re-run finds every path already the sentinel and names nothing.
CREATE FUNCTION _principal_erasure_redacted_fields(p_subject uuid, p_estate uuid[])
RETURNS jsonb
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', s.event_id, 'paths', s.paths)
                              ORDER BY s.event_id), '[]'::jsonb)
      FROM (SELECT ce.event_id, jsonb_agg(p.path ORDER BY p.path) AS paths
              FROM _principal_erasure_context_events(p_subject, p_estate) ce
              JOIN kb_events e ON e.id = ce.event_id
              JOIN _principal_erasure_redact_paths() p ON p.event_type = ce.event_type
             WHERE jsonb_typeof(e.payload -> p.path) = 'string'
               AND e.payload -> p.path IS DISTINCT FROM _erasure_sentinel_exact(p.class, e.id, e.payload)
             GROUP BY ce.event_id) s;
$$;

-- The estate contexts not yet erased: those without a context_erased event, in id order. The plan
-- names them and the act erases them, by this one definition.
CREATE FUNCTION _principal_erasure_contexts_to_erase(p_estate uuid[])
RETURNS uuid[]
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT ARRAY(
        SELECT g.id FROM kb_contexts g
         WHERE g.id = ANY(p_estate)
           AND NOT EXISTS (SELECT 1 FROM kb_events e
                             JOIN kb_event_types t ON t.id = e.event_type_id
                            WHERE t.name = 'context_erased'
                              AND e.producing_anchor_table = 'kb_contexts'
                              AND e.producing_anchor_id = g.id)
         ORDER BY g.id);
$$;

-- One context event's payload with the named paths set to their sentinels. The act's rewrite.
CREATE FUNCTION _principal_erasure_payload_redaction(p_event uuid, p_type text, p_payload jsonb,
                                                     p_paths jsonb)
RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_path text;
    v_out  jsonb := p_payload;
BEGIN
    FOR v_path IN SELECT jsonb_array_elements_text(p_paths) LOOP
        v_out := jsonb_set(v_out, ARRAY[v_path],
                           _erasure_sentinel_exact(_erasure_path_class(p_type, v_path, p_payload),
                                                   p_event, p_payload),
                           false);
    END LOOP;
    RETURN v_out;
END;
$$;

CREATE OR REPLACE FUNCTION _erasure_sentinel_admits(p_class text, p_event uuid, p_payload jsonb, p_at text[],
                                         p_old jsonb, p_new jsonb, p_authority text DEFAULT 'erasure')
RETURNS boolean
LANGUAGE plpgsql IMMUTABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_marks integer;
    v_scalar boolean;
BEGIN
    IF p_new IS NULL OR p_authority IS NULL OR p_authority NOT IN ('erasure', 'scrub', 'principal') THEN
        RETURN false;
    END IF;
    -- A context's name and slug are the person act's alone (20261022100000), and the person act
    -- reaches nothing else: each authority's classes are disjoint, both ways.
    IF p_authority = 'principal' THEN
        IF p_class NOT IN ('context-name', 'context-slug') THEN
            RETURN false;
        END IF;
        RETURN p_new = _erasure_sentinel_exact(p_class, p_event, p_payload);
    END IF;
    IF p_class IN ('context-name', 'context-slug') THEN
        RETURN false;
    END IF;
    IF p_authority = 'scrub'
       AND p_class NOT IN ('keep', 'title', 'origin-uri', 'property-value', 'doc-type',
                           'property-key', 'facet-value') THEN
        RETURN false;
    END IF;
    CASE p_class
        WHEN 'keep' THEN
            RETURN p_new = p_old;
        WHEN 'property-key' THEN
            IF p_authority = 'scrub' THEN
                RETURN jsonb_typeof(p_new) = 'string'
                   AND p_new #>> '{}' ~ '^scrubbed-key-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$';
            END IF;
            RETURN jsonb_typeof(p_new) = 'string' AND p_new #>> '{}' ~ '^erased-key-[1-9][0-9]*$';
        WHEN 'edge-label' THEN
            IF _erasure_label_is_kept(p_old) THEN
                RETURN p_new = p_old;
            END IF;
            RETURN jsonb_typeof(p_new) = 'string' AND p_new #>> '{}' ~ '^erased-label-[1-9][0-9]*$';
        WHEN 'remote-source-url' THEN
            IF NOT coalesce(_erasure_location_is_remote(p_payload, p_at), false) THEN
                RETURN p_new = p_old;
            END IF;
            RETURN jsonb_typeof(p_new) = 'string'
               AND p_new #>> '{}' ~ ('^erased:' || _erasure_location_block(p_payload, p_at)::text
                                     || ':[1-9][0-9]*$');
        WHEN 'facet-value' THEN
            SELECT count(*), bool_or(m.inner_key IS NULL)
              INTO v_marks, v_scalar
              FROM _facet_marks(p_old) m;
            IF v_scalar THEN
                RETURN p_new = to_jsonb('erased:' || p_event::text);
            END IF;
            RETURN jsonb_typeof(p_new) = 'object'
               AND (SELECT count(*) FROM jsonb_object_keys(p_new)) = v_marks
               AND NOT EXISTS (SELECT 1 FROM jsonb_each(p_new) kv
                                WHERE kv.key !~ CASE p_authority
                                          WHEN 'scrub'
                                          THEN '^scrubbed-facet-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}-[1-9][0-9]*$'
                                          ELSE '^erased-facet-[1-9][0-9]*$' END
                                   OR kv.value <> '"erased"'::jsonb);
        ELSE
            RETURN p_new = _erasure_sentinel_exact(p_class, p_event, p_payload);
    END CASE;
END;
$$;

CREATE OR REPLACE FUNCTION _project_field_redactions(p_event uuid, p_payload jsonb, p_authority text)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_name  text := CASE p_authority
                        WHEN 'erasure' THEN '_project_resource_erased_redactions'
                        WHEN 'scrub'   THEN '_project_resource_scrubbed_redactions'
                        WHEN 'principal' THEN '_project_principal_erased_redactions'
                    END;
    v_entry jsonb;
    v_path  text;
    v_type  text;
    v_trail jsonb;
BEGIN
    IF v_name IS NULL THEN
        RAISE EXCEPTION '_project_field_redactions: event % names no known authority %', p_event, p_authority;
    END IF;
    -- The subject's trail, once: event id → true. For the person act (`principal`), its scope:
    -- the context events of the estate it recorded, never a resource's trail.
    IF p_authority = 'principal' THEN
        SELECT coalesce(jsonb_object_agg(ce.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _principal_erasure_context_events(
                   (p_payload ->> 'subject_id')::uuid,
                   ARRAY(SELECT jsonb_array_elements_text(
                             coalesce(p_payload -> 'estate_contexts', '[]'::jsonb))::uuid)) ce
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    ELSE
        SELECT coalesce(jsonb_object_agg(ts.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _resource_erasure_trail_scope((p_payload ->> 'subject_id')::uuid) ts
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    END IF;
    FOR v_entry IN
        SELECT e FROM jsonb_array_elements(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) e
    LOOP
        SELECT et.name INTO v_type
          FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE ev.id = (v_entry ->> 'event')::uuid;
        IF v_type IS NULL THEN
            RAISE EXCEPTION '%: event % names a missing event %',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        -- The replay walk projects the record as the ledger holds it, and a D14 order inversion
        -- can put a trail event's own block or edge after the act: checking the trail there would
        -- abort the whole replay. Live, kb_events_redaction_in_trail checks it again at the
        -- rewrite, so this check is the earlier of two.
        IF NOT v_trail ? (v_entry ->> 'event')
           AND coalesce(current_setting('temper.replaying', true), '') <> 'on' THEN
            RAISE EXCEPTION '%: event % names event %, which is not in its subject''s trail',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        FOR v_path IN SELECT jsonb_array_elements_text(v_entry -> 'paths') LOOP
            -- Each authority on its own allowlist: the person act names only the context paths of
            -- _principal_erasure_redact_paths, and the resource acts never name one.
            IF NOT EXISTS (
                SELECT 1 FROM _erasure_redact_paths() r
                 WHERE p_authority <> 'principal'
                   AND r.path = v_path AND r.class <> 'keep'
                   AND (r.event_type = v_type
                        OR (r.event_type IS NULL AND v_path LIKE 'metadata.%'))
                UNION ALL
                SELECT 1 FROM _principal_erasure_redact_paths() r
                 WHERE p_authority = 'principal'
                   AND r.path = v_path AND r.event_type = v_type) THEN
                RAISE EXCEPTION '%: event % names path % of a % event, which the allowlist does not hold',
                    v_name, p_event, v_path, v_type;
            END IF;
            INSERT INTO kb_event_field_redactions (event_id, path, redacted_by, authority)
            VALUES ((v_entry ->> 'event')::uuid, v_path, p_event, p_authority);
        END LOOP;
    END LOOP;
END;
$$;


CREATE FUNCTION _project_principal_erased_redactions(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT _project_field_redactions(p_event, p_payload, 'principal');
$$;

COMMENT ON FUNCTION _project_principal_erased_redactions(uuid, jsonb) IS
$c$The principal_erased redaction projection (R7): one kb_event_field_redactions row per (event, path)
of redacted_fields, authority `principal`. The same body as the resource acts'
(_project_field_redactions): every named event is a context event of the estate the record names
(skipped while temper.replaying is on), and every path is on _principal_erasure_redact_paths for its
event's type. The act calls it after appending its record; replay calls it at the event's position.$c$;

CREATE OR REPLACE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_type   text;
    v_path   text;
    v_loc    record;
    v_class  text;
    v_meta   boolean;
    v_rows   integer := 0;
    v_new    jsonb;
    v_old_p  jsonb := OLD.payload;
    v_new_p  jsonb;
    v_old_m  jsonb := OLD.metadata;
    v_new_m  jsonb;
BEGIN
    IF TG_OP <> 'UPDATE' THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    v_new_p := NEW.payload;
    v_new_m := NEW.metadata;
    IF (to_jsonb(OLD) - 'payload' - 'metadata') IS DISTINCT FROM (to_jsonb(NEW) - 'payload' - 'metadata') THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    SELECT et.name INTO v_type FROM kb_event_types et WHERE et.id = OLD.event_type_id;
    FOR v_path IN
        SELECT DISTINCT r.path FROM kb_event_field_redactions r WHERE r.event_id = OLD.id
    LOOP
        v_rows := v_rows + 1;
        v_class := _erasure_path_class(v_type, v_path, OLD.payload);
        IF v_class IS NULL THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        v_meta := v_path LIKE 'metadata.%';
        FOR v_loc IN
            SELECT x.at, x.val
              FROM _erasure_expand(CASE WHEN v_meta THEN OLD.metadata ELSE OLD.payload END,
                                   CASE WHEN v_meta THEN substr(v_path, 10) ELSE v_path END) x
        LOOP
            v_new := (CASE WHEN v_meta THEN NEW.metadata ELSE NEW.payload END) #> v_loc.at;
            IF v_new IS DISTINCT FROM v_loc.val THEN
                IF NOT EXISTS (
                    SELECT 1
                      FROM kb_event_field_redactions r
                      JOIN kb_events e ON e.id = r.redacted_by
                      JOIN kb_event_types t ON t.id = e.event_type_id
                     WHERE r.event_id = OLD.id AND r.path = v_path
                       AND t.name = CASE r.authority
                                        WHEN 'erasure' THEN 'resource_erased'
                                        WHEN 'scrub'   THEN 'resource_scrubbed'
                                        WHEN 'principal' THEN 'principal_erased'
                                    END
                       AND e.created = now()
                       AND e.payload -> 'redacted_fields' @> jsonb_build_array(jsonb_build_object(
                               'event', OLD.id::text, 'paths', jsonb_build_array(v_path)))
                       AND CASE r.authority
                               WHEN 'principal' THEN
                                   EXISTS (SELECT 1 FROM kb_profiles s
                                            WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                              AND s.tombstoned_at IS NOT NULL)
                               ELSE
                                   EXISTS (SELECT 1 FROM kb_resources s
                                            WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                              AND CASE r.authority
                                                      WHEN 'erasure' THEN s.erased_at IS NOT NULL
                                                      WHEN 'scrub'   THEN s.erased_at IS NULL
                                                  END)
                           END
                       AND _erasure_sentinel_admits(v_class, OLD.id, OLD.payload, v_loc.at,
                                                    v_loc.val, v_new, r.authority)) THEN
                    RAISE EXCEPTION 'event ledger is append-only';
                END IF;
            END IF;
            IF v_meta THEN
                v_old_m := jsonb_set(v_old_m, v_loc.at, 'null'::jsonb, false);
                v_new_m := jsonb_set(v_new_m, v_loc.at, 'null'::jsonb, false);
            ELSE
                v_old_p := jsonb_set(v_old_p, v_loc.at, 'null'::jsonb, false);
                v_new_p := jsonb_set(v_new_p, v_loc.at, 'null'::jsonb, false);
            END IF;
        END LOOP;
    END LOOP;
    IF v_rows = 0
       OR v_old_p IS DISTINCT FROM v_new_p
       OR v_old_m IS DISTINCT FROM v_new_m THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.kb_events_redaction_in_trail()
 RETURNS trigger
 LANGUAGE plpgsql
 SET search_path = public, pg_temp
AS $function$
DECLARE
    v_subject uuid;
    v_record  uuid;
    v_payload jsonb;
    v_keys    jsonb;
    v_values  jsonb;
    v_scrubbed uuid;
    v_facets  jsonb;
BEGIN
    FOR v_subject IN
        SELECT DISTINCT (e.payload ->> 'subject_id')::uuid
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority <> 'principal'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF EXISTS (
            WITH trail AS MATERIALIZED (
                SELECT ts.event_id FROM _resource_erasure_trail_scope(v_subject) ts
            )
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority <> 'principal'
              JOIN kb_events e ON e.id = r.redacted_by
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND e.created = now()
               AND (e.payload ->> 'subject_id')::uuid = v_subject
               AND NOT EXISTS (SELECT 1 FROM trail t WHERE t.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;

    FOR v_record IN
        SELECT DISTINCT r.redacted_by
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority = 'scrub'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF v_keys IS NULL THEN
            SELECT coalesce(jsonb_object_agg(o.id::text, o.payload -> 'property_key'), '{}'::jsonb)
              INTO v_keys
              FROM old_rows o
             WHERE o.payload ? 'property_key';
            SELECT coalesce(jsonb_object_agg(o.id::text, o.payload -> 'value'), '{}'::jsonb)
              INTO v_values
              FROM old_rows o
             WHERE o.payload ? 'value';
        END IF;
        SELECT e.payload INTO v_payload FROM kb_events e WHERE e.id = v_record;
        v_scrubbed := (v_payload ->> 'subject_id')::uuid;
        IF EXISTS (
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'scrub' AND r.redacted_by = v_record
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND NOT coalesce(_field_scrub_event_is_prior(
                       v_scrubbed,
                       v_payload #>> '{field,kind}',
                       (v_payload #>> '{field,family}')::uuid,
                       n.id, r.path, v_keys), false)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        -- The exact sentinel (S7). Metadata paths need no arm: none is a scrubbed field's own path,
        -- so the prior check above has already refused any scrub row naming one. Read only while
        -- the subject is not erased: on an erased subject a scrub row authorises nothing
        -- (kb_events_append_only), so a change there stands on an erasure row, under erasure's
        -- sentinels; that is the state when one transaction scrubs R and then erases it.
        v_facets := _field_scrub_facet_inner_keys(v_scrubbed, v_keys, v_values);
        IF EXISTS (SELECT 1 FROM kb_resources s WHERE s.id = v_scrubbed AND s.erased_at IS NULL)
           AND EXISTS (
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_types t ON t.id = o.event_type_id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'scrub' AND r.redacted_by = v_record
              LEFT JOIN _field_scrub_property_events(v_scrubbed, v_keys) pe ON pe.event_id = n.id
             CROSS JOIN LATERAL _erasure_expand(o.payload, r.path) x
             WHERE n.payload IS DISTINCT FROM o.payload
               AND r.path NOT LIKE 'metadata.%'
               AND (n.payload #> x.at) IS DISTINCT FROM x.val
               AND (n.payload #> x.at) IS DISTINCT FROM _field_scrub_sentinel(
                       _erasure_path_class(t.name, r.path, o.payload), o.id, o.payload, x.val,
                       pe.handle, v_facets)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        -- A family's key text is renamed whole, in one statement. Exactness alone holds one
        -- statement to one sentinel per family, but the handle is the lowest id STILL carrying the
        -- text: a statement renaming only part of a family's prior key text would leave the rest a
        -- family with a new handle, which a later statement could rename to a second sentinel. So
        -- when this record renames any key of a family, every event of that family whose key text
        -- is prior must carry a renamed key once the statement is done. The act renames them all
        -- in its one UPDATE (see resource_field_scrub_execute's header).
        IF EXISTS (SELECT 1 FROM kb_resources s WHERE s.id = v_scrubbed AND s.erased_at IS NULL)
           AND EXISTS (
            SELECT 1
              FROM _field_scrub_property_events(v_scrubbed, v_keys) pe
              JOIN kb_events cur ON cur.id = pe.event_id
             WHERE pe.handle IN (
                       SELECT pr.handle
                         FROM new_rows n
                         JOIN old_rows o ON o.id = n.id
                         JOIN kb_event_field_redactions r
                           ON r.event_id = n.id AND r.authority = 'scrub'
                          AND r.redacted_by = v_record AND r.path = 'property_key'
                         JOIN _field_scrub_property_events(v_scrubbed, v_keys) pr
                           ON pr.event_id = n.id
                        WHERE n.payload -> 'property_key' IS DISTINCT FROM o.payload -> 'property_key')
               AND cur.payload ->> 'property_key' IS NOT DISTINCT FROM pe.key_text
               AND coalesce(_field_scrub_event_is_prior(
                       v_scrubbed,
                       v_payload #>> '{field,kind}',
                       (v_payload #>> '{field,family}')::uuid,
                       pe.event_id, 'property_key', v_keys), false)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;

    -- The person act (20261022100000): every event its rows let this statement change is a
    -- context event of the estate its record names. The resource acts' loops above never read a
    -- `principal` row, whose subject is a profile and has no resource trail.
    FOR v_record IN
        SELECT DISTINCT r.redacted_by
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority = 'principal'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        SELECT e.payload INTO v_payload FROM kb_events e WHERE e.id = v_record;
        IF EXISTS (
            WITH scope AS MATERIALIZED (
                SELECT ce.event_id
                  FROM _principal_erasure_context_events(
                           (v_payload ->> 'subject_id')::uuid,
                           ARRAY(SELECT jsonb_array_elements_text(
                                     coalesce(v_payload -> 'estate_contexts', '[]'::jsonb))::uuid)) ce
            )
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'principal' AND r.redacted_by = v_record
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND NOT EXISTS (SELECT 1 FROM scope s WHERE s.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;
    RETURN NULL;
END;
$function$;

-- Section 4. The plan.
CREATE OR REPLACE FUNCTION principal_erasure_survey_plan(p_subject uuid)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_exists     uuid;
    v_governed   uuid[] := '{}';
    v_resources  uuid[] := '{}';
    v_hashes     text[] := '{}';
    v_blob_hash  text[] := '{}';
    v_targets    jsonb  := '[]'::jsonb;
    v_strikes    jsonb  := '[]'::jsonb;
    v_row        record;
    v_rel        boolean;
    v_struck     text[] := '{}';
    v_team  record;
    v_n     integer;
    v_text_rem text;
    v_prof  kb_profiles%ROWTYPE;
    v_charters   uuid[] := '{}';
    v_resrows    jsonb  := '[]'::jsonb;
    v_disp       text;
    v_counts     jsonb  := jsonb_build_object('erase', 0, 'complete', 0, 'skip', 0, 'charter', 0);
    v_ctx_erase  uuid[] := '{}';
    v_ctx_fields jsonb  := '[]'::jsonb;
    v_trail      uuid[] := '{}';
BEGIN
    SELECT id INTO v_exists FROM kb_profiles WHERE id = p_subject;
    IF v_exists IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: subject % not found', p_subject;
    END IF;

    -- The estate contexts (R1, ruled 2026-10-10): the subject's @me contexts and their personal
    -- team's contexts, from _erasure_estate_contexts, the ONE definition. Other team contexts and
    -- map homes are NOT the estate (disposition iii). The act records this set in its event and
    -- the redaction reads it from there, so the arms no longer re-derive it.
    v_governed := ARRAY(SELECT _erasure_estate_contexts(p_subject));

    -- ── Scope: the estate, HOME-PURE (ruled 2026-09-11 with Pete — the "scope of
    -- engagement" ruling) ─────────────────────────────────────────────────────────────
    -- Everything homed in a governed context wipes with the estate, whoever authored,
    -- owns or emitted it: writing into someone's PRIVATE context under a grant declares
    -- the content's scope of engagement — it lives and dies with that estate (the
    -- terms-of-use documentation states this). The 20260909000025 actor halves are
    -- dissolved: the owner/originator OR kept reassigned-in-place authorship in reach
    -- (20260703140000 moves owner_profile_id in place), and the emitted half was already
    -- subsumed by the governed-home EXISTS it carried. Erasure keeps the resource rows
    -- live (D3), so a grantee whose prose dies with the estate holds their remaining
    -- access only against a retired context (arm 13), and their attribution dies by the
    -- pseudonym break, never by edits.
    SELECT coalesce(array_agg(DISTINCT h.resource_id), '{}') INTO v_resources
      FROM kb_resource_homes h
     WHERE h.anchor_table = 'kb_contexts'
       AND h.anchor_id = ANY(v_governed);

    -- ── Each estate resource's disposition, in resource-id order: the order the act erases them
    --    in (2c), which is also the order every person act takes their locks in.
    --      charter  — a cogmap's telos. Resource erasure refuses it; the act empties its text
    --                 by hash below and holds it until map-grain erasure (2a).
    --      erase    — live or soft-deleted: resource erasure runs on it (2b).
    --      complete — erased, and the ledger still holds paths to rewrite (an erasure from
    --                 before cut 2): resource erasure's completion pass runs (2f).
    --      skip     — erased and complete (2f). ──────────────────────────────────────────────
    FOR v_row IN
        SELECT r.id,
               r.erased_at IS NOT NULL AS erased,
               EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.telos_resource_id = r.id) AS charter
          FROM kb_resources r
         WHERE r.id = ANY(v_resources)
         ORDER BY r.id
    LOOP
        v_disp := CASE
            WHEN v_row.charter THEN 'charter'
            WHEN NOT v_row.erased THEN 'erase'
            WHEN resource_erasure_completion_fields(v_row.id) <> '[]'::jsonb THEN 'complete'
            ELSE 'skip'
        END;
        IF v_disp = 'charter' THEN
            v_charters := v_charters || v_row.id;
        END IF;
        v_resrows := v_resrows || jsonb_build_array(jsonb_build_object(
            'resource_id', v_row.id, 'disposition', v_disp));
        v_counts := jsonb_set(v_counts, ARRAY[v_disp], to_jsonb((v_counts->>v_disp)::integer + 1));
    END LOOP;

    -- The text hashes the act empties BY HASH (D5, ruled 2026-10-10): the charters' chunk and
    -- verbatim block content hashes. Every other estate resource is emptied row by row by
    -- resource erasure, which admits no hash to kb_erased_content.
    SELECT coalesce(array_agg(DISTINCT h), '{}') INTO v_hashes FROM (
        SELECT c.content_hash AS h FROM kb_chunks c
         WHERE c.resource_id = ANY(v_charters)
        UNION
        SELECT bc.content_hash AS h
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
         WHERE b.resource_id = ANY(v_charters)
    ) s;

    -- ── The blob pass: HOME-PURE (the 2026-09-12 ruling), WOULD-STRIKE structured ───────
    -- Scope = every blob row homed in a governed context, WHOEVER committed it, plus the
    -- subject's own rows (owner/originator/emitted) wherever they home — the subject's
    -- team- and map-homed rows stay the named remainder (disposition iii). Governed-home
    -- LIVE rows report would-strike entries; governed-home struck rows report
    -- already-erased; EVERY other home is the named remainder. released_would_be SIMULATES
    -- the act's own sequential refcount: the ONE blob_delete live-row predicate (live rows
    -- with the hash, this one included, <= 1) minus the same-hash rows EARLIER IN THE
    -- PLAN'S OWN STRIKE SET — execute consumes the entries in order, so those siblings are
    -- already emptied when the act reaches this row. Run at survey time WITHOUT the hash's
    -- advisory lock: a PREDICTION, honest about the moment it ran; the wrapper's
    -- strike-time verdict is authoritative in the act. The enumeration is ORDERED BY id —
    -- the act strikes in plan order and the survey must walk the SAME sequence when it
    -- simulates it, so the order cannot be left to the scan.
    FOR v_row IN
        SELECT b.id, b.content_hash, b.blob_pathname, b.content_type,
               (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed)) AS governed_home
          FROM kb_blobs b
         WHERE (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed))
            OR b.owner_profile_id = p_subject
            OR b.originator_profile_id = p_subject
            OR b.asserted_by_event_id IN (
               SELECT e.id FROM kb_events e
                 JOIN kb_entities en ON en.id = e.emitter_entity_id
                WHERE en.profile_id = p_subject)
          ORDER BY b.id
    LOOP
        IF v_row.governed_home AND v_row.content_type IS NOT NULL THEN
            -- The act's own SEQUENTIAL refcount, simulated: the strike loop consumes the
            -- plan's rows by id IN ORDER and each blob_delete counts live rows at ITS
            -- moment, so every same-hash row already in this plan's strike set is emptied
            -- when the act reaches this row — subtract it (accumulated below, in plan
            -- order) from the ONE live-row predicate. Two same-hash rows in the estate
            -- therefore predict released=false then released=true, exactly as the act
            -- strikes them.
            v_rel := ((SELECT count(*) FROM kb_blobs live
                        WHERE live.content_hash = v_row.content_hash
                          AND live.content_type IS NOT NULL)
                      - (SELECT count(*) FROM unnest(v_struck) prior
                          WHERE prior = v_row.content_hash)) <= 1;
            v_struck := v_struck || ARRAY[v_row.content_hash];
            v_blob_hash := v_blob_hash || ARRAY[v_row.content_hash];
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'would_strike', jsonb_build_object(
                    'blob_id',           v_row.id,
                    'content_hash',      v_row.content_hash,
                    'pathname',          v_row.blob_pathname,
                    'released_would_be', v_rel)));
            v_strikes := v_strikes || jsonb_build_array(jsonb_build_object(
                'blob_id',           v_row.id,
                'released_would_be', v_rel));
        ELSIF v_row.governed_home THEN
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'already-erased'));
        ELSE
            -- The named remainder: independent_obligation-shaped, hash named (D2 vocabulary),
            -- never admitted to the erased-content set.
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'independent_obligation: home governed by a team or map; '
                           || 'hash ' || v_row.content_hash || ' not struck'));
        END IF;
    END LOOP;

    v_hashes := v_hashes || v_blob_hash;

    -- ── Per-target outcomes, read against the PRE-redaction state (the mutations live in
    -- _erasure_apply_redaction alone; these reads only report what it will find).
    -- 20260911000000: every read below scopes to GOVERNED homes with the same predicate
    -- the redaction spells — the record reports "erased" only for rows the act actually
    -- redacts; a same-hash row in an ungoverned home is nobody's outcome (it is not the
    -- subject's data and was never a target). ────────────────────────────────────────────
    SELECT * INTO v_prof FROM kb_profiles WHERE id = p_subject;
    IF v_prof.tombstoned_at IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','already-erased'));
    ELSE
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','tombstoned'));
        IF v_prof.email IS NOT NULL THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.email','outcome','erased'));
        END IF;
        IF v_prof.preferences <> '{}'::jsonb THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.preferences','outcome','erased'));
        END IF;
    END IF;

    SELECT * INTO v_team FROM kb_teams t WHERE t.personal_of = p_subject;
    IF FOUND THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_teams.slug',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END),
            jsonb_build_object('target','kb_teams.name',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END));
    END IF;

    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
      JOIN kb_slack_grant_vault v ON l.profile_id = p_subject
       AND l.auth_provider = 'slack' AND v.slack_principal_id = l.auth_provider_user_id;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_grant_vault.slack_principal_id',
                'outcome','deleted'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
      JOIN kb_slack_link_intents i ON l.profile_id = p_subject
       AND l.auth_provider = 'slack' AND i.slack_principal_id = l.auth_provider_user_id;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_link_intents.slack_principal_id',
                'outcome','deleted'));
    END IF;

    -- The auth-link identifiers, the entity names, and the subject's own artifact content —
    -- outcomes read against the PRE-redaction state exactly like every arm above (the
    -- mutations live in _erasure_apply_redaction alone; these reads only report what it
    -- will find). The artifact remainder subset is named with the disposition-iii
    -- vocabulary the blob arm set: another principal's kind namespace on a governed
    -- resource is an independent obligation, re-named on every run because it stands.
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject AND l.email IS NOT NULL;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.email',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject
       AND l.auth_provider_user_id IS DISTINCT FROM 'erased-' || l.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.auth_provider_user_id',
                'outcome','sentinel-scrubbed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_entities en
     WHERE en.profile_id = p_subject
       AND en.name IS DISTINCT FROM 'erased-' || en.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_entities.name',
                'outcome','sentinel-scrubbed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE da.kind_owner_table = 'kb_profiles'
       AND da.kind_owner_id = p_subject
       AND dac.content <> '{}'::jsonb
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE dac.content <> '{}'::jsonb
       AND (da.kind_owner_table <> 'kb_profiles' OR da.kind_owner_id <> p_subject)
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content (other-kind)',
                'outcome','independent_obligation: the artifact''s kind is owned by '
                           || 'another principal; content on a governed resource not '
                           || 'struck'));
    END IF;

    IF EXISTS (SELECT 1 FROM kb_chunk_content cc
                 JOIN kb_chunks c ON c.id = cc.chunk_id
                WHERE c.content_hash = ANY(v_hashes)
                  AND c.resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunk_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunk_content cc
                                               JOIN kb_chunks c ON c.id = cc.chunk_id
                                              WHERE c.content_hash = ANY(v_hashes)
                                                AND cc.content <> ''
                                                AND c.resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_block_content
                WHERE content_hash = ANY(v_hashes)
                  AND block_revision_id IN (
                      SELECT br.id FROM kb_block_revisions br
                        JOIN kb_content_blocks b ON b.id = br.block_id
                       WHERE b.resource_id IN (
                             SELECT h.resource_id FROM kb_resource_homes h
                              WHERE h.anchor_table = 'kb_contexts'
                                AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_block_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_block_content
                                              WHERE content_hash = ANY(v_hashes)
                                                AND content <> ''
                                                AND block_revision_id IN (
                                                    SELECT br.id FROM kb_block_revisions br
                                                      JOIN kb_content_blocks b ON b.id = br.block_id
                                                     WHERE b.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed))))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    -- The text remainder (the attribution ruling, 2026-09-13, on the 2026-09-06 clause):
    -- prose ATTRIBUTED to the subject by authorship — content blocks whose genesis event
    -- the subject's entity emitted — in homes outside the governed estate (team and map
    -- alike). Named for audit with count + hashes (capped at 8, "and N more" beyond),
    -- never struck, never admitted to v_hashes: nothing in a shared home is the act's to
    -- strike, and the redaction's reads stay governed-scoped. Attribution is the emitting
    -- entity, the blob arm's own emitted-half shape; direct authorship only — carried
    -- attribution is provenance the pseudonym break already covers.
    v_text_rem := (
        SELECT 'independent_obligation: home governed by a team or map; '
               || count(DISTINCT bc.content_hash) || ' text hash(s) not struck'
               || CASE WHEN count(DISTINCT bc.content_hash) <= 8
                       THEN ': ' || array_to_string(array_agg(DISTINCT bc.content_hash), ', ')
                       ELSE ': ' || array_to_string(ARRAY(
                                   SELECT DISTINCT bc2.content_hash
                                     FROM kb_block_content bc2
                                     JOIN kb_block_revisions br2 ON br2.id = bc2.block_revision_id
                                     JOIN kb_content_blocks b2 ON b2.id = br2.block_id
                                     JOIN kb_events ge2 ON ge2.id = b2.genesis_event_id
                                     JOIN kb_entities ge2_en ON ge2_en.id = ge2.emitter_entity_id
                                    WHERE ge2_en.profile_id = p_subject
                                      AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg2
                                                       WHERE hg2.resource_id = b2.resource_id
                                                         AND hg2.anchor_table = 'kb_contexts'
                                                         AND hg2.anchor_id = ANY(v_governed))
                                    ORDER BY 1 LIMIT 8), ', ')
                            || '; and ' || (count(DISTINCT bc.content_hash) - 8) || ' more'
                  END
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
          JOIN kb_events ge ON ge.id = b.genesis_event_id
          JOIN kb_entities ge_en ON ge_en.id = ge.emitter_entity_id
         WHERE ge_en.profile_id = p_subject
           AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg
                            WHERE hg.resource_id = b.resource_id
                              AND hg.anchor_table = 'kb_contexts'
                              AND hg.anchor_id = ANY(v_governed))
        HAVING count(DISTINCT bc.content_hash) > 0);
    IF v_text_rem IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_block_content.content',
            'outcome', v_text_rem));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.embedding',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END),
            jsonb_build_object('target','kb_chunks.embedded_with',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    -- The heading trail, arm (4a) of the redaction (20261020100000). Claimed only when the act
    -- will null one: a chunk with no heading trail never held one, so "already-erased" would
    -- claim an act that never happened. A re-run of the act therefore omits the target.
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND header_path IS NOT NULL
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.header_path', 'outcome','erased'));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_resource_search_index si
                WHERE si.resource_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                          WHERE c.content_hash = ANY(v_hashes)
                                            AND c.resource_id IN (
                                                SELECT h.resource_id FROM kb_resource_homes h
                                                 WHERE h.anchor_table = 'kb_contexts'
                                                   AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_resource_search_index.search_vector',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_resource_search_index si
                                              WHERE si.resource_id IN (
                                                    SELECT DISTINCT c.resource_id FROM kb_chunks c
                                                     WHERE c.content_hash = ANY(v_hashes)
                                                       AND c.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed)))
                                              AND si.search_vector <> '')
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    SELECT count(*) INTO v_n FROM kb_cogmaps m
     WHERE m.shape_materialized_event_id IS NOT NULL
       AND m.id IN (
           SELECT r.home_anchor_id FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources'
              AND mem.member_id = ANY(v_resources));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_cogmaps.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;
    -- The estate contexts the act erases (R7, ruled 2026-10-10): every one without a
    -- context_erased event yet. Each gets one, which retires it and replaces its name and slug with
    -- the sentinels; a re-run finds none left. The context rows' earlier names and slugs on the
    -- ledger (context_renamed, context_retired, context_restored) are rewritten to the same
    -- sentinels under the act's own authority: redacted_fields, paths only.
    v_ctx_erase := _principal_erasure_contexts_to_erase(v_governed);
    IF cardinality(v_ctx_erase) > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.is_active','outcome','retired'),
            jsonb_build_object('target','kb_contexts.name',     'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_contexts.slug',     'outcome','sentinel-scrubbed'));
    END IF;
    v_ctx_fields := _principal_erasure_redacted_fields(p_subject, v_governed);
    IF jsonb_array_length(v_ctx_fields) > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_events.payload',
            'outcome', jsonb_array_length(v_ctx_fields)
                       || ' context event(s) naming an estate context; names and slugs sentinel-scrubbed'));
    END IF;

    -- What the subject wrote outside the estate (R8, disposition iii): it stays, and the record
    -- names it by count per carrier class, never by content. A carrier is one of the subject's own
    -- ledger events (its emitting entity is theirs) still holding the text: outside the trail of
    -- every estate resource, which resource erasure rewrites, and with no redaction row at the
    -- path, which an earlier erasure or scrub already rewrote.
    v_trail := ARRAY(SELECT DISTINCT ts.event_id
                       FROM unnest(v_resources) AS er(id),
                            LATERAL _resource_erasure_trail_scope(er.id) ts);
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_events.payload (' || c.cls || ')',
                   'outcome', 'independent_obligation: authored by the subject outside the estate; '
                              || c.n || ' event(s) kept')
                   ORDER BY c.cls)
          FROM (SELECT s.cls, count(*) AS n
                  FROM (SELECT CASE
                                 WHEN t.name IN ('resource_created', 'resource_updated')
                                      AND jsonb_typeof(e.payload -> 'title') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'title')
                                   THEN 'titles'
                                 -- A doc_type value is a type name, not authored text.
                                 WHEN t.name IN ('property_set', 'property_asserted')
                                      AND e.payload ? 'value'
                                      AND e.payload ->> 'property_key' IS DISTINCT FROM 'doc_type'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'value')
                                   THEN 'property values'
                                 WHEN t.name = 'relationship_asserted'
                                      AND NOT _erasure_label_is_kept(e.payload -> 'label')
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'label')
                                   THEN 'edge labels'
                                 WHEN t.name = 'citation_audited'
                                      AND jsonb_typeof(e.payload -> 'reason') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'reason')
                                   THEN 'citation-audit reasons'
                                 -- A rename the subject made of a context outside the estate: the
                                 -- estate's own are rewritten by this act.
                                 WHEN t.name = 'context_renamed'
                                      AND NOT (e.producing_anchor_id = ANY(v_governed))
                                      AND jsonb_typeof(e.payload -> 'to_name') = 'string'
                                      AND NOT EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                                       WHERE r.event_id = e.id AND r.path = 'to_name')
                                   THEN 'context names'
                               END AS cls
                          FROM kb_events e
                          JOIN kb_event_types t ON t.id = e.event_type_id
                          JOIN kb_entities en ON en.id = e.emitter_entity_id
                         WHERE en.profile_id = p_subject
                           AND t.name IN ('resource_created', 'resource_updated', 'property_set',
                                          'property_asserted', 'relationship_asserted',
                                          'citation_audited', 'context_renamed')
                           AND NOT (e.id = ANY(v_trail))) s
                 WHERE s.cls IS NOT NULL
                 GROUP BY s.cls) c), '[]'::jsonb);
    SELECT count(*) INTO v_n FROM kb_contexts c
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND ( c.id IN (
               SELECT r.home_anchor_id FROM kb_cogmap_regions r
                 JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                WHERE r.home_anchor_table = 'kb_contexts' AND NOT r.is_folded
                  AND mem.member_table = 'kb_resources'
                  AND mem.member_id = ANY(v_resources))
          -- Every estate context (R1).
          OR c.id = ANY(v_governed) );
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;

    -- The charters held (2a), and the resources already complete (2f), named in the record.
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_resources',
                   'outcome', 'charter ' || ch || '; held until map-grain erasure (task '
                              || '01a0e960-0ca2-7f42-b33e-1ed19b024e6b); text emptied; title, '
                              || 'properties, edges and ledger kept') ORDER BY ch)
          FROM unnest(v_charters) AS ch), '[]'::jsonb);
    IF (v_counts->>'skip')::integer > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources',
            'outcome', (v_counts->>'skip') || ' estate resource(s) already erased and complete; skipped'));
    END IF;

    RETURN jsonb_build_object(
        'estate_contexts', (SELECT coalesce(jsonb_agg(g ORDER BY g), '[]'::jsonb) FROM unnest(v_governed) AS g),
        'estate',          v_counts || jsonb_build_object('total', jsonb_array_length(v_resrows)),
        'resources',       v_resrows,
        'charters',        (SELECT coalesce(jsonb_agg(ch ORDER BY ch), '[]'::jsonb) FROM unnest(v_charters) AS ch),
        'redacted_hashes', to_jsonb(v_hashes),
        'targets',         v_targets,
        'already_erased',  (v_prof.tombstoned_at IS NOT NULL),
        'blob_strikes',    v_strikes,
        'contexts_to_erase', (SELECT coalesce(jsonb_agg(g ORDER BY g), '[]'::jsonb) FROM unnest(v_ctx_erase) AS g),
        'redacted_fields',   v_ctx_fields);
END;
$$

;

-- Section 5. The act.
CREATE OR REPLACE FUNCTION principal_erasure_execute(p_subject uuid, p_operator uuid, p_emitter uuid, p_request_ref uuid)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_plan     jsonb;
    v_targets  jsonb;
    v_hashes   text[] := '{}';
    v_row      jsonb;
    v_bid uuid; v_rel boolean; v_path text;
    v_i        integer;
    v_ev       uuid;
    v_out      jsonb;
    v_erasures jsonb := '[]'::jsonb;
    v_refs     jsonb;
    v_estate   uuid[];
    v_rid      uuid;
    v_erased   boolean;
    v_homed    boolean;
    v_raced    integer := 0;
    v_payload  jsonb;
    v_cid      uuid;
    v_cpayload jsonb;
    v_cev      uuid;
    v_ctx_erase  uuid[];
    v_ctx_fields jsonb;
BEGIN
    -- The request reference is the correlation id of every event the act appends: the resource
    -- erasures' events, the strikes and the completion. Replay finds the act's span by it (D7).
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: p_request_ref is required';
    END IF;


    -- ONE computation per act. The existence RAISE lives in the plan (its message keeps the
    -- act's voice); the Rust gate resolves existence before either door is reached.
    v_plan := principal_erasure_survey_plan(p_subject);
    v_targets := v_plan->'targets';
    -- The plan's hash array comes back out IN ORDER (ORDINALITY makes the order load-bearing,
    -- not incidental): it is the redacted set the event carries.
    v_hashes := (SELECT coalesce(array_agg(h ORDER BY ord), '{}')
                   FROM jsonb_array_elements_text(v_plan->'redacted_hashes')
                        WITH ORDINALITY AS t(h, ord));

    -- ── The resource erasures (R5, 2c): every `erase` and `complete` resource, in the plan's
    --    resource-id order, through resource_erasure_execute itself, so each is that act whole:
    --    its edge folds, its resource_erased record, its body and its ledger rewrite. No blob list
    --    is passed: the strike loop below strikes every live blob homed in the estate. Each takes
    --    its own resource's locks as it reaches it, in id order, so two person acts cannot
    --    deadlock on them. A raise from any of them (a raced fold or remote source) aborts the
    --    whole act; the service retries it.
    --
    --    The plan was read before any of these locks, so each resource is decided again under its
    --    own: the act queue, then the row, the order resource_erasure_execute itself takes them
    --    (both are re-entrant in this transaction). A resource another act erased completely
    --    since the plan, or one moved out of the estate since, is skipped and counted; the kind
    --    recorded is the state found here, not the plan's prediction. ─────────────────────────
    v_estate := ARRAY(SELECT jsonb_array_elements_text(v_plan->'estate_contexts')::uuid);
    FOR v_row IN
        SELECT r FROM jsonb_array_elements(v_plan->'resources') r
         WHERE r->>'disposition' IN ('erase', 'complete')
    LOOP
        v_rid := (v_row->>'resource_id')::uuid;
        PERFORM pg_advisory_xact_lock(_resource_act_queue_key(v_rid));
        SELECT r.erased_at IS NOT NULL,
               EXISTS (SELECT 1 FROM kb_resource_homes h
                        WHERE h.resource_id = r.id AND h.anchor_table = 'kb_contexts'
                          AND h.anchor_id = ANY(v_estate))
          INTO v_erased, v_homed
          FROM kb_resources r WHERE r.id = v_rid
           FOR UPDATE;
        IF NOT coalesce(v_homed, false)
           OR (v_erased AND resource_erasure_completion_fields(v_rid) = '[]'::jsonb) THEN
            v_raced := v_raced + 1;
            CONTINUE;
        END IF;
        v_out := resource_erasure_execute(v_rid, p_operator, p_emitter, p_request_ref, '{}'::uuid[]);
        -- Keyed `resource` and `event`, never `resource_id`: the completion payload carries no
        -- key a trail function joins on (the D2 shape), so no resource's trail ever reaches it.
        v_erasures := v_erasures || jsonb_build_array(jsonb_build_object(
            'resource', v_rid,
            'event',    v_out->>'event_id',
            'kind',     CASE WHEN v_erased THEN 'completion' ELSE 'erasure' END));
    END LOOP;
    IF v_raced > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources',
            'outcome', v_raced || ' estate resource(s) erased or moved by another act after the plan; skipped'));
    END IF;

    -- ── The strike loop: the plan's would-strike rows, IN ORDER, through the wrapper —
    -- per-row blob_delete('blob_erased', …) events exactly as before (the pairing with the
    -- completion is a fact, D1). Each structured entry is replaced IN PLACE with the prose
    -- built from the WRAPPER'S verdict — authoritative at strike time; the plan's
    -- released_would_be was a prediction and is discarded here.
    FOR v_i IN 0 .. jsonb_array_length(v_targets) - 1 LOOP
        v_row := v_targets->v_i->'would_strike';
        IF v_row IS NOT NULL THEN
            SELECT blob_id, released, pathname
              INTO v_bid, v_rel, v_path
              FROM blob_delete('blob_erased',
                               jsonb_build_object('blob_id', (v_row->>'blob_id')::uuid),
                               p_emitter,
                               p_correlation => p_request_ref);
            v_targets := jsonb_set(v_targets, ARRAY[v_i::text],
                jsonb_build_object(
                    'target',  'kb_blobs',
                    'outcome', blob_strike_outcome_text(v_rel, v_path)));
        END IF;
    END LOOP;

    -- ── The estate contexts are locked here, after the resource erasures and before anything
    --    reads them for the record (R7). context_rename, context_retire, context_restore and
    --    context_reassign take the same row lock before they read the row, so one that races the
    --    act waits for it and then finds the context erased; without it, a rename could land a real
    --    name back on an erased context, or a retirement write a real slug onto the ledger after the
    --    rewrite below. After the resource erasures, not before: each takes its resource's locks and
    --    then its home context's row, the order a standalone resource erasure takes them, so the
    --    two acts cannot deadlock on the pair. In id order, so two person acts cannot either. What
    --    the act erases and rewrites is read under the lock, not taken from the plan.
    PERFORM 1 FROM kb_contexts c
      WHERE c.id = ANY(v_estate)
      ORDER BY c.id
        FOR NO KEY UPDATE;
    v_ctx_erase  := _principal_erasure_contexts_to_erase(v_estate);
    v_ctx_fields := _principal_erasure_redacted_fields(p_subject, v_estate);

    -- ── The ONE completion event (D1), last in the record (2c, 2d). NULL-anchored (admin);
    -- emitter = the OPERATOR. It names the estate it reached (the redaction reads it back from
    -- here), the resource erasures it ran, in order, and the charters it holds. References carry
    -- the subject, the request, and each resource erasure (rel `erasure`).
    v_refs := jsonb_build_array(
        jsonb_build_object('rel','subject',
            'target', jsonb_build_object('kind','kb_profiles','id', p_subject)),
        jsonb_build_object('rel','request',
            'target', jsonb_build_object('kind','kb_events','id', p_request_ref)))
        || coalesce((SELECT jsonb_agg(jsonb_build_object('rel','erasure',
                         'target', jsonb_build_object('kind','kb_events','id', e->>'event'))
                         ORDER BY ord)
                       FROM jsonb_array_elements(v_erasures) WITH ORDINALITY AS t(e, ord)),
                    '[]'::jsonb);
    v_payload := jsonb_build_object(
        'subject_table',     'kb_profiles',
        'subject_id',        p_subject,
        'actor',             p_operator,
        'redacted_hashes',   to_jsonb(v_hashes),
        'targets',           v_targets,
        'estate_contexts',   v_plan->'estate_contexts',
        'resource_erasures', v_erasures,
        'charters_held',     v_plan->'charters',
        'redacted_fields',   v_ctx_fields);
    v_ev := _event_append(
        'principal_erased', p_emitter, NULL, NULL, v_payload,
        p_references => v_refs,
        p_correlation => p_request_ref);
    -- The act's leave to rewrite the estate contexts' ledger copies (R7): one
    -- kb_event_field_redactions row per (event, path), authority `principal`. Replay calls the same
    -- projector at the event's position.
    PERFORM _project_principal_erased_redactions(v_ev, v_payload);

    -- ── The identity arms, the charters' text and the watermarks: the ONE definition, reading the
    --    estate from the event just appended.
    PERFORM _erasure_apply_redaction(p_subject, v_hashes, v_ev);

    -- ── The custody closure (R7): one context_erased per estate context not yet erased, in id
    --    order. It retires the context and replaces its name and slug with the sentinels; replay
    --    re-applies the same projector, so the closure no longer rests on the verbatim restore of
    --    kb_contexts. The payload carries only the sentinels: no from_* field, so no real name or
    --    slug is written to the ledger. Emitter = the operator; correlation = the request.
    FOREACH v_cid IN ARRAY v_ctx_erase LOOP
        v_cpayload := jsonb_build_object('context_id', v_cid,
                                         'to_name',    'erased',
                                         'to_slug',    'erased-' || v_cid::text);
        v_cev := _event_append('context_erased', p_emitter, 'kb_contexts', v_cid, v_cpayload,
                               p_correlation => p_request_ref);
        PERFORM _project_context_erased(v_cev, v_cpayload);
    END LOOP;

    -- ── The ledger rewrite (R7): every estate context's earlier name and slug, to the same
    --    sentinels. One statement, after the tombstone above, which the verifier requires of the
    --    `principal` authority, and after the redaction rows it reads.
    UPDATE kb_events e
       SET payload = _principal_erasure_payload_redaction(e.id, t.name, e.payload, f.paths)
      FROM jsonb_to_recordset(v_ctx_fields) AS f(event uuid, paths jsonb),
           kb_event_types t
     WHERE e.id = f.event
       AND t.id = e.event_type_id;

    RETURN jsonb_build_object(
        'event_id',          v_ev,
        'redacted_hashes',   to_jsonb(v_hashes),
        'targets',           v_targets,
        'already_erased',    (v_plan->>'already_erased')::boolean,
        'estate_contexts',   v_plan->'estate_contexts',
        'resource_erasures', v_erasures,
        'charters_held',     v_plan->'charters');
END;
$$;

-- Section 6. The redaction loses arm 13.
CREATE OR REPLACE FUNCTION _erasure_apply_redaction(p_subject uuid, p_hashes text[], p_event uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
    v_sentinel text := 'erased-' || p_subject::text;
    v_team     uuid;
    -- The estate the act recorded (D1 of the 2026-10-10 person-erasure design): the subject's
    -- @me contexts and their personal team's contexts, read from THIS event's payload, never
    -- re-derived. Live and replay therefore reach the same set whatever has happened to
    -- personal_of since. Required: no principal was erased before the key existed (Q5, ruled
    -- 2026-10-10, guarded by 20261021100000), so an event without it is a defect, not history.
    v_estate   uuid[] := (SELECT ARRAY(SELECT jsonb_array_elements_text(e.payload->'estate_contexts')::uuid)
                            FROM kb_events e
                           WHERE e.id = p_event AND e.payload ? 'estate_contexts');
BEGIN
    IF v_estate IS NULL THEN
        RAISE EXCEPTION '_erasure_apply_redaction: event % records no estate_contexts', p_event;
    END IF;

    -- (1) The personal-team denormalization (D5 blind-spot row 1 — the only denormalization
    --     carrier Derivation C found). Found by kb_teams.personal_of, never by recomputing the
    --     slug: a personal team whose slug was held at genesis carries a `-N` suffix, and the
    --     team at the bare slug is someone else's (20261013100000). Scrubbed to the same
    --     derivation applied to the sentinel identity. The trigger is AFTER INSERT only, so
    --     this never re-fires.
    SELECT t.id INTO v_team FROM kb_teams t WHERE t.personal_of = p_subject;

    UPDATE kb_teams
       SET slug = 'personal-' || v_sentinel,
           name = v_sentinel || ' (personal)'
      WHERE id = v_team;

    -- Arms (2)-(5), (11) and (12)'s member arm reach by hash. Since 20261021100000 every estate
    -- resource but a charter has already been erased by resource erasure in this transaction,
    -- row by row, before this body runs, and the act's hashes are the charters' and the struck
    -- blobs' alone (D5). So for a husk these arms find nothing left to empty, and what they still
    -- do is empty the charters the act holds (D3).
    --
    -- (2) Chunk prose emptied BY HASH, hash kept (D3: emptied, not deleted — the CAS
    --     retention rule never fires) — GOVERNED HOMES ONLY (ruled 2026-09-10, decision
    --     01a08dc2): the hash is the record key, never the reach. A same-hash chunk in a
    --     home the subject does not govern keeps its prose; erasure never reaches beyond
    --     the estate its subject governs. The predicate is HOME-CUSTODY shaped, not
    --     authorship shaped: a resource homed in the subject's governed context but owned
    --     by a grantee is IN the estate (the ruling — the context wipes with the estate)
    --     and its content redacts; a same-hash row in a home the subject does not govern
    --     is never the subject's to erase.
    UPDATE kb_chunk_content cc
       SET content = ''
      FROM kb_chunks c
     WHERE cc.chunk_id = c.id
       AND c.content_hash = ANY(p_hashes)
       AND cc.content <> ''
       AND c.resource_id IN (
           SELECT h.resource_id
             FROM kb_resource_homes h
            WHERE h.anchor_table = 'kb_contexts'
              AND h.anchor_id = ANY(v_estate));

    -- (3) Verbatim block bytes, same shape, same governed reach (the content row resolves
    --     to its resource through the revision chain).
    UPDATE kb_block_content
       SET content = ''
      WHERE content_hash = ANY(p_hashes)
        AND content <> ''
        AND block_revision_id IN (
            SELECT br.id
              FROM kb_block_revisions br
              JOIN kb_content_blocks b ON b.id = br.block_id
             WHERE b.resource_id IN (
                   SELECT h.resource_id
                     FROM kb_resource_homes h
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY(v_estate)));

    -- (4) Embeddings + provenance nulled TOGETHER — `embedding IS NULL` and
    --     `embedded_with IS NULL` can never disagree (20260713000040:84-87). Governed
    --     homes only: a same-hash chunk elsewhere keeps its vector and stays searchable.
    UPDATE kb_chunks
       SET embedding = NULL,
           embedded_with = NULL
      WHERE content_hash = ANY(p_hashes)
        AND embedding IS NOT NULL
        AND resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate));

    -- (4a) The heading trail nulled (20261020100000): header_path is authored heading prose,
    --      so a heading naming the subject is the subject's text like the chunk body (2). The
    --      resource act's D2.1 spelling (NULL, not ''). Same hash, same governed reach: a
    --      same-hash chunk elsewhere keeps its own trail.
    UPDATE kb_chunks
       SET header_path = NULL
      WHERE content_hash = ANY(p_hashes)
        AND header_path IS NOT NULL
        AND resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate));

    -- (5) Search vectors emptied wholesale for every hash-affected GOVERNED resource (the
    --     _rebuild_resource_search_vector site, 20260711000060:74-83): the vector folds
    --     title+body+meta, so a partial redaction would leave redacted terms searchable.
    --     Resources outside the governed homes keep their vectors — their content was
    --     never touched.
    UPDATE kb_resource_search_index si
       SET search_vector = ''
      WHERE si.resource_id IN (
            SELECT DISTINCT c.resource_id FROM kb_chunks c
             WHERE c.content_hash = ANY(p_hashes)
               AND c.resource_id IN (
                   SELECT h.resource_id
                     FROM kb_resource_homes h
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY(v_estate)))
        AND si.search_vector <> '';

    -- (6) The erased-content set (D4): first admit keeps attribution — ON CONFLICT DO
    --     NOTHING is what makes a ledger-order rebuild byte-identical. The set remains the
    --     RECORD of what the act redacted (hash + admitting event); its former role as an
    --     instance-wide write-path oracle is retired (see block_mutate below).
    INSERT INTO kb_erased_content (content_hash, erased_by_event_id)
    SELECT h, p_event FROM unnest(p_hashes) AS h
    ON CONFLICT (content_hash) DO NOTHING;

    -- (7) The tombstone (D5): identifiers nulled/sentinel'd, the UUID KEPT — it is the
    --     pseudonym. tombstoned_at is the admitting event's occurred_at, never now().
    UPDATE kb_profiles pr
       SET handle        = v_sentinel,
           display_name  = v_sentinel,
           email         = NULL,
           preferences   = '{}'::jsonb,
           tombstoned_at = COALESCE(pr.tombstoned_at, v_occurred)
      WHERE pr.id = p_subject;

    -- (8) The no-FK external identifiers (D5 blind-spot row 3): they identify the person
    --     through an EXTERNAL system, so the pseudonym break does not cover them — deleted,
    --     the slack_disconnect_service.rs:168,224 precedent. The principals resolve through
    --     the auth-link rows (auth_provider 'slack' — slack_link_service::SLACK_AUTH_PROVIDER).
    DELETE FROM kb_slack_grant_vault v
     USING kb_profile_auth_links l
      WHERE l.profile_id = p_subject
        AND l.auth_provider = 'slack'
        AND v.slack_principal_id = l.auth_provider_user_id;

    DELETE FROM kb_slack_link_intents i
     USING kb_profile_auth_links l
      WHERE l.profile_id = p_subject
        AND l.auth_provider = 'slack'
        AND i.slack_principal_id = l.auth_provider_user_id;

    -- (9) The auth-link identifiers (the manifest's identifier | full columns on
    --     kb_profile_auth_links): the email and the IdP's subject id identify the person
    --     DIRECTLY — the pseudonym break does not cover them, the same reasoning as the
    --     Slack stores above. They are unclaimed only HERE, after (8): the Slack-store
    --     deletes resolve their principals THROUGH auth_provider_user_id, so nulling first
    --     would leave the external stores standing over a destroyed lookup. email is
    --     nullable and NULLed; auth_provider_user_id is NOT NULL and sentineled — the D5
    --     nulled/sentinel'd split. THE SENTINEL IS THE ROW'S OWN ID ('erased-' || id): the
    --     kb_profiles derivation generalized, and unique by construction under
    --     kb_profile_auth_links_auth_provider_auth_provider_user_id_key, which two rows of
    --     one provider sharing a subject-keyed sentinel would violate. Every provider's
    --     rows, not just Slack's: the manifest declares the columns, not a provider.
    UPDATE kb_profile_auth_links l
       SET email                 = NULL,
           auth_provider_user_id = 'erased-' || l.id::text
      WHERE l.profile_id = p_subject
        AND (l.email IS NOT NULL OR l.auth_provider_user_id IS DISTINCT FROM 'erased-' || l.id::text);

    -- (10) The subject's entities' names (manifest identifier | full: an agent-instance
    --      name may embed the person's name). Sentineled to 'erased-' || the entity's own
    --      id — the display_name sentinel derivation (the row's kept UUID is the
    --      pseudonym), and the ONLY per-row spelling that satisfies the
    --      (profile_id, name) UNIQUE grain: two of the subject's entities cannot share
    --      one subject-keyed sentinel. The entity UUIDs stay.
    UPDATE kb_entities en
       SET name = 'erased-' || en.id::text
      WHERE en.profile_id = p_subject
        AND en.name IS DISTINCT FROM 'erased-' || en.id::text;

    -- (11) The subject's own data-artifact content (manifest content | full). Artifacts are
    --      reached only through what points at them, so the reach is the governed-home join:
    --      the resource is homed in one of the subject's OWN personal contexts (the same
    --      owner arm (12) spells) AND the kind is owned by the subject's profile. Emptied,
    --      never deleted — the artifact id, kb_data_artifacts.content_hash and
    --      kb_data_artifact_content.content_hash all stay, keeping the metadata/bytes split's
    --      hash-retention shape (D3). A team-kind or other-profile-kind artifact on the
    --      subject's governed resource is NOT the subject's data — the disposition-iii
    --      remainder the act's targets name, never struck here. NOT admitted to
    --      kb_erased_content: the set is the record of the act's redacted hashes, and no
    --      artifact consult site exists — the tombstone is what keeps the subject from
    --      re-committing.
    UPDATE kb_data_artifact_content dac
       SET content = '{}'::jsonb
      FROM kb_data_artifacts da
      WHERE dac.artifact_id = da.id
        AND da.kind_owner_table = 'kb_profiles'
        AND da.kind_owner_id = p_subject
        AND da.resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate))
        AND dac.content <> '{}'::jsonb;

    -- (12) The centroid sites MARKED for recompute (D5): the formation watermark nulled on
    --     every anchor whose live regions hold an ESTATE member, plus every estate context
    --     (their telos aggregates their goals). Keyed by home since 20261021100000, not by the
    --     act's hashes, which are now the charters' alone: every estate resource is erased,
    --     and resource erasure's own step (7) marks only a resource's home context and the maps
    --     holding it, never another principal's context whose region holds it.
    UPDATE kb_cogmaps m
       SET shape_materialized_event_id = NULL
      WHERE m.shape_materialized_event_id IS NOT NULL
        AND m.id IN (
            SELECT r.home_anchor_id
              FROM kb_cogmap_regions r
              JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
             WHERE r.home_anchor_table = 'kb_cogmaps'
               AND NOT r.is_folded
               AND mem.member_table = 'kb_resources'
               AND mem.member_id IN (SELECT h.resource_id
                                      FROM kb_resource_homes h
                                     WHERE h.anchor_table = 'kb_contexts'
                                       AND h.anchor_id = ANY(v_estate)));

    UPDATE kb_contexts c
       SET shape_materialized_event_id = NULL
      WHERE c.shape_materialized_event_id IS NOT NULL
        AND ( c.id IN (
                SELECT r.home_anchor_id
                  FROM kb_cogmap_regions r
                  JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                 WHERE r.home_anchor_table = 'kb_contexts'
                   AND NOT r.is_folded
                   AND mem.member_table = 'kb_resources'
                   AND mem.member_id IN (SELECT h.resource_id
                                      FROM kb_resource_homes h
                                     WHERE h.anchor_table = 'kb_contexts'
                                       AND h.anchor_id = ANY(v_estate)))
            -- Every estate context, the personal team's included (R1).
            OR c.id = ANY(v_estate) );

    -- (13) The custody closure is no longer here. Since 20261022100000 the act appends one
    --      context_erased per estate context after this body, which retires the context and
    --      replaces its name and slug with sentinels, and replay re-applies it at its position.
END;
$$


;

-- Section 7. The principal_erased payload_schema, re-registered with the required key redacted_fields.
-- Section 0 guarantees no earlier payload exists to fail it. The literal is the committed fixture
-- crates/temper-substrate/tests/fixtures/payloads/principal_erased.v1.schema.json, pasted byte for
-- byte; payload_schema.rs pins the two together.
UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PrincipalErased",
  "description": "`principal_erased` — the ONE admin event of a completed erasure (erasure spec D1).\n\nONE TYPE for identity erasure and content erasure: the distinction rides the per-target\noutcomes as data, never the type boundary — two types would cost the pairing (one request,\none operator, one reference) and double registration for no additional guarantee. The\nsubject is the PSEUDONYM the act itself broke; the event never re-identifies — no name, no\nemail, no case description. The request reference does not ride here: it lives on\n`kb_events.\"references\"`, the apparatus this act owns.\n\nThe redacted set is keyed on CONTENT HASH (D2) — never row ids, which would collide with\nthe trail functions' join-key shapes. The erased-content set (D4) rebuilds from these\npayloads alone.",
  "type": "object",
  "properties": {
    "actor": {
      "description": "The acting operator, distinguishable from the subject — self-serve vs administrative\nis a comparison the auditor needs. `None` only where no actor exists to name; inventing\none would put a fabricated attribution on the ledger (the standing-event precedent).",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "charters_held": {
      "description": "The estate's charters, emptied and held until map-grain erasure (D3).",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "estate_contexts": {
      "description": "The estate the act reached (20261021100000, person-erasure design D1): the subject's @me\ncontexts and their personal team's contexts, in id order. The redaction reads it back from\nhere, so replay reaches the set live reached. Required, and written even when empty: the\nredaction raises without it, and no `principal_erased` predates it (the migration guards).",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "redacted_fields": {
      "description": "The estate contexts' ledger copies of their names and slugs this act rewrote to the\nsentinels (20261022100000, R7): `context_renamed`, `context_retired` and\n`context_restored` paths, in the shape `resource_erased` uses. Paths only, never values.\nRequired, and written even when empty: no `principal_erased` predates it (the migration\nguards).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/RedactedEventFields"
      }
    },
    "redacted_hashes": {
      "description": "The redacted set (D2): bare sha256 hex, exactly as `content_hash` carries it — the key\nevery reader already shares and no trail join-key shape can match.",
      "type": "array",
      "items": {
        "type": "string"
      }
    },
    "resource_erasures": {
      "description": "The resource erasures the act ran over the estate, in the order it ran them (D4).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/EstateResourceErasureRef"
      }
    },
    "subject_id": {
      "description": "The pseudonym — `kb_profiles.id`. It survives; its identifying power does not (D5).",
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "$ref": "#/$defs/AnchorTable"
    },
    "targets": {
      "description": "Per-target outcomes and the named remainder (D6's accepted-in-part arm): anything\nunhonourable is named here. Partial completion is data, never a silent success.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/ErasureTargetOutcome"
      }
    }
  },
  "required": [
    "subject_table",
    "subject_id",
    "estate_contexts",
    "resource_erasures",
    "charters_held",
    "redacted_fields"
  ],
  "$defs": {
    "AnchorTable": {
      "description": "A polymorphic anchor/endpoint reference. Serializes table names exactly as the DDL spells them.",
      "type": "string",
      "enum": [
        "kb_contexts",
        "kb_cogmaps",
        "kb_resources",
        "kb_edges",
        "kb_content_blocks",
        "kb_teams",
        "kb_profiles",
        "kb_connections",
        "kb_machine_clients",
        "kb_blobs",
        "kb_events"
      ]
    },
    "ErasureTargetOutcome": {
      "description": "One target of a completed erasure and what happened to it (erasure spec, \"per-target\noutcomes\"). The target names itself the way the personal-data manifest does — `table` or\n`table.column`; the outcome is the act's own record of what redaction applied. Deliberately\nopen-textured in v1: ceilings are DATA, not types (D1), and the per-target vocabulary is the\nexecution build's to pin. `unhonourable_scope` outcomes land here, never silent.",
      "type": "object",
      "properties": {
        "outcome": {
          "description": "What the act did to it (erased / sentinel-scrubbed / accepted-in-part / …).",
          "type": "string"
        },
        "target": {
          "description": "Manifest identity of the target (`kb_profiles.display_name`, `kb_teams.slug`, …).",
          "type": "string"
        }
      },
      "required": [
        "target",
        "outcome"
      ]
    },
    "EstateResourceErasureKind": {
      "description": "Whether that resource erasure was a first erasure or a completion pass (D2's `complete`).",
      "type": "string",
      "enum": [
        "erasure",
        "completion"
      ]
    },
    "EstateResourceErasureRef": {
      "description": "One resource erasure a person act ran: its subject and its `resource_erased` event. Keyed\n`resource` and `event`, never `resource_id`: the completion payload carries no key the trail\nfunctions join on.",
      "type": "object",
      "properties": {
        "event": {
          "type": "string",
          "format": "uuid"
        },
        "kind": {
          "$ref": "#/$defs/EstateResourceErasureKind"
        },
        "resource": {
          "type": "string",
          "format": "uuid"
        }
      },
      "required": [
        "resource",
        "event",
        "kind"
      ]
    },
    "EventId": {
      "description": "A `kb_events.id` value. Always UUIDv7 (time-sortable).",
      "type": "string",
      "format": "uuid"
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "RedactedEventFields": {
      "description": "The ledger paths of one event: redacted (`redacted_fields`) or named-and-unreached\n(`ledger_remainder`). ONE shape for both, so the cut-2 completion pass derives what it redacts\nfrom what cut 1 recorded without translating (resource erasure spec D12). The event is keyed\n`event`, never `event_id` — no trail join-key shape rides an admin payload (D1). Paths only,\nnever values: the record of a redaction must not carry what was redacted.",
      "type": "object",
      "properties": {
        "event": {
          "$ref": "#/$defs/EventId"
        },
        "paths": {
          "description": "JSON paths within that event's `payload` (or `metadata`), e.g. `title`, `origin_uri`.",
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "event",
        "paths"
      ]
    }
  }
}
$JS$::jsonb
 WHERE name = 'principal_erased';

COMMENT ON FUNCTION _erasure_apply_redaction(uuid, text[], uuid) IS
    'The person act''s identity redaction (20261022100000). Reads the estate it reaches from its principal_erased event (estate_contexts: the subject''s @me and personal-team contexts) and raises without it. Every non-charter estate resource has already been erased by resource erasure in the same transaction, so the hash arms (chunk and block content, embeddings, header_path, search vectors, subject-kind artifact content) reach only a charter the act holds; kb_erased_content is first-admit refilled from the event''s hashes. It also tombstones the profile, scrubs the personal team''s slug and name, deletes the no-FK Slack stores, unclaims the auth-link identifiers, sentinels the entity names, and nulls the formation watermark of every anchor whose live regions hold an estate member and of every estate context. It no longer retires the estate contexts: the act appends context_erased for each. Event-free and idempotent: the act calls it, and replay calls the same function at the event''s position.';

COMMENT ON FUNCTION _project_context_erased(uuid, jsonb) IS
    'The context_erased projector (20261022100000, R7): retires one estate context and writes the sentinel name and slug its payload carries. Pure re-apply, never authorizes; principal_erasure_execute appends the event, and replay calls this at its position.';

SELECT declare_migration(
    20261022100000,
    'additive',
    'A new event type (context_erased, domain, NULL payload_schema) and its projector; four new SQL functions for the person act''s ledger authority (_principal_erasure_redact_paths, _principal_erasure_context_events, _principal_erasure_redacted_fields, _principal_erasure_payload_redaction) and its projector entry (_project_principal_erased_redactions); kb_event_field_redactions'' authority CHECK widened to admit ''principal''; the principal_erased payload_schema re-registered with one new required key (Section 0 refuses a ledger holding any earlier principal_erased payload); and CREATE OR REPLACE, every signature unchanged, of context_retire, context_restore, context_rename, _erasure_path_class, _erasure_sentinel_exact, _erasure_sentinel_admits, _project_field_redactions, kb_events_append_only, kb_events_redaction_in_trail, principal_erasure_survey_plan, principal_erasure_execute and _erasure_apply_redaction. A binary without this migration keeps working: it calls the same functions with the same arguments. What changes is a refusal (context_restore of an erased context raises TE001, which such a binary renders as a 500, not a 410), a row lock the three context verbs now take, and the resource acts'' ledger rewrites are admitted exactly as before.'
);
