-- Beat 2b N3: expose the replay-sufficient kb_events.payload and a humanized actor
-- name on the R5 element trail. Additive over the shipped 20260703130000 trail
-- functions. RETURNS TABLE gains columns → DROP + CREATE. Both functions are called
-- only from Rust (event_service::element_trail); no SQL dependents. emitter_entity_id
-- is a NOT NULL FK to kb_entities, so the actor JOIN never drops rows.
DROP FUNCTION element_trail_edge(uuid, uuid);
CREATE FUNCTION element_trail_edge(
    p_profile uuid, p_edge uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz,
                metadata jsonb, payload jsonb, actor_name text)
LANGUAGE sql STABLE AS $$
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name
    FROM kb_edges edg
    JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = edg.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE edg.id = p_edge
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$$;

DROP FUNCTION element_trail_node(uuid, uuid);
CREATE FUNCTION element_trail_node(
    p_profile uuid, p_resource uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz,
                metadata jsonb, payload jsonb, actor_name text)
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
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$$;
