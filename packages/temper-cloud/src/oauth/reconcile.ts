import { createHmac } from "node:crypto";
import { logger } from "../logger.js";
import { requireEnv } from "./env.js";

/** Header carrying the Unix-seconds timestamp the signature was computed over. */
export const TIMESTAMP_HEADER = "X-Temper-Timestamp";
/** Header carrying the lowercase-hex HMAC-SHA256 signature. */
export const SIGNATURE_HEADER = "X-Temper-Signature";

/**
 * HMAC-SHA256 over the message `{timestamp}.{body}`, lowercase hex. Mirrors the Rust
 * verifier `temper_core::internal_sig::sign` field-for-field; the two are pinned together
 * by the shared known-answer vector in tests/oauth/wire-contract.test.ts. We sign the RAW
 * body string exactly as it is sent (never a re-serialized form) so there is no
 * cross-language canonicalization to drift on.
 */
export function signReconcile(secret: string, timestamp: number, body: string): string {
  return createHmac("sha256", secret).update(`${timestamp}.${body}`).digest("hex");
}

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
 * Calls the internal temper-api reconcile endpoint (server-to-server), signing the request with
 * `HMAC(secret, "{timestamp}.{body}")` so the secret never crosses the wire and a captured request
 * is replay-proof. Throws on transport error or non-2xx — the ACS handler catches and proceeds
 * (fail-open), so a provisioning hiccup never blocks login. The verifier is temper-api's
 * `require_internal_signature` middleware.
 */
export async function reconcileMemberships(payload: ReconcileRequest): Promise<void> {
  const url = requireEnv("INTERNAL_RECONCILE_URL");
  const secret = requireEnv("INTERNAL_RECONCILE_SECRET");
  // Sign the exact body string we send — raw-body discipline, no re-serialization.
  const body = JSON.stringify(payload);
  const timestamp = Math.floor(Date.now() / 1000);
  const signature = signReconcile(secret, timestamp, body);
  const res = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      [TIMESTAMP_HEADER]: String(timestamp),
      [SIGNATURE_HEADER]: signature,
    },
    body,
  });
  if (!res.ok) {
    throw new Error(`reconcile endpoint returned ${res.status}`);
  }
  logger.info({ idp_key: payload.idp_key, groups: payload.groups.length }, "saml reconcile ok");
}
