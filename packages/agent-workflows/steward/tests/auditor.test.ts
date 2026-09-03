import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type MockApi, type MockIssuer, startMockApi, startMockIssuer } from "temper-ts/testing";

import {
  DEFAULT_AUDITOR_FALLBACKS,
  DEFAULT_AUDITOR_MODEL,
  DEFAULT_MODEL,
  resolveAuditorModelConfig,
  resolveModelConfig,
} from "../agent/lib/model-config.js";
import { AUDITOR_TOOLS, STEWARD_TOOLS } from "../agent/lib/tool-allowlists.js";

let issuer: MockIssuer | undefined;
let api: MockApi | undefined;
let refusing: { url: string; close(): Promise<void> } | undefined;

/**
 * A token endpoint that refuses every mint with one status.
 *
 * Deliberately NOT an option on `startMockIssuer`. That mock's own header says the auth0 flavor "is
 * asserted against NOTHING real" and must not be trusted "for the exact status or error code it says
 * so with" — so teaching it a quota response would dress a local guess as a pinned contract. This
 * server claims nothing about Auth0; it exists to put one status on the wire so the classifier can
 * be witnessed against it.
 */
async function startRefusingIssuer(
  status: number,
  body: unknown,
): Promise<{ url: string; close(): Promise<void> }> {
  const server: Server = createServer((_req, res) => {
    res.writeHead(status, { "content-type": "application/json" });
    res.end(JSON.stringify(body));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://127.0.0.1:${port}/oauth/token`,
    close: () =>
      new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  };
}

const AUDITOR_ENV_NAMES = [
  "TEMPER_AUDITOR_M2M_CLIENT_ID",
  "TEMPER_AUDITOR_M2M_CLIENT_SECRET",
  "TEMPER_AUDITOR_M2M_TOKEN_URL",
  "TEMPER_AUDITOR_M2M_AUDIENCE",
  "TEMPER_AUDITOR_TOKEN",
];
const OPTIONAL_AGENT_ENV_NAMES = ["TEMPER_AUDITOR_ENABLED"];
const STEWARD_ENV_NAMES = [
  "TEMPER_M2M_CLIENT_ID",
  "TEMPER_M2M_CLIENT_SECRET",
  "TEMPER_M2M_TOKEN_URL",
  "TEMPER_M2M_AUDIENCE",
  "TEMPER_CONNECT_CONNECTOR",
  "TEMPER_TOKEN",
];

beforeEach(() => {
  vi.resetModules();
  for (const name of [...AUDITOR_ENV_NAMES, ...STEWARD_ENV_NAMES, ...OPTIONAL_AGENT_ENV_NAMES]) {
    delete process.env[name];
  }
});

afterEach(async () => {
  await issuer?.close();
  await api?.close();
  await refusing?.close();
  issuer = undefined;
  api = undefined;
  refusing = undefined;
});

describe("resolveAuditorModelConfig", () => {
  // LOAD-BEARING (spec §5.3). Running the auditor on a different model is the one lever that
  // genuinely attacks shared trained priors between the two personas. A default that reproduced the
  // steward's would leave it un-pulled for every deployment that never sets the env — which is most
  // of them — while every other structural separation kept looking correct.
  it("defaults to a DIFFERENT model from the steward", () => {
    expect(resolveAuditorModelConfig({}).primary).toBe(DEFAULT_AUDITOR_MODEL);
    expect(resolveAuditorModelConfig({}).primary).not.toBe(resolveModelConfig({}).primary);
  });

  it("reproduces its documented defaults when nothing is configured", () => {
    expect(resolveAuditorModelConfig({})).toEqual({
      primary: DEFAULT_AUDITOR_MODEL,
      fallbacks: [...DEFAULT_AUDITOR_FALLBACKS],
    });
  });

  it("takes the primary from AUDITOR_MODEL and the list from AUDITOR_MODEL_FALLBACKS", () => {
    expect(
      resolveAuditorModelConfig({
        AUDITOR_MODEL: "openai/gpt-5.5",
        AUDITOR_MODEL_FALLBACKS: " anthropic/claude-haiku-4.5 , , openai/gpt-5.5,",
      }),
    ).toEqual({
      primary: "openai/gpt-5.5",
      // Trimmed, empties dropped, and the primary removed from its own fallback list.
      fallbacks: ["anthropic/claude-haiku-4.5"],
    });
  });

  it("supports an explicitly empty fallback list — the way to hold the independence lever under failure", () => {
    expect(resolveAuditorModelConfig({ AUDITOR_MODEL_FALLBACKS: "" }).fallbacks).toEqual([]);
  });

  // The two resolvers share one implementation; this is what stops that sharing from becoming
  // coupling. Setting one persona's model must never move the other's.
  it("does not cross-talk with the steward's env", () => {
    expect(resolveAuditorModelConfig({ STEWARD_MODEL: "openai/gpt-5.5" }).primary).toBe(
      DEFAULT_AUDITOR_MODEL,
    );
    expect(resolveModelConfig({ AUDITOR_MODEL: "openai/gpt-5.5" }).primary).toBe(DEFAULT_MODEL);
  });
});

describe("the persona boundary between the two allow-lists", () => {
  // LOAD-BEARING. Every audit run begins in a ROOT STEWARD session: eve offers no way to start a
  // session directly on a declared subagent, so `schedules/auditor.ts` opens a root session and
  // tells it — by PROMPT — to make exactly one call, into the `auditor` subagent, and to touch no
  // temper tool itself. Everything downstream of that instruction is prompt-strength, and the
  // subagent's report (derived from attacker-authorable resource text) returns into that same root
  // session.
  //
  // What keeps the hop safe is capability, not prose: the steward's credential simply cannot emit an
  // audit, because `record_citation_audit` is not in its allow-list. That was true only by accident
  // of two separately-authored lists until this test existed. Add the tool to the steward's list —
  // the plausible edit, since the steward is where the schedule lives — and this reds.
  it("the steward cannot record a citation audit, however its session is talked to", () => {
    expect(STEWARD_TOOLS).not.toContain("record_citation_audit");
    expect(AUDITOR_TOOLS).toContain("record_citation_audit");
  });

  // The other direction of the same boundary, and the reason the auditor is a separate agent at all:
  // an auditor that can author findings is a citer, and spec §7's self-audit denial arm should never
  // be the only thing between the two roles. The authored-4 is four authoring verbs — create, edge
  // assert, facet, fold — over three tools since the registry consolidated the edge verbs into
  // `relationship` (`assert`/`fold` via its `action` discriminator).
  it("the auditor holds none of the authored-4", () => {
    for (const authored of ["create_resource", "relationship", "facet_set"]) {
      expect(AUDITOR_TOOLS).not.toContain(authored);
      expect(STEWARD_TOOLS).toContain(authored);
    }
  });

  // The auditor completes ITS job; the steward's watermark is not its to advance.
  it("the auditor cannot advance the steward's watermark", () => {
    expect(AUDITOR_TOOLS).not.toContain("steward_advance_watermark");
  });
});

describe("auditor credentials", () => {
  it("reads a wholly disjoint env name set from the steward's", async () => {
    const { AUDITOR_CREDENTIALS, STEWARD_CREDENTIALS } = await import(
      "../agent/lib/temper-auth.js"
    );
    const auditorNames = Object.values(AUDITOR_CREDENTIALS).filter(
      (v): v is string => typeof v === "string",
    );
    const stewardNames = Object.values(STEWARD_CREDENTIALS).filter(
      (v): v is string => typeof v === "string",
    );

    // One credential is one emitter entity (spec §5.2). Any shared env name is a path by which the
    // two personas end up authenticating as one principal, and the ledger stops being able to tell
    // an audit from the citation it audits.
    expect(auditorNames.some((name) => stewardNames.includes(name))).toBe(false);
  });

  // A Connect connector is scoped to the DEPLOYMENT, not to a principal. An auditor that fell back
  // to it would authenticate as the deployment — i.e. as the steward — silently, through a fallback
  // nobody chose.
  it("has no Vercel Connect branch", async () => {
    const { AUDITOR_CREDENTIALS } = await import("../agent/lib/temper-auth.js");
    expect(AUDITOR_CREDENTIALS.connector).toBeUndefined();
  });

  it("mints with TEMPER_AUDITOR_M2M_* and never with the steward's credential", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_auditor",
      clientSecret: "s3cr3t",
    });
    api = await startMockApi();
    process.env.TEMPER_AUDITOR_M2M_CLIENT_ID = "tmpr_auditor";
    process.env.TEMPER_AUDITOR_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_AUDITOR_M2M_TOKEN_URL = issuer.url;
    // The steward's credential is present and configured — and must be ignored.
    process.env.TEMPER_TOKEN = "steward-dev-token";

    const { auditorFetch } = await import("../agent/lib/temper-auth.js");
    const res = await auditorFetch(api.url, { method: "POST", body: "{}" });

    expect(res.status).toBe(200);
    expect(api.bearers).toEqual(["temper-as-token-1"]);
    expect(issuer.requests[0]?.params.client_id).toBe("tmpr_auditor");
  });

  // Silence here would be the worst outcome: an auditor that quietly borrows the steward's token
  // authenticates fine, sweeps fine, and produces a ledger in which nothing is independently
  // assessed. Throwing is the whole point.
  it("throws rather than borrowing the steward's token when unconfigured", async () => {
    process.env.TEMPER_TOKEN = "steward-dev-token";

    const { auditorFetch } = await import("../agent/lib/temper-auth.js");
    await expect(auditorFetch("http://127.0.0.1:1/nope", { method: "GET" })).rejects.toThrow(
      /TEMPER_AUDITOR_TOKEN/,
    );
  });

  // ── "not configured" vs "configured and failing" ────────────────────────────────────────────
  //
  // These two states must never collapse into each other. The auditor is OPTIONAL and its schedule
  // ships in the repo, so every fork that deploys this agent gets the cron; without a skip they all
  // fail on every tick on a credential they never meant to set, and a permanently-red cron trains
  // people to ignore red crons. But a deployment that DID configure an auditor and got it wrong must fail
  // loudly — silence there means believing you are auditing when you are not.
  //
  // The predicate is the seam. It must never become an excuse to authenticate as someone else:
  // "throws rather than borrowing" above is the invariant this must not weaken.
  it("reports the auditor as unconfigured when neither credential var is set", async () => {
    const { credentialConfigured, AUDITOR_CREDENTIALS } = await import(
      "../agent/lib/temper-auth.js"
    );
    expect(credentialConfigured(AUDITOR_CREDENTIALS)).toBe(false);
  });

  it("does NOT read the steward's credential as the auditor being configured", async () => {
    // The exact shape of the security hole: a fully-configured steward must leave the auditor
    // unconfigured, so the tick skips rather than proceeding under the steward's identity.
    process.env.TEMPER_M2M_CLIENT_ID = "steward-client";
    process.env.TEMPER_M2M_CLIENT_SECRET = "steward-secret";
    process.env.TEMPER_M2M_TOKEN_URL = "https://issuer.example/oauth/token";
    process.env.TEMPER_CONNECT_CONNECTOR = "some-connector";
    process.env.TEMPER_TOKEN = "steward-dev-token";

    const { credentialConfigured, AUDITOR_CREDENTIALS } = await import(
      "../agent/lib/temper-auth.js"
    );
    expect(credentialConfigured(AUDITOR_CREDENTIALS)).toBe(false);
  });

  it("reports a PARTIALLY configured auditor as configured, so it fails loudly instead of skipping", async () => {
    // Client id present, secret and token url absent. This is a misconfiguration by someone who
    // meant to run an auditor — it must NOT be mistaken for absence and silently skipped.
    process.env.TEMPER_AUDITOR_M2M_CLIENT_ID = "tmpr_auditor";

    const { credentialConfigured, AUDITOR_CREDENTIALS, auditorFetch } = await import(
      "../agent/lib/temper-auth.js"
    );
    expect(credentialConfigured(AUDITOR_CREDENTIALS)).toBe(true);
    // Asserts that it throws naming a MISSING AUDITOR var — not which one. Whichever `build`
    // evaluates first is an argument-order detail free to change; that the failure is loud and
    // points at the auditor's own env is the invariant.
    await expect(auditorFetch("http://127.0.0.1:1/nope", { method: "GET" })).rejects.toThrow(
      /TEMPER_AUDITOR_M2M_(CLIENT_SECRET|TOKEN_URL)/,
    );
  });

  it("treats a declared-but-empty credential var as absent", async () => {
    // Vercel surfaces a declared-with-no-value variable as "", not undefined. Reading that as
    // "configured" would send the tick into `build`, which rejects "" too — turning an empty
    // declaration into the every-tick hard failure this guard exists to prevent.
    process.env.TEMPER_AUDITOR_M2M_CLIENT_ID = "";
    process.env.TEMPER_AUDITOR_TOKEN = "";

    const { credentialConfigured, AUDITOR_CREDENTIALS } = await import(
      "../agent/lib/temper-auth.js"
    );
    expect(credentialConfigured(AUDITOR_CREDENTIALS)).toBe(false);
  });

  it("reports configured on either the M2M client id or the dev static token", async () => {
    const mod = "../agent/lib/temper-auth.js";
    process.env.TEMPER_AUDITOR_M2M_CLIENT_ID = "tmpr_auditor";
    let { credentialConfigured, AUDITOR_CREDENTIALS } = await import(mod);
    expect(credentialConfigured(AUDITOR_CREDENTIALS)).toBe(true);

    delete process.env.TEMPER_AUDITOR_M2M_CLIENT_ID;
    process.env.TEMPER_AUDITOR_TOKEN = "dev-token";
    vi.resetModules();
    ({ credentialConfigured, AUDITOR_CREDENTIALS } = await import(mod));
    expect(credentialConfigured(AUDITOR_CREDENTIALS)).toBe(true);
  });

  // THE WIRING. Everything above pins the predicate; this pins that the schedule actually CONSULTS
  // it. Deleting the guard from `run` leaves every predicate test above green — the same gap that
  // let the citation-grain defect live between a correct sweep and a correct queue.
  it("the tick SKIPS entirely when no auditor credential is configured", async () => {
    process.env.TEMPER_API_URL = "https://example.invalid";
    // A fully-configured steward, to prove the skip is not merely "no env at all".
    process.env.TEMPER_TOKEN = "steward-dev-token";

    const schedule = (await import("../agent/schedules/auditor.js")).default;
    const receive = vi.fn();
    const waitUntil = vi.fn();

    await schedule.run?.({ receive, waitUntil, appAuth: {} as never });

    // No background work parked, no session started, and NOT a thrown error.
    expect(waitUntil).not.toHaveBeenCalled();
    expect(receive).not.toHaveBeenCalled();
  });

  it("the tick PROCEEDS when an auditor credential is configured", async () => {
    process.env.TEMPER_API_URL = "http://127.0.0.1:1";
    process.env.TEMPER_AUDITOR_TOKEN = "auditor-dev-token";

    const schedule = (await import("../agent/schedules/auditor.js")).default;
    const receive = vi.fn();
    const waitUntil = vi.fn();

    await schedule.run?.({ receive, waitUntil, appAuth: {} as never });

    // The guard must not block a deployment that DID configure an auditor. Parking the work is the
    // whole assertion — the parked promise then fails against an unreachable host, which is neither
    // awaited nor asserted on (its retry backoff is a network timing detail, not this claim).
    expect(waitUntil).toHaveBeenCalledTimes(1);
    (waitUntil.mock.calls[0]?.[0] as Promise<unknown>).catch(() => {});
  });

  it("still carries the shared re-mint-once-on-401 behavior", async () => {
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_auditor",
      clientSecret: "s3cr3t",
    });
    api = await startMockApi({ rejectFirst: 1 });
    process.env.TEMPER_AUDITOR_M2M_CLIENT_ID = "tmpr_auditor";
    process.env.TEMPER_AUDITOR_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_AUDITOR_M2M_TOKEN_URL = issuer.url;

    const { auditorFetch } = await import("../agent/lib/temper-auth.js");
    const res = await auditorFetch(api.url, { method: "POST", body: "{}" });

    expect(res.status).toBe(200);
    expect(api.bearers).toEqual(["temper-as-token-1", "temper-as-token-2"]);
  });
});

