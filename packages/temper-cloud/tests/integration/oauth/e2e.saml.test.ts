import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { createLocalJWKSet, exportPKCS8, generateKeyPair, jwtVerify } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { handleAuthorize, handleSamlAcs, handleToken } from "../../../src/oauth/endpoints.js";
import { getPublicJwks } from "../../../src/oauth/keys.js";
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

  it("ACS issues a reconcile call carrying the asserted groups (fail-open)", async () => {
    // Configure the seeded IdP for group provisioning + point the reconcile client at a stub.
    await sql`UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'test'`;
    process.env.INTERNAL_RECONCILE_URL = "https://api.internal/internal/saml/reconcile";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";

    const reconcileCalls: Array<{ url: string; body: unknown; secret: string | null }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init: RequestInit) => {
        const headers = new Headers(init.headers);
        reconcileCalls.push({
          url,
          body: JSON.parse(init.body as string),
          secret: headers.get("X-Temper-Internal-Secret"),
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

      // the reconcile POST fired with the asserted groups + secret header
      expect(reconcileCalls).toHaveLength(1);
      expect(reconcileCalls[0].url).toBe("https://api.internal/internal/saml/reconcile");
      expect(reconcileCalls[0].secret).toBe("s3cr3t");
      expect(reconcileCalls[0].body).toMatchObject({
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
