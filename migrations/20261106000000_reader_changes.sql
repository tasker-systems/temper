-- What a reader changed: the resources their own profile changed, and the contexts they changed
-- something in, each at the time of their latest change there. Read by the list door's
-- `changed_by=@me` filter and `changed_by_me` sort, and by the contexts read's
-- `last_changed_by_you`, so "your change" has one definition on every door that answers it.
--
-- What a later edit must not break:
--   1. THE READER IS THE EMITTER'S PROFILE. An event is the reader's when its emitter entity
--      belongs to the reader's profile (`kb_entities.profile_id`). Every surface a person or their
--      agent writes from emits as that profile's `<handle>@<surface>` entity (`resolve_emitter`),
--      so a change an agent makes under the reader's profile is the reader's. A machine client is
--      its own profile (`kb_machine_clients.profile_id`), so its changes are never the reader's,
--      whoever registered it. Do not widen this to `registered_by_profile_id` or to team members.
--   2. THE SAME EVENTS A TRAIL SHOWS. Domain events only, never `webhook_received`, and only
--      anchored ones — the filter `element_trail_node` applies (20261007100000). A resource counts
--      as changed by the reader exactly when its trail holds one of these, through the same three
--      arms: the event names the resource, owns a property of it, or names one of its blocks. A
--      change here that the trail does not show, or the reverse, is a defect in one of the two.
--   3. GATED WHERE THEY ANSWER. `resources_changed_by` returns only resources
--      `resources_visible_to(p_profile)` admits; `contexts_changed_by` only contexts
--      `context_readable_by_profile` admits — which floors on `is_active`, so a retired context
--      carries no time. Each caller joins its own gate as well, which is harmless and stays. The
--      events helper `_profile_change_events` is ungated and is read by these two alone; it is not
--      one of the verdict-handed ungated fragments (20260808000030), since it is handed no verdict,
--      and it must not be called elsewhere.
--   4. READS ONLY. Nothing here, and nothing calling it, records that it was read.
--
-- Additive: three CREATE FUNCTION. No DROP, no CREATE OR REPLACE, no DEFAULT.

CREATE FUNCTION _profile_change_events(p_profile uuid)
RETURNS TABLE (occurred_at timestamptz, payload jsonb,
               producing_anchor_table varchar, producing_anchor_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT ev.occurred_at, ev.payload, ev.producing_anchor_table, ev.producing_anchor_id
      FROM kb_entities en
      JOIN kb_events ev ON ev.emitter_entity_id = en.id
      JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE en.profile_id = p_profile
       AND ev.producing_anchor_table IS NOT NULL
       AND et.category = 'domain'
       AND et.name <> 'webhook_received'
$$;

COMMENT ON FUNCTION _profile_change_events(uuid) IS
    'UNGATED, and private to resources_changed_by and contexts_changed_by: the anchored domain events (never webhook_received) emitted by any entity of p_profile — the reader''s own changes, including those of agents acting under the profile, and never a machine client''s, which is its own profile. Not one of the verdict-handed ungated fragments (20260808000030): it is handed none. Call the gated readers instead. (20261106000000)';

CREATE FUNCTION resources_changed_by(p_profile uuid)
RETURNS TABLE (resource_id uuid, changed_at timestamptz)
LANGUAGE sql STABLE AS $$
    WITH ev AS MATERIALIZED (
        SELECT * FROM _profile_change_events(p_profile)
    )
    SELECT touched.resource_id, max(touched.occurred_at)
      FROM (
            SELECT (ev.payload ->> 'resource_id')::uuid AS resource_id, ev.occurred_at
              FROM ev
             WHERE ev.payload ? 'resource_id'
            UNION ALL
            SELECT (ev.payload -> 'owner' ->> 'id')::uuid, ev.occurred_at
              FROM ev
             WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
            UNION ALL
            SELECT b.resource_id, ev.occurred_at
              FROM ev
              JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
           ) AS touched
      JOIN resources_visible_to(p_profile) v ON v.resource_id = touched.resource_id
     GROUP BY touched.resource_id
$$;

COMMENT ON FUNCTION resources_changed_by(uuid) IS
    'Each resource p_profile changed and can still read, at the time of their latest change to it: the resources whose element trail holds one of _profile_change_events(p_profile), through the trail''s three arms (names the resource, owns a property of it, names one of its blocks), gated by resources_visible_to(p_profile). (20261106000000)';

CREATE FUNCTION contexts_changed_by(p_profile uuid)
RETURNS TABLE (context_id uuid, changed_at timestamptz)
LANGUAGE sql STABLE AS $$
    SELECT ev.producing_anchor_id, max(ev.occurred_at)
      FROM _profile_change_events(p_profile) ev
     WHERE ev.producing_anchor_table = 'kb_contexts'
       AND context_readable_by_profile(p_profile, ev.producing_anchor_id)
     GROUP BY ev.producing_anchor_id
$$;

COMMENT ON FUNCTION contexts_changed_by(uuid) IS
    'Each context p_profile changed something in and can still read, at the time of their latest change produced there, from _profile_change_events(p_profile), gated by context_readable_by_profile — which admits no retired context, so a retired context carries no time. (20261106000000)';

SELECT declare_migration(
    20261106000000,
    'additive',
    'New functions _profile_change_events(uuid), resources_changed_by(uuid) and contexts_changed_by(uuid): a profile''s own anchored domain events, and the resources and contexts they changed with the latest time of each, gated by the profile''s read predicates. Read by the list door''s changed_by filter and sort and by the contexts read''s last_changed_by_you. No existing object is altered or replaced, and no parameter carries a DEFAULT; a deployed binary that does not call them is unaffected.'
);
