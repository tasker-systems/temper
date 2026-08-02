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
  build: {
    // `agent/instrumentation.ts` imports `temper-telemetry-ts`, whose dist pulls the native
    // @opentelemetry SDK. eve keeps that subtree external (never Rolldown-bundled) but only
    // *ships* an external package into the hosted function when it is named here — otherwise the
    // deployed index.mjs imports `temper-telemetry-ts` from a node_modules that was never traced,
    // and the function dies at startup with ERR_MODULE_NOT_FOUND. Its transitive @opentelemetry
    // deps ride along because `build:dep` runs `npm ci` in the client, so nft can trace the whole
    // subtree (static + the dynamic instrumentHttp imports) into server/node_modules.
    externalDependencies: ["temper-telemetry-ts"],
  },
});
