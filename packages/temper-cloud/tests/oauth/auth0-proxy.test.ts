import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** Everything the proxy reads. `client_id` below is a request parameter, not env. */
const ENV = {
  MCP_BASE_URL: "https://temperkb.io",
  AUTH_ISSUER: "https://tenant.auth0.com/",
  MCP_PROXY_SECRET: "a".repeat(48),
};

/**
 * Mints a state token the way `/oauth/authorize` does, with the secret set, and
 * returns it for a case that then takes the secret away. A hand-written string
 * would be rejected on its own shape before the key was ever consulted, so it
 * could not tell a configuration fault from a malformed token.
 */
async function mintStashedState(): Promise<string> {
  const { proxyAuthorize } = await import("../../src/oauth/auth0-proxy.js");
  const url = new URL("https://temperkb.io/oauth/authorize");
  url.searchParams.set("client_id", "test-client-id");
  url.searchParams.set("redirect_uri", "http://127.0.0.1:19876/mcp/oauth/callback");
  url.searchParams.set("response_type", "code");
  url.searchParams.set("state", "client-csrf-state");

  const res = await proxyAuthorize(new Request(url.toString()));
  const stashed = new URL(res.headers.get("location") as string).searchParams.get("state");
  if (!stashed) throw new Error("fixture failed to mint a state token");
  return stashed;
}

