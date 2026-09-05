import type postgres from "postgres";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { verifyDcrClientSecret } from "../../../src/oauth/register.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";

const { handleClientRegistration } = await import("../../../src/oauth/register.js");

/**
 * The registration store's persistence witness: a Connect-class registration lands a row whose
 * hash verifies the returned secret — and NOTHING lands in kb_machine_clients (the
 * disjointness the migration header calls load-bearing).
 */
describe("DCR persistence (kb_oauth_dcr_clients)", () => {
  let sql: postgres.Sql;
  let db: NeonClient;

  beforeAll(() => {
    process.env.AS_ISSUER = process.env.AS_ISSUER ?? "https://as.test";
    ({ sql, db } = makeTestDb());
  });

  beforeEach(async () => {
    await truncateOauthTables(sql);
  });

  async function registerConnectClient(): Promise<{ clientId: string; secret: string }> {
    const res = await handleClientRegistration(
      new Request("https://as.test/oauth/clients", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          client_name: "probe connector",
          redirect_uris: ["https://connect.vercel.com/callback"],
          grant_types: ["client_credentials"],
          token_endpoint_auth_method: "client_secret_basic",
        }),
      }),
      db,
    );
    expect(res.status).toBe(201);
    const body = (await res.json()) as { client_id: string; client_secret: string };
    return { clientId: body.client_id, secret: body.client_secret };
  }

  it("persists the registration with the secret hash, and the hash verifies", async () => {
    const { clientId, secret } = await registerConnectClient();

    const rows = await sql`
      SELECT client_name, grant_types, redirect_uris, token_endpoint_auth_method
      FROM kb_oauth_dcr_clients WHERE client_id = ${clientId}
    `;
    expect(rows).toHaveLength(1);
    const row = rows[0] as {
      client_name: string;
      grant_types: string[];
      redirect_uris: string[];
      token_endpoint_auth_method: string;
    };
    expect(row.client_name).toBe("probe connector");
    expect(row.grant_types).toEqual(["client_credentials"]);
    expect(row.redirect_uris).toEqual(["https://connect.vercel.com/callback"]);
    expect(row.token_endpoint_auth_method).toBe("client_secret_basic");

    expect(await verifyDcrClientSecret(db, clientId, secret)).toBe(true);
    expect(await verifyDcrClientSecret(db, clientId, "wrong-secret")).toBe(false);
  });

  it("never touches kb_machine_clients — the store disjointness is the containment boundary", async () => {
    const { clientId } = await registerConnectClient();

    const rows = await sql`
      SELECT 1 FROM kb_machine_clients WHERE client_id = ${clientId}
    `;
    expect(rows).toHaveLength(0);
  });
});
