import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    exclude: [
      "tests/integration/**",
      "tests/workflow/embed.test.ts",
    ],
  },
});
