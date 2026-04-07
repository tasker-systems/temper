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
import { logger } from "../../packages/temper-cloud/src/logger.js";

export async function processContentIngest(
  resourceId: string,
  content: string,
  replace: boolean,
  profileId: string,
  contextId?: string,
  bodyHash?: string,
) {
  "use workflow";

  logger.info({ resourceId, replace }, "content-ingest: starting processing");
  const chunks = await chunkStep(content);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(resourceId, chunks, embeddings, replace, profileId, contextId, bodyHash);
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
  logger.info({ contentLength: text.length }, "content-ingest: chunking");
  const chunks = chunkText(text);
  logger.info({ chunkCount: chunks.length }, "content-ingest: chunking complete");
  return chunks;
}

async function embedStep(texts: string[]): Promise<number[][]> {
  "use step";
  logger.info({ chunkCount: texts.length }, "content-ingest: embedding");
  const embeddings = await embedTexts(texts);
  logger.info("content-ingest: embedding complete");
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
  passedContextId?: string,
  passedBodyHash?: string,
): Promise<void> {
  "use step";

  logger.info({ chunkCount: chunks.length, resourceId, replace }, "content-ingest: storing chunks");
  const db = getDb();

  // Verify the profile can modify this resource before writing anything
  const canModify = await db`
    SELECT true FROM can_modify_resource(${profileId}::uuid, ${resourceId}::uuid)
  `;
  if (canModify.length === 0) {
    logger.warn({ resourceId, profileId }, "content-ingest: profile cannot modify resource, aborting");
    return;
  }

  // Resolve contextId and bodyHash — use passed values or fall back to scoped queries
  const emptyHash = canonicalJsonHash({});
  let contextId = passedContextId;
  let bodyHash = passedBodyHash;

  if (!contextId) {
    const contextRows = await db`
      SELECT r.kb_context_id FROM kb_resources r
      WHERE r.id = ${resourceId}::uuid
    `;
    contextId = contextRows.length > 0
      ? (contextRows[0].kb_context_id as string)
      : undefined;
    if (!contextId) {
      logger.warn({ resourceId }, "content-ingest: resource not found, aborting");
      return;
    }
  }

  if (!bodyHash) {
    const manifestRows = await db`
      SELECT body_hash FROM kb_resource_manifests
      WHERE resource_id = ${resourceId}::uuid
    `;
    bodyHash = manifestRows.length > 0
      ? (manifestRows[0].body_hash as string)
      : emptyHash;
  }

  // Now safe to write chunks
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

  logger.info({ resourceId }, "content-ingest: store complete");
}
