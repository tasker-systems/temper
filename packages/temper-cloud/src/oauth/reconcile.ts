import { createHmac, randomBytes } from "node:crypto";
import { logger } from "../logger.js";
import { requireEnv } from "./env.js";
import { InternalCallError, SubjectRefused } from "./reconcile-health.js";

/** Ceiling on an internal server-to-server call, so a slow dependency fails open rather than hanging. */
const INTERNAL_CALL_TIMEOUT_MS = 5000;

/** Header carrying the Unix-seconds timestamp the signature was computed over. */
export const TIMESTAMP_HEADER = "X-Temper-Timestamp";
/** Header carrying the v2 construction's per-request nonce. */
export const NONCE_HEADER = "X-Temper-Nonce";
/** Header carrying the v2 construction's lowercase-hex signature. */
export const SIGNATURE_V2_HEADER = "X-Temper-Signature-V2";
/** The reconcile route as temper-api registers it (`internal_sig::DOOR_SAML_RECONCILE`). */
export const RECONCILE_DOOR = "/internal/saml/reconcile";
/** The resolve route as temper-api registers it (`internal_sig::DOOR_PRINCIPAL_RESOLVE`). */
export const RESOLVE_DOOR = "/internal/principal/resolve";

/**
 * The v2 construction: `"v2\n{METHOD}\n{door}\n{timestamp}\n{nonce}\n{body}"`, lowercase hex.
 * Mirrors `temper_core::internal_sig::sign_v2`; pinned by the v2 known-answer vector in
 * tests/oauth/wire-contract.test.ts. We sign the RAW body string exactly as it is sent (never a
 * re-serialized form) so there is no cross-language canonicalization to drift on. `door` is the
 * route temper-api registers, never read back from the URL we post to, so a prefix or rewrite in
 * front of the API does not break it. The URL's path must still end with the door
 * (`requireDoorUrl`), because a path the API does not serve is refused exactly as a bad signature
 * is, and only this side can catch a URL that names the wrong door or adds a segment after it. Binding the door and a
 * nonce is what lets the verifier accept a signature at one door, once.
 */
export function signRequestV2(
  secret: string,
  method: string,
  door: string,
  timestamp: number,
  nonce: string,
  body: string,
): string {
  return createHmac("sha256", secret)
    .update(`v2\n${method}\n${door}\n${timestamp}\n${nonce}\n${body}`)
    .digest("hex");
}

/**
 * The v2 headers for one signed POST: timestamp, a fresh nonce, and the signature. These three
 * are the whole credential; temper-api reads nothing else.
 */
