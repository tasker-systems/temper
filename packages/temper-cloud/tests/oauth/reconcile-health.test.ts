import { describe, expect, it, vi } from "vitest";
import type { NeonClient } from "../../src/db.js";
import { reconcileMemberships } from "../../src/oauth/reconcile.js";
import {
  classifyInternalCallFailure,
  InternalCallError,
  recordChannelFailure,
  recordChannelSuccess,
} from "../../src/oauth/reconcile-health.js";

const payload = {
  provider: "saml:acme",
  external_user_id: "nid-1",
  email: "a@corp.io",
  email_verified: true,
  idp_key: "acme",
  groups: ["engineering"],
};

/** Catch and return, so each case can assert on the classified error rather than only that it threw. */
async function failureOf(fn: () => Promise<unknown>): Promise<unknown> {
  try {
    await fn();
  } catch (err) {
    return err;
  }
  throw new Error("expected the call to fail, and it did not");
}

describe("classifying a reconcile failure", () => {
  /**
   * The four causes, each raised the way the deployment actually raises it. This is the test that
   * matters most in the file: the cause is not a label on the log line, it is what decides whether
   * one occurrence is treated as conclusive. A cause misclassified downward turns a channel that
   * will never recover into one the reader waits for a second occurrence of.
   */
  it.each([
    {
      what: "an unset URL is a deployment fact, and names the variable to set",
      env: { INTERNAL_RECONCILE_SECRET: "s3cr3t" },
      fetchImpl: async () => new Response(null, { status: 204 }),
      cause: "config_missing",
      detail: "INTERNAL_RECONCILE_URL",
    },
    {
      what: "an unset secret is a deployment fact, and names the variable to set",
      env: { INTERNAL_RECONCILE_URL: "https://api.internal/internal/saml/reconcile" },
      fetchImpl: async () => new Response(null, { status: 204 }),
      cause: "config_missing",
      detail: "INTERNAL_RECONCILE_SECRET",
    },
    {
      what: "a rejected signature is a deployment fact, not weather",
      env: {
        INTERNAL_RECONCILE_URL: "https://api.internal/internal/saml/reconcile",
        INTERNAL_RECONCILE_SECRET: "s3cr3t",
      },
      fetchImpl: async () => new Response("no", { status: 401 }),
      cause: "unauthorized",
      detail: "HTTP 401",
    },
    {
      what: "any other non-2xx can be weather and must recur to count",
      env: {
        INTERNAL_RECONCILE_URL: "https://api.internal/internal/saml/reconcile",
        INTERNAL_RECONCILE_SECRET: "s3cr3t",
      },
      fetchImpl: async () => new Response("boom", { status: 500 }),
      cause: "endpoint_error",
      detail: "HTTP 500",
    },
    {
      what: "a request that got no answer at all is transport",
      env: {
        INTERNAL_RECONCILE_URL: "https://api.internal/internal/saml/reconcile",
        INTERNAL_RECONCILE_SECRET: "s3cr3t",
      },
      fetchImpl: async () => {
        throw new TypeError("fetch failed");
      },
      cause: "transport",
      detail: "TypeError",
    },
  ])("$what", async ({ env, fetchImpl, cause, detail }) => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", undefined as unknown as string);
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", undefined as unknown as string);
    for (const [k, v] of Object.entries(env)) {
      vi.stubEnv(k, v);
    }
    vi.stubGlobal("fetch", vi.fn(fetchImpl));
    try {
      const err = await failureOf(() => reconcileMemberships(payload));
      expect(classifyInternalCallFailure(err)).toEqual({ cause, detail });
    } finally {
      vi.unstubAllGlobals();
      vi.unstubAllEnvs();
    }
  });

  /**
   * The fallback direction, asserted rather than assumed. An unclassified error is unbounded in
   * kind, so it must earn its alert by recurring — picking a conclusive cause here would fire a
   * page on the first instance of any error class nobody has examined.
   */
  it("an error from outside the classified set falls to the weather-capable cause", () => {
    expect(classifyInternalCallFailure(new Error("something else"))).toEqual({
      cause: "endpoint_error",
      detail: "unclassified",
    });
    expect(classifyInternalCallFailure("not even an error")).toEqual({
      cause: "endpoint_error",
      detail: "unclassified",
    });
  });

  /** A transport error whose name is not a plain identifier is reported, not stored verbatim. */
  it("a transport detail that is not a name is reported as unknown", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    const weird = new Error("boom");
    weird.name = "Some Name With Spaces And <angle brackets>";
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw weird;
      }),
    );
    try {
      const err = await failureOf(() => reconcileMemberships(payload));
      expect(classifyInternalCallFailure(err)).toEqual({
        cause: "transport",
        detail: "unknown",
      });
    } finally {
      vi.unstubAllGlobals();
      vi.unstubAllEnvs();
    }
  });
});

describe("recording cannot become a new way for authentication to fail", () => {
  /**
   * The load-bearing property of the whole mechanism, asserted where it can fail.
   *
   * `recordChannelFailure` runs INSIDE the ACS's fail-open catch. A throw from it does not land in
   * that catch — it has already been entered — so it escapes to the ACS's outer handler, which
   * answers `400 SAML assertion rejected`. A validly-authenticated human would be refused a login
   * by the code watching for de-provisioning failures. Both functions must therefore swallow, and
   * these two assertions are what stop a later refactor removing the swallow.
   */
  it("swallows a database failure on the failure path", async () => {
    const exploding = (() => {
      throw new Error("connection terminated");
    }) as unknown as NeonClient;
    await expect(
      recordChannelFailure(
        exploding,
        "saml_reconcile",
        new InternalCallError("transport", "TypeError", "x"),
      ),
    ).resolves.toBeUndefined();
  });

  it("swallows a database failure on the success path", async () => {
    const exploding = (() => {
      throw new Error("connection terminated");
    }) as unknown as NeonClient;
    await expect(recordChannelSuccess(exploding, "saml_reconcile")).resolves.toBeUndefined();
  });
});
