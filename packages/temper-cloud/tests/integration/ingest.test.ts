import type postgres from "postgres";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { NeonClient } from "../../src/db.js";
import { insertResource, updateResourceHash } from "../../src/ingest.js";
import type { IngestMetadata } from "../../src/ingest.js";
import { getTestDb } from "./helpers/db.js";

// The postgres.Sql tagged-template interface is compatible with NeonClient at runtime.
// We cast to avoid the type mismatch in test code.
function asNeonClient(sql: postgres.Sql): NeonClient {
  return sql as unknown as NeonClient;
}

describe("ingest event parity", () => {
  const TEST_PROFILE_ID = "00000000-0000-0000-0004-000000000001";
  const TEST_CONTEXT_ID = "00000000-0000-0000-0003-000000000001";
  const TEST_DOC_TYPE_ID = "00000000-0000-0000-0001-000000000004";

  let sql: postgres.Sql;
  const createdResourceIds: string[] = [];

  beforeAll(() => {
    sql = getTestDb();
  });

  afterEach(async () => {
    for (const id of createdResourceIds) {
      await sql`DELETE FROM kb_resource_audits WHERE resource_id = ${id}::uuid`;
      await sql`DELETE FROM kb_events WHERE resource_id = ${id}::uuid`;
      await sql`DELETE FROM kb_resource_manifests WHERE resource_id = ${id}::uuid`;
      await sql`DELETE FROM kb_resources WHERE id = ${id}::uuid`;
    }
    createdResourceIds.length = 0;
  });

  it("insertResource creates event + audit rows", async () => {
    const meta: IngestMetadata = {
      title: "Integration test resource",
      kb_context_id: TEST_CONTEXT_ID,
      kb_doc_type_id: TEST_DOC_TYPE_ID,
      origin_uri: "test://integration/insert",
    };

    const resource = await insertResource(asNeonClient(sql), meta, "sha256:test123abc", TEST_PROFILE_ID);
    createdResourceIds.push(resource.id);

    const events = await sql`
      SELECT id, event_type, device_id, payload
      FROM kb_events
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created DESC
      LIMIT 1
    `;
    expect(events).toHaveLength(1);
    expect(events[0].event_type).toBe("resource_created");
    expect(events[0].device_id).toBe("vercel-cloud");
    expect(events[0].payload).toMatchObject({
      body_hash: "sha256:test123abc",
    });

    const audits = await sql`
      SELECT resource_id, action, body_hash, device_id
      FROM kb_resource_audits
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created DESC
      LIMIT 1
    `;
    expect(audits).toHaveLength(1);
    expect(audits[0].action).toBe("create");
    expect(audits[0].body_hash).toBe("sha256:test123abc");
    expect(audits[0].device_id).toBe("vercel-cloud");
  });

  it("updateResourceHash creates body_updated event + audit", async () => {
    const meta: IngestMetadata = {
      title: "Update test resource",
      kb_context_id: TEST_CONTEXT_ID,
      kb_doc_type_id: TEST_DOC_TYPE_ID,
      origin_uri: "test://integration/update",
    };

    const resource = await insertResource(asNeonClient(sql), meta, "sha256:original", TEST_PROFILE_ID);
    createdResourceIds.push(resource.id);

    await updateResourceHash(asNeonClient(sql), resource.id, "sha256:updated", TEST_PROFILE_ID, TEST_CONTEXT_ID);

    const events = await sql`
      SELECT event_type FROM kb_events
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created ASC
    `;
    expect(events).toHaveLength(2);
    expect(events[0].event_type).toBe("resource_created");
    expect(events[1].event_type).toBe("body_updated");

    const audits = await sql`
      SELECT action, body_hash FROM kb_resource_audits
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created ASC
    `;
    expect(audits).toHaveLength(2);
    expect(audits[0].action).toBe("create");
    expect(audits[0].body_hash).toBe("sha256:original");
    expect(audits[1].action).toBe("update_body");
    expect(audits[1].body_hash).toBe("sha256:updated");
  });
});