// ── Fitting the auditor inside its allowance ──────────────────────────────────────────────────
//
// Three axes decide whether an optional agent runs on a given tick. They are deliberately
// different questions with deliberately different defaults:
//
//   credential   — absent means SKIP.    Nobody configured an auditor on this deployment.
//   enablement   — absent means RUN.     A merge may not turn a production cron off.
//   capacity     — refused means SKIP.   The issuer will not mint for this credential right now.
//
// The suite above pins the first. These pin the second and third, plus the cadence that is the
// mechanism by which the auditor is made to fit a fixed monthly allowance — not a tuning nicety.
describe("cadence — the auditor's budget mechanism", () => {
  it("runs once a day, not hourly", async () => {
    // Hourly on this corpus is ~24x its own rate of change, and the AI Gateway's allowance is a
    // fixed ceiling rather than something to top up — so cadence is how the auditor is made to fit
    // inside it. The assertion is the SHAPE (a fixed hour), not which hour: the hour is an
    // operator's to move, but a wildcard hour field would restore the spend this exists to bound.
    const schedule = (await import("../agent/schedules/auditor.js")).default;
    const [, hour, dayOfMonth, month, dayOfWeek] = (schedule.cron ?? "").split(" ");

    expect(schedule.cron).toBeDefined();
    expect(hour).not.toBe("*");
    expect(hour).toMatch(/^\d{1,2}$/);
    // ONCE a day means every day: a fixed hour alone also describes `30 3 * * 1` (weekly) and
    // `30 3 1 * *` (monthly). Both would pass a test by this name, and both drift the audit further
    // from its corpus than anything argued for here — so pin the remaining fields open.
    expect([dayOfMonth, month, dayOfWeek]).toEqual(["*", "*", "*"]);
  });

  it("still trails a steward tick rather than colliding with one", async () => {
    // The incumbent's reason for :30 outlives the cadence change: citations authored by a steward
    // tick stay auditable without the two ticks writing concurrently over one map.
    const auditor = (await import("../agent/schedules/auditor.js")).default;
    const steward = (await import("../agent/schedules/steward.js")).default;

    expect((auditor.cron ?? "").split(" ")[0]).not.toBe((steward.cron ?? "").split(" ")[0]);
  });
});

