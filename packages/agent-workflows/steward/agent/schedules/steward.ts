import { getToken } from "@vercel/connect";
import { defineSchedule } from "eve/schedules";

import eve from "../channels/eve.js";

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
 * Auth mirrors materialize.ts / connections/temper.ts: Vercel Connect (app subject) when
 * `TEMPER_CONNECT_CONNECTOR` is set, else the OAuth-obtained `TEMPER_TOKEN`. The dispatch call hits
 * `TEMPER_API_URL` (the REST base, e.g. https://temperkb.io) — distinct from `TEMPER_MCP_URL`.
 */
export default defineSchedule({
  cron: "0 * * * *", // hourly, UTC; the server's threshold + single-flight gate what actually runs
  async run({ receive, waitUntil, appAuth }) {
    waitUntil(
      (async () => {
        const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
        const token = await temperToken();

        const res = await fetch(`${apiUrl}/api/steward/dispatch`, {
          method: "POST",
          headers: {
            authorization: `Bearer ${token}`,
            "content-type": "application/json",
          },
          // Empty body → server defaults (ingest threshold + dispatch cap).
          body: "{}",
        });
        if (!res.ok) {
          throw new Error(`dispatch failed: ${res.status} ${await res.text()}`);
        }

        const { claimed } = (await res.json()) as {
          claimed: { id: string; cogmap_id: string }[];
        };

        // Fan out: one independent, fresh-context agent session per claimed job, each carrying a
        // SINGLE cogmap id. Each session ends by advancing the watermark, completing its job.
        await Promise.all(
          claimed.map((job) =>
            receive(eve, {
              target: {},
              auth: appAuth,
              message:
                `Run one steward tick over cognitive map ${job.cogmap_id} (dispatch job ${job.id}). ` +
                `This map was already selected by the deterministic drift sweep, so its ingest delta ` +
                `has cleared threshold — you do not need to re-check it. Pass this SINGLE cogmap id ` +
                `as the \`cogmap\` argument to every temper tool. Load the map-stewardship skill, ` +
                `then: open the invocation envelope, read the telos, distill the new/changed sources ` +
                `with the authored-4 (create / assert / facet / fold), then advance the watermark to ` +
                `the latest observed event (this completes the dispatch job) and close the envelope.`,
            }),
          ),
        );
      })(),
    );
  },
});

async function temperToken(): Promise<string> {
  const connector = process.env.TEMPER_CONNECT_CONNECTOR;
  if (connector) {
    return getToken(connector, { subject: { type: "app" } });
  }
  return requireEnv("TEMPER_TOKEN");
}

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required — the steward's dispatch target/credential is never hardcoded`);
  }
  return value;
}
