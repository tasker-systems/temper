import { defineAgent } from "eve";

import { resolveModelConfig } from "./lib/model-config.js";

const model = resolveModelConfig();

/**
 * The AI Gateway tries the primary, then each fallback IN ORDER, returning the first that succeeds.
 * Omit the key entirely when there is nothing to fall back to — an empty list is not a policy.
 */
function modelOptions(fallbacks: string[]) {
  return fallbacks.length === 0
    ? {}
    : { providerOptions: { gateway: { models: fallbacks } } };
}

export default defineAgent({
  // Config-driven, resolved at BUILD time — see lib/model-config.ts for why env is the only lever
  // eve offers, and why a model change takes a redeploy. Defaults reproduce the previous hardcoded
  // behavior exactly, so a deploy with no new env set is a no-op.
  model: model.primary,
  modelOptions: modelOptions(model.fallbacks),
  description:
    "Team self-cognition steward: distills a team's own temper resources into cogmap-homed nodes and tends the team's cognitive map via the authored-4 (create/assert/facet/fold), audited by the invocation envelope.",
});
