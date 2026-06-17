import { describe, expect, it } from "vitest";
import { buildCliCallbackResponse } from "../src/cli-callback.js";

describe("buildCliCallbackResponse", () => {
  it("redirects the code to the localhost port from state, regardless of host", () => {
    const res = buildCliCallbackResponse(
      "/api/auth/cli-callback?code=abc123&state=51789",
      "temper.acme.com",
    );
    expect(res.status).toBe(302);
    expect(res.headers.get("location")).toBe("http://localhost:51789?code=abc123");
  });

  it("works when req.url is absolute (host ignored for the target)", () => {
    const res = buildCliCallbackResponse(
      "https://temper.acme.com/api/auth/cli-callback?code=xy%2Fz&state=40000",
      null,
    );
    expect(res.headers.get("location")).toBe("http://localhost:40000?code=xy%2Fz");
  });

  it("returns 400 when code or state is missing", () => {
    const res = buildCliCallbackResponse("/api/auth/cli-callback?code=abc", "h");
    expect(res.status).toBe(400);
  });

  it("returns 400 for an out-of-range port", () => {
    const res = buildCliCallbackResponse("/api/auth/cli-callback?code=abc&state=80", "h");
    expect(res.status).toBe(400);
  });

  it("surfaces an Auth0 error param as 400", () => {
    const res = buildCliCallbackResponse(
      "/api/auth/cli-callback?error=access_denied&error_description=nope",
      "h",
    );
    expect(res.status).toBe(400);
  });
});
