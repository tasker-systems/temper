-- WS6 chunk 4c — forward migration: idempotent relationship_assert, the partial-unique property
-- index, and the resource/relationship/property mutation functions for the live write path.
--
-- Why a forward migration and not a regen of 20260613000001_install_temper_next.sql:
-- the install migration is FROZEN once merged+applied (sqlx checksum-tracks applied migrations, so
-- editing its bytes breaks `sqlx::migrate!` at API boot on any persistent DB). The artifact
-- (schema-artifact/01_schema.sql + 02_functions.sql) stays the design-master; artifact changes that
-- post-date the install migration land here as append-only forward migrations. The semantic drift
-- guard (crates/temper-next/tests/schema_drift.rs) proves that applying the temper_next migrations in
-- order reconstructs the artifact schema — superseding the old byte-for-byte single-file generator.
SET search_path TO temper_next, public;

-- ── kb_properties: table-level UNIQUE → partial-unique (active rows only) ─────
-- Matches 01_schema.sql: a folded property is history and may repeat a value, so property_set's
-- fold-then-reinsert (incl. revert-to-a-prior-value) never collides; active (owner, key, value)
-- duplicates stay forbidden (the multi-valued-facet guard).
ALTER TABLE kb_properties
    DROP CONSTRAINT IF EXISTS kb_properties_owner_table_owner_id_property_key_property_va_key;
CREATE UNIQUE INDEX IF NOT EXISTS uq_kb_properties_active ON kb_properties
    (owner_table, owner_id, property_key, property_value) WHERE NOT is_folded;

-- ── relationship_assert: idempotent on the active-edge invariant ──────────────
CREATE OR REPLACE FUNCTION _project_relationship_asserted(p_event uuid, p_payload jsonb)
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
            p_event, p_event, v_occurred)
    -- Idempotent on the active-edge invariant (uq_kb_edges_assertion): re-asserting the same active
    -- relationship updates the existing edge's weight (+ last_event_id) and returns ITS id rather than
    -- creating a duplicate active edge. asserted_by_event_id is left on the original assertion. The
    -- ON CONFLICT inference clause mirrors uq_kb_edges_assertion's columns + partial predicate exactly.
    ON CONFLICT (source_table, source_id, target_table, target_id, edge_kind, COALESCE(label, ''),
                 home_anchor_table, home_anchor_id) WHERE NOT is_folded
        DO UPDATE SET weight = EXCLUDED.weight, last_event_id = EXCLUDED.last_event_id
    RETURNING id INTO v_edge;
    RETURN v_edge;
END;
$$;

-- ============================================================================
-- Resource + relationship mutation functions for the live write path.
-- Resource mutations take the envelope from the resource's own home (the facet_set discipline);
-- edge mutations take it from the edge's home (the relationship_fold discipline). Each emits + projects
-- in one txn through _event_append, so replay is the same code path.
-- ============================================================================

