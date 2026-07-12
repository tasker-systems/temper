import { requireEnv } from "./env.js";
import { getPublicJwks } from "./keys.js";

/** RFC 8414 authorization-server metadata for Temper's own OAuth AS (SAML instances). */
export interface AsMetadata {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  registration_endpoint: string;
  jwks_uri: string;
  scopes_supported: string[];
  response_types_supported: string[];
  grant_types_supported: string[];
  code_challenge_methods_supported: string[];
  token_endpoint_auth_methods_supported: string[];
}

/** RFC 8414 authorization-server metadata for the legacy Auth0-fronted instance (temperkb.io). */
export interface Auth0AsMetadata {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  registration_endpoint: string;
  scopes_supported: string[];
  response_types_supported: string[];
  grant_types_supported: string[];
  code_challenge_methods_supported: string[];
  resource: string;
}

/**
 * Builds RFC 8414 metadata for the Temper AS itself. Trims a trailing slash from `issuer`.
 *
 * `registration_endpoint` advertises the thin DCR proxy (`crates/temper-mcp/src/discovery.rs`,
 * reachable at `/oauth/register` via the `vercel.json` catch-all) so MCP clients that require
 * dynamic client registration — current Claude Code/Desktop ignore a configured static `client_id`
 * and fall back to DCR regardless — can complete the OAuth handshake on SAML instances. The proxy
 * only echoes the pre-registered `MCP_CLIENT_ID`; it never persists client-supplied redirect URIs,
 * so the open-redirect protection at `/oauth/authorize` (`clients.ts`) is unweakened.
 *
 * `scopes_supported` matches the protected-resource metadata (`discovery.rs`) so a conformant client
 * requesting `offline_access` gets a refresh token rather than re-authing on each access-token expiry.
 */
export function buildAsMetadata(issuer: string): AsMetadata {
  const iss = issuer.replace(/\/+$/, "");

  return {
    issuer: iss,
    authorization_endpoint: `${iss}/oauth/authorize`,
    token_endpoint: `${iss}/oauth/token`,
    registration_endpoint: `${iss}/oauth/register`,
    jwks_uri: `${iss}/oauth/jwks`,
    scopes_supported: ["openid", "profile", "email", "offline_access"],
    response_types_supported: ["code"],
    // client_credentials (Phase B1): this AS mints machine tokens itself. Advertising it is not
    // cosmetic — a conformant client reads this document to decide whether M2M is possible at all.
    grant_types_supported: ["authorization_code", "refresh_token", "client_credentials"],
    code_challenge_methods_supported: ["S256"],
    // `none` for the PKCE public client; the secret-bearing methods are the machine grant's,
    // which `readClientCredentials` accepts in either form (Basic preferred, RFC 6749 §2.3.1).
    token_endpoint_auth_methods_supported: ["none", "client_secret_basic", "client_secret_post"],
  };
}

/**
 * Builds RFC 8414 metadata for the legacy Auth0-fronted instance. Byte-identical to the
 * retired Rust MCP handler (`crates/temper-mcp/src/discovery.rs`,
 * `authorization_server_metadata`): `auth0Domain` is trimmed of a trailing slash before use,
 * but `base` is used raw (no trimming) for `registration_endpoint`, matching Rust exactly.
 */
export function buildAuth0AsMetadata(cfg: {
  base: string;
  auth0Domain: string;
  mcpAudience: string;
}): Auth0AsMetadata {
  const auth0 = cfg.auth0Domain.replace(/\/+$/, "");

  return {
    issuer: `${auth0}/`,
    authorization_endpoint: `${auth0}/authorize`,
    token_endpoint: `${auth0}/oauth/token`,
    registration_endpoint: `${cfg.base}/oauth/register`,
    scopes_supported: ["openid", "profile", "email", "offline_access"],
    response_types_supported: ["code"],
    // client_credentials (Stage 4a): lets M2M agent principals mint tokens via Auth0.
    grant_types_supported: ["authorization_code", "refresh_token", "client_credentials"],
    code_challenge_methods_supported: ["S256"],
    resource: cfg.mcpAudience,
  };
}

/**
 * `GET /.well-known/oauth-authorization-server` — the single RFC 8414 handler for BOTH
 * instance types (SAML/AS instances that set `AS_ISSUER`, and the legacy Auth0-fronted
 * instance that doesn't). This migrated the doc off the Rust MCP function
 * (`crates/temper-mcp/src/discovery.rs`) so a single shared `vercel.json` can serve the right
 * AS metadata per instance without env-conditional routing, which Vercel's static route table
 * can't express. The Auth0 branch below is byte-identical to the former Rust handler.
 */
export async function handleAuthorizationServer(_req: Request): Promise<Response> {
  const asIssuer = process.env.AS_ISSUER;
  const body: AsMetadata | Auth0AsMetadata = asIssuer
    ? buildAsMetadata(asIssuer)
    : buildAuth0AsMetadata({
        base: requireEnv("MCP_BASE_URL"),
        auth0Domain: requireEnv("AUTH_ISSUER"),
        mcpAudience: process.env.MCP_AUDIENCE ?? requireEnv("AUTH_AUDIENCE"),
      });

  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

/**
 * `GET /oauth/jwks` — publishes the Temper AS's public JWKS. Only meaningful for SAML/AS
 * instances (`AS_ISSUER` set); Auth0-fronted instances host their JWKS at Auth0 and MCP never
 * served a local `/oauth/jwks`, so a 404 here preserves today's behavior for them.
 */
export async function handleJwks(_req: Request): Promise<Response> {
  if (!process.env.AS_ISSUER) {
    return new Response("Not Found", { status: 404 });
  }

  return new Response(JSON.stringify(await getPublicJwks()), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}
