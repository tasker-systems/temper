import { createHash } from "node:crypto";
import { createLocalJWKSet, exportPKCS8, generateKeyPair, jwtVerify } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { handleToken } from "../../../src/oauth/endpoints.js";
import { bindCodeToFlow, createPendingFlow } from "../../../src/oauth/flow.js";
import { getPublicJwks } from "../../../src/oauth/keys.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";

interface TokenSuccessBody {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token: string;
}

interface TokenErrorBody {
  error: string;
}

async function seedCode(
  db: NeonClient,
): Promise<{ code: string; verifier: string; relayState: string }> {
  const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const relayState = "rs-1";
  const code = "code-1";

  await createPendingFlow(db, {
    relayState,
    clientId: "cli",
    redirectUri: "http://localhost/cb",
    codeChallenge: challenge,
    codeChallengeMethod: "S256",
    oauthState: "st",
    audience: "aud",
    expiresAt: new Date(Date.now() + 600000),
  });
  await bindCodeToFlow(db, relayState, {
    code,
    claims: { sub: "u1", email: "u1@example.com", email_verified: true },
    expiresAt: new Date(Date.now() + 300000),
  });

  return { code, verifier, relayState };
}

function tokenRequest(body: Record<string, string>): Request {
  return new Request("https://as/oauth/token", {
    method: "POST",
    body: new URLSearchParams(body),
  });
}

describe("handleToken", () => {
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
    await truncateOauthTables(sql);
  });

  it("authorization_code: mints a verifiable access token + refresh token, code is single-use", async () => {
    const { code, verifier } = await seedCode(db);

    const res = await handleToken(
      tokenRequest({
        grant_type: "authorization_code",
        code,
        code_verifier: verifier,
        client_id: "cli",
      }),
      db,
    );
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("application/json");
    expect(res.headers.get("cache-control")).toBe("no-store");

    const body = (await res.json()) as TokenSuccessBody;
    expect(body.token_type).toBe("Bearer");
    expect(typeof body.expires_in).toBe("number");
    expect(body.refresh_token).toBeTruthy();

    const JWKS = createLocalJWKSet(await getPublicJwks());
    const { payload } = await jwtVerify(body.access_token, JWKS, {
      issuer: process.env.AS_ISSUER,
      audience: process.env.AS_AUDIENCE,
    });
    expect(payload.sub).toBe("u1");
    expect(payload.email).toBe("u1@example.com");
    expect(payload.email_verified).toBe(true);

    // Case 2: the same code cannot be redeemed twice.
    const replay = await handleToken(
      tokenRequest({
        grant_type: "authorization_code",
        code,
        code_verifier: verifier,
        client_id: "cli",
      }),
      db,
    );
    expect(replay.status).toBe(400);
    expect((await replay.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });

    // Case 3: refresh rotation — old refresh token issues a new pair, and is then single-use.
    const rotateRes = await handleToken(
      tokenRequest({ grant_type: "refresh_token", refresh_token: body.refresh_token }),
      db,
    );
    expect(rotateRes.status).toBe(200);
    const rotateBody = (await rotateRes.json()) as TokenSuccessBody;
    // Note: access_token is NOT asserted to differ — EdDSA signing is deterministic, and identical
    // claims + issuer + audience + iat (same second) can legitimately produce a byte-identical JWT.
    // The refresh token is the artifact whose single-use rotation this case is proving.
    expect(rotateBody.refresh_token).not.toBe(body.refresh_token);

    const { payload: rotatedPayload } = await jwtVerify(rotateBody.access_token, JWKS, {
      issuer: process.env.AS_ISSUER,
      audience: process.env.AS_AUDIENCE,
    });
    expect(rotatedPayload.sub).toBe("u1");

    const reuseOldRefresh = await handleToken(
      tokenRequest({ grant_type: "refresh_token", refresh_token: body.refresh_token }),
      db,
    );
    expect(reuseOldRefresh.status).toBe(400);
    expect((await reuseOldRefresh.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
  });

  it("rejects an unsupported grant_type", async () => {
    const res = await handleToken(tokenRequest({ grant_type: "client_credentials" }), db);
    expect(res.status).toBe(400);
    expect((await res.json()) as TokenErrorBody).toEqual({ error: "unsupported_grant_type" });
  });

  it("rejects authorization_code requests missing code or code_verifier", async () => {
    const missingCode = await handleToken(
      tokenRequest({ grant_type: "authorization_code", code_verifier: "v", client_id: "cli" }),
      db,
    );
    expect(missingCode.status).toBe(400);
    expect((await missingCode.json()) as TokenErrorBody).toEqual({ error: "invalid_request" });

    const missingVerifier = await handleToken(
      tokenRequest({ grant_type: "authorization_code", code: "c", client_id: "cli" }),
      db,
    );
    expect(missingVerifier.status).toBe(400);
    expect((await missingVerifier.json()) as TokenErrorBody).toEqual({ error: "invalid_request" });
  });

  it("rejects a refresh_token request missing refresh_token", async () => {
    const res = await handleToken(tokenRequest({ grant_type: "refresh_token" }), db);
    expect(res.status).toBe(400);
    expect((await res.json()) as TokenErrorBody).toEqual({ error: "invalid_request" });
  });
});
