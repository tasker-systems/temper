import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { buildAsMetadata, buildAuth0AsMetadata } from "../../src/oauth/metadata.js";

describe("buildAsMetadata", () => {
  it("builds the Temper AS's own metadata, trimming a trailing slash from the issuer", () => {
    const meta = buildAsMetadata("https://saml.example.com/");

    expect(meta).toEqual({
      issuer: "https://saml.example.com",
      authorization_endpoint: "https://saml.example.com/oauth/authorize",
      token_endpoint: "https://saml.example.com/oauth/token",
      registration_endpoint: "https://saml.example.com/oauth/register",
      jwks_uri: "https://saml.example.com/oauth/jwks",
      scopes_supported: ["openid", "profile", "email", "offline_access"],
      response_types_supported: ["code"],
      grant_types_supported: ["authorization_code", "refresh_token", "client_credentials"],
      code_challenge_methods_supported: ["S256"],
      token_endpoint_auth_methods_supported: ["none", "client_secret_basic", "client_secret_post"],
    });
  });

  // This AS has minted machine tokens since Phase B1, but advertised only two grants — so a
  // conformant client reading this document would conclude M2M was impossible against temper's
  // own issuer, while the Auth0-fronted metadata (below) advertised it correctly.
  it("advertises the client_credentials grant it actually implements", () => {
    const meta = buildAsMetadata("https://saml.example.com");

    expect(meta.grant_types_supported).toContain("client_credentials");
    expect(meta.token_endpoint_auth_methods_supported).toContain("client_secret_post");
    expect(meta.token_endpoint_auth_methods_supported).toContain("client_secret_basic");
  });

  it("leaves an issuer with no trailing slash unchanged", () => {
    const meta = buildAsMetadata("https://saml.example.com");

    expect(meta.issuer).toBe("https://saml.example.com");
    expect(meta.authorization_endpoint).toBe("https://saml.example.com/oauth/authorize");
  });

  it("advertises the DCR registration endpoint so MCP clients can complete the handshake", () => {
    // MCP clients (Claude Code/Desktop) require dynamic client registration and abort the OAuth
    // walk if the AS metadata omits registration_endpoint — the bug this field fixes (issue #293).
    const meta = buildAsMetadata("https://saml.example.com/");
    expect(meta.registration_endpoint).toBe("https://saml.example.com/oauth/register");
  });

  it("advertises offline_access so conformant clients get a refresh token", () => {
    const meta = buildAsMetadata("https://saml.example.com");
    expect(meta.scopes_supported).toContain("offline_access");
  });
});

describe("buildAuth0AsMetadata", () => {
  it("points authorize and token at the instance (proxied), not Auth0 directly", () => {
    const meta = buildAuth0AsMetadata({
      base: "https://temperkb.io",
      auth0Domain: "https://tenant.auth0.com/",
      audience: "https://api.temperkb.io",
    });

    expect(meta).toEqual({
      issuer: "https://tenant.auth0.com/",
      authorization_endpoint: "https://temperkb.io/oauth/authorize",
      token_endpoint: "https://temperkb.io/oauth/token",
      registration_endpoint: "https://temperkb.io/oauth/register",
      scopes_supported: ["openid", "profile", "email", "offline_access"],
      response_types_supported: ["code"],
      grant_types_supported: ["authorization_code", "refresh_token", "client_credentials"],
      code_challenge_methods_supported: ["S256"],
      resource: "https://api.temperkb.io",
    });
  });

  it("advertises client_credentials for M2M agent principals (Stage 4a)", () => {
    const meta = buildAuth0AsMetadata({
      base: "https://temperkb.io",
      auth0Domain: "https://tenant.auth0.com/",
      audience: "https://api.temperkb.io",
    });
    expect(meta.grant_types_supported).toContain("client_credentials");
    expect(meta.grant_types_supported).toContain("authorization_code");
    expect(meta.grant_types_supported).toContain("refresh_token");
  });

  it("trims a trailing slash from both auth0Domain and base before building endpoints", () => {
    const meta = buildAuth0AsMetadata({
      base: "https://temperkb.io/",
      auth0Domain: "https://tenant.auth0.com",
      audience: "https://api.temperkb.io",
    });

    expect(meta.issuer).toBe("https://tenant.auth0.com/");
    expect(meta.authorization_endpoint).toBe("https://temperkb.io/oauth/authorize");
    expect(meta.token_endpoint).toBe("https://temperkb.io/oauth/token");
    expect(meta.registration_endpoint).toBe("https://temperkb.io/oauth/register");
  });
});

describe("handleJwks", () => {
  const originalAsIssuer = process.env.AS_ISSUER;

  beforeEach(() => {
    delete process.env.AS_ISSUER;
  });

  afterEach(() => {
    if (originalAsIssuer === undefined) {
      delete process.env.AS_ISSUER;
    } else {
      process.env.AS_ISSUER = originalAsIssuer;
    }
  });

  it("returns 404 when AS_ISSUER is unset (Auth0 instances host JWKS at Auth0)", async () => {
    const { handleJwks } = await import("../../src/oauth/metadata.js");
    const res = await handleJwks(new Request("https://example.com/oauth/jwks"));

    expect(res.status).toBe(404);
  });
});

