-- The resource's byte identity, computed at read: `bytes_merkle` on ResourceView and list rows.
--
-- A client that cut citations from cached text has to prove its bytes are Temper's bytes without
-- downloading the body again. body_hash cannot answer that: it is a merkle over chunk prose, not
-- over bytes. Every block revision already stores the sha256 of its raw bytes
-- (kb_block_content.content_hash); this function folds the live blocks' hashes into one value:
--
--     bytes_merkle = hex(sha256( bytes_sha256(block seq 0) || bytes_sha256(block seq 1) || ... ))
--
-- each bytes_sha256 lowercase hex, concatenated with no separator, live (unfolded) blocks in seq
-- order, each block at its current revision. It is the shape of body_hash's own merkle, one level.
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * IT IS NULL UNLESS THE WHOLE BODY IS VERBATIM: at least one live block, and every live block's
--     current revision carries bytes — the same coverage rule _recompute_body_storage derives
--     'verbatim' from and the verbatim readback's HAVING checks. A derived body reads back as a
--     chunk reconstruction, so no stored hash describes it.
--   * IT IS STABLE, so a call shares its calling statement's snapshot. That is what makes the value
--     served beside `content` describe that content: `readback::body_with_bytes_merkle` computes
--     both in one statement. A VOLATILE body would take a snapshot per inner query and could
--     describe a later write than the bytes beside it.
--   * IT CARRIES NO GATE. A hash of the bytes is a disclosure about the bytes, so a caller reads it
--     only where it reads the content: inside a statement that has already admitted the row (the
--     SELECT list of the gated identity read, or the body read after ensure_visible).
--   * NO COLUMN. It is O(live blocks) over stored hashes, with no body read and nothing to maintain.
--     An emptied (erased) row carries sha256('') (20261127000010), so the value never names erased
--     text.

CREATE FUNCTION resource_bytes_merkle(p_resource uuid)
RETURNS text
LANGUAGE sql
STABLE
PARALLEL SAFE
AS $$
    SELECT encode(sha256(convert_to(string_agg(bc.content_hash, '' ORDER BY b.seq), 'UTF8')), 'hex')
      FROM kb_content_blocks b
      LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
     WHERE b.resource_id = p_resource
       AND NOT b.is_folded
    HAVING count(*) > 0
       AND count(*) = count(bc.block_revision_id)
$$;

COMMENT ON FUNCTION resource_bytes_merkle(uuid) IS
    'Byte identity of a verbatim body: hex sha256 over the live blocks'' kb_block_content.content_hash '
    '(current revision, seq order, no separator). NULL unless every live block carries bytes. STABLE, '
    'so it shares the calling statement''s snapshot. Ungated: call it only where the caller already '
    'reads the content (20261127000020).';

SELECT declare_migration(
    20261127000020,
    'additive',
    'Adds the function resource_bytes_merkle(uuid). No table, column, constraint or existing function '
    'changes, and nothing calls it until a binary that serves bytes_merkle ships, so a binary without '
    'this change is unaffected.'
);
