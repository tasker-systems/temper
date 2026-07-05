import { defineSchedule } from "eve/schedules";

import { fetchWithRetry } from "../lib/fetch-retry.js";
import { requireEnv, temperToken } from "../lib/temper-auth.js";

/**
 * Region materialization on its OWN threshold cadence (T6 acceptance #3).
 *
 * This is DELIBERATELY a code `run` handler doing a direct POST to the T4b
 * materialize route — NOT a model-driven `markdown` prompt. Region formation is
 * deterministic substrate work, not one of the steward's authored acts, so the
 * `cogmap_materialize*` tools stay OUT of the steward's MCP allow-list
 * (connections/temper.ts) and this trigger never routes through the model.
 *
 * The server self-gates: `POST /materialize` runs the clustering only when the
 * formation-event delta clears the materialize threshold, and is an idempotent
 * no-op (`MaterializeAck { materialized: false }`) below it. So a fixed cadence
 * is safe — the threshold, not the cron, decides when work actually happens.
 *
 * Auth is machine-identity-first via the shared `temperToken` (`../lib/temper-auth`), the SAME
 * ordering the MCP connection uses — M2M `client_credentials` on prod, then Connect, then
 * `TEMPER_TOKEN`. (An earlier Connect-first copy here hit the dead-end connector on prod and threw
 * before any request went out, so materialize silently never ran — this fix resolves that.)
 *
 * Fan-out (goal 019f3220): materialization now runs across ALL team-joined cogmaps, not one
 * env-pinned map. It enumerates candidates via `GET /api/steward/candidates` (the readable
 * team-joined maps) and POSTs a self-gating materialize per map. NO lease/queue is needed —
 * re-materialization is deterministic and idempotent (worst case: a little wasted compute), and the
 * server no-ops below threshold. The env pin `TEMPER_SELF_COGMAP_ID` is GONE.
 *
 * Targets are env-driven, never hardcoded:
 * - `TEMPER_API_URL` — the temper REST base (e.g. https://temperkb.io). Distinct
 *   from `TEMPER_MCP_URL`, which is the MCP endpoint the connection speaks to.
 */
export default defineSchedule({
  cron: "0 * * * *", // hourly, UTC; the server gates on the materialize threshold
  async run({ waitUntil }) {
    // waitUntil keeps the cron task alive until the POSTs settle (per eve docs).
    waitUntil(materializeTick());
  },
});

async function materializeTick(): Promise<void> {
  const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
  const token = await temperToken();

  const list = await fetchWithRetry(
    `${apiUrl}/api/steward/candidates`,
    { headers: { authorization: `Bearer ${token}` } },
    { label: "candidates" },
  );
  if (!list.ok) {
    throw new Error(`candidates fetch failed: ${list.status} ${await list.text()}`);
  }
  const ids = (await list.json()) as string[];
  console.log(`[steward-materialize] materializing ${ids.length} candidate cogmap(s)`);

  await Promise.all(
    ids.map(async (id) => {
      const res = await fetchWithRetry(
        `${apiUrl}/api/cognitive-maps/${id}/materialize`,
        {
          method: "POST",
          headers: {
            authorization: `Bearer ${token}`,
            "content-type": "application/json",
          },
          // Empty body → the server applies its DEFAULT_MATERIALIZE_THRESHOLD (self-gating no-op below).
          body: "{}",
        },
        { label: `materialize ${id}` },
      );
      if (!res.ok) {
        throw new Error(`materialize ${id} POST failed: ${res.status} ${await res.text()}`);
      }
    }),
  );
}
