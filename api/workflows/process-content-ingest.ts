import { chunkText } from "../../packages/temper-cloud/src/workflow/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/workflow/embed.js";
import {
  chunksToJsonb,
  type ChunkRow,
} from "../../packages/temper-cloud/src/workflow/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";
import { canonicalJsonHash } from "../../packages/temper-cloud/src/hash.js";
import {
  DEVICE_ID_CLOUD,
  insertEventAndAudit,
} from "../../packages/temper-cloud/src/events.js";

export async function processContentIngest(
  resourceId: string,
  content: string,
  replace: boolean,
  profileId: string,
) {
  "use workflow";

  console.log(
    `[content-ingest] Starting processing for resource ${resourceId}, replace=${replace}`,
  );
  const chunks = await chunkStep(content);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(resourceId, chunks, embeddings, replace, profileId);
}

async function chunkStep(
  text: string,
): Promise<
  Array<{
    header_path: string;
    content: string;
    content_hash: string;
    chunk_index: number;
  }>
> {
  "use step";
  console.log(`[content-ingest:chunk] Chunking ${text.length} chars`);
  const chunks = chunkText(text);
  console.log(`[content-ingest:chunk] Produced ${chunks.length} chunks`);
  return chunks;
}

async function embedStep(texts: string[]): Promise<number[][]> {
  "use step";
  console.log(`[content-ingest:embed] Embedding ${texts.length} chunks`);
  const embeddings = await embedTexts(texts);
  console.log(`[content-ingest:embed] Done`);
  return embeddings;
}

async function storeStep(
  resourceId: string,
  chunks: Array<{
    header_path: string;
    content: string;
    content_hash: string;
    chunk_index: number;
  }>,
  embeddings: number[][],
  replace: boolean,
  profileId: string,
): Promise<void> {
  "use step";

  console.log(
    `[content-ingest:store] Storing ${chunks.length} chunks for resource ${resourceId}, replace=${replace}`,
  );
  const db = getDb();

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

  const chunksJson = JSON.stringify(chunksToJsonb(chunkRows));

  if (replace) {
    await db`SELECT replace_resource_chunks(${resourceId}::uuid, ${chunksJson}::jsonb)`;
  } else {
    await db`SELECT persist_resource_chunks(${resourceId}::uuid, ${chunksJson}::jsonb)`;
  }

  // Fire body_processed event
  const contextRows =
    await db`SELECT kb_context_id FROM kb_resources WHERE id = ${resourceId}::uuid`;
  if (contextRows.length > 0) {
    const contextId = contextRows[0].kb_context_id as string;
    const emptyHash = canonicalJsonHash({});

    const manifestRows =
      await db`SELECT body_hash FROM kb_resource_manifests WHERE resource_id = ${resourceId}::uuid`;
    const bodyHash =
      manifestRows.length > 0
        ? (manifestRows[0].body_hash as string)
        : emptyHash;

    await insertEventAndAudit(db, {
      profileId,
      deviceId: DEVICE_ID_CLOUD,
      contextId,
      resourceId,
      eventType: "body_processed",
      action: "process_content",
      bodyHash,
      managedHash: emptyHash,
      openHash: emptyHash,
    });
  }

  console.log(`[content-ingest:store] Done for resource ${resourceId}`);
}
