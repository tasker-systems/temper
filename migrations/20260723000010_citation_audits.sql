-- Append-only citation-audit ledger + write path (Set 5, Task 1; spec
-- `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §4.1-4.3).
--
-- THE GRAIN FORCES THE REPRESENTATION. A citation is a (block, source) pair —
-- `kb_block_provenance` is keyed (block_id, source_kind, source_id, contributed_by_event_id)
-- (`canonical_schema.sql:603-613`). `kb_edges` cannot address a block: its CHECK admits only
-- ('kb_resources','kb_cogmaps') as source/target (`canonical_schema.sql:630,632`). An audit at
-- citation grain therefore cannot be an edge — it is a new append-only event projection,
-- `kb_citation_audits`, following the same events-as-primary pattern as
-- `block_provenance_annotated` (`20260710000001_block_provenance_annotate.sql`).
--
-- APPEND-ONLY, NO SUPERSESSION (spec §4.1). An audit is an event; the ledger is immutable by
-- design; an auditor does not retract a verdict, it emits a new one. There is deliberately NO
-- `is_superseded` column and NO unique index on (block_id, source_kind, source_id) — multiple
-- audits of one citation, including opposite-signed ones, are the whole point. A later +1.0
-- never erases an earlier -1.0; both are permanent rows. The only uniqueness is on the
-- emitting event, which is what makes replay idempotent rather than duplicating.
--
-- The decay-weighted aggregation that turns this trail into a live standing component (spec
-- §4.1 "the visible standing is a decay-weighted projection") is a later task; this migration
-- is the substrate + write path only.

CREATE TABLE kb_citation_audits (
    id                  UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    block_id            UUID NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    source_kind         provenance_source_kind NOT NULL,
    source_id           UUID NOT NULL,
    value               DOUBLE PRECISION NOT NULL CHECK (value >= -1.0 AND value <= 1.0),
    reason              TEXT,
    audited_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    created             TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Idempotency, the ONLY uniqueness (see header). Replay re-projects the same event;
    -- ON CONFLICT (audited_by_event_id) DO NOTHING makes that a no-op.
    UNIQUE (audited_by_event_id)
);
-- Every read of a citation's audit trail (Task 3's decay aggregation, the auditor's own
-- "has this citation already been audited" check) filters by the citation key. Non-unique,
-- mirroring `idx_kb_block_provenance_source` (`canonical_schema.sql`, adjacent table).
CREATE INDEX idx_kb_citation_audits_citation ON kb_citation_audits(block_id, source_kind, source_id);

-- ── register the event type ───────────────────────────────────────────────────────────────────
-- category IS SPELLED EXPLICITLY. kb_event_types.category is NOT NULL with no default —
-- `20260719000010_admin_cognition_firewall_declarative.sql` dropped the default precisely so an
-- omitted category aborts at apply time (23502) instead of landing silently mis-categorized;
-- see its own header (`:74-81`) and the precedent registration at
-- `20260720000020_principal_standing_events.sql:26`.
--
-- 'domain' puts audits in the element trail by default (spec §4.3): `element_trail_node` /
-- `element_trail_edge` filter `et.category = 'domain'`
-- (`20260719000010_admin_cognition_firewall_declarative.sql:139,165`), so an audit is
-- inspectable and challengeable like any other act.
--
-- Permissive (NULL schema) — the typed payload is validated Rust-side, like
-- `block_provenance_annotated` (`20260710000001_block_provenance_annotate.sql:16-22`).
INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('citation_audited', NULL, 1, 'domain')
ON CONFLICT (name) DO NOTHING;

-- ── _project_citation_audited: record the audit, nothing else ──────────────────────────────────
-- Returns the AUDIT id, not the block id — the `block_annotate` sibling
-- (`20260710000001_block_provenance_annotate.sql:28-38`) returns the block id, but there is no
-- caller here for whom the block id is the useful result; the audit row IS the effect.
--
-- Survives the ON CONFLICT no-op on replay: `INSERT ... RETURNING` yields no row on the replay
-- path (the row already exists from the first projection), so the fallback SELECT recovers the
-- existing audit id rather than returning NULL.
CREATE FUNCTION _project_citation_audited(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_audit uuid;
BEGIN
    INSERT INTO kb_citation_audits (block_id, source_kind, source_id, value, reason, audited_by_event_id)
    VALUES (
        (p_payload->>'block_id')::uuid,
        (p_payload #>> '{source,kind}')::provenance_source_kind,
        (p_payload #>> '{source,value}')::uuid,
        (p_payload->>'value')::double precision,
        p_payload->>'reason',
        p_event
    )
    ON CONFLICT (audited_by_event_id) DO NOTHING
    RETURNING id INTO v_audit;

    IF v_audit IS NULL THEN
        SELECT id INTO v_audit FROM kb_citation_audits WHERE audited_by_event_id = p_event;
    END IF;

    RETURN v_audit;
END;
$$;

-- ── citation_audit: the entry function (event + projection, one txn) ───────────────────────────
-- Mirrors `block_annotate`'s anchor resolution exactly
-- (`20260710000001_block_provenance_annotate.sql:44-68`): raise if the block doesn't exist,
-- resolve the event anchor from `kb_resource_homes` for the block's resource, append the event,
-- project it and return the projector's result.
--
-- THE SOURCE IS READ NESTED, matching the incumbent. `_insert_block_provenance`
-- (`20260704000003_block_provenance_write_path.sql:29-30`) reads a citation's source as
-- `v_inc #>> '{source,kind}'` / `#>> '{source,value}'`, because that is what Rust's
-- `ProvenanceSource` emits: `#[serde(tag = "kind", content = "value")]`
-- (`temper-core/src/types/provenance.rs:37`) produces `{"source":{"kind":…,"value":…}}`. Reading
-- two flat `source_kind`/`source_id` keys instead would force the Rust payload to carry stringly-
-- typed fields and a hand-written flattening step — a second spelling of a shape the codebase
-- already names, and so a drift site. The typed payload struct is `payloads::CitationAudited`
-- with a plain `source: ProvenanceSource` field, exactly like its `BlockProvenanceCorrected`
-- sibling (`payloads.rs`).
--
-- Only resource-kind citations are auditable (spec §6.2): standing reads only resource-kind
-- bases (`20260721000010_evidential_standing_memo.sql:110`), so an audit recorded against a
-- remote/event citation would be a silent no-op the auditor could never detect. Rejected here,
-- at the write path, rather than left for a reader to discover.
CREATE FUNCTION citation_audit(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION 'citation_audit: block % not found', v_block;
    END IF;
    IF (p_payload #>> '{source,kind}') <> 'resource' THEN
        RAISE EXCEPTION 'citation_audit: only resource-kind citations are auditable (got %)',
            p_payload #>> '{source,kind}';
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'citation_audit: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('citation_audited', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_citation_audited(v_ev, p_payload);
END;
$$;
