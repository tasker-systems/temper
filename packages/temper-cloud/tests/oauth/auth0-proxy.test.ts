import { afterEach, beforeEach, describe, expect, it } from "vitest";

const ENV = {
  MCP_BASE_URL: "https://temperkb.io",
  AUTH_ISSUER: "https://tenant.auth0.com/",
  AUTH_AUDIENCE: "https://api.temperkb.io",
  MCP_CLIENT_ID: "test-client-id",
  MCP_PROXY_SECRET: "a".repeat(48),
};

describe("auth0-proxy", () => {
  const saved: Record<string, string | undefined> = {};

  beforeEach(() => {
    for (const [k, v] of Object.entries(ENV)) {
      saved[k] = process.env[k];
      process.env[k] = v;
    }
  });

  afterEach(() => {
    for (const [k, v] of Object.entries(saved)) {
      if (v === undefined) delete process.env[k];
      else process.env[k] = v;
    }
  });

  describe("proxyAuthorize", () => {
    it("rewrites loopback redirect_uri and stashes original state", async () => {
      const { proxyAuthorize } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/oauth/authorize");
      url.searchParams.set("client_id", "test-client-id");
      url.searchParams.set("redirect_uri", "http://127.0.0.1:19876/mcp/oauth/callback");
      url.searchParams.set("response_type", "code");
      url.searchParams.set("code_challenge", "abc123");
      url.searchParams.set("code_challenge_method", "S256");
      url.searchParams.set("scope", "openid profile email offline_access");
      url.searchParams.set("state", "client-csrf-state");
      url.searchParams.set("resource", "https://api.temperkb.io");

      const res = await proxyAuthorize(new Request(url.toString()));
      expect(res.status).toBe(302);

      const location = res.headers.get("location");
      expect(location).not.toBeNull();
      const loc = new URL(location as string);
      expect(loc.origin).toBe("https://tenant.auth0.com");
      expect(loc.pathname).toBe("/authorize");
      expect(loc.searchParams.get("redirect_uri")).toBe(
        "https://temperkb.io/api/auth/mcp-callback",
      );
      expect(loc.searchParams.get("code_challenge")).toBe("abc123");
      expect(loc.searchParams.get("resource")).toBe("https://api.temperkb.io");

      // The state should be a signed token, not the original client state.
      const stashed = loc.searchParams.get("state");
      expect(stashed).not.toBeNull();
      expect(stashed).not.toBe("client-csrf-state");
      expect(stashed).not.toBe("client-csrf-state");
      expect(stashed).toContain(".");
    });

    it("passes through non-loopback redirect_uris unchanged (Claude Desktop)", async () => {
      const { proxyAuthorize } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/oauth/authorize");
      url.searchParams.set("client_id", "test-client-id");
      url.searchParams.set("redirect_uri", "https://claude.ai/api/mcp/auth_callback");
      url.searchParams.set("response_type", "code");
      url.searchParams.set("code_challenge", "abc");
      url.searchParams.set("code_challenge_method", "S256");
      url.searchParams.set("state", "my-state");

      const res = await proxyAuthorize(new Request(url.toString()));
      expect(res.status).toBe(302);

      const location = res.headers.get("location");
      expect(location).not.toBeNull();
      const loc = new URL(location as string);
      expect(loc.origin).toBe("https://tenant.auth0.com");
      expect(loc.searchParams.get("redirect_uri")).toBe("https://claude.ai/api/mcp/auth_callback");
      expect(loc.searchParams.get("state")).toBe("my-state");
    });

    it("returns 400 when redirect_uri is missing", async () => {
      const { proxyAuthorize } = await import("../../src/oauth/auth0-proxy.js");
      const res = await proxyAuthorize(new Request("https://temperkb.io/oauth/authorize?state=x"));
      expect(res.status).toBe(400);
    });
  });

  describe("handleMcpCallback", () => {
    it("round-trips: stashed state from authorize decodes back to original redirect_uri + state", async () => {
      const { proxyAuthorize, handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");

      // Step 1: authorize produces a stashed state
      const authUrl = new URL("https://temperkb.io/oauth/authorize");
      authUrl.searchParams.set("client_id", "test-client-id");
      authUrl.searchParams.set("redirect_uri", "http://127.0.0.1:19876/mcp/oauth/callback");
      authUrl.searchParams.set("response_type", "code");
      authUrl.searchParams.set("code_challenge", "xyz");
      authUrl.searchParams.set("code_challenge_method", "S256");
      authUrl.searchParams.set("state", "client-csrf-state");

      const authRes = await proxyAuthorize(new Request(authUrl.toString()));
      const authLocation = authRes.headers.get("location");
      expect(authLocation).not.toBeNull();
      const auth0Loc = new URL(authLocation as string);
      const stashedState = auth0Loc.searchParams.get("state");
      expect(stashedState).not.toBeNull();

      // Step 2: Auth0 redirects back with code + stashed state
      const cbUrl = new URL("https://temperkb.io/api/auth/mcp-callback");
      cbUrl.searchParams.set("code", "auth-code-from-auth0");
      cbUrl.searchParams.set("state", stashedState as string);

      const cbRes = handleMcpCallback(new Request(cbUrl.toString()));
      expect(cbRes.status).toBe(302);

      const cbLocation = cbRes.headers.get("location");
      expect(cbLocation).not.toBeNull();
      const finalLoc = new URL(cbLocation as string);
      expect(finalLoc.origin).toBe("http://127.0.0.1:19876");
      expect(finalLoc.pathname).toBe("/mcp/oauth/callback");
      expect(finalLoc.searchParams.get("code")).toBe("auth-code-from-auth0");
      expect(finalLoc.searchParams.get("state")).toBe("client-csrf-state");
    });

    it("returns 400 for a tampered state token", async () => {
      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/api/auth/mcp-callback");
      url.searchParams.set("code", "abc");
      url.searchParams.set("state", "eyJyciI6Imh0dHA6Ly9ldmlsLmNvbSJ9.aW52YWxpZA");

      const res = handleMcpCallback(new Request(url.toString()));
      expect(res.status).toBe(400);
    });

    it("returns 400 when code or state is missing", async () => {
      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const res = handleMcpCallback(
        new Request("https://temperkb.io/api/auth/mcp-callback?code=abc"),
      );
      expect(res.status).toBe(400);
    });

    it("surfaces Auth0 error params as 400", async () => {
      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/api/auth/mcp-callback");
      url.searchParams.set("error", "access_denied");
      url.searchParams.set("error_description", "nope");

      const res = handleMcpCallback(new Request(url.toString()));
      expect(res.status).toBe(400);
      expect(await res.text()).toContain("access_denied");
    });
  });

  describe("proxyToken", () => {
    it("rewrites loopback redirect_uri before forwarding to Auth0", async () => {
      const { proxyToken } = await import("../../src/oauth/auth0-proxy.js");
      const calls: Request[] = [];
      const origFetch = globalThis.fetch;
      globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const req = new Request(input, init);
        calls.push(req);
        return new Response(
          JSON.stringify({
            access_token: "tok",
            token_type: "Bearer",
            expires_in: 3600,
            refresh_token: "rt",
          }),
          {
            status: 200,
            headers: { "content-type": "application/json" },
          },
        );
      }) as typeof fetch;

      try {
        const body = new URLSearchParams({
          grant_type: "authorization_code",
          code: "abc",
          code_verifier: "verifier",
          redirect_uri: "http://127.0.0.1:19876/mcp/oauth/callback",
          client_id: "test-client-id",
        });

        const res = await proxyToken(
          new Request("https://temperkb.io/oauth/token", {
            method: "POST",
            headers: { "content-type": "application/x-www-form-urlencoded" },
            body: body.toString(),
          }),
        );

        expect(res.status).toBe(200);
        expect(calls).toHaveLength(1);
        expect(calls[0].url).toBe("https://tenant.auth0.com/oauth/token");

        const forwarded = new URLSearchParams(await calls[0].text());
        expect(forwarded.get("redirect_uri")).toBe("https://temperkb.io/api/auth/mcp-callback");
        expect(forwarded.get("code_verifier")).toBe("verifier");
      } finally {
        globalThis.fetch = origFetch;
      }
    });

    it("passes refresh_token grants through unchanged", async () => {
      const { proxyToken } = await import("../../src/oauth/auth0-proxy.js");
      const calls: Request[] = [];
      const origFetch = globalThis.fetch;
      globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push(new Request(input, init));
        return new Response(
          JSON.stringify({
            access_token: "tok",
            token_type: "Bearer",
            expires_in: 3600,
            refresh_token: "rt2",
          }),
          {
            status: 200,
            headers: { "content-type": "application/json" },
          },
        );
      }) as typeof fetch;

      try {
        const body = new URLSearchParams({
          grant_type: "refresh_token",
          refresh_token: "rt1",
          client_id: "test-client-id",
        });

        await proxyToken(
          new Request("https://temperkb.io/oauth/token", {
            method: "POST",
            headers: { "content-type": "application/x-www-form-urlencoded" },
            body: body.toString(),
          }),
        );

        const forwarded = new URLSearchParams(await calls[0].text());
        expect(forwarded.get("grant_type")).toBe("refresh_token");
        expect(forwarded.get("refresh_token")).toBe("rt1");
        expect(forwarded.has("redirect_uri")).toBe(false);
      } finally {
        globalThis.fetch = origFetch;
      }
    });
  });
});
