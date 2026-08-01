/**
 * Side-effecting entrypoint: register the tracer provider as early as possible.
 *
 * Imported for its side effect at the very top of `hooks.server.ts` — the earliest
 * server module SvelteKit loads — so the provider, propagator, and context manager are
 * in place before the first request is handled.
 */

import { initTelemetry } from './otel';

initTelemetry();