export function signedHeaders(
  secret: string,
  door: string,
  timestamp: number,
  body: string,
): Record<string, string> {
  const nonce = randomBytes(16).toString("hex");
  return {
    [TIMESTAMP_HEADER]: String(timestamp),
    [NONCE_HEADER]: nonce,
    [SIGNATURE_V2_HEADER]: signRequestV2(secret, "POST", door, timestamp, nonce, body),
  };
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
 * [`requireEnv`] with the failure classified.
 *
 * An unset `INTERNAL_RECONCILE_URL`, `INTERNAL_RESOLVE_URL` or `INTERNAL_RECONCILE_SECRET` is a
 * deployment fact, not a transient one: it fails for every login, forever, until someone sets it.
 * Naming the variable as the detail is what makes the operator action obvious from the signal
 * alone, which is the whole difference between "the reconcile channel is down" and "set this
 * variable".
 */
export function requireReconcileEnv(name: string): string {
  try {
    return requireEnv(name);
  } catch {
    throw new InternalCallError(
      "config_missing",
      name,
      `Missing required environment variable: ${name}`,
    );
  }
}

/**
 * The URL in `name`, checked to post to `door` before anything is signed.
 *
 * temper-api refuses an unauthenticated caller the same way whatever went wrong, so a URL with a
 * stray path segment, a trailing slash or the wrong door answers 401 exactly as a wrong secret
 * does, and the reader would page about the secret. The path is the half of the URL this side can
 * check against the door it signs for, so it is checked here, where the cause is known exactly. A
 * value that does not parse as a URL, names a scheme other than http(s), or carries userinfo is
 * the same kind of fact and gets the same cause: each would otherwise fail inside `fetch` and be
 * recorded as `transport`, which is weather and must recur before it pages. The detail is a fixed
 * string, never the URL itself.
 *
 * The path must END with the door, not equal it: a deployment may reach the API through a prefix
 * a proxy strips (`temper_core::internal_sig`), and that must keep working. Every door begins with
 * `/`, so the match is at a segment boundary. What it refuses is anything after the door (a stray
 * segment, a trailing slash), the other door, and no door at all. A stray segment BEFORE the door
 * passes, because this side cannot tell it from a prefix; the API's 401 stays the signal for that.
 */
export function requireDoorUrl(name: string, door: string): string {
  const url = requireReconcileEnv(name);
  let parsed: URL | null = null;
  try {
    parsed = new URL(url);
  } catch {
    parsed = null;
  }
  if (
    parsed === null ||
    (parsed.protocol !== "https:" && parsed.protocol !== "http:") ||
    parsed.username !== "" ||
    parsed.password !== "" ||
    !parsed.pathname.endsWith(door)
  ) {
    throw new InternalCallError(
      "config_missing",
      `${name} path`,
      `${name} does not post to ${door}`,
    );
  }
  return url;
}

/**
 * A subject temper-api refuses as machine-shaped (`reject_machine_shaped`, the `@clients` suffix),
 * refused here instead, before anything is sent.
 *
 * The API answers that refusal 401 behind a valid signature, which on the wire is the same answer
 * as a wrong secret. Recorded, it would page at once with deployment advice, and any IdP user whose
 * NameID wears the suffix could raise and hold that page by logging in. It is a fact about one
 * principal, not about the channel, so it throws `SubjectRefused`, which the recorder skips.
 */
export function refuseMachineShapedSubject(externalUserId: string): void {
  if (externalUserId.endsWith("@clients")) {
    throw new SubjectRefused("subject is machine-shaped; temper-api refuses it as a human");
  }
}

/**
 * A bounded detail for a transport failure — the error's constructor name and nothing else.
 *
 * Guarded rather than trusted: the recorded value reaches an operator's panel, and a name is only
 * useful if it is a name. Anything that is not a plain identifier is reported as `unknown` instead
 * of being stored, which keeps the column's contents an enumerable set.
 */
export function transportDetail(err: unknown): string {
  const name = err instanceof Error ? err.name : "";
  return /^[A-Za-z][A-Za-z0-9]{0,39}$/.test(name) ? name : "unknown";
}

/**
 * Calls the internal temper-api reconcile endpoint (server-to-server), signing the request with
 * `signedHeaders` so the secret never crosses the wire and the API accepts the request at this door,
 * once. Throws on transport error or non-2xx — the ACS handler catches and proceeds
 * (fail-open), so a provisioning hiccup never blocks login. The verifier is temper-api's
 * `require_internal_signature` middleware.
 */
export async function reconcileMemberships(payload: ReconcileRequest): Promise<void> {
  refuseMachineShapedSubject(payload.external_user_id);
  const url = requireDoorUrl("INTERNAL_RECONCILE_URL", RECONCILE_DOOR);
  const secret = requireReconcileEnv("INTERNAL_RECONCILE_SECRET");
  // Sign the exact body string we send — raw-body discipline, no re-serialization.
  const body = JSON.stringify(payload);
  const timestamp = Math.floor(Date.now() / 1000);
  // Built before the `try`, so a failure to construct the request is never recorded as transport.
  const headers = {
    "content-type": "application/json",
    ...signedHeaders(secret, RECONCILE_DOOR, timestamp, body),
  };
  let res: Response;
  try {
    res = await fetch(url, {
      method: "POST",
      headers,
      body,
      // Fail-open is only a mercy if it fails FAST. Both callers catch and carry on, but an
      // unbounded fetch does not reach the catch — it holds the request until the platform's own
      // function timeout, turning a degraded dependency into a stalled login or rotation. Since the
      // migration, the first rotation of every pre-existing chain takes the resolve branch, so this
      // sits on a hot path rather than a rare one.
      signal: AbortSignal.timeout(INTERNAL_CALL_TIMEOUT_MS),
    });
  } catch (fetchErr) {
    // The request never got an answer: DNS, connection refused, TLS, or the abort above firing.
    // Classified here rather than inferred later — see reconcile-health.ts on why the cause is
    // load-bearing. The detail is the error's NAME (TypeError, TimeoutError), never its message.
    throw new InternalCallError(
      "transport",
      transportDetail(fetchErr),
      "reconcile call did not reach the endpoint",
    );
  }
  if (!res.ok) {
    // 401/403 means the signature was refused or the door is not served. temper-api answers an
    // unauthenticated caller the same way whatever the reason, so this covers a wrong
    // INTERNAL_RECONCILE_SECRET, clock skew past the verifier's window, and a deployment of the API
    // that does not serve this door. Each is a deployment fact, not weather, and it is separated
    // from every other status precisely so one occurrence can be treated as conclusive. A wrong
    // path in our own URL never gets this far: `requireDoorUrl` refuses it before signing.
    throw new InternalCallError(
      res.status === 401 || res.status === 403 ? "unauthorized" : "endpoint_error",
      `HTTP ${res.status}`,
      `reconcile endpoint returned ${res.status}`,
    );
  }
  // `null` stays null in the log line rather than becoming 0: a login that carried no group signal
  // and one that asserted an empty group set are the two cases this whole path exists to keep
  // apart, and `groups: 0` would have read as the second.
  // Beyond that shape the line binds no identifier: the IdP key and the principal identity travel
  // the signed wire call this line confirms, never the log surface.
  logger.info({ groups: payload.groups?.length ?? null }, "saml reconcile ok");
}
