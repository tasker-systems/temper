import { defineAgent } from "eve";

export default defineAgent({
  model: "anthropic/claude-sonnet-4.5",
  description:
    "The @temper Slack agent: answers app mentions in a team's workspace. T1 proves the inbound pipe — it resolves the mentioning Slack user to an opaque eve principal and prompts to connect a temper account. Temper reach arrives in a later task.",
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
