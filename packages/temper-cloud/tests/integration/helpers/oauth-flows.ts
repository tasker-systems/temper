import { createHash } from "node:crypto";
import { expect } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { handleToken } from "../../../src/oauth/endpoints.js";
import { bindCodeToFlow, createPendingFlow } from "../../../src/oauth/flow.js";

/**
 * One full login and one refresh, driven through the real endpoint.
 *
 * Shared rather than copied because two suites now need a chain to exist before they can say
 * anything about it, and a second copy of the PKCE pair, the flow binding and the form encoding
 * would drift from this one silently — the tests would still pass, against two different notions
 * of what a login is.
 */

const VERIFIER = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
export const CHALLENGE = createHash("sha256").update(VERIFIER).digest("base64url");

export interface TokenSuccessBody {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token?: string;
}

export interface TokenErrorBody {
  error: string;
}

/**
 * Runs one full login and hands back the raw response, asserting nothing.
 *
 * `login` below is the ordinary caller and asserts 200; a suite that is testing what a login does
 * when it CANNOT succeed needs the response itself, and building the flow rows a second time to
 * get it is how the two notions of a login drift apart.
 */
export async function attemptLogin(
  db: NeonClient,
  opts: { relay: string; code: string; profileId: string | null; audience?: string },
): Promise<Response> {
  const asAudience = process.env.AS_AUDIENCE;
  expect(
    asAudience,
    "the suite must configure AS_AUDIENCE — the login fixture is authorized for the instance audience",
  ).toBeTruthy();
  await createPendingFlow(db, {
    relayState: opts.relay,
    clientId: "cli",
    redirectUri: "http://localhost/cb",
    codeChallenge: CHALLENGE,
    codeChallengeMethod: "S256",
    oauthState: "st",
    // The instance audience — what a login with no requested resource is authorized for, and the
    // value `/oauth/authorize` would store. Must be a SERVED audience: the chain write
    // (`storeRefreshToken`) validates fail-closed against `servedAudiences()`, so a fixture
    // audience the instance does not serve now fails the login it is meant to set up. A suite
    // testing a resource-scoped login overrides it — and is responsible for the env (e.g.
    // MCP_AUDIENCE) that makes that override served.
    audience: opts.audience ?? (asAudience as string),
    expiresAt: new Date(Date.now() + 600000),
  });
  await bindCodeToFlow(db, opts.relay, {
    code: opts.code,
    claims: { sub: "u1", email: "u1@example.com", email_verified: true },
    expiresAt: new Date(Date.now() + 300000),
    profileId: opts.profileId,
  });

  return handleToken(
    new Request("https://as/oauth/token", {
      method: "POST",
      body: new URLSearchParams({
        grant_type: "authorization_code",
        code: opts.code,
        code_verifier: VERIFIER,
        client_id: "cli",
      }),
    }),
    db,
  );
}

/** Runs one full login, returning the first refresh token of a brand-new chain. */
export async function login(
  db: NeonClient,
  opts: { relay: string; code: string; profileId: string | null; audience?: string },
): Promise<TokenSuccessBody> {
  const res = await attemptLogin(db, opts);
  expect(res.status).toBe(200);
  return (await res.json()) as TokenSuccessBody;
}

/** Presents a refresh token at the token endpoint. Returns the raw response — callers judge it. */
export function refresh(db: NeonClient, refreshToken: string | undefined): Promise<Response> {
  return handleToken(
    new Request("https://as/oauth/token", {
      method: "POST",
      body: new URLSearchParams({ grant_type: "refresh_token", refresh_token: refreshToken ?? "" }),
    }),
    db,
  );
}
