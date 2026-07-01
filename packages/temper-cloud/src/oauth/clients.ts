/**
 * Registered OAuth client allowlist for the Authorization Server.
 *
 * The AS is only safe to expose an `/oauth/authorize` endpoint for if it refuses to redirect to
 * an arbitrary `redirect_uri`: otherwise an attacker can craft an authorize link with their own
 * PKCE pair and a `redirect_uri` they control, trick a victim into completing the SAML login, and
 * capture the resulting authorization code (a stateless open-redirect-to-account-takeover chain).
 * `AS_CLIENTS` is the single source of truth for which `redirect_uri`s each registered `client_id`
 * may use.
 */

/** Maps a registered client_id to its allowlisted redirect_uris. */
export interface ClientRegistry {
  [clientId: string]: string[];
}

/**
 * Parses `AS_CLIENTS` (JSON `Record<string, string[]>`) into a `ClientRegistry`. Unset or an empty
 * string deny-alls (returns `{}`, so every `isRedirectUriAllowed` check fails closed). Throws if
 * the value is set but isn't valid JSON, or isn't an object mapping client ids to string arrays --
 * a malformed allowlist should fail loudly at startup, not silently deny (or worse, silently allow).
 */
export function loadClientRegistry(): ClientRegistry {
  const raw = process.env.AS_CLIENTS;
  if (!raw) {
    return {};
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error("AS_CLIENTS must be JSON {clientId: string[]}");
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("AS_CLIENTS must be JSON {clientId: string[]}");
  }
  for (const value of Object.values(parsed as Record<string, unknown>)) {
    if (!Array.isArray(value) || !value.every((uri) => typeof uri === "string")) {
      throw new Error("AS_CLIENTS must be JSON {clientId: string[]}");
    }
  }

  return parsed as ClientRegistry;
}

/**
 * Returns true iff `clientId` is registered and its allowlist contains `redirectUri` by exact
 * string match. Exact match only -- proportionate and safest for Phase 1: RFC 8252 loopback-any-port
 * matching (matching `redirect_uri`s that vary only in an ephemeral loopback port) can be added
 * later if/when a native loopback client is registered. Phase-1 clients (temper-cli, temper-ui) use
 * fixed redirect URIs, so exact match is sufficient.
 */
export function isRedirectUriAllowed(
  registry: ClientRegistry,
  clientId: string,
  redirectUri: string,
): boolean {
  const allowed = registry[clientId];
  return Array.isArray(allowed) && allowed.includes(redirectUri);
}