describe("the enable toggle — absence means ENABLED", () => {
  const mod = "../agent/lib/optional-agent.js";

  it("is enabled when the variable is not set at all", async () => {
    // THE polarity assertion, and the inverse of every other env predicate in this package.
    // `credentialConfigured` and `otlpExportConfigured` are both presence checks — absent means off.
    // Writing this one by copying either would turn off every auditor already running in production
    // the moment this merges, and a merge may not turn a production cron off any more than on.
    const { agentEnabled, AUDITOR_ENABLED } = await import(mod);
    expect(agentEnabled(AUDITOR_ENABLED)).toBe(true);
  });

  it("is enabled when the variable is declared but empty", async () => {
    // Vercel surfaces a declared-with-no-value variable as "", not undefined. `credentialConfigured`
    // reads that as ABSENT, and absent means enabled here — so an empty declaration must not be the
    // one keystroke that silently stops a deployment auditing.
    process.env.TEMPER_AUDITOR_ENABLED = "   ";
    const { agentEnabled, AUDITOR_ENABLED } = await import(mod);
    expect(agentEnabled(AUDITOR_ENABLED)).toBe(true);
  });

  it("disables only on an explicit recognized value, whatever its case or padding", async () => {
    const { agentEnabled, AUDITOR_ENABLED } = await import(mod);
    for (const value of ["0", "false", "FALSE", " off ", "no", "No"]) {
      process.env.TEMPER_AUDITOR_ENABLED = value;
      expect(agentEnabled(AUDITOR_ENABLED), `${JSON.stringify(value)} should disable`).toBe(false);
    }
  });

  it("still runs on an explicit affirmative", async () => {
    const { agentEnabled, AUDITOR_ENABLED } = await import(mod);
    for (const value of ["1", "true", "yes", "on"]) {
      process.env.TEMPER_AUDITOR_ENABLED = value;
      expect(agentEnabled(AUDITOR_ENABLED), `${JSON.stringify(value)} should enable`).toBe(true);
    }
  });

  it("fails toward ENABLED on a value it does not recognize, and says so", async () => {
    // A typo is not an operator decision. `TEMPER_AUDITOR_ENABLED=fasle` must keep auditing and
    // complain, because the alternative is a deployment that silently stopped for a reason nobody
    // can see — the same failure the credential guard's loud-on-partial rule exists to prevent.
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    process.env.TEMPER_AUDITOR_ENABLED = "fasle";

    const { agentEnabled, AUDITOR_ENABLED } = await import(mod);
    expect(agentEnabled(AUDITOR_ENABLED)).toBe(true);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0]?.[0]).toMatch(/TEMPER_AUDITOR_ENABLED/);
    warn.mockRestore();
  });

  // THE WIRING, for the same reason the credential axis pins it separately: every predicate test
  // above stays green if the schedule never consults the predicate.
  it("the tick SKIPS when disabled, even with a credential configured", async () => {
    process.env.TEMPER_API_URL = "http://127.0.0.1:1";
    process.env.TEMPER_AUDITOR_TOKEN = "auditor-dev-token";
    process.env.TEMPER_AUDITOR_ENABLED = "false";

    const schedule = (await import("../agent/schedules/auditor.js")).default;
    const receive = vi.fn();
    const waitUntil = vi.fn();

    await schedule.run?.({ receive, waitUntil, appAuth: {} as never });

    // A skip, never a fallback: nothing parked, nothing dispatched, and not a thrown error.
    expect(waitUntil).not.toHaveBeenCalled();
    expect(receive).not.toHaveBeenCalled();
  });
});

