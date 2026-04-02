import { describe, expect, it } from "vitest";
import { authenticateRequest, requireMethod } from "../src/middleware.js";

describe("requireMethod", () => {
  it("returns null for matching method", () => {
    const req = new Request("https://example.com/api/test", { method: "POST" });
    expect(requireMethod(req, "POST")).toBeNull();
  });

  it("returns 405 Response for non-matching method", () => {
    const req = new Request("https://example.com/api/test", { method: "GET" });
    const result = requireMethod(req, "POST");
    expect(result).not.toBeNull();
    expect(result?.status).toBe(405);
  });
});

describe("authenticateRequest", () => {
  it("rejects request without Authorization header", async () => {
    const req = new Request("https://example.com/api/test", { method: "POST" });
    const result = await authenticateRequest(req);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.response.status).toBe(401);
    }
  });

  it("rejects request with non-Bearer Authorization", async () => {
    const req = new Request("https://example.com/api/test", {
      method: "POST",
      headers: { Authorization: "Basic abc123" },
    });
    const result = await authenticateRequest(req);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.response.status).toBe(401);
    }
  });

  it("rejects request with invalid JWT", async () => {
    const req = new Request("https://example.com/api/test", {
      method: "POST",
      headers: { Authorization: "Bearer not-a-valid-jwt" },
    });
    const result = await authenticateRequest(req);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.response.status).toBe(401);
    }
  });
});
