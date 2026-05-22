import type postgres from "postgres";
import { beforeAll, describe, expect, it } from "vitest";
import type { NeonClient } from "../../src/db.js";
import { insertEventAndAudit } from "../../src/events.js";
import { type ChunkRow, chunksToJsonb } from "../../src/processing/store.js";
import { getTestDb } from "./helpers/db.js";

// The postgres.Sql tagged-template interface is compatible with NeonClient at runtime.
// We cast to avoid the type mismatch in test code.
function asNeonClient(sql: postgres.Sql): NeonClient {
  return sql as unknown as NeonClient;
}

describe("TS workflow chunk persistence with revision", () => {
  const TEST_PROFILE_ID = "00000000-0000-0000-0004-000000000001";
  const TEST_CONTEXT_ID = "00000000-0000-0000-0003-000000000001";
  const TEST_DOC_TYPE_ID = "00000000-0000-0000-0001-000000000004";

  let sql: postgres.Sql;

  beforeAll(() => {
    sql = getTestDb();
  });

  // No afterEach teardown: kb_events is append-only and a resource that has
  // events cannot be hard-deleted. The test uses gen_random_uuid() for every
  // identifier and asserts only on its own ids, so leftover rows are harmless.

  it("calls persist_resource_chunks with audit_id and body_hash", async () => {
    const insertResult = await sql`
      INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
                                originator_profile_id, owner_profile_id, created, updated)
      VALUES (gen_random_uuid(), ${TEST_CONTEXT_ID}::uuid, ${TEST_DOC_TYPE_ID}::uuid,
              'test://r-' || substr(gen_random_uuid()::text, 1, 8), 'T',
              't-' || substr(gen_random_uuid()::text, 1, 8),
              ${TEST_PROFILE_ID}::uuid, ${TEST_PROFILE_ID}::uuid, now(), now())
      RETURNING id
    `;
    const resourceId = insertResult[0].id as string;

    const { auditId } = await insertEventAndAudit(asNeonClient(sql), {
      profileId: TEST_PROFILE_ID,
      deviceId: "vercel-cloud",
      contextId: TEST_CONTEXT_ID,
      resourceId,
      eventType: "body_updated",
      action: "update_body",
      bodyHash: "body-abc",
      managedHash: "mh",
      openHash: "oh",
    });

    const chunkRows: ChunkRow[] = [
      {
        id: "",
        resource_id: resourceId,
        chunk_index: 0,
        version: 0,
        header_path: "",
        content: "hi",
        content_hash: "h0",
        embedding: new Array(768).fill(0),
      },
    ];
    const chunksJsonb = sql.json(chunksToJsonb(chunkRows));

    const revResult = await sql.unsafe(
      "SELECT persist_resource_chunks($1::uuid, $2::uuid, $3::text, $4)",
      [resourceId, auditId, "body-abc", chunksJsonb],
    );
    const revId = revResult[0].persist_resource_chunks as string;
    expect(revId).toMatch(/^[0-9a-f-]{36}$/);

    const revRows = await sql`
      SELECT audit_id, body_hash, chunk_count FROM kb_resource_revisions WHERE id = ${revId}::uuid
    `;
    const rev = revRows[0];
    expect(rev.audit_id).toBe(auditId);
    expect(rev.body_hash).toBe("body-abc");
    expect(rev.chunk_count).toBe(1);
  });
});
