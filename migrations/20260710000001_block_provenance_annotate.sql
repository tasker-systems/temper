-- Annotate-only block-provenance write path (issue #355).
--
-- Attaching sources to an existing resource today requires a `resource update` with a body — which
-- re-chunks + re-embeds the block (a full revise) for a metadata-only change. For a corpus imported
-- without sources, that is O(2 calls × corpus size) and rewrites body_hash/embeddings needlessly.
--
-- This migration adds a NEW event `block_provenance_annotated` that records `kb_block_provenance`
-- rows onto an existing block WITHOUT touching chunks: no supersede, no re-embed, no body_hash
-- recompute. It reuses the chunk-independent `_insert_block_provenance` helper (…003/…007) verbatim,
-- so `remote`-source resolution + the idempotent UNIQUE-key behavior are shared with the revise path.
--
-- Additive: a new event-type row + two new functions; no birth migration edited. Replay-stable +
-- idempotent — `_project_block_annotated` is a pure function of (p_event, payload) and
-- `_insert_block_provenance` ON CONFLICT-no-ops, so fire and replay produce identical rows.

-- ── register the event type ───────────────────────────────────────────────────
-- Permissive (NULL schema), like resource_updated/deleted/finalized/invocation_closed — the typed
-- payload is validated Rust-side (`payloads::verify_ledger_roundtrip`) and the substrate bootseed
-- stamps the committed JSON-Schema snapshot for its own namespaces; production needs no wire schema.
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('block_provenance_annotated', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- ── _project_block_annotated: record incorporation, nothing else ──────────────
-- Payload-only projector (no content sidecar). The whole effect is the provenance rows — chunks,
-- kb_block_revisions, body_hash, and the search vector are all untouched, which is exactly what makes
-- this an annotate and not a revise.
CREATE FUNCTION _project_block_annotated(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_block uuid := (p_payload->>'block_id')::uuid;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM kb_content_blocks WHERE id = v_block) THEN
        RAISE EXCEPTION '_project_block_annotated: block % not found', v_block;
    END IF;
    PERFORM _insert_block_provenance(v_block, p_event, p_payload->'incorporated');
    RETURN v_block;
END;
$$;

-- ── block_annotate: the entry function (event + projection, one txn) ───────────
-- Mirrors block_mutate's anchor resolution + correlation params, but carries NO content sidecar and
-- requires a non-empty `incorporated` (an annotate with nothing to attach is a caller error — the
-- mirror of block_mutate's empty-chunk-set guard).
CREATE FUNCTION block_annotate(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION 'block_annotate: block % not found', v_block;
    END IF;
    IF p_payload->'incorporated' IS NULL OR jsonb_array_length(p_payload->'incorporated') = 0 THEN
        RAISE EXCEPTION 'block_annotate: empty source set for block % (annotate requires >= 1 source)', v_block;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'block_annotate: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('block_provenance_annotated', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_block_annotated(v_ev, p_payload);
END;
$$;
