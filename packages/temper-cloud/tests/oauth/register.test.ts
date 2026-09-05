import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../src/db.js";

// The handler under test lands with this commit; importing it before it exists is the
// witness's first assertion — the suite cannot pass against a tree where registration
// still means the Auth0 echo.
const { handleClientRegistration } = await import("../../src/oauth/register.js");

/** A db stub: records nothing, answers every tagged-template query with no rows. */
const stubDb = (async () => []) as unknown as NeonClient;

const CONNECT_BODY = {
  client_name: "my temper connector",
  redirect_uris: ["https://connect.vercel.com/callback"],
  grant_types: ["client_credentials"],
  token_endpoint_auth_method: "client_secret_basic",
};

interface Registered {
  status: number;
  body: Record<string, unknown>;
}

async function register(body: unknown, db: NeonClient = stubDb): Promise<Registered> {
  const res = await handleClientRegistration(
    new Request("https://as.example.com/oauth/clients", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
    db,
  );
  const text = await res.text();
  let body_: Record<string, unknown>;
  try {
    body_ = JSON.parse(text) as Record<string, unknown>;
  } catch {
    body_ = { raw: text };
  }
  return { status: res.status, body: body_ };
}

describe("handleClientRegistration (AS-mode /oauth/clients)", () => {
  const originalAsIssuer = process.env.AS_ISSUER;
  const originalMcpClientId = process.env.MCP_CLIENT_ID;
  const originalAsClients = process.env.AS_CLIENTS;

  beforeEach(() => {
    process.env.AS_ISSUER = "https://as.example.com";
    delete process.env.MCP_CLIENT_ID;
    delete process.env.AS_CLIENTS;
  });

  afterEach(() => {
    for (const [name, value] of [
      ["AS_ISSUER", originalAsIssuer],
      ["MCP_CLIENT_ID", originalMcpClientId],
      ["AS_CLIENTS", originalAsClients],
    ] as const) {
      if (value === undefined) {
        delete process.env[name];
      } else {
        process.env[name] = value;
      }
    }
  });

  it("404s when AS_ISSUER is unset — Auth0 instances never advertise this door", async () => {
    delete process.env.AS_ISSUER;
    const res = await register(CONNECT_BODY);

    expect(res.status).toBe(404);
  });

  it("registers a Connect-class client: 201, conn_ id, a secret, the echo", async () => {
    const res = await register(CONNECT_BODY);

    expect(res.status).toBe(201);
    expect(String(res.body.client_id)).toMatch(/^conn_/);
    expect(typeof res.body.client_secret).toBe("string");
    expect((res.body.client_secret as string).length).toBeGreaterThanOrEqual(32);
    expect(res.body.client_name).toBe("my temper connector");
    expect(res.body.grant_types).toEqual(["client_credentials"]);
    expect(res.body.redirect_uris).toEqual(["https://connect.vercel.com/callback"]);
    expect(res.body.token_endpoint_auth_method).toBe("client_secret_basic");
    expect(res.body.client_id_issued_at).toEqual(expect.any(Number));
    // RFC 7591 §3.2.1: 0 means the secret does not expire.
    expect(res.body.client_secret_expires_at).toBe(0);
    // RFC 7592 sync is deferred — the honest signal is absence, not a dead URL.
    expect(res.body.registration_client_uri).toBeUndefined();
    expect(res.body.registration_access_token).toBeUndefined();
  });

  it("accepts a connector enabled for both grants (Connect sends the enabled set)", async () => {
    const res = await register({
      ...CONNECT_BODY,
      grant_types: ["authorization_code", "client_credentials", "refresh_token"],
    });

    expect(res.status).toBe(201);
    expect(res.body.grant_types).toEqual([
      "authorization_code",
      "client_credentials",
      "refresh_token",
    ]);
  });

  it("refuses a redirect_uri that is not the Connect callback, exact-match", async () => {
    const res = await register({
      ...CONNECT_BODY,
      redirect_uris: ["https://connect.vercel.com/callback/", "https://evil.example/cb"],
    });

    expect(res.status).toBe(400);
    expect(res.body.error).toBe("invalid_redirect_uri");
  });

  it("refuses a public (method none) machine client — the posture the probe declined", async () => {
    const res = await register({
      ...CONNECT_BODY,
      token_endpoint_auth_method: "none",
    });

    expect(res.status).toBe(400);
    expect(res.body.error).toBe("invalid_client_metadata");
  });

  describe("MCP-compat class (the SAML-instance door keeps its old semantics)", () => {
    beforeEach(() => {
      process.env.MCP_CLIENT_ID = "temper-mcp";
      process.env.AS_CLIENTS = JSON.stringify({ "temper-mcp": ["http://127.0.0.1:/callback"] });
    });

    it("filters a remote redirect out of the echo — the guard lives at /oauth/authorize", async () => {
      // The incumbent proxy semantic is accept-and-filter, never refuse: registration persists
      // nothing and the authorize endpoint (clients.ts, isRedirectUriAllowed) refuses anything
      // the AS_CLIENTS allowlist does not admit. The witness is the EMPTY echo, not a 400.
      const res = await register({
        client_name: "attacker",
        redirect_uris: ["https://evil.example/cb"],
        grant_types: ["authorization_code"],
        token_endpoint_auth_method: "none",
      });

      expect(res.status).toBe(201);
      expect(res.body.redirect_uris).toEqual([]);
    });

    it("returns the pre-registered client_id for an MCP-shaped request", async () => {
      const res = await register({
        client_name: "Claude Code",
        redirect_uris: ["http://localhost:53682/callback"],
        grant_types: ["authorization_code"],
        token_endpoint_auth_method: "none",
      });

      expect(res.status).toBe(201);
      expect(res.body.client_id).toBe("temper-mcp");
      expect(res.body.client_name).toBe("Claude Code");
      // Loopback port/host flexibility per RFC 8252 §7.3, path exact — the proxy's rule.
      expect(res.body.redirect_uris).toEqual(["http://localhost:53682/callback"]);
      expect(res.body.grant_types).toEqual(["authorization_code", "refresh_token"]);
      expect(res.body.response_types).toEqual(["code"]);
      expect(res.body.token_endpoint_auth_method).toBe("none");
      // No client is minted for this class — no secret is ever returned.
      expect(res.body.client_secret).toBeUndefined();
    });

    it("echoes only redirect_uris the AS_CLIENTS allowlist admits", async () => {
      const res = await register({
        client_name: "Claude Code",
        redirect_uris: ["http://localhost:53682/callback", "https://evil.example/cb"],
        grant_types: ["authorization_code"],
        token_endpoint_auth_method: "none",
      });

      expect(res.status).toBe(201);
      expect(res.body.redirect_uris).toEqual(["http://localhost:53682/callback"]);
    });

    it("503s when MCP_CLIENT_ID is not configured, matching the old proxy", async () => {
      delete process.env.MCP_CLIENT_ID;
      const res = await register({
        client_name: "Claude Code",
        redirect_uris: ["http://localhost:53682/callback"],
        grant_types: ["authorization_code"],
        token_endpoint_auth_method: "none",
      });

      expect(res.status).toBe(503);
      expect(res.body.error).toBe("temporarily_unavailable");
    });
  });
});
