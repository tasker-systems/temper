import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { buildAsMetadata, buildAuth0AsMetadata } from "../../src/oauth/metadata.js";

describe("buildAsMetadata", () => {
  it("builds the Temper AS's own metadata, trimming a trailing slash from the issuer", () => {
    const meta = buildAsMetadata("https://saml.example.com/");

    expect(meta).toEqual({
      issuer: "https://saml.example.com",
      authorization_endpoint: "https://saml.example.com/oauth/authorize",
      token_endpoint: "https://saml.example.com/oauth/token",
      jwks_uri: "https://saml.example.com/oauth/jwks",
      response_types_supported: ["code"],
      grant_types_supported: ["authorization_code", "refresh_token"],
      code_challenge_methods_supported: ["S256"],
      token_endpoint_auth_methods_supported: ["none"],
    });
  });

  it("leaves an issuer with no trailing slash unchanged", () => {
    const meta = buildAsMetadata("https://saml.example.com");

    expect(meta.issuer).toBe("https://saml.example.com");
    expect(meta.authorization_endpoint).toBe("https://saml.example.com/oauth/authorize");
  });
});

describe("buildAuth0AsMetadata", () => {
  it("is byte-identical to the retired Rust MCP handler's output", () => {
    const meta = buildAuth0AsMetadata({
      base: "https://temperkb.io",
      auth0Domain: "https://tenant.auth0.com/",
      mcpAudience: "https://api.temperkb.io",
    });

    expect(meta).toEqual({
      issuer: "https://tenant.auth0.com/",
      authorization_endpoint: "https://tenant.auth0.com/authorize",
      token_endpoint: "https://tenant.auth0.com/oauth/token",
      registration_endpoint: "https://temperkb.io/oauth/register",
      scopes_supported: ["openid", "profile", "email", "offline_access"],
      response_types_supported: ["code"],
      grant_types_supported: ["authorization_code", "refresh_token"],
      code_challenge_methods_supported: ["S256"],
      resource: "https://api.temperkb.io",
    });
  });

  it("trims a trailing slash from auth0Domain before building endpoints", () => {
    const meta = buildAuth0AsMetadata({
      base: "https://temperkb.io",
      auth0Domain: "https://tenant.auth0.com",
      mcpAudience: "https://api.temperkb.io",
    });

    expect(meta.issuer).toBe("https://tenant.auth0.com/");
    expect(meta.authorization_endpoint).toBe("https://tenant.auth0.com/authorize");
    expect(meta.token_endpoint).toBe("https://tenant.auth0.com/oauth/token");
  });

  it("uses base raw (no slash trimming) for registration_endpoint", () => {
    const meta = buildAuth0AsMetadata({
      base: "https://temperkb.io/",
      auth0Domain: "https://tenant.auth0.com",
      mcpAudience: "https://api.temperkb.io",
    });

    expect(meta.registration_endpoint).toBe("https://temperkb.io//oauth/register");
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
