import { createHash } from "node:crypto";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import {
  bindCodeToFlow,
  consumeCode,
  createPendingFlow,
  rotateRefreshToken,
  storeRefreshToken,
} from "../../../src/oauth/flow.js";
import type { MintedClaims } from "../../../src/oauth/mint.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";

// A real PKCE pair, computed the same way src/oauth/pkce.ts verifies:
// challenge = base64url(sha256(verifier)).
const CODE_VERIFIER = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CODE_CHALLENGE = createHash("sha256").update(CODE_VERIFIER).digest("base64url");

const CLAIMS: MintedClaims = { sub: "user-1", email: "user-1@example.com", email_verified: true };

function futureDate(secondsFromNow = 300): Date {
  return new Date(Date.now() + secondsFromNow * 1000);
}

function pastDate(secondsAgo = 300): Date {
  return new Date(Date.now() - secondsAgo * 1000);
}

describe("oauth flow store", () => {
  let sql: postgres.Sql;
  let db: NeonClient;

  beforeAll(() => {
    ({ sql, db } = makeTestDb());
  });

  beforeEach(async () => {
    await truncateOauthTables(sql);
  });

  afterAll(async () => {
    await sql.end();
  });

  async function seedPendingFlow(relayState: string) {
    await createPendingFlow(db, {
      relayState,
      clientId: "client-1",
      redirectUri: "https://client.example.com/callback",
      codeChallenge: CODE_CHALLENGE,
      codeChallengeMethod: "S256",
      oauthState: "opaque-oauth-state",
      audience: "https://api.example.com",
      expiresAt: futureDate(),
    });
  }

  describe("createPendingFlow", () => {
    it("inserts a row with status pending_saml", async () => {
      await seedPendingFlow("relay-1");

      const rows =
        await sql`SELECT status, client_id, redirect_uri FROM kb_oauth_flow WHERE relay_state = 'relay-1'`;
      expect(rows).toHaveLength(1);
      expect(rows[0]?.status).toBe("pending_saml");
      expect(rows[0]?.client_id).toBe("client-1");
      expect(rows[0]?.redirect_uri).toBe("https://client.example.com/callback");
    });
  });

  describe("bindCodeToFlow", () => {
    it("binds a code to a pending flow and returns redirect info", async () => {
      await seedPendingFlow("relay-2");

      const result = await bindCodeToFlow(db, "relay-2", {
        code: "auth-code-1",
        claims: CLAIMS,
        expiresAt: futureDate(),
      });

      expect(result).toEqual({
        redirectUri: "https://client.example.com/callback",
        oauthState: "opaque-oauth-state",
      });

      const rows =
        await sql`SELECT status, code_hash FROM kb_oauth_flow WHERE relay_state = 'relay-2'`;
      expect(rows[0]?.status).toBe("code_issued");
      expect(rows[0]?.code_hash).toBeTruthy();
    });

    it("throws when there is no matching pending flow", async () => {
      await expect(
        bindCodeToFlow(db, "nonexistent-relay", {
          code: "auth-code-x",
          claims: CLAIMS,
          expiresAt: futureDate(),
        }),
      ).rejects.toThrow();
    });

    it("throws when the pending flow has already expired (M1)", async () => {
      await createPendingFlow(db, {
        relayState: "relay-expired",
        clientId: "client-1",
        redirectUri: "https://client.example.com/callback",
        codeChallenge: CODE_CHALLENGE,
        codeChallengeMethod: "S256",
        oauthState: "opaque-oauth-state",
        audience: "https://api.example.com",
        expiresAt: pastDate(),
      });

      await expect(
        bindCodeToFlow(db, "relay-expired", {
          code: "auth-code-expired",
          claims: CLAIMS,
          expiresAt: futureDate(),
        }),
      ).rejects.toThrow();

      const rows = await sql`SELECT status FROM kb_oauth_flow WHERE relay_state = 'relay-expired'`;
      expect(rows[0]?.status).toBe("pending_saml");
    });
  });

  describe("consumeCode", () => {
    it("returns the bound claims for the right code + verifier", async () => {
      await seedPendingFlow("relay-3");
      await bindCodeToFlow(db, "relay-3", {
        code: "auth-code-3",
        claims: CLAIMS,
        expiresAt: futureDate(),
      });

      const claims = await consumeCode(db, "auth-code-3", CODE_VERIFIER, "client-1");

      expect(claims.sub).toBe(CLAIMS.sub);
      expect(claims.email).toBe(CLAIMS.email);
      expect(claims.email_verified).toBe(CLAIMS.email_verified);
    });

    it("throws on a second consumption of the same code (single-use)", async () => {
      await seedPendingFlow("relay-4");
      await bindCodeToFlow(db, "relay-4", {
        code: "auth-code-4",
        claims: CLAIMS,
        expiresAt: futureDate(),
      });

      await consumeCode(db, "auth-code-4", CODE_VERIFIER, "client-1");

      await expect(consumeCode(db, "auth-code-4", CODE_VERIFIER, "client-1")).rejects.toThrow();
    });

    it("throws on a wrong verifier without burning the code", async () => {
      await seedPendingFlow("relay-5");
      await bindCodeToFlow(db, "relay-5", {
        code: "auth-code-5",
        claims: CLAIMS,
        expiresAt: futureDate(),
      });

      await expect(consumeCode(db, "auth-code-5", "wrong-verifier", "client-1")).rejects.toThrow();

      // Still consumable afterward with the right verifier.
      const claims = await consumeCode(db, "auth-code-5", CODE_VERIFIER, "client-1");
      expect(claims.sub).toBe(CLAIMS.sub);
    });

    it("throws on an expired code", async () => {
      await seedPendingFlow("relay-6");
      await bindCodeToFlow(db, "relay-6", {
        code: "auth-code-6",
        claims: CLAIMS,
        expiresAt: pastDate(),
      });

      await expect(consumeCode(db, "auth-code-6", CODE_VERIFIER, "client-1")).rejects.toThrow();
    });

    it("throws when the code is redeemed with the wrong client_id (M5)", async () => {
      await seedPendingFlow("relay-7");
      await bindCodeToFlow(db, "relay-7", {
        code: "auth-code-7",
        claims: CLAIMS,
        expiresAt: futureDate(),
      });

      await expect(
        consumeCode(db, "auth-code-7", CODE_VERIFIER, "some-other-client"),
      ).rejects.toThrow();

      // Still consumable afterward by the client it was actually issued to.
      const claims = await consumeCode(db, "auth-code-7", CODE_VERIFIER, "client-1");
      expect(claims.sub).toBe(CLAIMS.sub);
    });
  });

  describe("storeRefreshToken / rotateRefreshToken", () => {
    it("rotates a stored refresh token and returns claims + clientId", async () => {
      await storeRefreshToken(db, {
        token: "refresh-token-1",
        clientId: "client-9",
        claims: CLAIMS,
        expiresAt: futureDate(60 * 60),
      });

      const result = await rotateRefreshToken(db, "refresh-token-1");

      expect(result.clientId).toBe("client-9");
      expect(result.claims.sub).toBe(CLAIMS.sub);
      expect(result.claims.email).toBe(CLAIMS.email);
      expect(result.claims.email_verified).toBe(CLAIMS.email_verified);
    });

    it("throws on a second rotation of the same token (single-use)", async () => {
      await storeRefreshToken(db, {
        token: "refresh-token-2",
        clientId: "client-9",
        claims: CLAIMS,
        expiresAt: futureDate(60 * 60),
      });

      await rotateRefreshToken(db, "refresh-token-2");

      await expect(rotateRefreshToken(db, "refresh-token-2")).rejects.toThrow();
    });
  });
});
