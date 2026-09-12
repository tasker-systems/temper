-- The element-trail readers project the act correlation id (`kb_events.correlation_id`).
--
-- The column has been stored since correlation threading: the backend mints a batch
-- correlation per invocation, threads it through `EventContext`, and every fired event
-- carries it — the receipt echoes it as the receipt-to-ledger pairing key. But no reader
-- returned it, so the pairing was undeliverable: an operator holding a receipt could not
-- find "the ledger's record of the batch" the trail names. Both readers gain the column;
-- everything else is re-emitted VERBATIM from `20260719000010` (the allowlist, the
-- `category = 'domain'` firewall, `producing_anchor_table IS NOT NULL` on every arm, and
-- the visibility predicates are untouched — the permissive direction is still the leaking
-- direction, and a category added without touching these readers stays EXCLUDED).
--
-- Nullable: events from before correlation threading, and acts that self-root, carry NULL
-- and the wire field is absent for them (additive on every read surface).
--
-- The return type changes, so `CREATE OR REPLACE` cannot apply it — the readers are dropped
-- and recreated, the shape `20260706000002` used for the same reason. The window is one
-- statement pair in one migration transaction; the readers serve no write path.

DROP FUNCTION element_trail_node(uuid, uuid);
DROP FUNCTION element_trail_edge(uuid, uuid);

CREATE OR REPLACE FUNCTION element_trail_node(
    p_profile uuid,
    p_resource uuid
) RETURNS TABLE (
    event_id uuid,
    kind text,
    actor_entity_id uuid,
    occurred_at timestamptz,
    metadata jsonb,
    payload jsonb,
    actor_name text,
    correlation_id uuid
) LANGUAGE sql STABLE AS $$
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
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name,
           ev.correlation_id
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE et.category = 'domain'
      AND EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$$;

CREATE OR REPLACE FUNCTION element_trail_edge(
    p_profile uuid,
    p_edge uuid
) RETURNS TABLE (
    event_id uuid,
    kind text,
    actor_entity_id uuid,
    occurred_at timestamptz,
    metadata jsonb,
    payload jsonb,
    actor_name text,
    correlation_id uuid
) LANGUAGE sql STABLE AS $$
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name,
           ev.correlation_id
    FROM kb_edges edg
    JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = edg.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE edg.id = p_edge
      AND ev.producing_anchor_table IS NOT NULL
      AND et.category = 'domain'
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$$;
