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
