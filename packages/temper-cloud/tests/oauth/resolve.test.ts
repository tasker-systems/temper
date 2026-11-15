import { afterEach, describe, expect, it, vi } from "vitest";
import type { NeonClient } from "../../src/db.js";
import { RESOLVE_DOOR, SIGNATURE_V2_HEADER } from "../../src/oauth/reconcile.js";
import {
  classifyInternalCallFailure,
  RESOLVE_CHANNEL,
  recordChannelFailure,
} from "../../src/oauth/reconcile-health.js";
import { resolvePrincipal } from "../../src/oauth/resolve.js";

const payload = { external_user_id: "nid-1", email: "a@corp.io", email_verified: true };
const RESOLVE_URL = `https://api.internal${RESOLVE_DOOR}`;
const PROFILE_ID = "0190b6a1-7c1e-7d2a-9f3b-1a2b3c4d5e6f";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

function configure(url: string | undefined = RESOLVE_URL): void {
  vi.stubEnv("INTERNAL_RESOLVE_URL", url as string);
  vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
}

async function failureOf(): Promise<unknown> {
  return resolvePrincipal(payload).then(
    () => {
      throw new Error("expected the resolve to fail, and it did not");
    },
    (e: unknown) => e,
  );
}

describe("resolvePrincipal", () => {
  it("POSTs a signed request to INTERNAL_RESOLVE_URL and returns the profile id", async () => {
    configure();
    const fetchMock = vi.fn(
      async () =>
        new Response(JSON.stringify({ profile_id: PROFILE_ID }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(resolvePrincipal(payload)).resolves.toEqual({ profile_id: PROFILE_ID });
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe(RESOLVE_URL);
    expect((init.headers as Record<string, string>)[SIGNATURE_V2_HEADER]).toMatch(/^[0-9a-f]{64}$/);
  });

  // FAILS IF: a 401 from the resolve door is not the conclusive `unauthorized`. Before it was
  // classified it was a plain Error, and the health reader could not tell it from weather.
  it("a 401 is unauthorized", async () => {
    configure();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("no", { status: 401 })),
    );
    expect(classifyInternalCallFailure(await failureOf())).toEqual({
      cause: "unauthorized",
      detail: "HTTP 401",
    });
  });

  // FAILS IF: a URL whose path is not the resolve door is signed and sent.
  it.each([
    ["a stray segment", "https://api.internal/v1/internal/principal/resolve"],
    ["the reconcile door", "https://api.internal/internal/saml/reconcile"],
    ["a value that is not a URL", "not a url"],
  ])("%s is config_missing and nothing is sent", async (_what, url) => {
    configure(url);
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    expect(classifyInternalCallFailure(await failureOf())).toEqual({
      cause: "config_missing",
      detail: "INTERNAL_RESOLVE_URL path",
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  // FAILS IF: an unset variable is a plain Error again, which the reader would wait on as weather.
  it.each([
    ["INTERNAL_RESOLVE_URL", { INTERNAL_RECONCILE_SECRET: "s3cr3t" }],
    ["INTERNAL_RECONCILE_SECRET", { INTERNAL_RESOLVE_URL: RESOLVE_URL }],
  ])("an unset %s is config_missing and names it", async (name, env) => {
    vi.stubEnv("INTERNAL_RESOLVE_URL", undefined as unknown as string);
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", undefined as unknown as string);
    for (const [k, v] of Object.entries(env)) {
      vi.stubEnv(k, v);
    }
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    expect(classifyInternalCallFailure(await failureOf())).toEqual({
      cause: "config_missing",
      detail: name,
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it.each([
    {
      what: "any other status is endpoint_error",
      fetchImpl: async () => new Response("boom", { status: 500 }),
      expected: { cause: "endpoint_error", detail: "HTTP 500" },
    },
    {
      what: "no answer at all is transport",
      fetchImpl: async () => {
        throw new TypeError("fetch failed");
      },
      expected: { cause: "transport", detail: "TypeError" },
    },
    {
      what: "a 200 with no usable profile id is endpoint_error",
      fetchImpl: async () => new Response(JSON.stringify({ profile_id: 7 }), { status: 200 }),
      expected: { cause: "endpoint_error", detail: "HTTP 200 no profile_id" },
    },
    {
      what: "a 200 that is not JSON is endpoint_error",
      fetchImpl: async () => new Response("<html>", { status: 200 }),
      expected: { cause: "endpoint_error", detail: "HTTP 200 unreadable body" },
    },
  ])("$what", async ({ fetchImpl, expected }) => {
    configure();
    vi.stubGlobal("fetch", vi.fn(fetchImpl));
    expect(classifyInternalCallFailure(await failureOf())).toEqual(expected);
  });

  // FAILS IF: a refused resolve does not reach the health table as `unauthorized` on
  // `principal_resolve`, which is the record the health cron pages from.
  it("a refused resolve is recorded on principal_resolve as unauthorized", async () => {
    configure();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("no", { status: 401 })),
    );
    const err = await failureOf();

    const writes: unknown[][] = [];
    const db = ((_strings: TemplateStringsArray, ...values: unknown[]) => {
      writes.push(values);
      return Promise.resolve([]);
    }) as unknown as NeonClient;
    await recordChannelFailure(db, RESOLVE_CHANNEL, err);

    expect(RESOLVE_CHANNEL).toBe("principal_resolve");
    expect(writes).toEqual([["principal_resolve", "unauthorized", "HTTP 401"]]);
  });
});
