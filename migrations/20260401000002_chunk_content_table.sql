-- Extract chunk content into a separate table for TOAST-friendly storage.
--
-- kb_chunks becomes lean (IDs + embedding + metadata) which improves HNSW
-- index scan performance.  Content is loaded via JOIN only when needed.

-- 1. Create the content table.
CREATE TABLE kb_chunk_content (
    chunk_id UUID PRIMARY KEY REFERENCES kb_chunks(id) ON DELETE CASCADE,
    content  TEXT NOT NULL
);

-- Let TOAST compress aggressively (lower target = more out-of-line storage).
ALTER TABLE kb_chunk_content SET (toast_tuple_target = 128);

-- 2. Migrate existing content.
INSERT INTO kb_chunk_content (chunk_id, content)
SELECT id, content FROM kb_chunks;

-- 3. Drop the old view first (it depends on kb_chunks.content).
DROP VIEW IF EXISTS kb_current_chunks;

-- 4. Drop content from chunks table.
ALTER TABLE kb_chunks DROP COLUMN content;

-- 5. Recreate the view with a JOIN to the content table.
CREATE VIEW kb_current_chunks AS
SELECT c.id, c.resource_id, c.chunk_index, c.version, c.header_path,
       cc.content, c.content_hash, c.embedding, c.created
  FROM kb_chunks c
  LEFT JOIN kb_chunk_content cc ON cc.chunk_id = c.id
 WHERE c.is_current = true
 ORDER BY c.resource_id, c.chunk_index;
