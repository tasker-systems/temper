import { extractFromBuffer } from "../../packages/temper-cloud/src/workflow/extract.js";
import { chunkText } from "../../packages/temper-cloud/src/workflow/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/workflow/embed.js";
import {
  buildVersionBumpQuery,
  buildStoreChunksQuery,
  buildStatusUpdateQuery,
  type ChunkRow,
} from "../../packages/temper-cloud/src/workflow/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";
import { randomUUID } from "crypto";

export async function processUpload(
  blobFileId: string,
  blobUrl: string,
  resourceId: string
) {
  "use workflow";

  const text = await extractStep(blobFileId, blobUrl);
  const chunks = await chunkStep(text);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(blobFileId, resourceId, chunks, embeddings);
}

async function extractStep(blobFileId: string, blobUrl: string): Promise<string> {
  "use step";

  const db = getDb();

  // Update status to processing
  const statusQuery = buildStatusUpdateQuery(blobFileId, "processing", null);
  await db(statusQuery.sql, statusQuery.params);

  // Download file from Vercel Blob
  const response = await fetch(blobUrl, {
    headers: {
      Authorization: `Bearer ${process.env.BLOB_READ_WRITE_TOKEN}`,
    },
  });

  if (!response.ok) {
    throw new Error(`Failed to download blob: ${response.status}`);
  }

  const buffer = Buffer.from(await response.arrayBuffer());
  const filename = blobUrl.split("/").pop() || "document";
  const result = await extractFromBuffer(buffer, filename);

  return result.content;
}

async function chunkStep(
  text: string
): Promise<Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>> {
  "use step";
  return chunkText(text);
}

async function embedStep(texts: string[]): Promise<number[][]> {
  "use step";
  return embedTexts(texts);
}

async function storeStep(
  blobFileId: string,
  resourceId: string,
  chunks: Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>,
  embeddings: number[][]
): Promise<void> {
  "use step";

  const db = getDb();

  // Determine next version for this resource
  const versionResult = await db`
    SELECT COALESCE(MAX(version), 0) + 1 AS next_version
    FROM kb_chunks WHERE resource_id = ${resourceId}::uuid
  `;
  const nextVersion = versionResult[0].next_version as number;

  // Mark old chunks as not current
  const bumpQuery = buildVersionBumpQuery(resourceId, nextVersion);
  await db(bumpQuery.sql, bumpQuery.params);

  // Build chunk rows with embeddings
  const chunkRows: ChunkRow[] = chunks.map((chunk, i) => ({
    id: randomUUID(),
    resource_id: resourceId,
    chunk_index: chunk.chunk_index,
    version: nextVersion,
    header_path: chunk.header_path,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding: embeddings[i],
  }));

  // Store chunks
  const storeQuery = buildStoreChunksQuery(chunkRows);
  if (storeQuery.sql) {
    await db(storeQuery.sql, storeQuery.params);
  }

  // Update blob_files status to processed
  const statusQuery = buildStatusUpdateQuery(blobFileId, "processed", null);
  await db(statusQuery.sql, statusQuery.params);
}
