import { extractFromBuffer } from "../../packages/temper-cloud/src/workflow/extract.js";
import { chunkText } from "../../packages/temper-cloud/src/workflow/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/workflow/embed.js";
import {
  buildStatusUpdateQuery,
  chunksToJsonb,
  type ChunkRow,
} from "../../packages/temper-cloud/src/workflow/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";

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
  await db.query(statusQuery.sql, statusQuery.params);

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

  // Build chunk rows with embeddings
  const chunkRows: ChunkRow[] = chunks.map((chunk, i) => ({
    id: "",
    resource_id: resourceId,
    chunk_index: chunk.chunk_index,
    version: 0,
    header_path: chunk.header_path,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding: embeddings[i],
  }));

  // Store chunks atomically via SQL function (handles version bump + insert)
  const chunksJson = JSON.stringify(chunksToJsonb(chunkRows));
  await db`SELECT persist_resource_chunks(${resourceId}::uuid, ${chunksJson}::jsonb)`;

  // Update blob_files status to processed
  const statusQuery = buildStatusUpdateQuery(blobFileId, "processed", null);
  await db.query(statusQuery.sql, statusQuery.params);
}
