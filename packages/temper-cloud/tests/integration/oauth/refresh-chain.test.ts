import { createHash } from "node:crypto";
import { exportPKCS8, generateKeyPair } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { handleToken } from "../../../src/oauth/endpoints.js";
import { bindCodeToFlow, createPendingFlow } from "../../../src/oauth/flow.js";
import { hashToken } from "../../../src/oauth/mint.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";

/**
 * The refresh chain's absolute bound, and the admission gate in front of rotation.
 *
 * Two properties an operator should be able to state about a long-lived session: how long it can
 * live at most, and who can end it. The bound is a property of the CHAIN — stamped at the last full
 * SAML login and inherited by every successor — so rotation cannot extend it, and only another
 * login (which reconciles against the IdP) moves it. The gate is the admission terminal set, asked
 * at the token endpoint rather than left entirely to the API gate the minted token later meets.
 */

const VERIFIER = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE = createHash("sha256").update(VERIFIER).digest("base64url");

interface TokenSuccessBody {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token?: string;
}

interface TokenErrorBody {
  error: string;
}

/** Runs one full login, returning the first refresh token of a brand-new chain. */
async function login(
  db: NeonClient,
  opts: { relay: string; code: string; profileId: string | null },
): Promise<TokenSuccessBody> {
  await createPendingFlow(db, {
    relayState: opts.relay,
    clientId: "cli",
    redirectUri: "http://localhost/cb",
    codeChallenge: CHALLENGE,
    codeChallengeMethod: "S256",
    oauthState: "st",
    audience: "aud",
    expiresAt: new Date(Date.now() + 600000),
  });
  await bindCodeToFlow(db, opts.relay, {
    code: opts.code,
    claims: { sub: "u1", email: "u1@example.com", email_verified: true },
    expiresAt: new Date(Date.now() + 300000),
    profileId: opts.profileId,
  });

  const res = await handleToken(
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
  expect(res.status).toBe(200);
  return (await res.json()) as TokenSuccessBody;
}

function refresh(db: NeonClient, refreshToken: string | undefined): Promise<Response> {
  return handleToken(
    new Request("https://as/oauth/token", {
      method: "POST",
      body: new URLSearchParams({ grant_type: "refresh_token", refresh_token: refreshToken ?? "" }),
    }),
    db,
  );
}

describe("refresh chain bound + admission gate", () => {
  let sql: postgres.Sql;
  let db: NeonClient;
  const handles: string[] = [];

  /** A real principal in a named standing, written the way a fixture does — directly. */
  async function principal(state: string): Promise<string> {
    const handle = `chain-test-${state}-${Date.now()}-${handles.length}`;
    handles.push(handle);
    const rows = await sql`
      INSERT INTO kb_profiles (handle, display_name, email, preferences)
      VALUES (${handle}, ${handle}, ${`${handle}@example.test`}, '{}')
      RETURNING id`;
    const id = (rows[0] as { id: string }).id;
    await sql`INSERT INTO kb_principal_standing (profile_id, state) VALUES (${id}, ${state})`;
    return id;
  }

  async function chainRow(refreshToken: string) {
    const rows = await sql`
      SELECT expires_at, chain_expires_at, profile_id
        FROM kb_oauth_refresh_tokens WHERE token_hash = ${hashToken(refreshToken)}`;
    return rows[0] as {
      expires_at: Date;
      chain_expires_at: Date;
      profile_id: string | null;
    };
  }

  beforeAll(async () => {
    const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
    process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
    process.env.AS_SIGNING_KID = "test-kid-1";
    process.env.AS_ISSUER = "https://issuer.test";
    process.env.AS_AUDIENCE = "https://audience.test";
    process.env.AS_ACCESS_TTL_SECONDS = "900";
    process.env.AS_REFRESH_TTL_SECONDS = "2592000";
    process.env.AS_CLIENTS = JSON.stringify({ cli: ["http://localhost/cb"] });
    ({ sql, db } = makeTestDb());
  });

  afterAll(async () => {
    for (const handle of handles) {
      await sql`DELETE FROM kb_profiles WHERE handle = ${handle}`;
    }
    await sql.end();
  });

  beforeEach(async () => {
    await truncateOauthTables(sql);
    process.env.AS_REFRESH_CHAIN_MAX_SECONDS = "7776000";
  });

  it("stamps the chain deadline once, and rotation does not move it", async () => {
    const first = await login(db, { relay: "rs-inherit", code: "c-inherit", profileId: null });
    const firstRow = await chainRow(first.refresh_token);

    const second = (await (await refresh(db, first.refresh_token)).json()) as TokenSuccessBody;
    const secondRow = await chainRow(second.refresh_token);
    const third = (await (await refresh(db, second.refresh_token)).json()) as TokenSuccessBody;
    const thirdRow = await chainRow(third.refresh_token);

    // The bound is byte-identical down the chain. This is the property the whole design rests on:
    // a deadline that belongs to the chain, not to whichever token is currently held.
    expect(secondRow.chain_expires_at.getTime()).toBe(firstRow.chain_expires_at.getTime());
    expect(thirdRow.chain_expires_at.getTime()).toBe(firstRow.chain_expires_at.getTime());

    // …while the per-token TTL does still slide. Asserting only the first would pass against a
    // build that froze both, which would be a different (and wrong) change.
    expect(thirdRow.expires_at.getTime()).toBeGreaterThanOrEqual(firstRow.expires_at.getTime());
  });

  it("refuses a rotation once the chain has ended, however live the token itself is", async () => {
    const issued = await login(db, { relay: "rs-capped", code: "c-capped", profileId: null });

    // Only the CHAIN is moved into the past. `expires_at` stays a month out and `revoked_at` stays
    // NULL, so the other two guards both say yes — the refusal can come from nowhere but the chain
    // bound, which is what makes this an assertion about that bound and not about its neighbours.
    await sql`
      UPDATE kb_oauth_refresh_tokens
         SET chain_expires_at = now() - interval '1 second'
       WHERE token_hash = ${hashToken(issued.refresh_token)}`;
    const row = await chainRow(issued.refresh_token);
    expect(row.expires_at.getTime()).toBeGreaterThan(Date.now());

    const res = await refresh(db, issued.refresh_token);
    expect(res.status).toBe(400);
    expect((await res.json()) as TokenErrorBody).toEqual({ error: "invalid_grant" });
  });

  it("never advertises an expiry the chain bound will not honour", async () => {
    // A chain shorter than one token's TTL — the case where an unclamped `now() + refreshTtl`
    // would hand a client a token dated past its own chain's end.
    process.env.AS_REFRESH_CHAIN_MAX_SECONDS = "3600";
    const issued = await login(db, { relay: "rs-clamp", code: "c-clamp", profileId: null });
    const row = await chainRow(issued.refresh_token);

    expect(row.expires_at.getTime()).toBe(row.chain_expires_at.getTime());
  });

  it("gives a principal whose admission has ended a token but not a renewable session", async () => {
    // Login is never refused on admission — a revoked principal needs a token to reach the ungated
    // review-request surface. What they must not get is a fresh chain, because that would undo an
    // administrator's revoke on the subject's very next sign-in.
    const revoked = await principal("revoked");
    const issued = await login(db, { relay: "rs-revoked", code: "c-revoked", profileId: revoked });

    expect(issued.access_token, "the self-service surface must stay reachable").toBeTruthy();
    expect(issued.refresh_token, "a terminal principal gets no renewable session").toBeUndefined();

    const live = await sql`
      SELECT count(*)::int AS n FROM kb_oauth_refresh_tokens
       WHERE profile_id = ${revoked} AND revoked_at IS NULL`;
    expect((live[0] as { n: number }).n, "a revoke must not be undone by logging in again").toBe(0);
  });

  it("refuses a deactivated principal too — both terminals, not just the one", async () => {
    const off = await principal("deactivated");
    const issued = await login(db, { relay: "rs-deact", code: "c-deact", profileId: off });
    expect(issued.refresh_token).toBeUndefined();

    // And a chain minted BEFORE the terminal is refused at its next rotation, not merely absent.
    const approved = await principal("approved");
    const live = await login(db, { relay: "rs-deact2", code: "c-deact2", profileId: approved });
    await sql`UPDATE kb_principal_standing SET state = 'deactivated' WHERE profile_id = ${approved}`;
    expect((await refresh(db, live.refresh_token)).status).toBe(400);
  });

  it("leaves a still-admitted principal's ordinary refresh untouched", async () => {
    const approved = await principal("approved");
    const issued = await login(db, { relay: "rs-ok", code: "c-ok", profileId: approved });

    const res = await refresh(db, issued.refresh_token);
    expect(res.status).toBe(200);
    const body = (await res.json()) as TokenSuccessBody;
    expect(body.refresh_token).not.toBe(issued.refresh_token);
    // The owner travels to the successor, so the chain stays endable after every rotation.
    expect((await chainRow(body.refresh_token)).profile_id).toBe(approved);
  });

  it("refuses a login on an unusable chain bound, in a form that names itself", async () => {
    // A bound the operator cannot state is not a bound, so refusing the configured value is right.
    // Letting that refusal escape uncaught is not: it would be a platform 500 with no reason
    // attached, on NEW logins only, while existing sessions kept rotating — a shape almost
    // impossible to trace back to a typo in an environment variable.
    process.env.AS_REFRESH_CHAIN_MAX_SECONDS = "7d";
    try {
      await createPendingFlow(db, {
        relayState: "rs-badcfg",
        clientId: "cli",
        redirectUri: "http://localhost/cb",
        codeChallenge: CHALLENGE,
        codeChallengeMethod: "S256",
        oauthState: "st",
        audience: "aud",
        expiresAt: new Date(Date.now() + 600000),
      });
      await bindCodeToFlow(db, "rs-badcfg", {
        code: "c-badcfg",
        claims: { sub: "u1", email: "u1@example.com", email_verified: true },
        expiresAt: new Date(Date.now() + 300000),
        profileId: null,
      });

      const res = await handleToken(
        new Request("https://as/oauth/token", {
          method: "POST",
          body: new URLSearchParams({
            grant_type: "authorization_code",
            code: "c-badcfg",
            code_verifier: VERIFIER,
            client_id: "cli",
          }),
        }),
        db,
      );
      expect(res.status, "a misconfiguration is the server's fault, not the client's").toBe(503);
      expect((await res.json()) as TokenErrorBody).toEqual({ error: "temporarily_unavailable" });
    } finally {
      process.env.AS_REFRESH_CHAIN_MAX_SECONDS = "7776000";
    }
  });

  it("treats a malformed resolve response as no answer, not as a refusal", async () => {
    // The two failures either side of the response boundary must land in the same place. A
    // transport error or non-2xx throws and the caller carries on without an owner; a 200 carrying
    // the wrong shape used to yield `undefined`, which survives a `!== null` guard, reaches
    // `principal_may_refresh(NULL)` — which answers FALSE, not null — and refuses a rotation whose
    // token has ALREADY been revoked by `rotateRefreshToken`. Success reported, user locked out.
    //
    // The population is not hypothetical: every chain backfilled by 20260825000010 carries a NULL
    // owner, so every one of them takes this re-resolve branch on its next refresh.
    process.env.INTERNAL_RESOLVE_URL = "https://api.internal/internal/principal/resolve";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";
    const issued = await login(db, { relay: "rs-junk", code: "c-junk", profileId: null });

    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(JSON.stringify({ status: "ok" }), {
            status: 200,
            headers: { "content-type": "application/json" },
          }),
      ),
    );
    try {
      const res = await refresh(db, issued.refresh_token);
      expect(res.status, "an unusable answer is no answer — it must not cost a session").toBe(200);
      const body = (await res.json()) as TokenSuccessBody;
      expect(body.refresh_token).not.toBe(issued.refresh_token);
      // …and the successor is still ownerless rather than carrying a junk id into a foreign key.
      expect((await chainRow(body.refresh_token)).profile_id).toBeNull();
    } finally {
      vi.unstubAllGlobals();
      delete process.env.INTERNAL_RESOLVE_URL;
      delete process.env.INTERNAL_RECONCILE_SECRET;
    }
  });

  it("still refreshes a principal who has never been approved — the state every login is born into", async () => {
    // `denied` is the birth state of every human who has ever logged in, and the only state from
    // which `Act::Request` is legal. Gating rotation on `has_system_access` (standing = approved)
    // rather than on admission having ENDED would strand exactly this principal: they hold a token
    // in order to reach the ungated join-request surface and ask for the access they lack.
    for (const state of ["denied", "requested"]) {
      const p = await principal(state);
      const issued = await login(db, {
        relay: `rs-${state}`,
        code: `c-${state}`,
        profileId: p,
      });

      const res = await refresh(db, issued.refresh_token);
      expect(res.status, `a ${state} principal must still refresh`).toBe(200);
      await truncateOauthTables(sql);
    }
  });
});
