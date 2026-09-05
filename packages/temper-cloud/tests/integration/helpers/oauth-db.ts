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

/**
 * Clears all OAuth/SAML AS tables between tests. `kb_oauth_refresh_replays` is named explicitly
 * even though CASCADE would reach it through its foreign key — a test that reads a replay count
 * should be able to see, here, that it starts from zero.
 *
 * `kb_internal_call_health` is here for a sharper reason than tidiness: it has no foreign key to
 * anything, so CASCADE does not reach it, and its rows carry a `consecutive_failures` counter that
 * accumulates ACROSS tests. A test asserting "one failure was recorded" would pass on its own and
 * read whatever the preceding tests happened to leave when the file runs in order.
 */
export async function truncateOauthTables(sql: postgres.Sql): Promise<void> {
  await sql`TRUNCATE kb_oauth_flow, kb_oauth_refresh_tokens, kb_oauth_refresh_replays, kb_saml_replay, kb_saml_idp, kb_internal_call_health, kb_oauth_dcr_clients RESTART IDENTITY CASCADE`;
}
