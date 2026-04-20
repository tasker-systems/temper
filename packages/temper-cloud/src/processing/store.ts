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

/**
 * Format chunks as JSONB array matching the persist_resource_chunks() SQL
 * function's expected input format (same as Rust chunks_to_jsonb).
 */
export function chunksToJsonb(chunks: ChunkRow[]): object[] {
  return chunks.map((c) => ({
    chunk_index: c.chunk_index,
    header_path: c.header_path,
    content: c.content,
    content_hash: c.content_hash,
    embedding: `[${c.embedding.join(",")}]`,
  }));
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
