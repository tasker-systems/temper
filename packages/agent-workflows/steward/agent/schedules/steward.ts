import { defineSchedule } from "eve/schedules";

/**
 * Cron BACKSTOP only. The real threshold-driven dispatch (the steward_ingest_delta
 * gate; the eve-vs-temper scheduler switch) is T6. The loop itself no-ops when the
 * ingest delta is under threshold, so a fixed cadence is safe.
 *
 * Markdown task-mode runs to completion and cannot park for OAuth — which is fine:
 * the temper connection auth is non-interactive (app-scoped Connect / getToken).
 *
 * The target cogmap is env-pinned (`TEMPER_SELF_COGMAP_ID`) and baked into the
 * prompt so the model has the exact id to pass as the `cogmap` argument to every
 * temper tool — the MVP 1:1 (one steward, one map) binding. Required at build
 * time, like the connection's `TEMPER_MCP_URL`.
 */
const COGMAP_ID = requireEnv("TEMPER_SELF_COGMAP_ID");

export default defineSchedule({
  cron: "0 * * * *", // hourly, UTC; the loop self-gates on the ingest threshold
  markdown:
    `Run one steward tick over the team self-cognition map — cognitive map id ` +
    `${COGMAP_ID}. Pass this id as the \`cogmap\` argument to every temper tool ` +
    `(steward_ingest_delta, invocation_open, cogmap_read_charter, search, ` +
    `create_resource, assert_relationship, facet_set, fold_relationship, ` +
    `steward_advance_watermark). Load the map-stewardship skill, then: check the ` +
    `ingest delta, and if it clears the threshold, tend the map per that skill — ` +
    `open the invocation envelope, read the telos, distill new/changed sources ` +
    `with the authored-4 (create / assert / facet / fold), then advance the ` +
    `watermark and close. If the delta is under threshold, open and close the ` +
    `envelope with a no-op outcome.`,
});

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required — the steward's target cogmap is never hardcoded`);
  }
  return value;
}
