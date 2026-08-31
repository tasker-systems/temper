import { createLocalJWKSet, exportPKCS8, generateKeyPair, jwtVerify } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { storeRefreshToken } from "../../../src/oauth/flow.js";
import { getPublicJwks } from "../../../src/oauth/keys.js";
import { hashToken } from "../../../src/oauth/mint.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";
import {
  login,
  refresh,
  type TokenErrorBody,
  type TokenSuccessBody,
} from "../helpers/oauth-flows.js";

/**
 * The refresh chain carries the RFC 8707 resource its login was authorized for.
 *
 * The authorization-code grant has minted the flow's audience since it captured the requested
 * `resource`; the refresh grant could not follow, because the chain row recorded no audience —
 * so every rotated session silently switched to the instance audience and survived only through
 * the MCP middleware's dual-accept. These tests pin the fix end to end: stamped at chain start,
 * inherited by every successor, minted into each access token, with the legacy NULL fallback and
 * the write-time fail-closed refusal beside them.
 */

const MCP_AUDIENCE = "https://inst.test/mcp";

/** A successful login/rotation must mint a refresh token — assert it, then hand it over. */
function need(t: string | undefined): string {
  expect(t, "a successful token response must carry a refresh_token").toBeTruthy();
  return t as string;
}

describe("refresh chain audience", () => {
  let sql: postgres.Sql;
  let db: NeonClient;

  async function chainRow(refreshToken: string) {
    const rows = await sql`
      SELECT audience FROM kb_oauth_refresh_tokens WHERE token_hash = ${hashToken(refreshToken)}`;
    return rows[0] as { audience: string | null };
  }

  async function tokenAud(accessToken: string): Promise<string> {
    const JWKS = createLocalJWKSet(await getPublicJwks());
    const { payload } = await jwtVerify(accessToken, JWKS, { issuer: process.env.AS_ISSUER });
    return payload.aud as string;
  }

  beforeAll(async () => {
    const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
    process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
    process.env.AS_SIGNING_KID = "test-kid-1";
    process.env.AS_ISSUER = "https://issuer.test";
    process.env.AS_AUDIENCE = "https://audience.test";
    process.env.AS_ACCESS_TTL_SECONDS = "900";
    process.env.AS_CLIENTS = JSON.stringify({ cli: ["http://localhost/cb"] });
    ({ sql, db } = makeTestDb());
  });

  afterAll(async () => {
    await sql.end();
  });

  beforeEach(async () => {
    await truncateOauthTables(sql);
    delete process.env.MCP_AUDIENCE;
  });

  it("stamps the flow's audience at chain start and preserves it across rotations", async () => {
    process.env.MCP_AUDIENCE = MCP_AUDIENCE;
    const first = await login(db, {
      relay: "rs-carry",
      code: "c-carry",
      profileId: null,
      audience: MCP_AUDIENCE,
    });
    expect(await chainRow(need(first.refresh_token))).toEqual({ audience: MCP_AUDIENCE });
    expect(await tokenAud(first.access_token)).toBe(MCP_AUDIENCE);

    // Two rotations — the audience must be byte-identical down the chain, the same inheritance
    // rule the chain deadline follows. A refreshed MCP session ends up on the resource its login
    // was authorized for, with nothing owed to the middleware's dual-accept.
    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;
    const third = (await (await refresh(db, second.refresh_token)).json()) as TokenSuccessBody;

    expect(await tokenAud(second.access_token)).toBe(MCP_AUDIENCE);
    expect(await tokenAud(third.access_token)).toBe(MCP_AUDIENCE);
    expect(await chainRow(need(second.refresh_token))).toEqual({ audience: MCP_AUDIENCE });
    expect(await chainRow(need(third.refresh_token))).toEqual({ audience: MCP_AUDIENCE });
  });

  it("keeps a legacy chain's refresh working on the instance audience, and the NULL inherits", async () => {
    // A chain minted before the column existed carries no audience — simulated here by nulling
    // it on an otherwise ordinary chain. Its refresh must still succeed (a deploy must not log
    // existing sessions out), mint the instance audience the way it always has, and leave the
    // successor NULL rather than inventing a value the login never asked for.
    const issued = await login(db, { relay: "rs-legacy", code: "c-legacy", profileId: null });
    await sql`
      UPDATE kb_oauth_refresh_tokens SET audience = NULL
       WHERE token_hash = ${hashToken(need(issued.refresh_token))}`;

    const res = await refresh(db, issued.refresh_token);
    expect(res.status).toBe(200);
    const body = (await res.json()) as TokenSuccessBody;
    expect(await tokenAud(body.access_token)).toBe("https://audience.test");
    expect(await chainRow(need(body.refresh_token))).toEqual({ audience: null });
  });

  it("answers a config-drifted rotation with 503 and withdraws the rotation mark", async () => {
    // The served set is env; if it narrows between authorize and a later rotation, the chain
    // write throws — AFTER the rotation guard has already revoked and marked the presented
    // token. Letting that throw escape would answer a platform 500, leave the mark claiming a
    // rotation that produced no successor, and turn the client's retry into a recorded replay.
    // The endpoint catches: named log, mark withdrawn, well-formed 503.
    process.env.MCP_AUDIENCE = MCP_AUDIENCE;
    const issued = await login(db, {
      relay: "rs-drift",
      code: "c-drift",
      profileId: null,
      audience: MCP_AUDIENCE,
    });
    delete process.env.MCP_AUDIENCE;

    const res = await refresh(db, issued.refresh_token);
    expect(res.status).toBe(503);
    expect((await res.json()) as TokenErrorBody).toEqual({ error: "temporarily_unavailable" });

    // The token is spent (revoked) but NOT marked as rotated — a retry is a plain failure, not
    // evidence of theft — and no successor row of any kind was minted.
    const rows = await sql`
      SELECT revoked_at, rotated_at FROM kb_oauth_refresh_tokens
       WHERE token_hash = ${hashToken(need(issued.refresh_token))}`;
    const row = rows[0] as { revoked_at: Date | null; rotated_at: Date | null };
    expect(row.revoked_at).not.toBeNull();
    expect(row.rotated_at).toBeNull();
    const successors =
      await sql`SELECT count(*)::int AS n FROM kb_oauth_refresh_tokens WHERE audience = ${MCP_AUDIENCE}`;
    expect((successors[0] as { n: number }).n).toBe(1); // the original row only
  });

  it("refuses to persist a chain audience the instance does not serve", async () => {
    // Same fail-closed discipline as the authorize handler, asked at the write: an out-of-set
    // audience can only be config drift or a misbehaving caller (the flow row is already
    // authorize-validated), so the refusal is a loud throw — and, because it precedes the
    // INSERT, no row of any kind is persisted.
    const before = await sql`SELECT count(*)::int AS n FROM kb_oauth_refresh_tokens`;
    await expect(
      storeRefreshToken(db, {
        token: "t-out-of-set",
        clientId: "cli",
        claims: { sub: "u1", email: "u1@example.com", email_verified: true },
        expiresAt: new Date(Date.now() + 60000),
        chainExpiresAt: new Date(Date.now() + 60000).toISOString(),
        chainId: null,
        profileId: null,
        audience: "https://evil.test/unrelated",
      }),
    ).rejects.toThrow(/not served by this authorization server/);
    const after = await sql`SELECT count(*)::int AS n FROM kb_oauth_refresh_tokens`;
    expect((after[0] as { n: number }).n).toBe((before[0] as { n: number }).n);
  });
});
