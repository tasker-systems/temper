import type { components } from "@tasker-systems/temper-ts";

/**
 * Runtime checks for a dispatch tick's response body (`POST /api/steward/dispatch`,
 * `POST /api/auditor/dispatch`).
 *
 * The types come from the generated OpenAPI client (`components["schemas"]["DispatchTickResponse"]`,
 * `components["schemas"]["AuditorDispatchTickResponse"]`), and these guards hold a parsed body to
 * that contract at the boundary: they check exactly the fields these schedules consume, so a
 * response that lost one fails at the parse site with a named error instead of surfacing later as
 * `undefined` inside a fan-out prompt or a session count.
 *
 * Extra fields are tolerated, matching serde's own default — the consumed fields, not the full key
 * set, are the contract.
 */

type StewardJob = components["schemas"]["ClaimedJob"];
type AuditorJob = components["schemas"]["ClaimedAuditJob"];
type AuditCitation = components["schemas"]["AuditCitation"];

function isClaimedJob(job: unknown): job is StewardJob {
  if (typeof job !== "object" || job === null) return false;
  const j = job as { id?: unknown; cogmap_id?: unknown };
  return typeof j.id === "string" && typeof j.cogmap_id === "string";
}

function isAuditCitation(citation: unknown): citation is AuditCitation {
  if (typeof citation !== "object" || citation === null) return false;
  const c = citation as { block_id?: unknown; finding_id?: unknown; source_id?: unknown };
  return (
    typeof c.block_id === "string" &&
    typeof c.finding_id === "string" &&
    typeof c.source_id === "string"
  );
}

function isClaimedAuditJob(job: unknown): job is AuditorJob {
  if (typeof job !== "object" || job === null) return false;
  const j = job as { id?: unknown; cogmap_id?: unknown; citations?: unknown };
  return (
    typeof j.id === "string" &&
    typeof j.cogmap_id === "string" &&
    Array.isArray(j.citations) &&
    j.citations.every(isAuditCitation)
  );
}

/**
 * The envelope both dispatch routes share: `claimed` plus the optional correlation echo.
 * `correlation_id` is `Option<Uuid>` server-side and `None` is skipped in serialization, so it is
 * absent or a string — anything else is a shape this package cannot trace.
 */
function isDispatchEnvelope<T>(
  value: unknown,
  isJob: (job: unknown) => job is T,
): value is { claimed: T[]; correlation_id?: string | null } {
  if (typeof value !== "object" || value === null) return false;
  const body = value as { claimed?: unknown; correlation_id?: unknown };
  if (!Array.isArray(body.claimed) || !body.claimed.every(isJob)) return false;
  return (
    body.correlation_id === undefined ||
    body.correlation_id === null ||
    typeof body.correlation_id === "string"
  );
}

export function isStewardDispatchResponse(
  value: unknown,
): value is components["schemas"]["DispatchTickResponse"] {
  return isDispatchEnvelope(value, isClaimedJob);
}

export function isAuditorDispatchResponse(
  value: unknown,
): value is components["schemas"]["AuditorDispatchTickResponse"] {
  return isDispatchEnvelope(value, isClaimedAuditJob);
}
