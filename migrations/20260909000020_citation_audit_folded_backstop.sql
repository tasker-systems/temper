-- The citation-audit SQL entry gains the folded-block refusal, matching the Rust gate
-- (temper-services `finding_of_block`, the 2026-09-09 defined-dangling-state build): a folded
-- block is not a live citation, and auditing one must land in the same not-found backstop as
-- an unknown block. The entry function's stated purpose is to backstop any non-Rust writer;
-- without this filter the Rust-side filter alone left that backstop admitting audits on gone
-- citations (silent success on a standing-inert citation). One line changes: the resolution
-- gains `AND NOT is_folded`, so a folded block hits the existing NULL → not-found RAISE — the
-- zero-rows dialect, no oracle over which refusal is which. Byte-for-byte 20260724000110's
-- definition otherwise. Function-only change: no table, no registry stamp, additive.
CREATE OR REPLACE FUNCTION citation_audit(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
BEGIN
    -- `AND NOT is_folded`: a folded block is not a live citation (the defined-dangling-state
    -- design) — the backstop must refuse it exactly as it refuses an unknown block.
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block AND NOT is_folded;
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

SELECT declare_migration(
    20260909000020,
    'additive',
    'the citation-audit SQL entry backstop matches the Rust gate: a folded block resolves to '
    'the not-found refusal instead of admitting an audit on a gone citation. Function-only '
    'change, one line — the block resolution gains AND NOT is_folded; the zero-rows dialect '
    'is unchanged.'
);
