import { chunkText } from "../../packages/temper-cloud/src/processing/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/processing/embed.js";
import {
  chunksToJsonb,
  type ChunkRow,
} from "../../packages/temper-cloud/src/processing/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";
import { insertEventAndAudit, DEVICE_ID_CLOUD } from "../../packages/temper-cloud/src/events.js";
import { createHash } from "node:crypto";

export async function processIngest(
  resourceId: string,
  markdown: string,
  profileId: string,
  contextId: string,
) {
  "use workflow";

  const chunks = await chunkStep(markdown);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(resourceId, profileId, contextId, markdown, chunks, embeddings);
}

async function chunkStep(
  markdown: string,
): Promise<
  Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>
> {
  "use step";
  return chunkText(markdown);
}

async function embedStep(texts: string[]): Promise<number[][]> {
  "use step";
  return embedTexts(texts);
}

async function storeStep(
  resourceId: string,
  profileId: string,
  contextId: string,
  bodyText: string,
  chunks: Array<{
    header_path: string;
    content: string;
    content_hash: string;
    chunk_index: number;
  }>,
  embeddings: number[][],
): Promise<void> {
  "use step";

  if (chunks.length === 0) return;

  const db = getDb();

  const bodyHash = `sha256:${createHash("sha256").update(bodyText).digest("hex")}`;

  const { auditId } = await insertEventAndAudit(db, {
    profileId,
    deviceId: DEVICE_ID_CLOUD,
    contextId,
    resourceId,
    eventType: "body_updated",
    action: "update_body",
    bodyHash,
    managedHash: "",
    openHash: "",
  });

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
  await db`
    SELECT persist_resource_chunks(
      ${resourceId}::uuid, ${auditId}::uuid, ${bodyHash}, ${chunksJson}::jsonb
    )
  `;
}
