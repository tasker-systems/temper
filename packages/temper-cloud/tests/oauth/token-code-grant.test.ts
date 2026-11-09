import { createHash } from "node:crypto";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NeonClient } from "../../src/db.js";
import { logger } from "../../src/logger.js";
import { handleToken } from "../../src/oauth/endpoints.js";

/**
 * The authorization_code grant answers `invalid_grant` only when the code itself is refused. A
 * token store that fails to answer says nothing about the code, and is answered
 * `temporarily_unavailable` — the same split the refresh grant makes.
 */

const VERIFIER = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

function codeRequest(verifier = VERIFIER): Request {
  return new Request("https://as/oauth/token", {
    method: "POST",
    body: new URLSearchParams({
      grant_type: "authorization_code",
      code: "code-1",
      code_verifier: verifier,
      client_id: "cli",
    }),
  });
}

/** A store that answers every statement with `rows`. */
function storeAnswering(rows: unknown[]): NeonClient {
  return (async () => rows) as unknown as NeonClient;
}

/** A store whose every statement fails the way a dropped Neon connection does. */
function storeFailing(): NeonClient {
  return (async () => {
    throw Object.assign(new Error("Error connecting to database: fetch failed"), {
      code: "ECONNRESET",
    });
  }) as unknown as NeonClient;
}

describe("handleToken authorization_code grant", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("answers invalid_grant for an unknown, expired or spent code, and logs no outage", async () => {
    const errorSpy = vi.spyOn(logger, "error").mockImplementation(() => undefined);

    const res = await handleToken(codeRequest(), storeAnswering([]));

    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "invalid_grant" });
    expect(errorSpy).not.toHaveBeenCalled();
  });

  it("answers invalid_grant for a PKCE mismatch", async () => {
    const errorSpy = vi.spyOn(logger, "error").mockImplementation(() => undefined);
    const challenge = createHash("sha256").update(VERIFIER).digest("base64url");
    const db = storeAnswering([{ code_challenge: challenge, expires_at: "2099-01-01T00:00:00Z" }]);

    const res = await handleToken(codeRequest("not-the-verifier"), db);

    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "invalid_grant" });
    expect(errorSpy).not.toHaveBeenCalled();
  });

  it("answers 503 temporarily_unavailable, and logs, when the store does not answer", async () => {
    const errorSpy = vi.spyOn(logger, "error").mockImplementation(() => undefined);

    const res = await handleToken(codeRequest(), storeFailing());

    expect(res.status).toBe(503);
    expect(await res.json()).toEqual({ error: "temporarily_unavailable" });
    expect(errorSpy).toHaveBeenCalledTimes(1);
    expect(errorSpy.mock.calls[0]?.[0]).toEqual({
      err: "Error connecting to database: fetch failed",
    });
  });
});
