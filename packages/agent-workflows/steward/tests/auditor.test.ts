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

const AUDITOR_ENV_NAMES = [
  "TEMPER_AUDITOR_M2M_CLIENT_ID",
  "TEMPER_AUDITOR_M2M_CLIENT_SECRET",
  "TEMPER_AUDITOR_M2M_TOKEN_URL",
  "TEMPER_AUDITOR_M2M_AUDIENCE",
  "TEMPER_AUDITOR_TOKEN",
];
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
  for (const name of [...AUDITOR_ENV_NAMES, ...STEWARD_ENV_NAMES]) {
    delete process.env[name];
  }
});

afterEach(async () => {
  await issuer?.close();
  await api?.close();
  issuer = undefined;
  api = undefined;
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
  // be the only thing between the two roles.
  it("the auditor holds none of the authored-4", () => {
    for (const authored of [
      "create_resource",
      "assert_relationship",
      "facet_set",
      "fold_relationship",
    ]) {
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
