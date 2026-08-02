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
    // `agent/instrumentation.ts` imports `temper-telemetry-ts` (the OTLP export bootstrap), whose
    // committed dist pulls the native @opentelemetry SDK. Two facts are load-bearing:
    //   1. `.vercelignore` strips `dist/` before the build runs, so the committed
    //      `temper-telemetry-ts` dist is GONE at eve-build time. `build:dep` must `npm run build`
    //      it back (exactly what the temper-ts client already does) — without the rebuild the
    //      import cannot resolve and the function dies at startup with
    //      `ERR_MODULE_NOT_FOUND: temper-telemetry-ts`.
    //   2. eve keeps the @opentelemetry subtree external (never Rolldown-bundled); an external
    //      package is only shipped into the hosted function when named here, so nitro traces the
    //      rebuilt dist + its @opentelemetry deps (static + the dynamic instrumentHttp imports)
    //      into server/node_modules.
    externalDependencies: ["temper-telemetry-ts"],
  },
});
