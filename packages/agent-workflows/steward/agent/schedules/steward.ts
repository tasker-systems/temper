import { defineSchedule } from "eve/schedules";

import worker from "../channels/worker.js";
import { fetchWithRetry } from "../lib/fetch-retry.js";
import { requireEnv, temperToken } from "../lib/temper-auth.js";

/**
 * Steward fan-out dispatcher (goal 019f3220). This is DELIBERATELY a code `run` handler, not a
 * model-driven `markdown` prompt: the drifted-map selection is deterministic substrate work
 * (`POST /api/steward/dispatch` runs reap→sweep→enqueue→claim server-side), and only the per-map
 * authored-4 distillation is model work. So this handler does the deterministic dispatch, then starts
 * ONE isolated agent session per claimed job — each tending a SINGLE cogmap. The fan-out is over the
 * workflow, never over an agent's target (one session, one map id).
 *
 * Single-flight + reaping live in the server (kb_workflow_jobs): a still-running map is not
 * re-claimed, and a crashed run's lease expires and is requeued — so a fixed hourly cadence is safe.
 * The env pin `TEMPER_SELF_COGMAP_ID` is GONE; map identity flows from the sweep.
 *
 * Auth is machine-identity-first via the shared `temperToken` (`../lib/temper-auth`), the SAME
 * ordering the MCP connection uses — M2M `client_credentials` mint on prod, then Connect, then
 * `TEMPER_TOKEN`. (An earlier Connect-first copy here hit the dead-end connector on prod and threw
 * before any request went out — the silent failure this fix resolves.) The dispatch call hits
 * `TEMPER_API_URL` (the REST base) — distinct from `TEMPER_MCP_URL`.
 *
 * Logging: the tick logs its outcome (claimed count) and any failure, so a no-op or a broken tick is
 * visible in the steward-agent logs instead of vanishing inside `waitUntil`. Each tick mints a
 * `correlationId` and threads it across the app boundary — logged here, sent as `x-steward-correlation-id`
 * to `/dispatch` (temper-api logs it on entry), so the cron → dispatch chain shares one key in the logs
 * of BOTH Vercel apps even when a hop fails before any DB row exists (the load-bearing, infra-resilient
 * trace). We also log the `/dispatch` response's `x-vercel-id` as a bridge into Vercel's per-request view.
 * Design: docs/superpowers/specs/2026-07-06-steward-dispatch-correlation-id-design.md
 */
export default defineSchedule({
  cron: "0 * * * *", // hourly, UTC; the server's threshold + single-flight gate what actually runs
  async run({ receive, waitUntil, appAuth }) {
    waitUntil(
      (async () => {
        // One id per tick, threaded cron → /dispatch → the agent session so the whole chain shares a
        // single join key across the two Vercel apps. `crypto.randomUUID` (v4) is sufficient — a
        // correlation key needs uniqueness, not v7 sortability; log timestamps order the trace. Logged
        // BEFORE the outbound fetch so a hop that dies (cold-start 500, fetch-never-lands, a receive()
        // that throws) is still pinned to this id via the catch below.
        const correlationId = crypto.randomUUID();
        console.log(`[steward-dispatch] tick ${correlationId} starting`);
        try {
          const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
          const token = await temperToken();

          // Retry on 5xx: this hourly call always hits a cold serverless function, which can 500 on
          // a Neon pool-acquire timeout at startup; a retry warms it and succeeds.
          const res = await fetchWithRetry(
            `${apiUrl}/api/steward/dispatch`,
            {
              method: "POST",
              headers: {
                authorization: `Bearer ${token}`,
                "content-type": "application/json",
                "x-steward-correlation-id": correlationId,
              },
              // Empty body → server defaults (ingest threshold + dispatch cap).
              body: "{}",
            },
            { label: "dispatch" },
          );
          if (!res.ok) {
            throw new Error(`dispatch failed: ${res.status} ${await res.text()}`);
          }

          // Bridge to Vercel's own request id for the infra-side view of this hop (design item 3).
          const dispatchVercelId = res.headers.get("x-vercel-id") ?? "unknown";

          const { claimed } = (await res.json()) as {
            claimed: { id: string; cogmap_id: string }[];
          };

          console.log(
            `[steward-dispatch] tick ${correlationId}: claimed ${claimed.length} job(s)` +
              (claimed.length
                ? `: ${claimed.map((j) => `${j.id}→${j.cogmap_id}`).join(", ")}`
                : " (no drift)") +
              ` (dispatch vercel-id ${dispatchVercelId})`,
          );

          // Fan out: one independent, fresh-context agent session per claimed job, each carrying a
          // SINGLE cogmap id. Each session ends by advancing the watermark, completing its job.
          await Promise.all(
            claimed.map((job) =>
              receive(worker, {
                target: {},
                auth: appAuth,
                message:
                  `Run one steward tick over cognitive map ${job.cogmap_id} (dispatch job ${job.id}; ` +
                  `correlation ${correlationId}). ` +
                  `This map was already selected by the deterministic drift sweep, so its ingest delta ` +
                  `has cleared threshold — you do not need to re-check it. Pass this SINGLE cogmap id ` +
                  `as the \`cogmap\` argument to every temper tool. Load the map-stewardship skill, ` +
                  `then: open the invocation envelope, read the telos, distill the new/changed sources ` +
                  `with the authored-4 (create / assert / facet / fold), then advance the watermark to ` +
                  `the latest observed event (this completes the dispatch job) and close the envelope.`,
              }),
            ),
          );
        } catch (err) {
          console.error(`[steward-dispatch] tick ${correlationId} failed:`, err);
          throw err;
        }
      })(),
    );
  },
});