describe("auth0-proxy", () => {
  const saved: Record<string, string | undefined> = {};

  beforeEach(() => {
    for (const [k, v] of Object.entries(ENV)) {
      saved[k] = process.env[k];
      process.env[k] = v;
    }
    // The proxy serves Auth0-fronted instances, which are exactly the ones with
    // no AS_ISSUER. Every case below is one of those unless it says otherwise.
    saved.AS_ISSUER = process.env.AS_ISSUER;
    delete process.env.AS_ISSUER;
    // The derived key is cached for the life of the module, so a case that
    // changes MCP_PROXY_SECRET would otherwise be answered by the key a previous
    // case derived.
    vi.resetModules();
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

      // The state should be an encrypted token, not the original client state.
      const stashed = loc.searchParams.get("state");
      expect(stashed).not.toBeNull();
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

    it("names MCP_PROXY_SECRET in a 503 when a loopback flow has no key to stash with", async () => {
      delete process.env.MCP_PROXY_SECRET;

      const { proxyAuthorize } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/oauth/authorize");
      url.searchParams.set("client_id", "test-client-id");
      url.searchParams.set("redirect_uri", "http://127.0.0.1:19876/mcp/oauth/callback");
      url.searchParams.set("response_type", "code");
      url.searchParams.set("state", "client-csrf-state");

      const res = await proxyAuthorize(new Request(url.toString()));
      expect(res.status).toBe(503);
      expect(await res.text()).toContain("MCP_PROXY_SECRET");
    });

    it("serves a loopback flow when MCP_PROXY_SECRET is exactly the documented minimum", async () => {
      // Both playbooks promise "at least 32 characters". A `<` that slipped to
      // `<=` would refuse the value an operator generated by following them.
      process.env.MCP_PROXY_SECRET = "a".repeat(32);

      const { proxyAuthorize, stateKey } = await import("../../src/oauth/auth0-proxy.js");
      expect(stateKey()).toHaveLength(32);

      const url = new URL("https://temperkb.io/oauth/authorize");
      url.searchParams.set("client_id", "test-client-id");
      url.searchParams.set("redirect_uri", "http://127.0.0.1:19876/mcp/oauth/callback");
      url.searchParams.set("response_type", "code");
      url.searchParams.set("state", "client-csrf-state");

      const res = await proxyAuthorize(new Request(url.toString()));
      expect(res.status).toBe(302);
    });

    it("still passes a non-loopback redirect_uri through when MCP_PROXY_SECRET is unset", async () => {
      // The pass-through branch stashes nothing and so needs no key. Refusing it
      // as well would turn a gap in the proxy's configuration into an outage for
      // the browser clients that never used the proxy.
      delete process.env.MCP_PROXY_SECRET;

      const { proxyAuthorize } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/oauth/authorize");
      url.searchParams.set("client_id", "test-client-id");
      url.searchParams.set("redirect_uri", "https://claude.ai/api/mcp/auth_callback");
      url.searchParams.set("response_type", "code");
      url.searchParams.set("state", "my-state");

      const res = await proxyAuthorize(new Request(url.toString()));
      expect(res.status).toBe(302);
      const loc = new URL(res.headers.get("location") as string);
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

      // Step 3: the client exchanges what the relay actually handed it. Taking the
      // code and redirect_uri from `finalLoc` rather than restating them is the
      // point: it makes the three steps one flow instead of three tests that
      // happen to spell the same literals.
      const { proxyToken } = await import("../../src/oauth/auth0-proxy.js");
      const forwarded: Request[] = [];
      const origFetch = globalThis.fetch;
      globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
        forwarded.push(new Request(input, init));
        return new Response(JSON.stringify({ access_token: "tok", token_type: "Bearer" }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }) as typeof fetch;

      try {
        const tokenRes = await proxyToken(
          new Request("https://temperkb.io/oauth/token", {
            method: "POST",
            headers: { "content-type": "application/x-www-form-urlencoded" },
            body: new URLSearchParams({
              grant_type: "authorization_code",
              code: finalLoc.searchParams.get("code") as string,
              code_verifier: "verifier",
              redirect_uri: `${finalLoc.origin}${finalLoc.pathname}`,
              client_id: "test-client-id",
            }).toString(),
          }),
        );
        expect(tokenRes.status).toBe(200);

        // Auth0 must see the redirect_uri it was given at /authorize — the relay.
        const sent = new URLSearchParams(await forwarded[0].text());
        expect(sent.get("redirect_uri")).toBe(auth0Loc.searchParams.get("redirect_uri") as string);
        expect(sent.get("code")).toBe("auth-code-from-auth0");
        expect(sent.get("code_verifier")).toBe("verifier");
      } finally {
        globalThis.fetch = origFetch;
      }
    });

    it("returns 400 for a tampered state token", async () => {
      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/api/auth/mcp-callback");
      url.searchParams.set("code", "abc");
      url.searchParams.set("state", "eyJyciI6Imh0dHA6Ly9ldmlsLmNvbSJ9.aW52YWxpZA");

      const res = handleMcpCallback(new Request(url.toString()));
      expect(res.status).toBe(400);
    });

    it("returns 400 if the stashed redirect_uri is non-loopback (defense-in-depth)", async () => {
      const { handleMcpCallback, encodeStashedState, stateKey } = await import(
        "../../src/oauth/auth0-proxy.js"
      );
      // Manually craft a token with an evil redirect_uri — the authorize proxy
      // would never produce this, but this tests the relay's guard.
      const key = stateKey();
      expect(key).toBeDefined();
      const stashed = encodeStashedState(
        key as Buffer,
        "https://evil.example.com/callback",
        "state",
      );
      const url = new URL("https://temperkb.io/api/auth/mcp-callback");
      url.searchParams.set("code", "abc");
      url.searchParams.set("state", stashed);

      const res = handleMcpCallback(new Request(url.toString()));
      expect(res.status).toBe(400);
      expect(await res.text()).toContain("Invalid redirect_uri");
    });

    it("returns 400 when code or state is missing", async () => {
      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const res = handleMcpCallback(
        new Request("https://temperkb.io/api/auth/mcp-callback?code=abc"),
      );
      expect(res.status).toBe(400);
    });

    it("reports the relay absent with a 404 when AS_ISSUER is set", async () => {
      // On an instance where Temper is the authorization server there is no Auth0
      // to redirect here, so the endpoint the proxy owns does not exist. The
      // request below is the one that succeeds in Auth0 mode (the round-trip case
      // above), so a 404 here is the mode deciding it, not the request's shape.
      const { proxyAuthorize, handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");

      // Build the state token as an Auth0-fronted instance would, so the request
      // below is exactly the one the round-trip case above serves.
      const authUrl = new URL("https://temperkb.io/oauth/authorize");
      authUrl.searchParams.set("client_id", "test-client-id");
      authUrl.searchParams.set("redirect_uri", "http://127.0.0.1:19876/mcp/oauth/callback");
      authUrl.searchParams.set("response_type", "code");
      authUrl.searchParams.set("state", "client-csrf-state");
      const authRes = await proxyAuthorize(new Request(authUrl.toString()));
      const stashedState = new URL(authRes.headers.get("location") as string).searchParams.get(
        "state",
      );
      expect(stashedState).not.toBeNull();

      process.env.AS_ISSUER = "https://temper.acme.com";
      const cbUrl = new URL("https://temperkb.io/api/auth/mcp-callback");
      cbUrl.searchParams.set("code", "auth-code-from-auth0");
      cbUrl.searchParams.set("state", stashedState as string);

      const res = handleMcpCallback(new Request(cbUrl.toString()));
      expect(res.status).toBe(404);
    });

    it("names MCP_PROXY_SECRET in a 503 when it is unset, rather than blaming the token", async () => {
      // The token below is well-formed and would decode under the key that minted
      // it, so the only thing left to explain the answer is the missing secret.
      // The decode failure path returns 400 "Invalid or expired state token", and
      // a configuration fault arriving in that costume sends the operator looking
      // at the client.
      const stashed = await mintStashedState();

      vi.resetModules();
      delete process.env.MCP_PROXY_SECRET;

      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/api/auth/mcp-callback");
      url.searchParams.set("code", "abc");
      url.searchParams.set("state", stashed);

      const res = handleMcpCallback(new Request(url.toString()));
      expect(res.status).toBe(503);
      expect(await res.text()).toContain("MCP_PROXY_SECRET");
    });

    it("names MCP_PROXY_SECRET in a 503 when it is shorter than the derivation accepts", async () => {
      const stashed = await mintStashedState();

      vi.resetModules();
      process.env.MCP_PROXY_SECRET = "a".repeat(31);

      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/api/auth/mcp-callback");
      url.searchParams.set("code", "abc");
      url.searchParams.set("state", stashed);

      const res = handleMcpCallback(new Request(url.toString()));
      expect(res.status).toBe(503);
      expect(await res.text()).toContain("MCP_PROXY_SECRET");
    });

    it("reports the relay absent for an Auth0 error too, so absence does not depend on the request", async () => {
      // Without this, the mode check could sit below the error branch and a SAML
      // instance would answer `?error=access_denied` with a 400 that describes a
      // login it never ran — announcing an endpoint it does not serve.
      process.env.AS_ISSUER = "https://temper.acme.com";

      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const url = new URL("https://temperkb.io/api/auth/mcp-callback");
      url.searchParams.set("error", "access_denied");
      url.searchParams.set("error_description", "nope");

      const res = handleMcpCallback(new Request(url.toString()));
      expect(res.status).toBe(404);
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

    it("still forwards when MCP_PROXY_SECRET is unset, because it stashes nothing", async () => {
      delete process.env.MCP_PROXY_SECRET;

      const { proxyToken } = await import("../../src/oauth/auth0-proxy.js");
      const origFetch = globalThis.fetch;
      globalThis.fetch = (async () =>
        new Response(JSON.stringify({ access_token: "tok" }), {
          status: 200,
          headers: { "content-type": "application/json" },
        })) as typeof fetch;

      try {
        const res = await proxyToken(
          new Request("https://temperkb.io/oauth/token", {
            method: "POST",
            headers: { "content-type": "application/x-www-form-urlencoded" },
            body: new URLSearchParams({
              grant_type: "refresh_token",
              refresh_token: "rt1",
              client_id: "test-client-id",
            }).toString(),
          }),
        );
        expect(res.status).toBe(200);
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
