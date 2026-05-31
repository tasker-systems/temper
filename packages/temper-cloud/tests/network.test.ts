import type { TestContext } from "vitest";
import { describe, expect, it } from "vitest";
import {
  isNetworkConnectivityError,
  runOrSkipOnNetworkFlake,
} from "./integration/helpers/network.js";

/**
 * Minimal fake of vitest's test context. The real `ctx.skip(reason)` aborts the
 * test by throwing a skip signal; we emulate that by throwing a tagged error so
 * the helper's control flow (and our assertions) match production behavior.
 */
const SKIP_TAG = "__SKIP__:";
function fakeCtx(): TestContext & { skips: string[] } {
  const skips: string[] = [];
  return {
    skips,
    skip: (note?: string): never => {
      skips.push(note ?? "");
      throw new Error(`${SKIP_TAG}${note ?? ""}`);
    },
  } as unknown as TestContext & { skips: string[] };
}

function skipReason(err: unknown): string | null {
  return err instanceof Error && err.message.startsWith(SKIP_TAG)
    ? err.message.slice(SKIP_TAG.length)
    : null;
}

describe("runOrSkipOnNetworkFlake", () => {
  it("returns the value when the operation resolves", async () => {
    const ctx = fakeCtx();
    const result = await runOrSkipOnNetworkFlake(ctx, "op", () => Promise.resolve([1, 2, 3]));
    expect(result).toEqual([1, 2, 3]);
    expect(ctx.skips).toHaveLength(0);
  });

  it("skips when the operation throws a network connectivity error", async () => {
    const ctx = fakeCtx();
    const netErr = Object.assign(new Error("fetch failed"), {
      cause: Object.assign(new Error("getaddrinfo ENOTFOUND huggingface.co"), {
        code: "ENOTFOUND",
      }),
    });
    const err = await runOrSkipOnNetworkFlake(ctx, "model pull", () =>
      Promise.reject(netErr),
    ).catch((e) => e);
    expect(skipReason(err)).toMatch(/unreachable/i);
    expect(ctx.skips).toHaveLength(1);
  });

  it("skips when the operation outruns its budget (hang)", async () => {
    const ctx = fakeCtx();
    const err = await runOrSkipOnNetworkFlake(
      ctx,
      "model pull",
      () => new Promise<number[]>(() => {}), // never resolves
      { budgetMs: 10 },
    ).catch((e) => e);
    expect(skipReason(err)).toMatch(/exceeded 10ms budget/i);
    expect(ctx.skips).toHaveLength(1);
  });

  it("rethrows a non-network error without skipping", async () => {
    const ctx = fakeCtx();
    const boom = new Error("assertion blew up");
    const err = await runOrSkipOnNetworkFlake(ctx, "op", () => Promise.reject(boom)).catch(
      (e) => e,
    );
    expect(err).toBe(boom);
    expect(skipReason(err)).toBeNull();
    expect(ctx.skips).toHaveLength(0);
  });
});

describe("isNetworkConnectivityError", () => {
  it("is true for a wrapped cause chain carrying a known code", () => {
    const err = Object.assign(new Error("fetch failed"), {
      cause: Object.assign(new Error("connect"), { code: "UND_ERR_CONNECT_TIMEOUT" }),
    });
    expect(isNetworkConnectivityError(err)).toBe(true);
  });

  it("is true for a recognizable message even without a code", () => {
    expect(isNetworkConnectivityError(new Error("getaddrinfo failed"))).toBe(true);
  });

  it("is false for an ordinary error", () => {
    expect(isNetworkConnectivityError(new Error("expected 768, got 384"))).toBe(false);
  });
});