-- ── resource_deleted (soft-delete) ───────────────────────────────────────────
CREATE OR REPLACE FUNCTION _project_resource_deleted(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resources SET is_active = false,
        updated = (SELECT occurred_at FROM kb_events WHERE id = p_event)
        WHERE id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_delete: resource % not found', v_resource; END IF;
    RETURN v_resource;
END;
$$;

CREATE OR REPLACE FUNCTION resource_delete(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_delete: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_deleted', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_resource_deleted(v_ev, p_payload);
END;
$$;

-- ── resource_updated (mutable kb_resources columns) ──────────────────────────
-- COALESCE keeps an unset field at its current value — the payload carries only the changed columns.
CREATE OR REPLACE FUNCTION _project_resource_updated(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resources SET
        title      = COALESCE(p_payload->>'title', title),
        origin_uri = COALESCE(p_payload->>'origin_uri', origin_uri),
        updated    = (SELECT occurred_at FROM kb_events WHERE id = p_event)
        WHERE id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_update: resource % not found', v_resource; END IF;
    RETURN v_resource;
END;
$$;

CREATE OR REPLACE FUNCTION resource_update(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_update: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_updated', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_resource_updated(v_ev, p_payload);
END;
$$;

-- ── resource_rehomed (context move) ──────────────────────────────────────────
-- Re-point the resource's single home row to the destination anchor. The event envelope is the
-- DESTINATION home (post-move, that is where the resource lives) — read from the payload, not the row.
CREATE OR REPLACE FUNCTION _project_resource_rehomed(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resource_homes SET
        anchor_table = p_payload#>>'{home,table}',
        anchor_id    = (p_payload#>>'{home,id}')::uuid
        WHERE resource_id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_rehome: resource % has no home', v_resource; END IF;
    RETURN v_resource;
END;
$$;

CREATE OR REPLACE FUNCTION resource_rehome(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_rehomed', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload);
    RETURN _project_resource_rehomed(v_ev, p_payload);
END;
$$;

-- ── property_set (single-valued upsert) ──────────────────────────────────────
-- Fold prior ACTIVE rows for (owner, property_key) then insert the new value, so a single-valued key
-- (the resource-frontmatter shape) holds exactly one current row. Distinct from facet_set/
-- property_asserted (append — the multi-valued facet shape). Replay-safe: deterministic from payload.
CREATE OR REPLACE FUNCTION _project_property_set(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_owner_tbl text := p_payload#>>'{owner,table}';
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
        v_key text := p_payload->>'property_key';
BEGIN
    UPDATE kb_properties SET is_folded = true, last_event_id = p_event
        WHERE owner_table = v_owner_tbl AND owner_id = v_owner
          AND property_key = v_key AND NOT is_folded;
    INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                               asserted_by_event_id, last_event_id, created)
    VALUES (v_prop, v_owner_tbl, v_owner, v_key, p_payload->'value',
            (p_payload->>'weight')::double precision, p_event, p_event, v_occurred);
    RETURN v_prop;
END;
$$;

-- Envelope = the owner resource's home (the facet_set discipline). A homeless owner is an error.
CREATE OR REPLACE FUNCTION property_set(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_owner ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'property_set: resource % has no home to anchor the property event', v_owner;
    END IF;
    v_ev := _event_append('property_set', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_property_set(v_ev, p_payload);
END;
$$;

-- ── relationship_retyped (edge_kind / polarity) ──────────────────────────────
CREATE OR REPLACE FUNCTION _project_relationship_retyped(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
BEGIN
    UPDATE kb_edges SET
        edge_kind = (p_payload->>'edge_kind')::edge_kind,
        polarity  = (p_payload->>'polarity')::edge_polarity,
        last_event_id = p_event
        WHERE id = v_edge;
    IF NOT FOUND THEN RAISE EXCEPTION 'relationship_retype: edge % not found', v_edge; END IF;
    RETURN v_edge;
END;
$$;

CREATE OR REPLACE FUNCTION relationship_retype(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN RAISE EXCEPTION 'relationship_retype: edge % not found', v_edge; END IF;
    v_ev := _event_append('relationship_retyped', p_emitter, v_home_tbl, v_home, p_payload);
    RETURN _project_relationship_retyped(v_ev, p_payload);
END;
$$;

-- ── relationship_reweighted (weight) ─────────────────────────────────────────
CREATE OR REPLACE FUNCTION _project_relationship_reweighted(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
BEGIN
    UPDATE kb_edges SET weight = (p_payload->>'weight')::double precision, last_event_id = p_event
        WHERE id = v_edge;
    IF NOT FOUND THEN RAISE EXCEPTION 'relationship_reweight: edge % not found', v_edge; END IF;
    RETURN v_edge;
END;
$$;

CREATE OR REPLACE FUNCTION relationship_reweight(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN RAISE EXCEPTION 'relationship_reweight: edge % not found', v_edge; END IF;
    v_ev := _event_append('relationship_reweighted', p_emitter, v_home_tbl, v_home, p_payload);
    RETURN _project_relationship_reweighted(v_ev, p_payload);
END;
$$;
