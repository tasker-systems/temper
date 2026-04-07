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

  it("accepts payload with optional context_id and body_hash", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "# Hello",
      replace: false,
      context_id: "019d6313-0e44-7842-9256-9ee385be3a52",
      body_hash: "sha256:abc123",
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.payload.context_id).toBe("019d6313-0e44-7842-9256-9ee385be3a52");
      expect(result.payload.body_hash).toBe("sha256:abc123");
    }
  });

  it("accepts payload without optional fields", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "# Hello",
      replace: false,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.payload.context_id).toBeUndefined();
      expect(result.payload.body_hash).toBeUndefined();
    }
  });

  it("rejects invalid context_id UUID", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "# Hello",
      replace: false,
      context_id: "not-a-uuid",
    });
    expect(result.ok).toBe(false);
  });
});
