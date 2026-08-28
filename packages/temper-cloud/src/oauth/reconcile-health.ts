import type { NeonClient } from "../db.js";
import { logger } from "../logger.js";

/**
 * Whether temper-cloud's fail-open internal calls are REACHING temper-api, recorded where an
 * operator can be told about it.
 *
 * The reconcile call is fail-open by design (spec §3.8): a provisioning failure must never block a
 * login. What that costs is that a failure leaves nothing behind but a `logger.error` on a surface
 * with no telemetry pipeline -- `logger.ts` is five lines of pino to stdout -- so de-provisioning
 * can stop for everyone with nothing anywhere saying so. This module is the positive record that
 * makes it sayable, and `kb_internal_call_health` (20260828000010) carries the reasoning for the
 * table's shape.
 *
 * ## Nothing in here may throw
 *
 * That is the load-bearing property, not a nicety. Both recording calls sit on the authentication
 * path: the success write inside the reconcile's `try`, the failure write inside its `catch`. A
 * throw from the first would be caught as a reconcile failure the reconcile did not have; a throw
 * from the second would escape the inner catch entirely and reach the ACS's outer handler, which
 * answers `400 SAML assertion rejected` -- turning an observability write into a login failure, the
 * exact regression the fail-open decision forbids. So every function here swallows its own errors
 * and says so at `warn`. A channel that cannot record is a channel with no signal, which is where
 * we already were; a channel that can fail a login is worse than no channel.
 *
 * ## The cause is load-bearing, not descriptive
 *
 * Two of the four causes are deployment facts -- an unset variable, a secret the API disagrees with
 * -- and one occurrence of either settles the question. The other two can be weather and must
 * recur. The reader (`internal_call_health_service` in temper-api) branches on exactly this, which
 * is why classification happens at the point the error is raised rather than by string-matching a
 * message downstream, and why the database enforces the vocabulary rather than trusting this file.
 */

/** The channel name for the SAML membership reconcile call. */
export const RECONCILE_CHANNEL = "saml_reconcile";

/**
 * How an internal call failed. Closed, and pinned to the CHECK on
 * `kb_internal_call_health.last_failure_cause` -- a value not in this union fails the write.
 */
export type InternalCallFailureCause =
  | "config_missing"
  | "unauthorized"
  | "transport"
  | "endpoint_error";

/**
 * An internal call that did not complete, carrying WHY in a form the reader can branch on.
 *
 * Classified where it is raised. The alternative -- one plain `Error` per site and a regex over
 * `message` in the recorder -- was rejected because the classification decides an alert threshold:
 * a message reworded during unrelated maintenance would silently reclassify a conclusive failure as
 * one awaiting recurrence, and nothing would fail.
 */
export class InternalCallError extends Error {
  readonly failureCause: InternalCallFailureCause;
  /** Bounded operator detail -- a variable name or an HTTP status. Never a raw error string. */
  readonly failureDetail: string;

  constructor(cause: InternalCallFailureCause, detail: string, message: string) {
    super(message);
    this.name = "InternalCallError";
    this.failureCause = cause;
    this.failureDetail = detail;
  }
}

/**
 * Classify an error caught around an internal call.
 *
 * An [`InternalCallError`] carries its own answer. Anything else is UNEXPECTED, and the fallback
 * deliberately picks the weather-capable `endpoint_error` rather than a conclusive cause: an
 * unclassified error is unbounded in kind, so it must earn its alert by recurring instead of
 * firing one immediately on a class nobody has examined.
 *
 * The detail is never taken from the error's message. node-saml errors can embed assertion XML --
 * NameID, email -- in theirs, which is why the ACS's own outer catch logs `err.message` and never
 * the error, and why this goes further and does not store even that.
 */
export function classifyInternalCallFailure(err: unknown): {
  cause: InternalCallFailureCause;
  detail: string;
} {
  if (err instanceof InternalCallError) {
    return { cause: err.failureCause, detail: err.failureDetail };
  }
  return { cause: "endpoint_error", detail: "unclassified" };
}

/**
 * Record that a call on this channel completed.
 *
 * Ends the current run of failures -- `failing_since` and `consecutive_failures` describe a live
 * condition -- and deliberately leaves `last_failure_at`, `last_failure_cause` and
 * `last_failure_detail`, which describe the last failure that happened. Clearing those too would
 * destroy the forensic record on the first success after an outage, which is the moment an operator
 * most wants to read it.
 *
 * **It also does not touch `failures_total`, and that is the point of that column.** A channel
 * failing intermittently -- a flaky egress losing half the logins -- is reset by every success it
 * does manage, so the run counter never accumulates and the pair reports a healthy channel while
 * half of all de-provisioning is not happening. The monotonic total is the only thing that keeps
 * rising through that.
 */
export async function recordChannelSuccess(db: NeonClient, channel: string): Promise<void> {
  try {
    await db`INSERT INTO kb_internal_call_health (channel, last_success_at, consecutive_failures)
      VALUES (${channel}, now(), 0)
      ON CONFLICT (channel) DO UPDATE
         SET last_success_at      = now(),
             consecutive_failures = 0,
             failing_since        = NULL`;
  } catch (recordErr) {
    logger.warn(
      { channel, err: recordErr instanceof Error ? recordErr.message : String(recordErr) },
      "internal call health: could not record a success (the call itself succeeded)",
    );
  }
}

/**
 * Record that a call on this channel did not complete.
 *
 * `failing_since` is COALESCEd rather than assigned: it names when the CURRENT run began, so the
 * second failure of a run must not move it. That is what makes "has this outlived the window"
 * answerable at all -- assigning `now()` on every failure would make every run look one call old,
 * and a channel down for a week would be indistinguishable from one that failed a moment ago.
 */
export async function recordChannelFailure(
  db: NeonClient,
  channel: string,
  err: unknown,
): Promise<void> {
  const { cause, detail } = classifyInternalCallFailure(err);
  try {
    await db`INSERT INTO kb_internal_call_health
        (channel, last_failure_at, failing_since, consecutive_failures, failures_total,
         last_failure_cause, last_failure_detail)
      VALUES (${channel}, now(), now(), 1, 1, ${cause}, ${detail})
      ON CONFLICT (channel) DO UPDATE
         SET last_failure_at      = now(),
             failing_since        = COALESCE(kb_internal_call_health.failing_since, now()),
             consecutive_failures = kb_internal_call_health.consecutive_failures + 1,
             failures_total       = kb_internal_call_health.failures_total + 1,
             last_failure_cause   = EXCLUDED.last_failure_cause,
             last_failure_detail  = EXCLUDED.last_failure_detail`;
  } catch (recordErr) {
    // The one place the whole mechanism can go silent, so it says so loudly rather than returning
    // as if it had recorded. It still must not rethrow -- see the module doc.
    logger.warn(
      {
        channel,
        cause,
        detail,
        err: recordErr instanceof Error ? recordErr.message : String(recordErr),
      },
      "internal call health: could not record a failure -- this channel's signal is missing",
    );
  }
}
