import { defineSchedule } from "eve/schedules";

/**
 * Cron BACKSTOP only. The real threshold-driven dispatch (the steward_ingest_delta
 * gate; the eve-vs-temper scheduler switch) is T6. The loop itself no-ops when the
 * ingest delta is under threshold, so a fixed cadence is safe.
 *
 * Markdown task-mode runs to completion and cannot park for OAuth — which is fine:
 * the temper connection auth is non-interactive (app-scoped Connect / getToken).
 */
export default defineSchedule({
  cron: "0 * * * *", // hourly, UTC; the loop self-gates on the ingest threshold
  markdown:
    "Run one steward tick over the team self-cognition map. Load the " +
    "map-stewardship skill, then: check the ingest delta, and if it clears the " +
    "threshold, tend the map per that skill — open the invocation envelope, read " +
    "the telos, distill new/changed sources with the authored-4 (create / assert / " +
    "facet / fold), then advance the watermark and close. If the delta is under " +
    "threshold, open and close the envelope with a no-op outcome.",
});
