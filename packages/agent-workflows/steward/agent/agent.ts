import { defineAgent } from "eve";

export default defineAgent({
  // Judgment-heavy distillation + supersession; sonnet-5 is the scaffold default.
  // Bump to a larger model here if synthesis quality warrants it.
  model: "anthropic/claude-sonnet-5",
  description:
    "Team self-cognition steward: distills a team's own temper resources into cogmap-homed nodes and tends the team's cognitive map via the authored-4 (create/assert/facet/fold), audited by the invocation envelope.",
});
