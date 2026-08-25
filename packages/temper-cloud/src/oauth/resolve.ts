import { logger } from "../logger.js";
import { requireEnv } from "./env.js";
import { SIGNATURE_HEADER, signReconcile, TIMESTAMP_HEADER } from "./reconcile.js";

/**
 * Wire payload for the internal principal-resolve call. Mirrors the Rust
 * `temper_core::types::ResolvePrincipalRequest` field-for-field; parity is enforced by
 * tests/oauth/wire-contract.test.ts, the same way `ReconcileRequest` is.
 */
export interface ResolvePrincipalRequest {
  external_user_id: string;
  email: string;
  email_verified: boolean | null;
}

/** Mirrors `temper_core::types::ResolvePrincipalResponse`. */
export interface ResolvePrincipalResponse {
  profile_id: string;
}

/**
 * Asks temper-api which profile a token `sub` resolves to, so the refresh chain we are about to
 * mint can carry an owner an administrator is able to end.
 *
 * We do not do this lookup ourselves. The authoritative provider label is temper-api's
 * `AUTH_PROVIDER_NAME`, and a second copy of it in this service's environment would drift in
 * silence — the join would match nothing, `standing_service::apply` would revoke nothing, and
 * nothing anywhere would say so. Asking the side that owns the config keeps one answer.
 *
 * Shares the reconcile gate's key and signing scheme (`INTERNAL_RECONCILE_SECRET`, HMAC over
 * `{timestamp}.{raw body}`) because it is the same caller making the same kind of call at the same
 * trust level. Throws on transport error or non-2xx; every caller treats that as "no owner yet"
 * rather than as a reason to refuse a login.
 */
export async function resolvePrincipal(
  payload: ResolvePrincipalRequest,
): Promise<ResolvePrincipalResponse> {
  const url = requireEnv("INTERNAL_RESOLVE_URL");
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
    throw new Error(`principal resolve endpoint returned ${res.status}`);
  }
  const parsed = (await res.json()) as ResolvePrincipalResponse;
  logger.info({ profile_id: parsed.profile_id }, "principal resolve ok");
  return parsed;
}
