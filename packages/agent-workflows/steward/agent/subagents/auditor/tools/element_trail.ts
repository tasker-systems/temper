import { defineTool } from "eve/tools";
import { z } from "zod";

import { auditorFetch, requireEnv } from "../../../lib/temper-auth.js";

/**
 * The ledger read for ONE graph element — the auditor's only route to a citing act's own record.
 *
 * **Why an authored tool and not an MCP tool.** `GET /api/graph/elements/{kind}/{id}/trail` is
 * REST-only: temper-mcp exposes `get_block_provenance` and `record_citation_audit` but no element
 * trail (spec §8, which names this gap and says the auditor "reaches it via `temperFetch` … unless a
 * tool is added"). Adding the MCP tool is a cross-surface change to temper-mcp; this is the same
 * `temperFetch` reach the spec describes, packaged so the model can actually make the call. It runs
 * in the app runtime under the auditor's own credential, so the server applies the same access gate
 * to it as to every MCP read (`resources_visible_to` for nodes; anchor **and** both endpoints for
 * edges), and the surface is firewalled to `category = 'domain'`.
 *
 * **It is a DISCRETE per-element call, and must stay one.** Spec §8: "the auditor fetches trails only
 * for the acts it decides to weigh, never as part of a first query pass." The input takes exactly one
 * id for that reason — there is no batch form to reach for.
 *
 * **KNOWN THIN INPUT, accepted for the first cut.** The SQL behind this returns the citing event's
 * whole `metadata jsonb`, but the DTO lifts only `confidence`
 * (`crates/temper-core/src/types/element_trail.rs:37-39`): `rationale`, `persona`, and `model` — the
 * situational-distance material §3.3 asks the auditor to weigh — are present in the column and
 * dropped by the projection. So a trail entry here carries the act's confidence band and its
 * replay-sufficient `payload`, but not the author's own stated reasoning. The auditor works from what
 * is here and says so in its verdict's `reason`; widening the DTO is a deliberate follow-up, recorded
 * as a decision rather than discovered as a surprise.
 */
export default defineTool({
  description:
    "Read the time-ordered event trail for ONE graph element — a resource/node, or an edge. Use it to see the act that authored a citation: who emitted it, when, its confidence band, and its payload. Call it only for the specific acts you have decided to weigh, one at a time; never sweep it across a finding's whole citation set. Note that the author's free-text rationale is NOT carried by this projection — only the confidence band and the payload are.",
  inputSchema: z.object({
    kind: z
      .enum(["node", "edge"])
      .describe("'node' for a resource id (a finding or a cited source), 'edge' for an edge id"),
    id: z.uuid().describe("The resource id (kind='node') or edge id (kind='edge')"),
  }),
  async execute({ kind, id }) {
    const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
    const res = await auditorFetch(
      `${apiUrl}/api/graph/elements/${kind}/${id}/trail`,
      { headers: { accept: "application/json" } },
      { label: `element-trail ${kind}/${id}` },
    );
    if (!res.ok) {
      // Surfaced to the model rather than thrown: an unreadable element is an ordinary outcome for a
      // read-scoped principal, and the auditor's correct response is to weigh the citation without
      // this input (or to skip it), not to abort the whole session's remaining findings.
      return {
        ok: false as const,
        status: res.status,
        error: await res.text(),
      };
    }
    return { ok: true as const, trail: await res.json() };
  },
});
