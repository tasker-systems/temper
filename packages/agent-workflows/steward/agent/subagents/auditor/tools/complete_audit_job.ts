import { defineTool } from "eve/tools";
import { z } from "zod";

import { auditorFetch, requireEnv } from "../../../lib/temper-auth.js";

/**
 * Close out this run's dispatch job — the auditor's twin of the steward's watermark advance.
 *
 * The steward's job completion rides `steward_advance_watermark` (a clean advance IS run
 * completion). The auditor advances no watermark, so without this call its job sits `in_progress`
 * until `workflow_job_reap` expires the lease and re-queues it — re-dispatching the SAME cogmap up
 * to `max_attempts` times (`migrations/20260705000001_workflow_jobs.sql:110-128`) and appending a
 * duplicate verdict on every re-run, because the audit trail is append-only and nothing dedups it.
 *
 * REST rather than MCP for the same reason as `element_trail`: `POST /api/auditor/{cogmap}/complete`
 * is part of the auditor's own dispatch surface and has no MCP tool. Gated on cogmap READability,
 * not writability — the auditor is deliberately provisioned without cogmap write.
 *
 * Idempotent: completing when no job is active returns `job_id: null` and is not an error.
 */
export default defineTool({
  description:
    "Mark this run's citation-audit dispatch job complete. Call it exactly once, LAST, after every finding in your list has been worked and before you close the invocation. Skipping it makes the server re-dispatch this same cogmap and record duplicate verdicts.",
  inputSchema: z.object({
    cogmap_id: z.uuid().describe("The cognitive map this run audited within"),
  }),
  async execute({ cogmap_id }) {
    const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
    const res = await auditorFetch(
      `${apiUrl}/api/auditor/${cogmap_id}/complete`,
      { method: "POST", headers: { accept: "application/json" } },
      { label: `auditor-complete ${cogmap_id}` },
    );
    if (!res.ok) {
      return { ok: false as const, status: res.status, error: await res.text() };
    }
    return { ok: true as const, ack: await res.json() };
  },
});
