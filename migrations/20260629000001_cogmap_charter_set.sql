-- Post-birth telos-charter delivery (L0 telos charter, 2026-06-28 spec). `block_mutate` is revise-only
-- and genesis leaves a fresh telos with zero blocks, so neither existing primitive can populate an empty
-- telos. `cogmap_charter_set` replaces the telos's FULL block set uniformly (0→N first delivery, N→M
-- re-delivery) via fold-then-reproject — one `charter_set` event whose projection folds the prior blocks
-- (the same supersede-on-revise discipline as block_mutated) and projects the new role-tagged set through
-- the shared _project_blocks path with the p_content sidecar. Additive: data + function (+ one constraint
-- relaxation, below) only.

-- ── seq-uniqueness must bind only LIVE blocks ────────────────────────────────────────────────────────
-- charter_set folds the prior charter THEN re-projects the new role-tagged set at the SAME seqs (0..N) —
-- so a folded seq-0 block and the freshly-projected live seq-0 block must coexist on the same telos. The
-- canonical schema's total `UNIQUE (resource_id, seq)` (auto-named kb_content_blocks_resource_id_seq_key)
-- forbids that, breaking every re-delivery. Replace it with a PARTIAL unique index over non-folded blocks:
-- this is the conceptually-correct invariant (fold IS the supersede mechanism, folded rows are superseded
-- history), and it is strictly more permissive — every existing row is non-folded today, so all current
-- data still satisfies it. No code references the dropped constraint by name (grep-verified); no INSERT
-- uses it as an ON CONFLICT arbiter (_project_blocks inserts fresh block ids without ON CONFLICT).
ALTER TABLE kb_content_blocks DROP CONSTRAINT kb_content_blocks_resource_id_seq_key;
CREATE UNIQUE INDEX kb_content_blocks_resource_seq_live
    ON kb_content_blocks (resource_id, seq) WHERE NOT is_folded;

-- The `charter_set` event type. Payload = { cogmap_id, blocks:[BlockManifest] } (the CharterSet struct in
-- payloads.rs). Registered with a NULL payload_schema (permissive/unregistered posture), matching the
-- repo's documented "a name with no committed snapshot stays NULL" contract (canonical_seed.sql) and the
-- two most recent typed-in-Rust events (`delegated_launch`, `invocation_closed`) — both carry Rust payload
-- structs but NULL registry schema and are absent from `TYPED_EVENT_NAMES`. Stamping a schema here would
-- make `kb_event_types` carry one more published schema than `TYPED_EVENT_NAMES`, breaking the
-- registry==snapshots invariant (`bootseed_publishes_payload_schemas`). schema_version 1. (The registry
-- column is `schema_version`, not the brief's `payload_version`.)
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('charter_set', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- Projection half (replay-stable): fold the telos's prior blocks then project the new role-tagged set.
-- Shared by the full `cogmap_charter_set` mutation below AND ledger replay (replay.rs reapplies this
-- against an already-appended `charter_set` event), mirroring the genesis/`_project_cogmap_seeded` and
-- block_mutate/`_project_block_mutated` event-vs-projection split. `_project_blocks` recomputes the telos
-- body_hash at its tail, so the merkle refreshes after the fold+reproject.
CREATE FUNCTION _project_charter_set(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_telos uuid := cogmap_telos((p_payload->>'cogmap_id')::uuid);
BEGIN
    -- supersede the prior charter (0 rows on first delivery), then project the new role-tagged set.
    UPDATE kb_content_blocks SET is_folded = true, last_event_id = p_event
        WHERE resource_id = v_telos AND NOT is_folded;
    PERFORM _project_blocks(v_telos, p_event, p_payload->'blocks', p_content);
    RETURN v_telos;
END;
$$;

-- Replace a cogmap's telos charter with the payload's role-tagged blocks. Fold-then-reproject (via the
-- shared `_project_charter_set` half). Anchored on the cogmap (the telos is cogmap-homed). Rejects an empty
-- charter (a telos with no blocks would blank its identity). Returns the telos resource id.
CREATE FUNCTION cogmap_charter_set(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
        v_cogmap uuid := (p_payload->>'cogmap_id')::uuid;
        v_telos  uuid := cogmap_telos(v_cogmap);
BEGIN
    IF v_telos IS NULL THEN
        RAISE EXCEPTION 'cogmap_charter_set: cogmap % has no telos', v_cogmap;
    END IF;
    IF p_payload->'blocks' IS NULL OR jsonb_array_length(p_payload->'blocks') = 0 THEN
        RAISE EXCEPTION 'cogmap_charter_set: empty charter for cogmap % (would blank the telos)', v_cogmap;
    END IF;
    v_ev := _event_append('charter_set', p_emitter, 'kb_cogmaps', v_cogmap, p_payload);
    RETURN _project_charter_set(v_ev, p_payload, p_content);
END;
$$;
