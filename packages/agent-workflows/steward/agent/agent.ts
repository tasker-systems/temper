import { defineAgent } from "eve";

import { gatewayModelOptions, resolveModelConfig } from "./lib/model-config.js";

const model = resolveModelConfig();

export default defineAgent({
  // Config-driven, resolved at BUILD time — see lib/model-config.ts for why env is the only lever
  // eve offers, and why a model change takes a redeploy. Defaults reproduce the previous hardcoded
  // behavior exactly, so a deploy with no new env set is a no-op.
  model: model.primary,
  modelOptions: gatewayModelOptions(model.fallbacks),
  description:
    "Team self-cognition steward: distills a team's own temper resources into cogmap-homed nodes and tends the team's cognitive map via the authored-4 (create/assert/facet/fold), audited by the invocation envelope.",
});
