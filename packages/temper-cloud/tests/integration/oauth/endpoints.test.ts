import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { exportPKCS8, generateKeyPair } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import {
  handleAuthorize,
  handleSamlAcs,
  handleSamlLogin,
  handleSamlMetadata,
} from "../../../src/oauth/endpoints.js";
import { loadIdpFixtureCert, makeSignedSamlResponse } from "../../../test-fixtures/saml.js";
import { makeTestDb, truncateOauthTables } from "../helpers/oauth-db.js";

const CERTS_DIR = fileURLToPath(new URL("../../../test-fixtures/certs/", import.meta.url));
const idpCertPem = loadIdpFixtureCert(`${CERTS_DIR}idp-cert.pem`);
const idpKeyPem = loadIdpFixtureCert(`${CERTS_DIR}idp-key.pem`);

const SP_ENTITY_ID = "https://sp.example.com/saml/metadata";
const ACS_URL = "https://sp.example.com/saml/acs";
const IDP_SSO_URL = "https://idp.example.com/sso";
const IDP_ENTITY_ID = "https://idp.example.com/metadata";
const REDIRECT_URI = "http://localhost:9999/cb";

function pkcePair(): { verifier: string; challenge: string } {
  const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  return { verifier, challenge };
}

function authorizeRequest(opts?: {
  state?: string;
  challenge?: string;
  codeChallengeMethod?: string | null;
  responseType?: string;
}): Request {
  const { challenge } = pkcePair();
  const params = new URLSearchParams({
    response_type: opts?.responseType ?? "code",
    client_id: "cli",
    redirect_uri: REDIRECT_URI,
    code_challenge: opts?.challenge ?? challenge,
    state: opts?.state ?? "st-123",
  });
  const method = opts?.codeChallengeMethod === undefined ? "S256" : opts.codeChallengeMethod;
  if (method !== null) {
    params.set("code_challenge_method", method);
  }
  return new Request(`https://as/oauth/authorize?${params.toString()}`);
}

function relayStateFromLocation(location: string): string {
  const url = new URL(location, "https://as");
  const rs = url.searchParams.get("rs");
  if (!rs) {
    throw new Error(`no rs in location: ${location}`);
  }
  return rs;
}

