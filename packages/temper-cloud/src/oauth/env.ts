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