describe("capacity — correct credentials are not sufficient credentials", () => {
  const mod = "../agent/lib/optional-agent.js";

  // Auth0 enforces its own monthly quota on M2M token issuance, so the auditor's credentials can be
  // entirely correct and still be rejected. That is a funding ceiling, not a misconfiguration, and
  // it belongs in the same quiet skip as an absent credential.
  //
  // It answers **429**, NOT the AI Gateway's 402. Different vendor, different failure, different
  // status — and matching the gateway's code by analogy would match one Auth0 never sends.
  it("classifies a 429 from the token endpoint as cannot-run-right-now", async () => {
    const { tokenIssuanceUnavailable } = await import(mod);
    const err = Object.assign(new Error("token mint failed (429): quota"), {
      name: "TokenMintError",
      status: 429,
    });
    expect(tokenIssuanceUnavailable(err)).toBe(true);
  });

  it("does NOT classify a 401 that way — a wrong credential must stay loud", async () => {
    // The distinction the whole guard exists to draw. "Cannot afford to run" is quiet; "believes it
    // is auditing and is not" is the far worse failure and must never be silenced.
    const { tokenIssuanceUnavailable } = await import(mod);
    const err = Object.assign(new Error("token mint failed (401): invalid_client"), {
      name: "TokenMintError",
      status: 401,
    });
    expect(tokenIssuanceUnavailable(err)).toBe(false);
  });

  it("does NOT classify a 429 that did not come from the token endpoint", async () => {
    // A 429 from temper's own API is rate limiting, not a funding ceiling, and arrives as a plain
    // Error from the schedule's own `res.ok` check. The structural name check is what separates
    // them; without it this would quietly swallow an unrelated class of failure.
    const { tokenIssuanceUnavailable } = await import(mod);
    expect(tokenIssuanceUnavailable(new Error("auditor dispatch failed: 429 slow down"))).toBe(false);
    expect(tokenIssuanceUnavailable({ status: 429 })).toBe(false);
  });

  it("does NOT classify a missing-env failure — that is a misconfiguration", async () => {
    // A partially-configured auditor throws a plain Error out of `requireEnv`, and the incumbent is
    // explicit that this must be loud: it is a misconfiguration by someone who MEANT to run one.
    const { tokenIssuanceUnavailable } = await import(mod);
    expect(
      tokenIssuanceUnavailable(new Error("TEMPER_AUDITOR_M2M_CLIENT_SECRET is required")),
    ).toBe(false);
  });

  // THE WIRING. As with the other two axes, the predicate tests above all stay green if the
  // schedule never consults it.
  it("the tick ends QUIETLY when the token endpoint answers 429", async () => {
    refusing = await startRefusingIssuer(429, {
      error: "too_many_requests",
      error_description: "Client quota exceeded",
    });
    process.env.TEMPER_API_URL = "https://example.invalid";
    process.env.TEMPER_AUDITOR_M2M_CLIENT_ID = "auditor-client";
    process.env.TEMPER_AUDITOR_M2M_CLIENT_SECRET = "s3cr3t";
    process.env.TEMPER_AUDITOR_M2M_TOKEN_URL = refusing.url;

    const schedule = (await import("../agent/schedules/auditor.js")).default;
    const receive = vi.fn();
    const waitUntil = vi.fn();
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    await schedule.run?.({ receive, waitUntil, appAuth: {} as never });

    // The tick still PARKS — the credential is configured, so the guard above must not stop it — and
    // the parked promise must then SETTLE rather than reject. Resolving is the whole assertion: it
    // is what "degrade quietly" means at this seam.
    expect(waitUntil).toHaveBeenCalledTimes(1);
    await expect(waitUntil.mock.calls[0]?.[0]).resolves.toBeUndefined();
    expect(receive).not.toHaveBeenCalled();
    expect(error).not.toHaveBeenCalled();
    // Quiet is not invisible. A degraded auditor that logged at the same level as a deployment which
    // never had one would be indistinguishable from the resting state in a log stream.
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0]?.[0]).toMatch(/issuance quota/);
    error.mockRestore();
    warn.mockRestore();
  });

  it("the tick still FAILS LOUDLY when the token endpoint rejects the credential", async () => {
    // Same code path, one status apart. The mock issuer is used here for the one thing it is pinned
    // on — the AS's own 401 for a secret that does not verify.
    issuer = await startMockIssuer({
      flavor: "temper-as",
      clientId: "tmpr_auditor",
      clientSecret: "the-real-secret",
    });
    process.env.TEMPER_API_URL = "https://example.invalid";
    process.env.TEMPER_AUDITOR_M2M_CLIENT_ID = "tmpr_auditor";
    process.env.TEMPER_AUDITOR_M2M_CLIENT_SECRET = "the-wrong-secret";
    process.env.TEMPER_AUDITOR_M2M_TOKEN_URL = issuer.url;

    const schedule = (await import("../agent/schedules/auditor.js")).default;
    const receive = vi.fn();
    const waitUntil = vi.fn();
    const error = vi.spyOn(console, "error").mockImplementation(() => {});

    await schedule.run?.({ receive, waitUntil, appAuth: {} as never });

    expect(waitUntil).toHaveBeenCalledTimes(1);
    await expect(waitUntil.mock.calls[0]?.[0]).rejects.toThrow(/401/);
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });
});
