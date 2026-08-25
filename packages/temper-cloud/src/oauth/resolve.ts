import { logger } from "../logger.js";
import { requireEnv } from "./env.js";
import { SIGNATURE_HEADER, signReconcile, TIMESTAMP_HEADER } from "./reconcile.js";

/** Ceiling on an internal server-to-server call, so a slow dependency fails open rather than hanging. */
const INTERNAL_CALL_TIMEOUT_MS = 5000;

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
 * Any RFC 4122 UUID. Checked before the value is allowed to leave this module, because everywhere
 * it goes afterwards treats it as one: a `::uuid` cast in `principal_may_refresh` (a malformed
 * value is a SQL error, not a false), and a foreign key on both `kb_oauth_refresh_tokens` and
 * `kb_oauth_flow` (where a well-formed but unknown id surfaces as a rejected SAML assertion,
 * blaming the IdP for something the resolve leg did).
 */
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

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
    // Fail-open is only a mercy if it fails FAST. Both callers catch and carry on, but an
    // unbounded fetch does not reach the catch — it holds the request until the platform's own
    // function timeout, turning a degraded dependency into a stalled login or rotation. Since the
    // migration, the first rotation of every pre-existing chain takes the resolve branch, so this
    // sits on a hot path rather than a rare one.
    signal: AbortSignal.timeout(INTERNAL_CALL_TIMEOUT_MS),
  });
  if (!res.ok) {
    throw new Error(`principal resolve endpoint returned ${res.status}`);
  }

  // Validate, do not assert. `as ResolvePrincipalResponse` is a compile-time claim about a value
  // that arrives at runtime from another process, and the two failure postures either side of this
  // line are opposite: a transport error or non-2xx THROWS, and both callers treat that as "no
  // owner yet" and carry on. A malformed 200 asserted into the type does not throw — it yields
  // `undefined`, which survives the callers' `!== null` guard, reaches
  // `principal_may_refresh(NULL)` (which answers FALSE, not null), and refuses a rotation whose
  // token `rotateRefreshToken` has already revoked. That is an irrecoverable logout produced by a
  // misconfiguration that logs success. Throwing here collapses both failures onto the fail-open
  // path the design actually intends.
  const parsed: unknown = await res.json();
  const profileId = (parsed as { profile_id?: unknown } | null)?.profile_id;
  if (typeof profileId !== "string" || !UUID_RE.test(profileId)) {
    throw new Error("principal resolve endpoint returned no usable profile_id");
  }
  logger.info({ profile_id: profileId }, "principal resolve ok");
  return { profile_id: profileId };
}
