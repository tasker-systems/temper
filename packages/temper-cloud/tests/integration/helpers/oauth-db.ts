import postgres from "postgres";
import type { NeonClient } from "../../../src/db.js";

/**
 * Builds a DB connection for OAuth flow-store integration tests. node-saml's
 * `neon()` driver cannot connect to local Postgres, so tests use the
 * `postgres` package instead, cast to `NeonClient` — its tagged-template
 * call signature (`await sql\`...\`` returning a rows array) is compatible
 * with how the store functions use their `db` parameter.
 */
export function makeTestDb(): { sql: postgres.Sql; db: NeonClient } {
  const url = process.env.TEST_DATABASE_URL ?? process.env.DATABASE_URL;
  if (!url) {
    throw new Error("TEST_DATABASE_URL or DATABASE_URL is required for integration tests");
  }
  const sql = postgres(url);
  return { sql, db: sql as unknown as NeonClient };
}

/** Clears all OAuth/SAML AS tables between tests. */
export async function truncateOauthTables(sql: postgres.Sql): Promise<void> {
  await sql`TRUNCATE kb_oauth_flow, kb_oauth_refresh_tokens, kb_saml_replay, kb_saml_idp RESTART IDENTITY CASCADE`;
}
