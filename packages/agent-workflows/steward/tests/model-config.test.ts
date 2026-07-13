import { describe, expect, it } from "vitest";
import { DEFAULT_FALLBACKS, DEFAULT_MODEL, resolveModelConfig } from "../agent/lib/model-config.js";

describe("resolveModelConfig", () => {
  it("reproduces today's behavior when nothing is configured", () => {
    expect(resolveModelConfig({})).toEqual({
      primary: DEFAULT_MODEL,
      fallbacks: [...DEFAULT_FALLBACKS],
    });
  });

  it("takes the primary from STEWARD_MODEL", () => {
    expect(resolveModelConfig({ STEWARD_MODEL: "anthropic/claude-sonnet-5" }).primary).toBe(
      "anthropic/claude-sonnet-5",
    );
  });

  it("takes an ordered fallback list from STEWARD_MODEL_FALLBACKS", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL_FALLBACKS: "anthropic/claude-haiku-4.5,openai/gpt-5.5",
      }).fallbacks,
    ).toEqual(["anthropic/claude-haiku-4.5", "openai/gpt-5.5"]);
  });

  it("trims whitespace and drops empty entries", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL_FALLBACKS: " anthropic/claude-haiku-4.5 , , openai/gpt-5.5,",
      }).fallbacks,
    ).toEqual(["anthropic/claude-haiku-4.5", "openai/gpt-5.5"]);
  });

  // The gateway walks the list AFTER the primary fails. Leaving the primary in it would re-try the
  // model that just failed before reaching a model that might work.
  it("drops the primary out of its own fallback list", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL: "minimax/minimax-m3",
        STEWARD_MODEL_FALLBACKS: "minimax/minimax-m3,anthropic/claude-haiku-4.5",
      }).fallbacks,
    ).toEqual(["anthropic/claude-haiku-4.5"]);
  });

  it("dedupes repeated fallbacks, keeping first-seen order", () => {
    expect(
      resolveModelConfig({
        STEWARD_MODEL_FALLBACKS: "openai/gpt-5.5,anthropic/claude-haiku-4.5,openai/gpt-5.5",
      }).fallbacks,
    ).toEqual(["openai/gpt-5.5", "anthropic/claude-haiku-4.5"]);
  });

  it("supports an explicitly empty fallback list", () => {
    expect(resolveModelConfig({ STEWARD_MODEL_FALLBACKS: "" }).fallbacks).toEqual([]);
  });
});
