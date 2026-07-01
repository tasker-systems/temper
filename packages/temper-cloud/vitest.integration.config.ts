import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/integration/**/*.test.ts"],
    testTimeout: 120_000,
    // The integration suite was exclusively the document-upload pipeline tests
    // (retired alongside the legacy-schema TS write path). The directory now
    // holds only shared helpers; tolerate an empty suite so CI stays green.
    passWithNoTests: true,
    // Integration test files share one real Postgres database and each truncates its own tables
    // in `beforeEach` (e.g. oauth/flow.test.ts and oauth/endpoints.test.ts both truncate
    // kb_oauth_flow/kb_saml_idp). Vitest's default per-file worker parallelism lets one file's
    // TRUNCATE race another file's in-flight test against the same tables, causing intermittent
    // failures. Run integration files sequentially to keep the shared-DB fixture deterministic.
    fileParallelism: false,
  },
});
