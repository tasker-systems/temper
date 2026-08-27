import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { createLocalJWKSet, exportPKCS8, generateKeyPair, jwtVerify } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { handleAuthorize, handleSamlAcs, handleToken } from "../../../src/oauth/endpoints.js";
import { getPublicJwks } from "../../../src/oauth/keys.js";
import { hashToken } from "../../../src/oauth/mint.js";
import { SIGNATURE_HEADER, signReconcile, TIMESTAMP_HEADER } from "../../../src/oauth/reconcile.js";
import { loadIdpFixtureCert, makeSignedSamlResponse } from "../../../test-fixtures/saml.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";

interface TokenSuccessBody {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token: string;
}

const CERTS_DIR = fileURLToPath(new URL("../../../test-fixtures/certs/", import.meta.url));

const SP_ENTITY_ID = "https://sp.example.com/saml/metadata";
const ACS_URL = "https://sp.example.com/saml/acs";
const IDP_SSO_URL = "https://idp.example.com/sso";
const IDP_ENTITY_ID = "https://idp.example.com/metadata";
const REDIRECT_URI = "http://localhost:9999/cb";

describe("e2e: full mock-IdP SAML login", () => {
  let sql: postgres.Sql;
  let db: NeonClient;
  let idpCertPem: string;
  let idpKeyPem: string;
  let nextCertPem: string;
  let nextKeyPem: string;

  beforeAll(async () => {
    const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
    process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
    process.env.AS_SIGNING_KID = "test-kid-1";
    process.env.AS_ISSUER = "https://issuer.test";
    process.env.AS_AUDIENCE = "https://audience.test";
    process.env.AS_ACCESS_TTL_SECONDS = "900";
    process.env.AS_CLIENTS = JSON.stringify({ cli: [REDIRECT_URI] });

    ({ sql, db } = makeTestDb());

    idpCertPem = loadIdpFixtureCert(`${CERTS_DIR}idp-cert.pem`);
    idpKeyPem = loadIdpFixtureCert(`${CERTS_DIR}idp-key.pem`);
    nextCertPem = loadIdpFixtureCert(`${CERTS_DIR}idp-cert-secondary.pem`);
    nextKeyPem = loadIdpFixtureCert(`${CERTS_DIR}idp-key-secondary.pem`);
  });

  afterAll(async () => {
    await sql.end();
  });

  beforeEach(async () => {
    await truncateOauthTables(sql);
    await sql`INSERT INTO kb_saml_idp (
      idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url,
      nameid_format, email_attr, stable_id_attr
    ) VALUES (
      'test', true, ${idpCertPem}, ${IDP_SSO_URL}, ${IDP_ENTITY_ID}, ${SP_ENTITY_ID}, ${ACS_URL},
      'urn:oasis:names:tc:SAML:2.0:nameid-format:persistent', 'email', 'uid'
    )`;
  });

  it("full mock-IdP SAML → code → token issues a JWT whose sub is the persistent NameID", async () => {
    // 1. PKCE
    const verifier = `e2e-verifier-${"a".repeat(50)}`;
    const challenge = createHash("sha256").update(verifier).digest("base64url");

    // 2. authorize
    const authRes = await handleAuthorize(
      new Request(
        "https://as.example.com/oauth/authorize?response_type=code" +
          "&client_id=cli&redirect_uri=" +
          encodeURIComponent(REDIRECT_URI) +
          "&code_challenge=" +
          challenge +
          "&code_challenge_method=S256&state=e2e-state",
      ),
      db,
    );
    expect(authRes.status).toBe(302);
    const rs = new URLSearchParams(
      new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
    ).get("rs");
    expect(rs).toBeTruthy();

    // 3. synthesize signed assertion
    const { samlResponseB64 } = makeSignedSamlResponse({
      spEntityId: SP_ENTITY_ID,
      acsUrl: ACS_URL,
      nameId: "persistent-user-xyz",
      nameIdFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
      attributes: { email: "e2e@example.com", uid: "persistent-user-xyz" },
      idpKeyPem,
      idpCertPem,
    });

    // 4. ACS
    const acsRes = await handleSamlAcs(
      new Request("https://sp.example.com/saml/acs", {
        method: "POST",
        body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
      }),
      db,
    );
    expect(acsRes.status).toBe(302);
    const loc = new URL(acsRes.headers.get("location") as string);
    expect(loc.origin + loc.pathname).toBe("http://localhost:9999/cb");
    expect(loc.searchParams.get("state")).toBe("e2e-state");
    const code = loc.searchParams.get("code");
    expect(code).toBeTruthy();

    // 5. token
    const tokRes = await handleToken(
      new Request("https://as.example.com/oauth/token", {
        method: "POST",
        body: new URLSearchParams({
          grant_type: "authorization_code",
          code: code as string,
          code_verifier: verifier,
          client_id: "cli",
        }),
      }),
      db,
    );
    expect(tokRes.status).toBe(200);
    const body = (await tokRes.json()) as TokenSuccessBody;
    expect(body.token_type).toBe("Bearer");
    expect(body.refresh_token).toBeTruthy();

    // 6. verify JWT
    const jwks = createLocalJWKSet(await getPublicJwks());
    const { payload } = await jwtVerify(body.access_token, jwks, {
      issuer: process.env.AS_ISSUER,
      audience: process.env.AS_AUDIENCE,
    });
    expect(payload.sub).toBe("persistent-user-xyz");
    expect(payload.email).toBe("e2e@example.com");
    expect(payload.email_verified).toBe(true);
  });

  /**
   * The unit tests in `tests/saml/` prove `toSamlConfig` offers both certs to node-saml. They
   * cannot prove `loadActiveIdp` READS the second one — a SELECT that omits the column would leave
   * every one of them green. This drives the overlap through the real ACS handler against the real
   * table, so the column has to survive the round-trip for the login to complete.
   */
  it("ACS accepts an assertion signed by the incoming cert once an overlap window is open", async () => {
    async function acsWith(keyPem: string, certPem: string): Promise<Response> {
      const challenge = createHash("sha256")
        .update(`rollover-verifier-${"a".repeat(50)}`)
        .digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code" +
            "&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            `&code_challenge=${challenge}` +
            "&code_challenge_method=S256&state=rollover-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "rollover-user",
        nameIdFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
        attributes: { email: "rollover@example.com", uid: "rollover-user" },
        idpKeyPem: keyPem,
        idpCertPem: certPem,
      });
      return handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );
    }

    // Before the incoming cert is added, it is just an unknown signer.
    expect((await acsWith(nextKeyPem, nextCertPem)).status).toBe(400);

    // Open the overlap window — the one write an operator makes to start a rotation.
    await sql`UPDATE kb_saml_idp SET idp_cert_secondary = ${nextCertPem} WHERE idp_key = 'test'`;

    // Both keys now complete a login, which is what makes the IdP's cutover a non-event.
    expect((await acsWith(idpKeyPem, idpCertPem)).status).toBe(302);
    expect((await acsWith(nextKeyPem, nextCertPem)).status).toBe(302);

    // Cut over and drop the outgoing cert. The write IS the revocation: `loadActiveIdp` runs per
    // request, so there is no window in which the retired key still works.
    await sql`UPDATE kb_saml_idp
              SET idp_cert = idp_cert_secondary, idp_cert_secondary = NULL
              WHERE idp_key = 'test'`;
    expect((await acsWith(nextKeyPem, nextCertPem)).status).toBe(302);
    expect((await acsWith(idpKeyPem, idpCertPem)).status).toBe(400);
  });

  /**
   * The trap this feature is one careless line away from. Collecting certs "for the active IdP" is
   * a widening of ONE row's signer set; relaxing `WHERE is_active = true LIMIT 1` instead would
   * make every configured IdP's certificate a valid signer, and nothing on the happy path would
   * notice — a second IdP's assertions would simply start working.
   *
   * So this pins the boundary from the outside: a second IdP row exists, holding a real cert, and
   * an assertion signed by it is refused. It stays refused when that row is `is_active = true`,
   * which is the state a relaxed predicate would silently start accepting.
   */
  it("refuses an assertion signed by another IdP's certificate, active or not", async () => {
    async function acsSignedByOtherIdp(): Promise<Response> {
      const challenge = createHash("sha256")
        .update(`other-idp-verifier-${"a".repeat(50)}`)
        .digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code" +
            "&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            `&code_challenge=${challenge}` +
            "&code_challenge_method=S256&state=other-idp-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "other-idp-user",
        nameIdFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
        attributes: { email: "other@example.com", uid: "other-idp-user" },
        idpKeyPem: nextKeyPem,
        idpCertPem: nextCertPem,
      });
      return handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );
    }

    async function acsWithFirstIdp(): Promise<Response> {
      const challenge = createHash("sha256")
        .update(`first-idp-verifier-${"a".repeat(50)}`)
        .digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code" +
            "&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            `&code_challenge=${challenge}` +
            "&code_challenge_method=S256&state=first-idp-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "first-idp-user",
        nameIdFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
        attributes: { email: "first@example.com", uid: "first-idp-user" },
        idpKeyPem,
        idpCertPem,
      });
      return handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );
    }

    // A second IdP, configured with its own certificate, inactive.
    await sql`INSERT INTO kb_saml_idp (
      idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url,
      nameid_format, email_attr, stable_id_attr
    ) VALUES (
      'other', false, ${nextCertPem}, ${IDP_SSO_URL}, ${IDP_ENTITY_ID}, ${SP_ENTITY_ID}, ${ACS_URL},
      'urn:oasis:names:tc:SAML:2.0:nameid-format:persistent', 'email', 'uid'
    )`;
    expect((await acsSignedByOtherIdp()).status).toBe(400);

    // An open overlap window on the first IdP does not change that answer.
    await sql`UPDATE kb_saml_idp SET idp_cert_secondary = idp_cert WHERE idp_key = 'test'`;
    expect((await acsSignedByOtherIdp()).status).toBe(400);

    // Now make the second row active too. This is a state the one-active-IdP invariant forbids and
    // `temper admin saml verify --db` refuses, and `WHERE is_active = true LIMIT 1` carries no
    // ORDER BY — so WHICH row loads is undetermined and must not be asserted. What must hold
    // regardless is the property the trap is about: the signer sets of two rows are never unioned.
    // Exactly one of these two assertions succeeds, whichever row won.
    await sql`UPDATE kb_saml_idp SET is_active = true WHERE idp_key = 'other'`;
    const otherAccepted = (await acsSignedByOtherIdp()).status === 302;
    const testAccepted = (await acsWithFirstIdp()).status === 302;
    expect(otherAccepted).not.toBe(testAccepted);
  });

  it("ACS issues a reconcile call carrying the asserted groups (fail-open)", async () => {
    // Configure the seeded IdP for group provisioning + point the reconcile client at a stub.
    await sql`UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'test'`;
    process.env.INTERNAL_RECONCILE_URL = "https://api.internal/internal/saml/reconcile";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";

    const reconcileCalls: Array<{
      url: string;
      body: unknown;
      rawBody: string;
      timestamp: string | null;
      signature: string | null;
    }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init: RequestInit) => {
        const headers = new Headers(init.headers);
        const rawBody = init.body as string;
        reconcileCalls.push({
          url,
          body: JSON.parse(rawBody),
          rawBody,
          timestamp: headers.get(TIMESTAMP_HEADER),
          signature: headers.get(SIGNATURE_HEADER),
        });
        return new Response(null, { status: 204 });
      }),
    );

    try {
      // authorize -> relay state
      const verifier = `e2e-grp-verifier-${"a".repeat(50)}`;
      const challenge = createHash("sha256").update(verifier).digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            "&code_challenge=" +
            challenge +
            "&code_challenge_method=S256&state=grp-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");

      // signed assertion carrying a multi-valued 'groups' attribute
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "grp-user-1",
        attributes: { email: "grp@example.com", uid: "grp-user-1" },
        multiValuedAttributes: { groups: ["engineering", "eng-leads"] },
        idpKeyPem,
        idpCertPem,
      });

      const acsRes = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );

      // login still completes (fail-open is irrelevant here since the stub returns 204)
      expect(acsRes.status).toBe(302);
      expect(
        new URL(acsRes.headers.get("location") as string).searchParams.get("code"),
      ).toBeTruthy();

      // the reconcile POST fired with the asserted groups and a valid HMAC signature
      expect(reconcileCalls).toHaveLength(1);
      const call = reconcileCalls[0];
      expect(call.url).toBe("https://api.internal/internal/saml/reconcile");
      // The secret never travels the wire; a fresh signature over the body does.
      const timestamp = Number(call.timestamp);
      expect(Number.isInteger(timestamp)).toBe(true);
      expect(call.signature).toBe(signReconcile("s3cr3t", timestamp, call.rawBody));
      expect(call.body).toMatchObject({
        idp_key: "test",
        external_user_id: "grp-user-1",
        groups: ["engineering", "eng-leads"],
      });
    } finally {
      vi.unstubAllGlobals();
      delete process.env.INTERNAL_RECONCILE_URL;
      delete process.env.INTERNAL_RECONCILE_SECRET;
    }
  });

  it("ACS resolves the login's owner and stamps it on the chain the token endpoint mints", async () => {
    // The point of the resolve leg: a chain carries an owner, so an administrator's revoke has
    // something to match on. This is also the case that catches a MISCONFIGURED deploy —
    // `INTERNAL_RESOLVE_URL` unset means the call throws, the ACS fails open, and login looks
    // perfectly healthy while every chain it mints is held by its lifetime alone.
    process.env.INTERNAL_RESOLVE_URL = "https://api.internal/internal/principal/resolve";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";

    const handle = `e2e-owner-${Date.now()}`;
    const rows = await sql`
      INSERT INTO kb_profiles (handle, display_name, email, preferences)
      VALUES (${handle}, ${handle}, ${`${handle}@example.test`}, '{}') RETURNING id`;
    const profileId = (rows[0] as { id: string }).id;
    // A JIT-provisioned profile is born WITH standing (`Denied`, via `provision_conn`), and the
    // chain-minting predicate denies on ABSENCE — so a fixture profile lacking a standing row
    // would get no refresh token, and would fail here for a reason unrelated to the relay.
    await sql`INSERT INTO kb_principal_standing (profile_id, state) VALUES (${profileId}, 'denied')`;

    const resolveCalls: Array<{ url: string; rawBody: string; signature: string | null }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init: RequestInit) => {
        const headers = new Headers(init.headers);
        resolveCalls.push({
          url,
          rawBody: init.body as string,
          signature: headers.get(SIGNATURE_HEADER),
        });
        return new Response(JSON.stringify({ profile_id: profileId }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }),
    );

    try {
      const verifier = `e2e-own-verifier-${"a".repeat(50)}`;
      const challenge = createHash("sha256").update(verifier).digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            "&code_challenge=" +
            challenge +
            "&code_challenge_method=S256&state=own-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");

      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "own-user-1",
        attributes: { email: "own@example.com", uid: "own-user-1" },
        idpKeyPem,
        idpCertPem,
      });

      const acsRes = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );
      const code = new URL(acsRes.headers.get("location") as string).searchParams.get("code");

      // The resolve POST fired, at the configured URL, signed with the shared reconcile key.
      expect(resolveCalls).toHaveLength(1);
      const call = resolveCalls[0];
      expect(call.url).toBe("https://api.internal/internal/principal/resolve");
      expect(JSON.parse(call.rawBody)).toEqual({
        external_user_id: "own-user-1",
        email: "own@example.com",
        email_verified: true,
      });
      expect(call.signature).toBeTruthy();

      const tokenRes = await handleToken(
        new Request("https://as.example.com/oauth/token", {
          method: "POST",
          body: new URLSearchParams({
            grant_type: "authorization_code",
            code: code as string,
            code_verifier: verifier,
            client_id: "cli",
          }),
        }),
        db,
      );
      expect(tokenRes.status).toBe(200);
      const body = (await tokenRes.json()) as TokenSuccessBody;

      // …and the owner travelled all the way onto the stored chain, which is the only thing that
      // makes `standing_service::apply`'s terminal hook able to find it.
      const stored = await sql`
        SELECT profile_id, chain_expires_at FROM kb_oauth_refresh_tokens
         WHERE token_hash = ${hashToken(body.refresh_token)}`;
      expect((stored[0] as { profile_id: string }).profile_id).toBe(profileId);
    } finally {
      vi.unstubAllGlobals();
      delete process.env.INTERNAL_RESOLVE_URL;
      delete process.env.INTERNAL_RECONCILE_SECRET;
      await sql`DELETE FROM kb_profiles WHERE handle = ${handle}`;
    }
  });

  it("ACS completes login even when reconcile fails (fail-open)", async () => {
    await sql`UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'test'`;
    process.env.INTERNAL_RECONCILE_URL = "https://api.internal/internal/saml/reconcile";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("boom", { status: 500 })),
    );
    try {
      const verifier = `e2e-fo-verifier-${"a".repeat(50)}`;
      const challenge = createHash("sha256").update(verifier).digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            "&code_challenge=" +
            challenge +
            "&code_challenge_method=S256&state=fo-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "fo-user-1",
        attributes: { email: "fo@example.com", uid: "fo-user-1" },
        multiValuedAttributes: { groups: ["engineering"] },
        idpKeyPem,
        idpCertPem,
      });
      const acsRes = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );
      expect(acsRes.status).toBe(302);
      expect(
        new URL(acsRes.headers.get("location") as string).searchParams.get("code"),
      ).toBeTruthy();
    } finally {
      vi.unstubAllGlobals();
      delete process.env.INTERNAL_RECONCILE_URL;
      delete process.env.INTERNAL_RECONCILE_SECRET;
    }
  });

  it("skips reconcile when the assertion omits the configured groups attribute", async () => {
    await sql`UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'test'`;
    process.env.INTERNAL_RECONCILE_URL = "https://api.internal/internal/saml/reconcile";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    try {
      const verifier = `e2e-nosig-verifier-${"a".repeat(50)}`;
      const challenge = createHash("sha256").update(verifier).digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            "&code_challenge=" +
            challenge +
            "&code_challenge_method=S256&state=nosig-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");
      // No multiValuedAttributes → assertion carries no 'groups' attribute at all.
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "nosig-user-1",
        attributes: { email: "nosig@example.com", uid: "nosig-user-1" },
        idpKeyPem,
        idpCertPem,
      });
      const acsRes = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );
      expect(acsRes.status).toBe(302);
      expect(
        new URL(acsRes.headers.get("location") as string).searchParams.get("code"),
      ).toBeTruthy();
      expect(fetchMock).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
      delete process.env.INTERNAL_RECONCILE_URL;
      delete process.env.INTERNAL_RECONCILE_SECRET;
    }
  });
});
