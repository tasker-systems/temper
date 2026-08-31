/**
 * Shared environment-variable access for the OAuth Authorization Server modules.
 */

/** Read a required environment variable, throwing if it is unset or empty. */
export function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing required environment variable: ${name}`);
  }
  return value;
}

/**
 * True when this instance runs its own authorization server — the mode that backs
 * SAML SSO and temper-issued machine credentials. False when an external IdP
 * (Auth0, Okta) fronts it.
 *
 * `AS_ISSUER`'s presence is the mode signal, and this is the name for reading it.
 * The Rust surfaces parse the same signal once into `AuthMode`
 * (`crates/temper-services/src/auth_config.rs`) so temper-api and temper-mcp
 * cannot drift; this is that name on the TypeScript side, so the OAuth entry
 * points cannot drift from each other either.
 */
export function isTemperAsMode(): boolean {
  return Boolean(process.env.AS_ISSUER);
}

/**
 * The RFC 8707 resource indicators this authorization server serves tokens for: the instance
 * audience (AS_AUDIENCE) and, when configured, the MCP surface's own resource indicator
 * (MCP_AUDIENCE). Unset/empty MCP_AUDIENCE collapses the set to the one instance audience —
 * the exact fallback every consumer of this set resolves identically.
 *
 * The one home for this set because three readers must not drift: `/oauth/authorize` validates a
 * requested `resource` against it fail-closed, the PRM advertises the MCP member of it, and
 * `storeRefreshToken` refuses to persist a chain audience outside it (the write-time check that
 * keeps an audience minted at chain start the same one every rotation hands back). The Rust
 * counterpart is `AuthConfig`'s resolved audience pair (crates/temper-services/src/auth_config.rs).
 */
export function servedAudiences(): string[] {
  // Trimmed to match `mintAccessToken`'s read of the same variable (mint.ts) and the Rust boot
  // gate's parsing (auth_config.rs trims both sides) — a padded AS_AUDIENCE would otherwise be
  // stored raw on chains and minted trimmed, a mismatch that can only fail closed but shouldn't
  // exist at all. Fail-closed on trim-to-empty is fine: the minted fallback would refuse the same
  // value at the same moment.
  const asAudience = requireEnv("AS_AUDIENCE").trim();
  const mcpAudience = process.env.MCP_AUDIENCE?.trim() || asAudience;
  return mcpAudience === asAudience ? [asAudience] : [asAudience, mcpAudience];
}
