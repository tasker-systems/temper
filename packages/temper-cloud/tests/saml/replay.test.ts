import { describe, expect, it } from "vitest";
import type { NeonClient } from "../../src/db.js";
import { guardReplay, ReplayError } from "../../src/saml/replay.js";

function fakeDb(results: unknown[][]): NeonClient {
  const queue = [...results];
  const next = () => queue.shift() ?? [];
  return ((..._args: unknown[]) => Promise.resolve(next())) as unknown as NeonClient;
}

describe("guardReplay", () => {
  it("resolves on first use and throws ReplayError on replay", async () => {
    const db = fakeDb([[{ assertion_id: "a1" }], []]);

    await expect(
      guardReplay(db, "a1", new Date("2026-07-01T01:00:00.000Z")),
    ).resolves.toBeUndefined();
    await expect(guardReplay(db, "a1", new Date("2026-07-01T01:00:00.000Z"))).rejects.toThrow(
      ReplayError,
    );
  });

  it("includes the assertion id in the error message", async () => {
    const db = fakeDb([[]]);

    await expect(guardReplay(db, "a2", new Date("2026-07-01T01:00:00.000Z"))).rejects.toThrow(
      /SAML assertion replayed: a2/,
    );
  });
});
