import { defineAgent } from "eve";

export default defineAgent({
  model: "anthropic/claude-sonnet-4.5",
  description:
    "The @temper Slack agent: answers app mentions in a team's workspace. T1 proves the inbound pipe — it resolves the mentioning Slack user to an opaque eve principal and prompts to connect a temper account. Temper reach arrives in a later task.",
  build: {
    // `agent/instrumentation.ts` imports `temper-telemetry-ts`, whose dist pulls the native
    // @opentelemetry SDK. eve keeps that subtree external (never Rolldown-bundled) but only
    // *ships* an external package into the hosted function when it is named here — otherwise the
    // deployed index.mjs imports `temper-telemetry-ts` from a node_modules that was never traced,
    // and the function dies at startup with ERR_MODULE_NOT_FOUND. Its transitive @opentelemetry
    // deps ride along because the `build:dep` prebuild runs `npm ci` in the client, so nft can
    // trace the whole subtree (static + the dynamic instrumentHttp imports) into server/node_modules.
    externalDependencies: ["temper-telemetry-ts"],
  },
});