describe("oauth endpoints", () => {
  let sql: postgres.Sql;
  let db: NeonClient;

  beforeAll(async () => {
    const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
    process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
    process.env.AS_SIGNING_KID = "test-kid-1";
    process.env.AS_ISSUER = "https://issuer.test";
    process.env.AS_AUDIENCE = "https://audience.test";
    process.env.AS_CLIENTS = JSON.stringify({ cli: [REDIRECT_URI] });
    ({ sql, db } = makeTestDb());
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

  describe("handleAuthorize", () => {
    it("stores a pending flow and redirects to the SAML login handoff", async () => {
      const res = await handleAuthorize(authorizeRequest(), db);

      expect(res.status).toBe(302);
      const location = res.headers.get("location");
      expect(location).toBeTruthy();
      expect(location).toMatch(/^\/oauth\/saml\/login\?rs=/);

      const rs = relayStateFromLocation(location as string);
      const rows =
        await sql`SELECT status, relay_state FROM kb_oauth_flow WHERE relay_state = ${rs}`;
      expect(rows).toHaveLength(1);
      expect(rows[0]?.status).toBe("pending_saml");
    });

    it("rejects an unsupported response_type", async () => {
      const res = await handleAuthorize(authorizeRequest({ responseType: "token" }), db);
      expect(res.status).toBe(400);
    });

    it("rejects a missing/non-S256 code_challenge_method", async () => {
      const resMissing = await handleAuthorize(authorizeRequest({ codeChallengeMethod: null }), db);
      expect(resMissing.status).toBe(400);

      const resPlain = await handleAuthorize(
        authorizeRequest({ codeChallengeMethod: "plain" }),
        db,
      );
      expect(resPlain.status).toBe(400);
    });

    it("rejects a missing state", async () => {
      const { challenge } = pkcePair();
      const params = new URLSearchParams({
        response_type: "code",
        client_id: "cli",
        redirect_uri: REDIRECT_URI,
        code_challenge: challenge,
        code_challenge_method: "S256",
      });
      const res = await handleAuthorize(
        new Request(`https://as/oauth/authorize?${params.toString()}`),
        db,
      );
      expect(res.status).toBe(400);
    });

    it("rejects an unregistered redirect_uri with a 400 and never 302s to it (C1)", async () => {
      const { challenge } = pkcePair();
      const unregistered = "https://attacker.example.com/cb";
      const params = new URLSearchParams({
        response_type: "code",
        client_id: "cli",
        redirect_uri: unregistered,
        code_challenge: challenge,
        code_challenge_method: "S256",
        state: "st-123",
      });

      const res = await handleAuthorize(
        new Request(`https://as/oauth/authorize?${params.toString()}`),
        db,
      );

      expect(res.status).toBe(400);
      expect(res.headers.get("location")).toBeNull();

      const rows = await sql`SELECT * FROM kb_oauth_flow`;
      expect(rows).toHaveLength(0);
    });
  });

  describe("handleSamlLogin", () => {
    it("redirects to the IdP SSO endpoint carrying the relay state", async () => {
      const authRes = await handleAuthorize(authorizeRequest(), db);
      const rs = relayStateFromLocation(authRes.headers.get("location") as string);

      const res = await handleSamlLogin(new Request(`https://as/oauth/saml/login?rs=${rs}`), db);

      expect(res.status).toBe(302);
      const location = res.headers.get("location") as string;
      expect(location.startsWith(IDP_SSO_URL)).toBe(true);
      expect(location).toContain("SAMLRequest=");
    });
  });

  describe("handleSamlAcs", () => {
    it("validates the assertion and redirects back to the client with a code (happy path)", async () => {
      const authRes = await handleAuthorize(authorizeRequest({ state: "st-123" }), db);
      const rs = relayStateFromLocation(authRes.headers.get("location") as string);

      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "persistent-user-1",
        nameIdFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
        attributes: { email: "user@example.com", uid: "persistent-user-1" },
        idpKeyPem,
        idpCertPem,
      });

      const res = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs }),
        }),
        db,
      );

      expect(res.status).toBe(302);
      const location = res.headers.get("location") as string;
      expect(location.startsWith(REDIRECT_URI)).toBe(true);
      const url = new URL(location);
      expect(url.searchParams.get("code")).toBeTruthy();
      expect(url.searchParams.get("state")).toBe("st-123");

      const rows = await sql`SELECT status FROM kb_oauth_flow WHERE relay_state = ${rs}`;
      expect(rows[0]?.status).toBe("code_issued");
    });

    it("rejects a replayed assertion", async () => {
      const authRes1 = await handleAuthorize(authorizeRequest({ state: "st-1" }), db);
      const rs1 = relayStateFromLocation(authRes1.headers.get("location") as string);
      const authRes2 = await handleAuthorize(authorizeRequest({ state: "st-2" }), db);
      const rs2 = relayStateFromLocation(authRes2.headers.get("location") as string);

      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "persistent-user-1",
        nameIdFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
        attributes: { email: "user@example.com", uid: "persistent-user-1" },
        idpKeyPem,
        idpCertPem,
      });

      const res1 = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs1 }),
        }),
        db,
      );
      expect(res1.status).toBe(302);

      const res2 = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs2 }),
        }),
        db,
      );
      expect(res2.status).toBe(400);
    });
  });

  describe("handleSamlMetadata", () => {
    it("serves SP metadata XML", async () => {
      const res = await handleSamlMetadata(new Request("https://sp/oauth/saml/metadata"), db);

      expect(res.status).toBe(200);
      expect(res.headers.get("content-type")).toBe("application/xml");
      const body = await res.text();
      expect(body).toContain("EntityDescriptor");
    });
  });
});
