export interface ChunkRow {
  id: string;
  resource_id: string;
  chunk_index: number;
  version: number;
  header_path: string;
  content: string;
  content_hash: string;
  embedding: number[];
}

interface QueryResult {
  sql: string;
  params: (string | number | boolean | null)[];
}

export function buildVersionBumpQuery(resourceId: string, newVersion: number): QueryResult {
  return {
    sql: `UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND version < $2 AND is_current = true`,
    params: [resourceId, newVersion],
  };
}

export function buildStoreChunksQuery(chunks: ChunkRow[]): QueryResult {
  if (chunks.length === 0) return { sql: "", params: [] };

  const values: string[] = [];
  const params: (string | number | boolean | null)[] = [];
  let paramIndex = 1;

  for (const chunk of chunks) {
    const embeddingStr = `[${chunk.embedding.join(",")}]`;
    values.push(
      `($${paramIndex}, $${paramIndex + 1}, $${paramIndex + 2}, $${paramIndex + 3}, $${paramIndex + 4}, $${paramIndex + 5}, $${paramIndex + 6}, $${paramIndex + 7}::vector, true)`,
    );
    params.push(
      chunk.id,
      chunk.resource_id,
      chunk.chunk_index,
      chunk.version,
      chunk.header_path,
      chunk.content,
      chunk.content_hash,
      embeddingStr,
    );
    paramIndex += 8;
  }

  const sql = `INSERT INTO kb_chunks (id, resource_id, chunk_index, version, header_path, content, content_hash, embedding, is_current)
VALUES ${values.join(",\n")}
ON CONFLICT (resource_id, chunk_index, version) DO UPDATE SET
  header_path = EXCLUDED.header_path,
  content = EXCLUDED.content,
  content_hash = EXCLUDED.content_hash,
  embedding = EXCLUDED.embedding,
  is_current = EXCLUDED.is_current`;

  return { sql, params };
}

export function buildStatusUpdateQuery(
  blobFileId: string,
  status: "pending" | "processing" | "processed" | "failed",
  errorMessage: string | null,
): QueryResult {
  return {
    sql: `UPDATE kb_blob_files SET status = $1, error_message = $2, updated_at = now() WHERE id = $3`,
    params: [status, errorMessage, blobFileId],
  };
}
