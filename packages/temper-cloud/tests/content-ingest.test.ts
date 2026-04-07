import { describe, expect, it } from "vitest";
import { validatePayload } from "../src/content-ingest.js";

describe("validatePayload", () => {
  it("accepts valid payload", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "# Hello\n\nWorld",
      replace: false,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.payload.resource_id).toBe("019d6313-0e44-7842-9256-9ee385be3a51");
      expect(result.payload.replace).toBe(false);
    }
  });

  it("rejects missing resource_id", () => {
    const result = validatePayload({ content: "hello", replace: false });
    expect(result.ok).toBe(false);
  });

  it("rejects empty content", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "",
      replace: false,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects invalid UUID", () => {
    const result = validatePayload({
      resource_id: "not-a-uuid",
      content: "hello",
      replace: false,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects missing replace flag", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "hello",
    });
    expect(result.ok).toBe(false);
  });
});
