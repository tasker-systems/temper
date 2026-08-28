import { createHmac } from "node:crypto";
import { logger } from "../logger.js";
import { requireEnv } from "./env.js";
import { InternalCallError } from "./reconcile-health.js";

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
 * [`requireEnv`] with the failure classified.
 *
 * An unset `INTERNAL_RECONCILE_URL` or `INTERNAL_RECONCILE_SECRET` is a deployment fact, not a
 * transient one: it fails for every login, forever, until someone sets it. Naming the variable as
 * the detail is what makes the operator action obvious from the signal alone, which is the whole
 * difference between "the reconcile channel is down" and "set this variable".
 */
function requireReconcileEnv(name: string): string {
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
 * A bounded detail for a transport failure — the error's constructor name and nothing else.
 *
 * Guarded rather than trusted: the recorded value reaches an operator's panel, and a name is only
 * useful if it is a name. Anything that is not a plain identifier is reported as `unknown` instead
 * of being stored, which keeps the column's contents an enumerable set.
 */
function transportDetail(err: unknown): string {
  const name = err instanceof Error ? err.name : "";
  return /^[A-Za-z][A-Za-z0-9]{0,39}$/.test(name) ? name : "unknown";
}

/**
 * Calls the internal temper-api reconcile endpoint (server-to-server), signing the request with
 * `HMAC(secret, "{timestamp}.{body}")` so the secret never crosses the wire and a captured request
 * is replay-proof. Throws on transport error or non-2xx — the ACS handler catches and proceeds
 * (fail-open), so a provisioning hiccup never blocks login. The verifier is temper-api's
 * `require_internal_signature` middleware.
 */
export async function reconcileMemberships(payload: ReconcileRequest): Promise<void> {
  const url = requireReconcileEnv("INTERNAL_RECONCILE_URL");
  const secret = requireReconcileEnv("INTERNAL_RECONCILE_SECRET");
  // Sign the exact body string we send — raw-body discipline, no re-serialization.
  const body = JSON.stringify(payload);
  const timestamp = Math.floor(Date.now() / 1000);
  const signature = signReconcile(secret, timestamp, body);
  let res: Response;
  try {
    res = await fetch(url, {
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
    // 401/403 is the API refusing the signature — a wrong INTERNAL_RECONCILE_SECRET, or clock skew
    // past the verifier's window. That is a deployment fact, not weather, and it is separated from
    // every other status precisely so one occurrence can be treated as conclusive.
    throw new InternalCallError(
      res.status === 401 || res.status === 403 ? "unauthorized" : "endpoint_error",
      `HTTP ${res.status}`,
      `reconcile endpoint returned ${res.status}`,
    );
  }
  // `null` stays null in the log line rather than becoming 0: a login that carried no group signal
  // and one that asserted an empty group set are the two cases this whole path exists to keep
  // apart, and `groups: 0` would have read as the second.
  logger.info(
    { idp_key: payload.idp_key, groups: payload.groups?.length ?? null },
    "saml reconcile ok",
  );
}
