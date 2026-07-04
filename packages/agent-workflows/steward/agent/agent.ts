import { defineAgent } from "eve";

export default defineAgent({
  // Distillation + supersession are judgment-heavy, and sonnet/opus 4.x are the
  // quality bar — but their per-tick cost isn't sustainable at the dev/community
  // tier (the loop runs hourly; see schedules/steward.ts), which is where confidence
  // comes from: we need to run it enough to trust it keeps working. minimax-m3 is a
  // sound model at ~10x lower cost with a matching 1M context window (vs m2.7's 205K,
  // which risks overflow on large ingest deltas). We knowingly accept a fidelity drop
  // vs sonnet here — TRIAL: validate one real tick's cogmap output + authored-4
  // tool-call reliability against the sonnet baseline (cogmap 019f2391) before trusting
  // it; fall back to anthropic/claude-haiku-4.5 (in-family, 3x cheaper) if it fumbles
  // the tool sequence. Enterprise deployments override this to sonnet/opus + a tighter
  // cadence.
  model: "minimax/minimax-m3",
  description:
    "Team self-cognition steward: distills a team's own temper resources into cogmap-homed nodes and tends the team's cognitive map via the authored-4 (create/assert/facet/fold), audited by the invocation envelope.",
});