describe("handleAuthorizationServer (RFC 8414 §3.1 path-suffixed form)", () => {
  const originalAsIssuer = process.env.AS_ISSUER;
  const originalAuthIssuer = process.env.AUTH_ISSUER;
  const originalMcpBaseUrl = process.env.MCP_BASE_URL;
  const originalAuthAudience = process.env.AUTH_AUDIENCE;

  function requestFor(suffix: string): Request {
    // The route hands the suffix over as a query param because a routes rewrite need not
    // preserve the original path in req.url — the handler must read it there.
    return new Request(
      `https://temper.example.com/api/oauth/authorization-server?issuer_path=${suffix}`,
    );
  }

  beforeEach(() => {
    process.env.AS_ISSUER = "https://temper.example.com";
    process.env.AUTH_ISSUER = "https://tenant.auth0.com";
    process.env.MCP_BASE_URL = "https://temper.example.com";
    process.env.AUTH_AUDIENCE = "https://api.temper.example.com";
  });

  afterEach(() => {
    for (const [name, value] of [
      ["AS_ISSUER", originalAsIssuer],
      ["AUTH_ISSUER", originalAuthIssuer],
      ["MCP_BASE_URL", originalMcpBaseUrl],
      ["AUTH_AUDIENCE", originalAuthAudience],
    ] as const) {
      if (value === undefined) {
        delete process.env[name];
      } else {
        process.env[name] = value;
      }
    }
  });

  it("serves the bare well-known for a pathless issuer (unchanged behavior)", async () => {
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    const res = await handleAuthorizationServer(requestFor(""));
    const body = (await res.json()) as { issuer: string };

    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("application/json");
    expect(body.issuer).toBe("https://temper.example.com");
  });

  it("serves the matching suffix when the issuer bears a path", async () => {
    process.env.AS_ISSUER = "https://host.example.com/tenants/acme";
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    const res = await handleAuthorizationServer(requestFor("tenants/acme"));
    const body = (await res.json()) as { issuer: string };

    expect(res.status).toBe(200);
    expect(body.issuer).toBe("https://host.example.com/tenants/acme");
  });

  it("404s a suffix the pathless issuer never claimed", async () => {
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    const res = await handleAuthorizationServer(requestFor("tenants/acme"));

    expect(res.status).toBe(404);
  });

  it("404s every suffix but the issuer's own when the issuer bears a path", async () => {
    process.env.AS_ISSUER = "https://host.example.com/tenants/acme";
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    const res = await handleAuthorizationServer(requestFor("tenants/other"));

    expect(res.status).toBe(404);
  });

  // The security-relevant invariant, locked as a table: suffix matching is pure string
  // equality after one searchParams decode and a slash-trim — no path normalization, no
  // case folding, no encoding games. A traversal-shaped or encoded suffix selects nothing
  // but its own equality, so none of these may serve the document.
  it("404s adversarial suffixes — no normalization but the slash-trim (security pass, 2026-08-29)", async () => {
    process.env.AS_ISSUER = "https://host.example.com/tenants/acme";
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    for (const suffix of [
      "tenants/Acme", // case
      "tenants/acme/..", // traversal suffix — inert: nothing is resolved
      "%252Ftenants%252Facme", // double-encoded slash decodes once, stays literal %2F
      "tenants//acme", // inner double slash
      "tenants%5Cacme", // backslash is not a separator here
      "tenants%20acme", // percent-encoded space
      "+", // form-encoded space
      "tenants/acme%00", // trailing null byte
      "％2F", // fullwidth slash
    ]) {
      const res = await handleAuthorizationServer(requestFor(suffix));
      expect(res.status, `suffix ${suffix}`).toBe(404);
    }
  });

  it("serves slash-padded aliases of the claimed path (same path, semantically)", async () => {
    process.env.AS_ISSUER = "https://host.example.com/tenants/acme";
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    for (const suffix of ["/tenants/acme/", "/tenants/acme", "tenants/acme/"]) {
      const res = await handleAuthorizationServer(requestFor(suffix));
      expect(res.status, `suffix ${suffix}`).toBe(200);
    }
  });

  it("serves the bare well-known in AS mode without any AUTH_* env (?? short-circuits)", async () => {
    delete process.env.AUTH_ISSUER;
    delete process.env.MCP_BASE_URL;
    delete process.env.AUTH_AUDIENCE;
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    const res = await handleAuthorizationServer(requestFor(""));
    const body = (await res.json()) as { issuer: string };

    expect(res.status).toBe(200);
    expect(body.issuer).toBe("https://temper.example.com");
  });

  it("404s suffixed requests on the Auth0 arm (the Auth0 domain is pathless)", async () => {
    delete process.env.AS_ISSUER;
    const { handleAuthorizationServer } = await import("../../src/oauth/metadata.js");
    const res = await handleAuthorizationServer(requestFor("tenants/acme"));

    expect(res.status).toBe(404);
  });
});
