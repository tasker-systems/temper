import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { type MockIssuer, startMockIssuer } from "../src/testing/index.js";

const contract = JSON.parse(
  readFileSync(
    fileURLToPath(new URL("../../../tests/contracts/m2m-token-request.json", import.meta.url)),
    "utf8",
  ),
) as {
  content_type: string;
  grant_type: string;
  response: { fields: string[]; token_type: string };
};

let issuer: MockIssuer | undefined;

afterEach(async () => {
  await issuer?.close();
  issuer = undefined;
});

/** The form-encoded mint every client emits. Deliberately NOT using ClientCredentials — this test proves the MOCK, not the client. */
async function mint(url: string, params: Record<string, string>, init: RequestInit = {}) {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": contract.content_type, ...(init.headers ?? {}) },
    body: new URLSearchParams(params),
    ...init,
  });
}

/** `Response.json()` is `unknown` — an issuer's body is untrusted by construction, which is the point. */
async function bodyOf(res: Response): Promise<Record<string, unknown>> {
  return (await res.json()) as Record<string, unknown>;
}

describe("the temper-AS-shaped issuer", () => {
  it("mints the contract's response shape and never a refresh token", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "s3cr3t",
    });

    expect(res.status).toBe(200);
    const body = await bodyOf(res);
    for (const field of contract.response.fields) {
      expect(body[field]).toBeDefined();
    }
    expect(body.token_type).toBe(contract.response.token_type);
    expect(body.refresh_token).toBeUndefined();
    // The AS's AS_ACCESS_TTL_SECONDS default. Short enough that a tick can outlive its token.
    expect(body.expires_in).toBe(900);
  });

  it("ignores a request-supplied audience rather than rejecting it", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "s3cr3t",
      audience: "https://ignored.example",
    });

    expect(res.status).toBe(200);
  });

  it("refuses a JSON body with invalid_request rather than throwing", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await fetch(issuer.url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ grant_type: "client_credentials", client_id: "tmpr_a", client_secret: "s3cr3t" }),
    });

    expect(res.status).toBe(400);
    expect((await bodyOf(res)).error).toBe("invalid_request");
  });

  // The AS reads the body with `formData()`, so the axis is "is this form encoding", NOT "is this
  // JSON". A mock that refused only JSON would mint happily for a client with a perfectly good body
  // and a wrong header — and that client would be refused in production.
  it("refuses a body whose content type is not form encoding, JSON or otherwise", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });
    const body = "grant_type=client_credentials&client_id=tmpr_a&client_secret=s3cr3t";

    const asText = await fetch(issuer.url, {
      method: "POST",
      headers: { "content-type": "text/plain" },
      body,
    });
    expect(asText.status).toBe(400);
    expect((await bodyOf(asText)).error).toBe("invalid_request");

    // No content-type header at all: `formData()` throws on this too. A Blob with an empty `type`
    // is how fetch is asked to send a body and set no content-type for it.
    const untyped = await fetch(issuer.url, { method: "POST", body: new Blob([body]) });
    expect(untyped.status).toBe(400);
    expect((await bodyOf(untyped)).error).toBe("invalid_request");
  });

  // The other half of modelling `formData()`: it parses multipart, so accepting only urlencoded
  // would make "the mock accepts it" and "the AS accepts it" mean different things.
  it("accepts a multipart form body, as formData() does", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const form = new FormData();
    form.set("grant_type", contract.grant_type);
    form.set("client_id", "tmpr_a");
    form.set("client_secret", "s3cr3t");

    const res = await fetch(issuer.url, { method: "POST", body: form });

    expect(res.status).toBe(200);
    expect(issuer.requests[0]?.params.client_id).toBe("tmpr_a");
  });

  // The AS reserves invalid_client for credentials that were SUPPLIED and did not verify; a request
  // carrying none at all is malformed.
  it("answers invalid_request, not invalid_client, when credentials are absent entirely", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(issuer.url, { grant_type: contract.grant_type });

    expect(res.status).toBe(400);
    expect((await bodyOf(res)).error).toBe("invalid_request");
  });

  // `Basic OnNlY3JldA==` decodes to `:secret` — an EMPTY client id. The AS's `sep > 0` refuses to
  // read that as credentials and falls through to the body, which here carries none.
  it("does not read an empty client id out of a Basic header", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(
      issuer.url,
      { grant_type: contract.grant_type },
      { headers: { authorization: `Basic ${Buffer.from(":s3cr3t").toString("base64")}` } },
    );

    expect(res.status).toBe(400);
    expect(issuer.requests[0]?.basic).toBeUndefined();
  });

  it("accepts credentials via HTTP Basic", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(
      issuer.url,
      { grant_type: contract.grant_type },
      { headers: { authorization: `Basic ${Buffer.from("tmpr_a:s3cr3t").toString("base64")}` } },
    );

    expect(res.status).toBe(200);
    expect(issuer.requests[0]?.basic).toEqual({ clientId: "tmpr_a", clientSecret: "s3cr3t" });
  });

  it("accepts the previous secret inside its grace window and rejects it after", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "new-secret",
      previousSecret: "old-secret",
      previousSecretExpiresAt: Date.now() + 60_000,
    });

    const inside = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "old-secret",
    });
    expect(inside.status).toBe(200);

    await issuer.close();
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_a",
      clientSecret: "new-secret",
      previousSecret: "old-secret",
      previousSecretExpiresAt: Date.now() - 1,
    });

    const lapsed = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "old-secret",
    });
    expect(lapsed.status).toBe(401);
    expect((await bodyOf(lapsed)).error).toBe("invalid_client");
  });

  it("rejects a wrong secret with invalid_client", async () => {
    issuer = await startMockIssuer({ flavor: "temper-as", clientId: "tmpr_a", clientSecret: "s3cr3t" });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "tmpr_a",
      client_secret: "wrong",
    });

    expect(res.status).toBe(401);
    expect((await bodyOf(res)).error).toBe("invalid_client");
  });
});

describe("the Auth0-shaped issuer", () => {
  // Without this, an auth0 issuer given no audience demands none (`undefined !== undefined` is
  // false) — so a test could "prove" the Auth0 path while asserting the only thing that flavor is
  // for.
  it("refuses to start without the audience it exists to demand", async () => {
    await expect(
      startMockIssuer({ flavor: "auth0", clientId: "auth0_a", clientSecret: "s3cr3t" }),
    ).rejects.toThrow(TypeError);
  });

  it("requires an audience", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    const without = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "auth0_a",
      client_secret: "s3cr3t",
    });
    expect(without.status).toBe(400);

    const with_ = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "auth0_a",
      client_secret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });
    expect(with_.status).toBe(200);
  });

  // Auth0 tolerates JSON as an extension. This is EXACTLY why the gem's JSON mint stayed green for
  // months: the only issuer it ever faced forgave it. The mock forgives it too, or it would not be
  // faithful — and a client that only ever meets a strict mock would never catch this class of bug.
  it("tolerates a JSON body", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    const res = await fetch(issuer.url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        grant_type: "client_credentials",
        client_id: "auth0_a",
        client_secret: "s3cr3t",
        audience: "https://temperkb.io/api",
      }),
    });

    expect(res.status).toBe(200);
  });

  it("mints a long-lived token", async () => {
    issuer = await startMockIssuer({
      flavor: "auth0",
      clientId: "auth0_a",
      clientSecret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    const res = await mint(issuer.url, {
      grant_type: contract.grant_type,
      client_id: "auth0_a",
      client_secret: "s3cr3t",
      audience: "https://temperkb.io/api",
    });

    expect((await bodyOf(res)).expires_in).toBe(86_400);
  });
});
