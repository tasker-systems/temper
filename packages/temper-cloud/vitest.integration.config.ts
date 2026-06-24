import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/integration/**/*.test.ts"],
    testTimeout: 120_000,
    // The integration suite was exclusively the document-upload pipeline tests
    // (retired alongside the legacy-schema TS write path). The directory now
    // holds only shared helpers; tolerate an empty suite so CI stays green.
    passWithNoTests: true,
  },
});
