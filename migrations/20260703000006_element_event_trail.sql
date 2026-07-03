-- R5 element event-trail: time-ordered events for a node or edge.

-- Edge trail: every relationship_* payload embeds a stable edge_id. Gate via the
-- edge's HOME ANCHOR (anchor_readable_by_profile), NOT edges_visible_to — a folded
-- edge must still show its trail (the fold event is part of the story).
CREATE FUNCTION element_trail_edge(
    p_profile uuid, p_edge uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz, metadata jsonb)
LANGUAGE sql STABLE AS $$
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata
    FROM kb_edges edg
    JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = edg.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    WHERE edg.id = p_edge
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
    ORDER BY ev.id;
$$;

-- Node trail: NO single key exists — union three grounded key-shapes, then gate
-- once via resources_visible_to. (1) resource-keyed events (created/updated/deleted/
-- rehomed/block_created); (2) property events whose owner IS this resource
-- (guard owner.table='kb_resources'); (3) block events that carry only block_id →
-- join kb_content_blocks to attribute them.
CREATE FUNCTION element_trail_node(
    p_profile uuid, p_resource uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz, metadata jsonb)
LANGUAGE sql STABLE AS $$
    WITH ev_ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
        WHERE b.resource_id = p_resource
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    WHERE EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$$;
