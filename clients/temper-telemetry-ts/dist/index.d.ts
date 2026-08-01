/**
 * temper-telemetry-ts — shared OpenTelemetry span-export bootstrap + W3C trace-context
 * helpers for Temper's Node hops (temper-ui, eve agents), the TS analog of the Rust
 * `temper-telemetry` crate. Consumers resolve OTLP config and W3C context one same way.
 *
 * - Generic: no framework types. The SvelteKit request-span glue and the eve
 *   `instrumentation.ts` wiring live in their respective consumers.
 * - Shipped as compiled `dist/`, consumed via `file:` links, so one package serves
 *   consumers on three workspace-isolated toolchains (bun / npm-TS7 / npm-TS5).
 */
export { forceFlush, getTracer, initTelemetry, isTelemetryEnabled, type InitTelemetryOptions } from './otel.js';
export { activeTraceparent, extractContext } from './context.js';
