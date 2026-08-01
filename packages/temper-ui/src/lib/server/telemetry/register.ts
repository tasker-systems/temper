/**
 * Side-effecting entrypoint: register the tracer provider as early as possible.
 *
 * Imported for its side effect at the very top of `hooks.server.ts` — the earliest
 * server module SvelteKit loads — so the provider, propagator, and context manager are
 * in place before the first request is handled.
 */

import { initTelemetry } from 'temper-telemetry-ts';

// temper-ui injects trace context at its own known call sites (proxy + SSR loaders),
// so it does not need HTTP auto-instrumentation (instrumentHttp stays off). The eve
// agents flip that on — their outbound MCP fetch is internal to the framework.
initTelemetry({ serviceName: 'temper-ui' });
