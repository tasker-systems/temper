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
--
-- `created` COMES FROM THE OWNING EVENT, NEVER `now()` — the module rule every sibling projector
-- states and follows (`20260624000002_canonical_functions.sql:614,791`: *"Projected timestamps come
-- from the owning event's occurred_at, never now() (replay-stable by construction)"*; `_project_blocks`,
-- `_project_block_mutated`, `_project_resource_created` all open with the same
-- `v_occurred` lookup, `:625,679,725,795,873`). Leaving it to the column DEFAULT is not a cosmetic
-- deviation here: `resource_citation_quality` weights each audit by `pow(0.5, age/30d)`
-- (`20260724000120_standing_citation_components.sql:170-188`), so a replay that stamps every row with
-- the replay clock collapses every weight to ~1 and can INVERT a band — a trail recording
-- "-1.0 in January, +1.0 in July" reprojects as a dead heat. `kb_citation_audits` is the first table
-- whose *relative ordering between two of its own rows* is load-bearing, so this is the first place
-- the shortcut is not survivable.
CREATE FUNCTION _project_citation_audited(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_audit uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_citation_audits (block_id, source_kind, source_id, value, reason,
                                    audited_by_event_id, created)
    VALUES (
        (p_payload->>'block_id')::uuid,
        (p_payload #>> '{source,kind}')::provenance_source_kind,
        (p_payload #>> '{source,value}')::uuid,
        (p_payload->>'value')::double precision,
        p_payload->>'reason',
        p_event,
        v_occurred
    )
    ON CONFLICT (audited_by_event_id) DO NOTHING
    RETURNING id INTO v_audit;

    IF v_audit IS NULL THEN
        SELECT id INTO v_audit FROM kb_citation_audits WHERE audited_by_event_id = p_event;
    END IF;

    RETURN v_audit;
END;
$$;

-- ── the two citation predicates the write path and its gate ask ───────────────────────────────
-- Both are `EXISTS` over `kb_block_provenance` — the table that IS the citation record — and both
-- exclude `is_corrected` rows, matching every incumbent citation reader
-- (`20260624000002_canonical_functions.sql:481`, `20260721000010_evidential_standing_memo.sql:55,68,108`,
-- `20260724000120_standing_citation_components.sql:104`). They are separate functions because they
-- answer separate questions and are called from separate layers; the shared join is three lines and
-- `citation_audit`'s existence guard calls `citation_is_live` rather than restating it.

-- "Is (block, source) actually a citation?" — the write path's guard.
--
-- Without it an audit of a NON-citation is accepted with 200 and is permanently inert: both
-- `resource_audit_coverage` and `resource_citation_quality` join through `resource_live_citations`
-- on the full citation key (`20260724000120_standing_citation_components.sql:132-137,176-180`), so the
-- row moves nothing. The reachable mistake is a one-row transposition while iterating
-- `get_block_provenance` — posting block B0 with a source cited on B1 — after which the finding
-- re-heads `audit_drift_sweep` with the identical `uncovered` count on every subsequent tick,
-- forever, and the auditor is never told because it got a success.
CREATE FUNCTION citation_is_live(p_block uuid, p_source_kind provenance_source_kind, p_source uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_block_provenance p
         WHERE p.block_id    = p_block
           AND p.source_kind = p_source_kind
           AND p.source_id   = p_source
           AND NOT p.is_corrected
    );
$$;

-- "Did this profile CONTRIBUTE that citation?" — the self-audit denial arm's exact question
-- (spec §7: *"the emitter of the block's contributing `block_mutated`/`block_annotated` event"*).
--
-- THIS IS A HISTORICAL FACT, NOT A CURRENT CAPABILITY. `can_modify_resource` — the proxy Set 5
-- shipped with — answers *"can you write this finding now?"*, and the two come apart the moment an
-- author loses write while keeping read: a revoked direct grant, a `reassign_resource` that moves the
-- home to another owner, or a team-role demotion below the authoring roles. In every one of those the
-- citer resolves to `Auditor` again and may grade its own work, which is precisely what §7 says is
-- *"enforced here or it is not enforced anywhere."* `originator_profile_id` cannot help:
-- `20260715000040_demote_originator_from_access.sql` establishes that *"originator is provenance only,
-- not access."* The provenance row's own `contributed_by_event_id` is the durable record, and it is
-- immutable — so this predicate is stable under every access change.
--
-- The emitter resolves to a profile through `kb_entities.profile_id`, the same chain
-- `writes::resolve_emitter` mints on the way in, so a machine principal's audits and a human's are
-- separated by exactly the thing that separates their credentials.
CREATE FUNCTION citation_contributed_by_profile(p_block uuid, p_source_kind provenance_source_kind,
                                                p_source uuid, p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_block_provenance p
          JOIN kb_events   e  ON e.id  = p.contributed_by_event_id
          JOIN kb_entities en ON en.id = e.emitter_entity_id
         WHERE p.block_id    = p_block
           AND p.source_kind = p_source_kind
           AND p.source_id   = p_source
           AND NOT p.is_corrected
           AND en.profile_id = p_profile
    );
$$;

COMMENT ON FUNCTION citation_is_live(uuid, provenance_source_kind, uuid) IS
  'Does an uncorrected kb_block_provenance row exist for this (block, source_kind, source)? The write '
  'path''s existence guard: an audit of a non-citation is inert for standing, so accepting one is a '
  'silently successful no-op.';
COMMENT ON FUNCTION citation_contributed_by_profile(uuid, provenance_source_kind, uuid, uuid) IS
  'Did this profile emit the event that contributed this citation? The exact self-audit question (spec '
  '§7) — a HISTORICAL fact off kb_block_provenance.contributed_by_event_id, unaffected by later access '
  'changes, unlike the can_modify_resource proxy.';

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
    -- The (block, source) pair must name a LIVE citation. Same shape and dialect as the kind guard
    -- above, and for the same reason spec §6.2 gives for that one: *"rather than silently accepting a
    -- no-op the auditor cannot detect."* See `citation_is_live`'s own header for the failure this
    -- prevents.
    IF NOT citation_is_live(v_block, 'resource'::provenance_source_kind,
                            (p_payload #>> '{source,value}')::uuid) THEN
        RAISE EXCEPTION 'citation_audit: (block %, source %) is not a live citation',
            v_block, p_payload #>> '{source,value}';
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
