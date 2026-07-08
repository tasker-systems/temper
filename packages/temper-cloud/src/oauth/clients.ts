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
 * RFC 8252 §7.3 loopback hosts. `localhost` is included pragmatically — some native clients
 * (Claude Code historically) use it even though the RFC recommends the IP literal.
 */
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "[::1]", "localhost"]);

/** True for `http://` redirect URIs whose host is a loopback host (RFC 8252 §7.3). */
function isLoopback(u: URL): boolean {
  return u.protocol === "http:" && LOOPBACK_HOSTS.has(u.hostname);
}

/**
 * Returns true iff `clientId` is registered and its allowlist permits `redirectUri`.
 *
 * Exact string match wins first — this is the only match non-loopback clients (temper-ui, and the
 * `claude.ai`/`claude.com` desktop/web connector with its fixed HTTPS callbacks) ever get, so they
 * are unaffected by the loopback path below.
 *
 * Loopback URIs additionally get RFC 8252 §7.3 port-flexible matching: a native CLI MCP client
 * (Claude Code) runs a local callback server on an ephemeral port (`http://127.0.0.1:<random>/callback`),
 * which can never exact-match an allowlist entry. An incoming loopback URI matches any allowlisted
 * loopback entry that shares its **path** — port AND loopback host are ignored (an allowlisted
 * `127.0.0.1` entry matches an incoming `localhost` URI and vice-versa). This stays confined to the
 * local machine (both sides must be loopback) and removes the operator foot-gun of having to
 * allowlist the exact loopback host the client happens to send. Non-loopback URIs never reach this
 * branch, so the open-redirect protection for remote redirect targets is unchanged.
 */
export function isRedirectUriAllowed(
  registry: ClientRegistry,
  clientId: string,
  redirectUri: string,
): boolean {
  const allowed = registry[clientId];
  if (!Array.isArray(allowed)) {
    return false;
  }
  if (allowed.includes(redirectUri)) {
    return true; // exact match — all non-loopback clients
  }

  let incoming: URL;
  try {
    incoming = new URL(redirectUri);
  } catch {
    return false;
  }
  if (!isLoopback(incoming)) {
    return false; // only loopback URIs get port-/host-flexible matching
  }

  return allowed.some((entry) => {
    let a: URL;
    try {
      a = new URL(entry);
    } catch {
      return false;
    }
    // Any loopback host matches any loopback host; the path must still match. Port is ignored.
    return isLoopback(a) && a.pathname === incoming.pathname;
  });
}
