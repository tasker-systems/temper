import { createHmac } from "node:crypto";
import { logger } from "../logger.js";
import { requireEnv } from "./env.js";

/** Ceiling on an internal server-to-server call, so a slow dependency fails open rather than hanging. */
const INTERNAL_CALL_TIMEOUT_MS = 5000;

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
  /**
   * The asserted group values, or `null` when the assertion carried NO group signal — mirrors
   * `extractGroups`'s own return type (`../saml/sp.ts`) rather than collapsing it at the wire.
   * `null` and `[]` are opposite instructions: "the provider said nothing, act on nothing" and
   * "the provider named this principal's groups and there are none". The API refuses the first as
   * an input to revocation and records that it did.
   */
  groups: string[] | null;
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
    // Fail-open is only a mercy if it fails FAST. Both callers catch and carry on, but an
    // unbounded fetch does not reach the catch — it holds the request until the platform's own
    // function timeout, turning a degraded dependency into a stalled login or rotation. Since the
    // migration, the first rotation of every pre-existing chain takes the resolve branch, so this
    // sits on a hot path rather than a rare one.
    signal: AbortSignal.timeout(INTERNAL_CALL_TIMEOUT_MS),
  });
  if (!res.ok) {
    throw new Error(`reconcile endpoint returned ${res.status}`);
  }
  // `null` stays null in the log line rather than becoming 0: a login that carried no group signal
  // and one that asserted an empty group set are the two cases this whole path exists to keep
  // apart, and `groups: 0` would have read as the second.
  logger.info(
    { idp_key: payload.idp_key, groups: payload.groups?.length ?? null },
    "saml reconcile ok",
  );
}
