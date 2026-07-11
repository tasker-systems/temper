import { createLocalJWKSet, exportPKCS8, generateKeyPair, jwtVerify } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { handleToken } from "../../../src/oauth/endpoints.js";
import { getPublicJwks } from "../../../src/oauth/keys.js";
import { hashToken } from "../../../src/oauth/mint.js";
import { makeTestDb } from "../helpers/oauth-db.js";

function tokenRequest(body: Record<string, string>): Request {
  return new Request("https://as/oauth/token", {
    method: "POST",
    body: new URLSearchParams(body),
  });
}

/** Seed a temper-issued machine client with a known secret. */
async function seedTemperClient(
  sql: postgres.Sql,
  clientId: string,
  secret: string,
  opts: { previousSecret?: string; previousExpiresInSeconds?: number } = {},
): Promise<void> {
  const profileId = crypto.randomUUID();
  await sql`
    INSERT INTO kb_profiles (id, handle, display_name, email, preferences)
    VALUES (${profileId}, ${`agent-${clientId}`}, ${`agent-${clientId}`}, NULL, '{}')
  `;
  const prevHash = opts.previousSecret ? hashToken(opts.previousSecret) : null;
  const prevExpiry =
    opts.previousExpiresInSeconds != null
      ? new Date(Date.now() + opts.previousExpiresInSeconds * 1000).toISOString()
      : null;
  await sql`
    INSERT INTO kb_machine_clients
      (client_id, issuer, label, profile_id, registered_by_profile_id,
       secret_hash, secret_hash_previous, secret_previous_expires_at)
    VALUES (${clientId}, 'temper', 'test', ${profileId}, ${profileId},
       ${hashToken(secret)}, ${prevHash}, ${prevExpiry})
  `;
}

describe("client_credentials grant", () => {
  let sql: postgres.Sql;
  let db: NeonClient;

  beforeAll(async () => {
    const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
    process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
    process.env.AS_SIGNING_KID = "test-kid-1";
    process.env.AS_ISSUER = "https://issuer.test";
    process.env.AS_AUDIENCE = "https://audience.test";
    process.env.AS_ACCESS_TTL_SECONDS = "900";
    ({ sql, db } = makeTestDb());
  });

  afterAll(async () => {
    await sql.end();
  });

  beforeEach(async () => {
    await sql`TRUNCATE kb_machine_clients CASCADE`;
    await sql`DELETE FROM kb_profiles WHERE handle LIKE 'agent-%'`;
  });

  it("mints a machine access token with the normalize_machine claim shape and no refresh token", async () => {
    await seedTemperClient(sql, "tmpr_cc1", "s3cr3t");

    const res = await handleToken(
      tokenRequest({
        grant_type: "client_credentials",
        client_id: "tmpr_cc1",
        client_secret: "s3cr3t",
      }),
      db,
    );
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      access_token: string;
      token_type: string;
      expires_in: number;
      refresh_token?: string;
    };
    expect(body.token_type).toBe("Bearer");
    expect(body.expires_in).toBe(900);
    expect(body.refresh_token).toBeUndefined();

    const jwks = createLocalJWKSet(await getPublicJwks());
    const { payload } = await jwtVerify(body.access_token, jwks, {
      issuer: "https://issuer.test",
      audience: "https://audience.test",
    });
    expect(payload.gty).toBe("client-credentials");
    expect(payload.azp).toBe("tmpr_cc1");
    expect(payload.sub).toBe("tmpr_cc1@clients");
    expect(payload.email).toBeUndefined();
  });

  it("rejects a wrong secret with invalid_client", async () => {
    await seedTemperClient(sql, "tmpr_cc2", "right");
    const res = await handleToken(
      tokenRequest({
        grant_type: "client_credentials",
        client_id: "tmpr_cc2",
        client_secret: "wrong",
      }),
      db,
    );
    expect(res.status).toBe(401);
    expect(((await res.json()) as { error: string }).error).toBe("invalid_client");
  });

  it("rejects a revoked client", async () => {
    await seedTemperClient(sql, "tmpr_cc3", "s");
    await sql`UPDATE kb_machine_clients SET revoked_at = now() WHERE client_id = 'tmpr_cc3'`;
    const res = await handleToken(
      tokenRequest({ grant_type: "client_credentials", client_id: "tmpr_cc3", client_secret: "s" }),
      db,
    );
    expect(res.status).toBe(401);
  });

  it("accepts the previous secret within its grace window and rejects it after", async () => {
    await seedTemperClient(sql, "tmpr_cc4", "new", {
      previousSecret: "old",
      previousExpiresInSeconds: 3600,
    });
    const ok = await handleToken(
      tokenRequest({
        grant_type: "client_credentials",
        client_id: "tmpr_cc4",
        client_secret: "old",
      }),
      db,
    );
    expect(ok.status).toBe(200);

    await seedTemperClient(sql, "tmpr_cc5", "new", {
      previousSecret: "old",
      previousExpiresInSeconds: -1,
    });
    const expired = await handleToken(
      tokenRequest({
        grant_type: "client_credentials",
        client_id: "tmpr_cc5",
        client_secret: "old",
      }),
      db,
    );
    expect(expired.status).toBe(401);
  });

  it("accepts credentials via HTTP Basic", async () => {
    await seedTemperClient(sql, "tmpr_cc6", "basic-secret");
    const basic = Buffer.from("tmpr_cc6:basic-secret").toString("base64");
    const res = await handleToken(
      new Request("https://as/oauth/token", {
        method: "POST",
        headers: { authorization: `Basic ${basic}` },
        body: new URLSearchParams({ grant_type: "client_credentials" }),
      }),
      db,
    );
    expect(res.status).toBe(200);
  });
});
