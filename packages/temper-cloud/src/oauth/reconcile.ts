import { logger } from "../logger.js";
import { requireEnv } from "./env.js";

/**
 * Wire payload for the internal SAML reconcile call. Mirrors the Rust
 * `temper_core::types::ReconcileRequest` field-for-field. temper-cloud cannot import temper-ui's
 * ts-rs-generated types (separate package), so — like `MintedClaims` in mint.ts — this is a local
 * interface whose parity with the Rust struct is enforced by tests/oauth/wire-contract.test.ts.
 */
export interface ReconcileRequest {
  provider: string | null;
  external_user_id: string;
  email: string;
  email_verified: boolean | null;
  idp_key: string;
  groups: string[];
}

/**
 * Calls the internal temper-api reconcile endpoint (server-to-server) with the shared secret.
 * Throws on transport error or non-2xx — the ACS handler catches and proceeds (fail-open), so a
 * provisioning hiccup never blocks login. The header name matches temper-api's INTERNAL_SECRET_HEADER.
 */
export async function reconcileMemberships(payload: ReconcileRequest): Promise<void> {
  const url = requireEnv("INTERNAL_RECONCILE_URL");
  const secret = requireEnv("INTERNAL_RECONCILE_SECRET");
  const res = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Temper-Internal-Secret": secret,
    },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    throw new Error(`reconcile endpoint returned ${res.status}`);
  }
  logger.info({ idp_key: payload.idp_key, groups: payload.groups.length }, "saml reconcile ok");
}
