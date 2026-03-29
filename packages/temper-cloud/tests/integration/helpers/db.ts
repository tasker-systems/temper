import { randomUUID } from "node:crypto";
import postgres from "postgres";

const TEST_DATABASE_URL =
  process.env.TEST_DATABASE_URL ?? "postgresql://temper:temper@localhost:5437/temper_development";

export function getTestDb() {
  return postgres(TEST_DATABASE_URL);
}

// Well-known seed IDs from migrations/20260326000002_r2_seed.sql
const SYSTEM_PROFILE_ID = "00000000-0000-0000-0004-000000000001";
const TEMPER_CONTEXT_ID = "00000000-0000-0000-0003-000000000001";
const SOURCE_DOC_TYPE_ID = "00000000-0000-0000-0001-000000000007";

export interface TestResource {
  id: string;
  profileId: string;
  contextId: string;
}

export async function createTestResource(sql: postgres.Sql, title: string): Promise<TestResource> {
  const id = randomUUID();
  const now = new Date().toISOString();

  await sql`
    INSERT INTO resources (id, kb_context_id, kb_doc_type_id, uri, title, slug,
                           originator_profile_id, owner_profile_id, created, updated)
    VALUES (${id}, ${TEMPER_CONTEXT_ID}, ${SOURCE_DOC_TYPE_ID},
            ${`test://${id}`}, ${title}, ${`test-${id}`},
            ${SYSTEM_PROFILE_ID}, ${SYSTEM_PROFILE_ID},
            ${now}::timestamptz, ${now}::timestamptz)
  `;

  return { id, profileId: SYSTEM_PROFILE_ID, contextId: TEMPER_CONTEXT_ID };
}

export async function createTestBlobFile(sql: postgres.Sql, resourceId: string): Promise<string> {
  const id = randomUUID();

  await sql`
    INSERT INTO blob_files (id, profile_id, resource_id, blob_url, pathname, content_type, status)
    VALUES (${id}, ${SYSTEM_PROFILE_ID}, ${resourceId},
            ${`https://blob.test/${id}`}, ${`test/${id}.md`},
            'text/markdown', 'pending')
  `;

  return id;
}

export async function cleanupTestResource(sql: postgres.Sql, resourceId: string): Promise<void> {
  await sql`DELETE FROM kb_chunks WHERE resource_id = ${resourceId}`;
  await sql`DELETE FROM blob_files WHERE resource_id = ${resourceId}`;
  await sql`DELETE FROM resources WHERE id = ${resourceId}`;
}
