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
 * Tracks SQL parameter indices to avoid manual `$N` counting.
 */
class ParamBuilder {
  private index = 1;
  readonly params: (string | number | boolean | null)[] = [];

  /** Add a value and return its `$N` placeholder. */
  add(value: string | number | boolean | null): string {
    this.params.push(value);
    return `$${this.index++}`;
  }

  /** Add a value with a type cast, e.g. `$N::vector`. */
  addCast(value: string | number | boolean | null, cast: string): string {
    return `${this.add(value)}::${cast}`;
  }
}

export function buildVersionBumpQuery(resourceId: string, newVersion: number): QueryResult {
  return {
    sql: `UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND version < $2 AND is_current = true`,
    params: [resourceId, newVersion],
  };
}

/**
 * Build two queries: one for kb_chunks (metadata + embedding) and one for
 * kb_chunk_content (content text, TOAST-optimised).
 */
export function buildStoreChunksQueries(chunks: ChunkRow[]): QueryResult[] {
  if (chunks.length === 0) return [];

  // --- kb_chunks INSERT (no content column) ---
  const chunkPb = new ParamBuilder();
  const chunkValues: string[] = [];

  for (const chunk of chunks) {
    const embeddingStr = `[${chunk.embedding.join(",")}]`;
    const id = chunkPb.add(chunk.id);
    const rid = chunkPb.add(chunk.resource_id);
    const ci = chunkPb.add(chunk.chunk_index);
    const ver = chunkPb.add(chunk.version);
    const hp = chunkPb.add(chunk.header_path);
    const ch = chunkPb.add(chunk.content_hash);
    const emb = chunkPb.addCast(embeddingStr, "vector");
    chunkValues.push(`(${id}, ${rid}, ${ci}, ${ver}, ${hp}, ${ch}, ${emb}, true)`);
  }

  const chunkSql = `INSERT INTO kb_chunks (id, resource_id, chunk_index, version, header_path, content_hash, embedding, is_current)
VALUES ${chunkValues.join(",\n")}
ON CONFLICT (resource_id, chunk_index, version) DO UPDATE SET
  header_path = EXCLUDED.header_path,
  content_hash = EXCLUDED.content_hash,
  embedding = EXCLUDED.embedding,
  is_current = EXCLUDED.is_current`;

  // --- kb_chunk_content INSERT ---
  const contentPb = new ParamBuilder();
  const contentValues: string[] = [];

  for (const chunk of chunks) {
    const cid = contentPb.add(chunk.id);
    const cnt = contentPb.add(chunk.content);
    contentValues.push(`(${cid}, ${cnt})`);
  }

  const contentSql = `INSERT INTO kb_chunk_content (chunk_id, content)
VALUES ${contentValues.join(",\n")}
ON CONFLICT (chunk_id) DO UPDATE SET content = EXCLUDED.content`;

  return [
    { sql: chunkSql, params: chunkPb.params },
    { sql: contentSql, params: contentPb.params },
  ];
}

/**
 * @deprecated Use buildStoreChunksQueries (plural) instead.
 */
export function buildStoreChunksQuery(chunks: ChunkRow[]): QueryResult {
  const queries = buildStoreChunksQueries(chunks);
  if (queries.length === 0) return { sql: "", params: [] };
  return queries[0];
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
