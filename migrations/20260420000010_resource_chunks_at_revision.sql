-- Point-in-time reconstruction: return the chunks as they existed at a given
-- revision. A chunk is live at revision R when:
--   * its first_revision_id was created at or before R, AND
--   * it was either never superseded, or superseded strictly after R.
CREATE OR REPLACE FUNCTION resource_chunks_at_revision(
    p_resource_id UUID,
    p_revision_id UUID
) RETURNS TABLE(
    id UUID, chunk_index INT, header_path TEXT, heading_depth SMALLINT,
    content TEXT, content_hash VARCHAR(64), embedding vector(768), version INT
)
LANGUAGE sql STABLE AS $$
    WITH target AS (
        SELECT created FROM kb_resource_revisions
         WHERE id = p_revision_id AND resource_id = p_resource_id
    )
    SELECT c.id, c.chunk_index, c.header_path, c.heading_depth,
           cc.content, c.content_hash, c.embedding, c.version
      FROM kb_chunks c
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
      JOIN kb_resource_revisions first_rev ON first_rev.id = c.first_revision_id
      LEFT JOIN kb_resource_revisions sup_rev ON sup_rev.id = c.superseded_revision_id
     WHERE c.resource_id = p_resource_id
       AND first_rev.created <= (SELECT created FROM target)
       AND (sup_rev.id IS NULL OR sup_rev.created > (SELECT created FROM target))
     ORDER BY c.chunk_index;
$$;
