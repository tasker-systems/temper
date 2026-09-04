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
  /**
   * Declared `false` rather than omitted: Vercel Connect reads this flag to decide whether CIMD
   * is available, and an absent field is not an honest `no`. Real dynamic registration runs at
   * `/oauth/clients` (see `registration_endpoint` below); CIMD would additionally require
   * `private_key_jwt` client auth or public machine clients, neither of which this AS ships.
   */
  client_id_metadata_document_supported: boolean;
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
 * `registration_endpoint` advertises the REAL dynamic-client-registration handler
 * (`api/oauth/clients.ts` → `src/oauth/register.ts`), not the Auth0-echo proxy at
 * `/oauth/register`: that proxy (`crates/temper-mcp/src/discovery.rs`, reached through the
 * `vercel.json` `/oauth/(.*)` catch-all) mints no client, so a client that registers through
 * it can never authenticate at the token endpoint as itself. The `/oauth/clients` handler's
 * MCP-compat class returns the same pre-registered `MCP_CLIENT_ID` the proxy did, so MCP
 * clients that only read this document (Claude Code/Desktop ignore a configured static
 * `client_id` and fall back to DCR regardless) keep working on SAML instances; the proxy is
 * left untouched for Auth0-fronted instances, whose document still advertises it.
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
    registration_endpoint: `${iss}/oauth/clients`,
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
    client_id_metadata_document_supported: false,
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
  audience: string;
}): Auth0AsMetadata {
  const auth0 = cfg.auth0Domain.replace(/\/+$/, "");
  const base = cfg.base.replace(/\/+$/, "");

  return {
    issuer: `${auth0}/`,
    // Authorize and token endpoints are proxied through temperkb.io so loopback
    // redirect_uris (http://127.0.0.1:<port>/callback) can be rewritten to the
    // relay URL. Auth0's exact-match callback allowlist rejects loopback URLs
    // with ports even for Native apps on some tenants. See auth0-proxy.ts.
    authorization_endpoint: `${base}/oauth/authorize`,
    token_endpoint: `${base}/oauth/token`,
    registration_endpoint: `${base}/oauth/register`,
    scopes_supported: ["openid", "profile", "email", "offline_access"],
    response_types_supported: ["code"],
    // client_credentials (Stage 4a): lets M2M agent principals mint tokens via Auth0.
    grant_types_supported: ["authorization_code", "refresh_token", "client_credentials"],
    code_challenge_methods_supported: ["S256"],
    resource: cfg.audience,
  };
}

/**
 * The well-known path suffix RFC 8414 §3.1 has this instance's own issuer answer on.
 *
 * A pathless issuer (`https://temper.example.com`) answers at the bare well-known (suffix "");
 * a path-bearing issuer (`https://host/tenants/acme`) answers at
 * `/.well-known/oauth-authorization-server/tenants/acme` — the well-known inserted between
 * host and path. Derived from the issuer the served document itself advertises, so the doc
 * and the path it answers on cannot disagree.
 */
function issuerWellKnownSuffix(issuer: string): string {
  try {
    return new URL(issuer).pathname.replace(/^\/+|\/+$/g, "");
  } catch {
    // An unparsable issuer has bigger problems than its well-known path; treat it as
    // pathless so the bare route still serves rather than 500ing the document away.
    return "";
  }
}

/**
 * `GET /.well-known/oauth-authorization-server` — the single RFC 8414 handler for BOTH
 * instance types (SAML/AS instances that set `AS_ISSUER`, and the legacy Auth0-fronted
 * instance that doesn't). This migrated the doc off the Rust MCP function
 * (`crates/temper-mcp/src/discovery.rs`) so a single shared `vercel.json` can serve the right
 * AS metadata per instance without env-conditional routing, which Vercel's static route table
 * can't express. The Auth0 branch below is byte-identical to the former Rust handler.
 *
 * The route also answers the RFC 8414 §3.1 path-suffixed form a conformant client computes
 * for a path-bearing issuer. Only the suffix the advertised issuer actually implies is
 * served; any other path 404s rather than serving a copy of this instance's document under
 * a path it never claimed. The suffix arrives as the `issuer_path` query param because a
 * `vercel.json` routes rewrite need not preserve the original path in `req.url`.
 */
export async function handleAuthorizationServer(req: Request): Promise<Response> {
  const asIssuer = process.env.AS_ISSUER;
  const requested = (new URL(req.url).searchParams.get("issuer_path") ?? "").replace(
    /^\/+|\/+$/g,
    "",
  );
  const expected = issuerWellKnownSuffix(asIssuer ?? requireEnv("AUTH_ISSUER"));
  if (requested !== expected) {
    return new Response("Not Found", { status: 404 });
  }

  const body: AsMetadata | Auth0AsMetadata = asIssuer
    ? buildAsMetadata(asIssuer)
    : buildAuth0AsMetadata({
        base: requireEnv("MCP_BASE_URL"),
        auth0Domain: requireEnv("AUTH_ISSUER"),
        // This document's `resource` answers API callers — the temper CLI and M2M clients read
        // it to learn what `aud` their tokens must carry — so it is AUTH_AUDIENCE, the API
        // audience. The MCP surface has its own resource indicator now: `MCP_AUDIENCE`, the
        // value the Rust PRM (`crates/temper-mcp/src/discovery.rs`) advertises because
        // conformant MCP clients refuse a resource that is neither the MCP server URL nor its
        // origin. Each discovery door states the fact for the audience that discovers through
        // it; they agree by construction when `MCP_AUDIENCE` is unset, which is the fallback
        // both sides resolve identically (`?.trim() ||` here, `unwrap_or_else` there — an
        // empty value must mean "absent", never a literal empty audience).
        audience: process.env.AUTH_AUDIENCE?.trim() || requireEnv("AUTH_AUDIENCE"),
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
