import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * The Auth0 loopback proxy has three entry points, and each one has to know which
 * deployment mode it is serving. Two of them dispatch — to the Temper AS when this
 * instance runs one, to the proxy when an external IdP fronts it — and the third,
 * the relay, has no AS counterpart to dispatch to and so reports itself absent.
 *
 * Those decisions live in three different files, which is why they are witnessed
 * together here rather than one at a time: the property is that they agree, and a
 * test that only ever sees one of them cannot observe agreement. All three read
 * the mode through `isTemperAsMode`, so what these cases pin is that each entry
 * point actually consults it and routes on the answer.
 */

const ENV = {
  MCP_BASE_URL: "https://temperkb.io",
  AUTH_ISSUER: "https://tenant.auth0.com/",
  MCP_PROXY_SECRET: "a".repeat(48),
};

/** Stands in for the Temper AS, so taking its branch is observable without a database. */
const AS_MARKER = "temper-as";

function asResponse(): Response {
  return new Response(AS_MARKER, { status: 200 });
}

describe("OAuth entry points agree on the deployment mode", () => {
  const saved: Record<string, string | undefined> = {};

  beforeEach(() => {
    for (const [k, v] of Object.entries(ENV)) {
      saved[k] = process.env[k];
      process.env[k] = v;
    }
    saved.AS_ISSUER = process.env.AS_ISSUER;
    delete process.env.AS_ISSUER;
    vi.resetModules();
    vi.doMock("../../src/oauth/endpoints.js", () => ({
      handleAuthorize: async () => asResponse(),
      handleToken: async () => asResponse(),
    }));
    vi.doMock("../../src/db.js", () => ({ getDb: () => ({}) }));
    return () => {
      vi.doUnmock("../../src/oauth/endpoints.js");
      vi.doUnmock("../../src/db.js");
      for (const [k, v] of Object.entries(saved)) {
        if (v === undefined) delete process.env[k];
        else process.env[k] = v;
      }
    };
  });

  const authorizeRequest = () =>
    new Request(
      "https://temperkb.io/oauth/authorize?response_type=code&client_id=c&state=x" +
        "&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback",
    );

  const tokenRequest = () =>
    new Request("https://temperkb.io/oauth/token", {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: "grant_type=refresh_token&refresh_token=rt&client_id=c",
    });

  describe("under an external IdP (no AS_ISSUER)", () => {
    it("/oauth/authorize goes to the proxy, which forwards to the IdP", async () => {
      const { GET } = await import("../../../../api/oauth/authorize.js");
      const res = await GET(authorizeRequest());

      expect(res.status).toBe(302);
      expect(new URL(res.headers.get("location") as string).origin).toBe(
        "https://tenant.auth0.com",
      );
    });

    it("/oauth/token goes to the proxy, which forwards to the IdP", async () => {
      const forwarded: string[] = [];
      const origFetch = globalThis.fetch;
      globalThis.fetch = (async (input: RequestInfo | URL) => {
        forwarded.push(new Request(input).url);
        return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
      }) as typeof fetch;

      try {
        const { POST } = await import("../../../../api/oauth/token.js");
        const res = await POST(tokenRequest());

        expect(res.status).toBe(200);
        expect(forwarded).toEqual(["https://tenant.auth0.com/oauth/token"]);
      } finally {
        globalThis.fetch = origFetch;
      }
    });

    it("the relay is served", async () => {
      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const res = handleMcpCallback(
        new Request("https://temperkb.io/api/auth/mcp-callback?error=access_denied"),
      );

      expect(res.status).toBe(400);
    });
  });

  describe("when this instance runs the Temper AS (AS_ISSUER set)", () => {
    beforeEach(() => {
      process.env.AS_ISSUER = "https://temper.acme.com";
    });

    it("/oauth/authorize goes to the AS, never to the IdP", async () => {
      const { GET } = await import("../../../../api/oauth/authorize.js");
      const res = await GET(authorizeRequest());

      expect(await res.text()).toBe(AS_MARKER);
    });

    it("/oauth/token goes to the AS, and nothing is forwarded off-instance", async () => {
      const forwarded: string[] = [];
      const origFetch = globalThis.fetch;
      globalThis.fetch = (async (input: RequestInfo | URL) => {
        forwarded.push(new Request(input).url);
        return new Response("{}", { status: 200 });
      }) as typeof fetch;

      try {
        const { POST } = await import("../../../../api/oauth/token.js");
        const res = await POST(tokenRequest());

        expect(await res.text()).toBe(AS_MARKER);
        // The AS issuer and the auth issuer are the same string on such an
        // instance, so a proxy forward here would address this very endpoint.
        expect(forwarded).toEqual([]);
      } finally {
        globalThis.fetch = origFetch;
      }
    });

    it("the relay is absent", async () => {
      const { handleMcpCallback } = await import("../../src/oauth/auth0-proxy.js");
      const res = handleMcpCallback(
        new Request("https://temperkb.io/api/auth/mcp-callback?error=access_denied"),
      );

      expect(res.status).toBe(404);
    });
  });
});
