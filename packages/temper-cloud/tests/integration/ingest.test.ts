import { randomUUID } from "node:crypto";
import type postgres from "postgres";
import { beforeAll, describe, expect, it } from "vitest";
import type { NeonClient } from "../../src/db.js";
import type { IngestMetadata } from "../../src/ingest.js";
import { insertResource, updateResourceHash } from "../../src/ingest.js";
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

  beforeAll(() => {
    sql = getTestDb();
  });

  // No afterEach teardown: kb_events is append-only and a resource that has
  // events cannot be hard-deleted. Each test uses a unique origin_uri and
  // asserts only on its own resource_id, so leftover rows are harmless.

  it("insertResource creates event + audit rows", async () => {
    const meta: IngestMetadata = {
      title: "Integration test resource",
      kb_context_id: TEST_CONTEXT_ID,
      kb_doc_type_id: TEST_DOC_TYPE_ID,
      origin_uri: `test://integration/insert/${randomUUID()}`,
    };

    const resource = await insertResource(
      asNeonClient(sql),
      meta,
      "sha256:test123abc",
      TEST_PROFILE_ID,
    );

    const events = await sql`
      SELECT e.id, et.name AS event_type, e.device_id, e.payload
      FROM kb_events e
      JOIN kb_event_types et ON et.id = e.event_type_id
      WHERE e.resource_id = ${resource.id}::uuid
      ORDER BY e.created DESC
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
      origin_uri: `test://integration/update/${randomUUID()}`,
    };

    const resource = await insertResource(
      asNeonClient(sql),
      meta,
      "sha256:original",
      TEST_PROFILE_ID,
    );

    await updateResourceHash(
      asNeonClient(sql),
      resource.id,
      "sha256:updated",
      TEST_PROFILE_ID,
      TEST_CONTEXT_ID,
    );

    const events = await sql`
      SELECT et.name AS event_type
      FROM kb_events e
      JOIN kb_event_types et ON et.id = e.event_type_id
      WHERE e.resource_id = ${resource.id}::uuid
      ORDER BY e.created ASC
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
