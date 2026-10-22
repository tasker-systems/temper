import { defineConfig } from 'vitest/config';

// The package's TS modules only: `toolchain/check.test.mjs` is a node:test harness, run by
// `node --test` (cargo make design-toolchain), not by vitest.
export default defineConfig({ test: { include: ['src/**/*.test.ts'] } });
