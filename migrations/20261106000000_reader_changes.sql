-- What a reader changed: the resources their own profile changed, and the contexts they changed
-- something in, each at the time of their latest change there. Read by the list door's
-- `changed_by=@me` filter and `changed_by_me` sort, and by `GET /api/me/context-activity`
-- (`context_service::activity_of`), so "your change" has one definition on every door that
-- answers it.
--
-- What a later edit must not break:
--   1. THE READER IS THE EMITTER'S PROFILE. An event is the reader's when its emitter entity
--      belongs to the reader's profile (`kb_entities.profile_id`). Every surface a person or their
--      agent writes from emits as that profile's `<handle>@<surface>` entity (`resolve_emitter`),
--      so a change an agent makes under the reader's profile is the reader's. A machine client is
--      its own profile (`kb_machine_clients.profile_id`), so its changes are never the reader's,
--      whoever registered it. Do not widen this to `registered_by_profile_id` or to team members.
--      `p_profile` is the principal, as in every gate function here: the doors bind the caller.
--   2. AUTHORED CHANGES ONLY. Domain events, anchored, never `webhook_received` (the filter
--      `element_trail_node` applies, 20261007100000), and never an event the SYSTEM computes:
--      `region_materialized`, `salience_refreshed` and `relationship_decayed`. A region job
--      carries the emitter of the write that first queued it (`queue_region_clocks`), and the
--      queue folds later writers into that one job, so a materialization driven by teammates'
--      writes is stamped with whoever wrote first. Counting it would move a reader's "last
--      changed" with no act of theirs. `changed_by_filter_test.rs` partitions every domain type
--      into authored or computed, so a new type is classified before it is counted or dropped.
--   3. THE TRAIL'S ARMS, OVER AUTHORED EVENTS. A resource counts when its element trail holds one
--      of these events, through the trail's three arms: the event names the resource, owns a
--      property of it, or names one of its blocks. The trail itself also shows the computed types
--      excluded above; none of their payloads carries those ids today, so the two name the same
--      resources, but that rests on the payload shapes, not on the filters agreeing.
--   4. GATED WHERE THEY ANSWER, ONCE. `resources_changed_by` aggregates per resource, then joins
--      `resources_visible_to(p_profile)` once; `contexts_changed_by` aggregates per context, then
--      joins `contexts_readable_by(p_profile)` once — which floors on `is_active`, so a retired
--      context carries no time. Gating per event instead re-derives the visible set for every
--      event in the reader's history (measured: 66 ms to 2.9 s on the contexts read at 50k events).
--      The events helper `_profile_change_events` is ungated and is read by these two alone;
--      `changed_by_filter_test.rs` derives its callers from `pg_proc` and fails on a third. It is
--      not one of the verdict-handed ungated fragments (20260808000030): it is handed no verdict.
--   5. READS ONLY. Nothing here, and nothing calling it, records that it was read.
--
-- Additive: three CREATE FUNCTION. No DROP, no CREATE OR REPLACE, no DEFAULT.

CREATE FUNCTION _profile_change_events(p_profile uuid)
RETURNS TABLE (occurred_at timestamptz,
               resource_id uuid, owner_resource_id uuid, block_id uuid,
               producing_anchor_table varchar, producing_anchor_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT ev.occurred_at,
           (ev.payload ->> 'resource_id')::uuid,
           CASE WHEN ev.payload -> 'owner' ->> 'table' = 'kb_resources'
                THEN (ev.payload -> 'owner' ->> 'id')::uuid END,
           (ev.payload ->> 'block_id')::uuid,
           ev.producing_anchor_table, ev.producing_anchor_id
      FROM kb_entities en
      JOIN kb_events ev ON ev.emitter_entity_id = en.id
      JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE en.profile_id = p_profile
       AND ev.producing_anchor_table IS NOT NULL
       AND et.category = 'domain'
       AND et.name <> ALL (ARRAY['webhook_received',
                                 'region_materialized', 'salience_refreshed', 'relationship_decayed'])
$$;

COMMENT ON FUNCTION _profile_change_events(uuid) IS
    'UNGATED, and private to resources_changed_by and contexts_changed_by: the anchored, authored domain events emitted by any entity of p_profile (never webhook_received, and never the system-computed region_materialized, salience_refreshed or relationship_decayed), with the three resource ids the element trail matches on. The reader''s own changes, including those of agents acting under the profile, and never a machine client''s, which is its own profile. Not one of the verdict-handed ungated fragments (20260808000030): it is handed none. Call the gated readers instead. (20261106000000)';

CREATE FUNCTION resources_changed_by(p_profile uuid)
RETURNS TABLE (resource_id uuid, changed_at timestamptz)
LANGUAGE sql STABLE AS $$
    WITH ev AS MATERIALIZED (
        SELECT * FROM _profile_change_events(p_profile)
    ),
    touched AS (
        SELECT ev.resource_id, ev.occurred_at FROM ev WHERE ev.resource_id IS NOT NULL
        UNION ALL
        SELECT ev.owner_resource_id, ev.occurred_at FROM ev WHERE ev.owner_resource_id IS NOT NULL
        UNION ALL
        SELECT b.resource_id, ev.occurred_at
          FROM ev JOIN kb_content_blocks b ON b.id = ev.block_id
    ),
    latest AS MATERIALIZED (
        SELECT touched.resource_id, max(touched.occurred_at) AS changed_at
          FROM touched
         GROUP BY touched.resource_id
    )
    SELECT latest.resource_id, latest.changed_at
      FROM latest
      JOIN resources_visible_to(p_profile) v ON v.resource_id = latest.resource_id
$$;

COMMENT ON FUNCTION resources_changed_by(uuid) IS
    'Each resource p_profile changed and can still read, at the time of their latest change to it: the resources whose element trail holds one of _profile_change_events(p_profile), through the trail''s three arms (names the resource, owns a property of it, names one of its blocks), aggregated per resource and then gated once by resources_visible_to(p_profile). (20261106000000)';

CREATE FUNCTION contexts_changed_by(p_profile uuid)
RETURNS TABLE (context_id uuid, changed_at timestamptz)
LANGUAGE sql STABLE AS $$
    WITH latest AS MATERIALIZED (
        SELECT ev.producing_anchor_id AS context_id, max(ev.occurred_at) AS changed_at
          FROM _profile_change_events(p_profile) ev
         WHERE ev.producing_anchor_table = 'kb_contexts'
         GROUP BY ev.producing_anchor_id
    )
    SELECT latest.context_id, latest.changed_at
      FROM latest
      JOIN contexts_readable_by(p_profile) rc ON rc.context_id = latest.context_id
$$;

COMMENT ON FUNCTION contexts_changed_by(uuid) IS
    'Each context p_profile changed something in and can still read, at the time of their latest authored change produced there, from _profile_change_events(p_profile), aggregated per context and then gated once by contexts_readable_by(p_profile) — which admits no retired context, so a retired context carries no time. (20261106000000)';

SELECT declare_migration(
    20261106000000,
    'additive',
    'New functions _profile_change_events(uuid), resources_changed_by(uuid) and contexts_changed_by(uuid): a profile''s own anchored, authored domain events, and the resources and contexts they changed with the latest time of each, gated by the profile''s read predicates. Read by the list door''s changed_by filter and sort and by GET /api/me/context-activity. No existing object is altered or replaced, and no parameter carries a DEFAULT; a deployed binary that does not call them is unaffected.'
);
