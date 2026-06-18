-- temper_next — invocation envelope + agent-authorship metadata.
--
-- Append-only to the frozen temper_next lineage (install 20260613000001 + 4c 20260616000001 +
-- can_modify 20260617000001 precede this). The artifact (schema-artifact/01_schema.sql +
-- 02_functions.sql) is the design-master; this is its faithful append. The semantic drift guard
-- (crates/temper-next/tests/schema_drift.rs) proves the lineage reconstructs the artifact, so the
-- function BODIES here are byte-identical to the artifact (unqualified names resolving against the
-- SET search_path below — never schema-qualify the body; that is what pg_get_functiondef fingerprints).
-- Idempotent: ADD COLUMN IF NOT EXISTS / CREATE TABLE IF NOT EXISTS / CREATE OR REPLACE FUNCTION.
SET search_path TO temper_next, public;

ALTER TABLE kb_events ADD COLUMN IF NOT EXISTS invocation_id UUID;
CREATE INDEX IF NOT EXISTS idx_kb_events_invocation ON kb_events(invocation_id);

CREATE TABLE IF NOT EXISTS kb_invocations (
    id                     UUID PRIMARY KEY,
    opened_by_event_id     UUID NOT NULL REFERENCES kb_events(id),
    status                 TEXT NOT NULL DEFAULT 'open'
                               CHECK (status IN ('open','completed','failed','abandoned')),
    trigger_kind           TEXT NOT NULL,
    originating_cogmap_id  UUID NOT NULL REFERENCES kb_cogmaps(id),
    parent_cogmap_id       UUID REFERENCES kb_cogmaps(id),
    scoped_entity_id       UUID NOT NULL REFERENCES kb_entities(id),
    telos_resource_id      UUID NOT NULL REFERENCES kb_resources(id),
    outcome                JSONB,
    opened_at              TIMESTAMPTZ NOT NULL,
    closed_by_event_id     UUID REFERENCES kb_events(id),
    closed_at              TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_kb_invocations_cogmap ON kb_invocations(originating_cogmap_id);
CREATE INDEX IF NOT EXISTS idx_kb_invocations_status ON kb_invocations(status);

-- The authored-act writers gain trailing p_metadata + p_invocation defaulted params (and _event_append
-- gains both too). Postgres overloads on signature, so a CREATE OR REPLACE with the wider arg list would
-- ADD a second overload alongside the frozen install migration's narrower one — leaving a stale duplicate
-- (and an ambiguous 5-positional-arg call). Drop the superseded narrow signatures first so the lineage
-- ends with exactly the artifact's single definition of each. These are plpgsql-internal callees with no
-- catalog dependents (function→function calls resolve at runtime), so the drops never cascade.
DROP FUNCTION IF EXISTS _event_append(text, uuid, text, uuid, jsonb, jsonb, uuid, integer);
DROP FUNCTION IF EXISTS resource_create(jsonb, jsonb, uuid);
DROP FUNCTION IF EXISTS relationship_assert(jsonb, uuid);
DROP FUNCTION IF EXISTS relationship_fold(jsonb, uuid);
DROP FUNCTION IF EXISTS facet_set(jsonb, uuid);

-- _event_append (extended) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION _event_append(
    p_type_name text, p_emitter uuid, p_anchor_table text, p_anchor_id uuid,
    p_payload jsonb,
    p_references jsonb DEFAULT '[]'::jsonb,
    p_correlation uuid DEFAULT NULL,
    p_payload_version int DEFAULT 1,
    p_metadata jsonb DEFAULT '{}'::jsonb,
    p_invocation uuid DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid := uuid_generate_v7();
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name = p_type_name;
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type % not seeded', p_type_name; END IF;
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id,
                           producing_anchor_table, producing_anchor_id,
                           payload, "references", payload_version, correlation_id,
                           metadata, invocation_id)
    VALUES (v_ev, v_et, p_emitter, p_anchor_table, p_anchor_id,
            p_payload, p_references, p_payload_version, COALESCE(p_correlation, v_ev),
            p_metadata, p_invocation);
    RETURN v_ev;
END;
$$;

-- resource_create (extended) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION resource_create(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_created', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_resource_created(v_ev, p_payload, p_content);
END;
$$;

-- relationship_assert (extended) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION relationship_assert(p_payload jsonb, p_emitter uuid,
                                    p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('relationship_asserted', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_relationship_asserted(v_ev, p_payload);
END;
$$;

-- relationship_fold (extended) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION relationship_fold(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home
        FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    v_ev := _event_append('relationship_folded', p_emitter, v_home_tbl, v_home, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_relationship_folded(v_ev, p_payload);
END;
$$;

-- facet_set (extended) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION facet_set(p_payload jsonb, p_emitter uuid,
                          p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_owner ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'facet_set: resource % has no home to anchor the property event', v_owner;
    END IF;
    v_ev := _event_append('property_asserted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_property_asserted(v_ev, p_payload);
END;
$$;

-- invocation_open (new) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION invocation_open(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_inv uuid := (p_payload->>'invocation_id')::uuid;
        v_orig uuid := (p_payload->>'originating_cogmap_id')::uuid;
        v_parent uuid := (p_payload->>'parent_cogmap_id')::uuid;
        v_ev uuid;
BEGIN
    IF v_parent IS NOT NULL AND NOT cogmaps_share_a_team(v_parent, v_orig) THEN
        RAISE EXCEPTION 'delegation gate: cogmaps % and % share no team', v_parent, v_orig;
    END IF;
    v_ev := _event_append('delegated_launch', p_emitter, 'kb_cogmaps', v_orig, p_payload,
                          p_invocation => v_inv);
    PERFORM _project_delegated_launch(v_ev, p_payload);
    RETURN v_inv;
END;
$$;

-- _project_delegated_launch (new) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION _project_delegated_launch(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_invocations (id, opened_by_event_id, status, trigger_kind,
        originating_cogmap_id, parent_cogmap_id, scoped_entity_id, telos_resource_id, opened_at)
    SELECT (p_payload->>'invocation_id')::uuid, p_event, 'open', p_payload->>'trigger_kind',
           (p_payload->>'originating_cogmap_id')::uuid, (p_payload->>'parent_cogmap_id')::uuid,
           (p_payload->>'scoped_entity_id')::uuid, c.telos_resource_id, v_occurred
    FROM kb_cogmaps c WHERE c.id = (p_payload->>'originating_cogmap_id')::uuid;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'delegated_launch: originating cogmap % not found',
            (p_payload->>'originating_cogmap_id')::uuid;
    END IF;
END;
$$;

-- invocation_close (new) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION invocation_close(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_inv uuid := (p_payload->>'invocation_id')::uuid;
        v_orig uuid; v_ev uuid;
BEGIN
    SELECT originating_cogmap_id INTO v_orig FROM kb_invocations WHERE id = v_inv;
    IF v_orig IS NULL THEN RAISE EXCEPTION 'invocation_close: unknown invocation %', v_inv; END IF;
    IF p_payload->>'disposition' NOT IN ('completed','failed','abandoned') THEN
        RAISE EXCEPTION 'invocation_close: invalid disposition %', p_payload->>'disposition';
    END IF;
    v_ev := _event_append('invocation_closed', p_emitter, 'kb_cogmaps', v_orig, p_payload,
                          p_invocation => v_inv);
    PERFORM _project_invocation_closed(v_ev, p_payload);
    RETURN v_ev;
END;
$$;

-- _project_invocation_closed (new) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION _project_invocation_closed(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    UPDATE kb_invocations
       SET status = p_payload->>'disposition',
           outcome = p_payload->'outcome',
           closed_by_event_id = p_event,
           closed_at = v_occurred
     WHERE id = (p_payload->>'invocation_id')::uuid;
END;
$$;
