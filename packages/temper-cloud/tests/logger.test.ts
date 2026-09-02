import pino from "pino";
import { describe, expect, it } from "vitest";
import { loggerOptions, REDACT_PATHS } from "../src/logger.js";

/**
 * Builds a logger from the production options, writing into memory instead of stdout, so the
 * redact behavior under test is the redact behavior that ships.
 */
function capture(bindings: Record<string, unknown>): Record<string, unknown> {
  const lines: string[] = [];
  const log = pino(
    { ...loggerOptions, level: "info" },
    { write: (chunk: string) => lines.push(chunk) },
  );
  log.info(bindings, "probe");
  return JSON.parse(lines.join("")) as Record<string, unknown>;
}

describe("logger redact configuration", () => {
  it("replaces credential-shaped bound fields with the censor, flat and nested", () => {
    const entry = capture({
      token: "t",
      access_token: "at",
      refresh_token: "rt",
      secret: "s",
      client_secret: "cs",
      authorization: "a",
      cookie: "c",
      AS_SIGNING_KEY_PKCS8: "pem",
      nested: { refresh_token: "rt2" },
    });

    expect(entry.token).toBe("[Redacted]");
    expect(entry.access_token).toBe("[Redacted]");
    expect(entry.refresh_token).toBe("[Redacted]");
    expect(entry.secret).toBe("[Redacted]");
    expect(entry.client_secret).toBe("[Redacted]");
    expect(entry.authorization).toBe("[Redacted]");
    expect(entry.cookie).toBe("[Redacted]");
    expect(entry.AS_SIGNING_KEY_PKCS8).toBe("[Redacted]");
    expect(entry.nested).toEqual({ refresh_token: "[Redacted]" });
  });

  it("leaves unlisted fields untouched", () => {
    const entry = capture({ profile_id: "p1", groups: 2 });
    expect(entry).toMatchObject({ profile_id: "p1", groups: 2 });
  });

  it("carries every path the backstop requires", () => {
    const required = [
      "token",
      "*.token",
      "access_token",
      "*.access_token",
      "refresh_token",
      "*.refresh_token",
      "secret",
      "*.secret",
      "client_secret",
      "*.client_secret",
      "authorization",
      "*.authorization",
      "cookie",
      "*.cookie",
      "AS_SIGNING_KEY_PKCS8",
      "*.AS_SIGNING_KEY_PKCS8",
      "as_signing_key_pkcs8",
      "*.as_signing_key_pkcs8",
      "AS_SIGNING_KID",
      "*.AS_SIGNING_KID",
    ];
    for (const path of required) {
      expect(REDACT_PATHS, path).toContain(path);
    }
  });
});
