import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * `TEMPER_MCP_URL` and `TEMPER_API_URL` are env-chosen and carry a bearer on every use, so a
 * plaintext value off loopback would put the steward's (or the auditor's) credential in the clear.
 * `requireEndpointEnv` is the one gate; these tests pin the gate, its wiring, and that no read of
 * either variable goes around it.
 */

const AGENT_DIR = fileURLToPath(new URL("../agent/", import.meta.url));
const URL_VARS = ["TEMPER_API_URL", "TEMPER_MCP_URL"] as const;

beforeEach(() => {
  vi.resetModules();
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("requireEndpointEnv", () => {
  // FAILS IF: plaintext http to a non-loopback host is accepted, or the refusal stops naming the
  // variable the operator has to fix.
  it.each(URL_VARS)("refuses non-loopback http in %s, naming the variable", async (name) => {
    vi.stubEnv(name, "http://temper.example.com/api");
    const { requireEndpointEnv } = await import("../agent/lib/temper-auth.js");

    expect(() => requireEndpointEnv(name)).toThrow(new RegExp(`^${name} is plaintext http`));
  });

  // FAILS IF: the gate over-reaches and breaks the local-development case, or https.
  it.each([
    "http://127.0.0.1:8080",
    "http://localhost:3000/api/mcp",
    "https://temperkb.io/",
    "https://temperkb.io/api/mcp",
  ])("passes %s through unchanged", async (value) => {
    vi.stubEnv("TEMPER_API_URL", value);
    const { requireEndpointEnv } = await import("../agent/lib/temper-auth.js");

    expect(requireEndpointEnv("TEMPER_API_URL")).toBe(value);
  });

  // Absence keeps requireEnv's message: an unset variable is not a scheme problem.
  it("still reports an unset variable as missing", async () => {
    vi.stubEnv("TEMPER_API_URL", "");
    const { requireEndpointEnv } = await import("../agent/lib/temper-auth.js");

    expect(() => requireEndpointEnv("TEMPER_API_URL")).toThrow(/TEMPER_API_URL is required/);
  });
});

describe("the MCP connections refuse a plaintext TEMPER_MCP_URL at load", () => {
  const CONNECTIONS = {
    steward: "../agent/connections/temper.js",
    auditor: "../agent/subagents/auditor/connections/temper.js",
  } as const;

  // FAILS IF: either connection reads its URL without the gate. Each connection carries a
  // different principal's bearer, so each is witnessed on its own.
  it.each(Object.entries(CONNECTIONS))("%s connection", async (_label, path) => {
    vi.stubEnv("TEMPER_MCP_URL", "http://temper.example.com/api/mcp");
    vi.stubEnv("TEMPER_TOKEN", "steward-dev-token");
    vi.stubEnv("TEMPER_AUDITOR_TOKEN", "auditor-dev-token");

    await expect(import(path)).rejects.toThrow(/^TEMPER_MCP_URL is plaintext http/);
  });

  it.each(Object.entries(CONNECTIONS))("%s connection still loads on loopback http", async (_label, path) => {
    vi.stubEnv("TEMPER_MCP_URL", "http://127.0.0.1:3000/api/mcp");
    vi.stubEnv("TEMPER_TOKEN", "steward-dev-token");
    vi.stubEnv("TEMPER_AUDITOR_TOKEN", "auditor-dev-token");

    const connection = (await import(path)).default as { url: string };
    expect(connection.url).toBe("http://127.0.0.1:3000/api/mcp");
  });
});

describe("the auditor's REST tools refuse a plaintext TEMPER_API_URL before any request", () => {
  // FAILS IF: a tool builds its request URL without the gate. The credential would ride
  // `auditorFetch`, so the refusal must come before it — no fetch at all.
  it.each([
    "../agent/subagents/auditor/tools/element_trail.js",
    "../agent/subagents/auditor/tools/complete_audit_job.js",
  ])("%s", async (path) => {
    vi.stubEnv("TEMPER_API_URL", "http://temper.example.com");
    vi.stubEnv("TEMPER_AUDITOR_TOKEN", "auditor-dev-token");
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const tool = (await import(path)).default as { execute: (...args: unknown[]) => Promise<unknown> };
    await expect(
      tool.execute({ kind: "node", id: "01a10df8-ecb5-7b23-9cf3-96d7c57c0ba1", job_id: "j", verdicts: [] }, {}),
    ).rejects.toThrow(/^TEMPER_API_URL is plaintext http/);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

/** Every `.ts` file under `agent/`, as `[path relative to agent/, source]`. */
function agentSources(): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.name.endsWith(".ts")) out.push([relative(AGENT_DIR, full), readFileSync(full, "utf8")]);
    }
  };
  walk(AGENT_DIR);
  return out;
}

/**
 * Every occurrence of either variable name in CODE under `agent/` (block comments and `//` lines
 * stripped), as `{ file, line, gated }`. Token-level on purpose: a call-shape regex misses
 * `` process.env[`TEMPER_API_URL`] `` and `const { TEMPER_API_URL } = process.env`, and a new read
 * in either shape would then pass. Still blind to a name computed at runtime (`env[name]`).
 */
function urlVarOccurrences(): Array<{ file: string; line: string; gated: boolean }> {
  const out: Array<{ file: string; line: string; gated: boolean }> = [];
  for (const [file, src] of agentSources()) {
    const code = src.replace(/\/\*[\s\S]*?\*\//g, "");
    for (const raw of code.split("\n")) {
      const line = raw.trim();
      if (line.startsWith("//")) continue;
      for (const m of line.matchAll(/\bTEMPER_(?:API|MCP)_URL\b/g)) {
        const gated = /requireEndpointEnv\(\s*["']$/.test(line.slice(0, m.index));
        out.push({ file, line, gated });
      }
    }
  }
  return out;
}

describe("no read of TEMPER_API_URL / TEMPER_MCP_URL bypasses the gate", () => {
  /**
   * The exact line allowed to read straight from `process.env`: telemetry takes the MCP URL as a
   * span-matching hint and never puts a credential on it. An exact line, not the whole file, so a
   * credential use added there is still caught.
   */
  const NO_CREDENTIAL = new Set(["instrumentation.ts: mcpEndpoint: process.env.TEMPER_MCP_URL,"]);
  const GATED_FILES = [
    "connections/temper.ts",
    "schedules/auditor.ts",
    "schedules/materialize.ts",
    "schedules/steward.ts",
    "subagents/auditor/connections/temper.ts",
    "subagents/auditor/tools/complete_audit_job.ts",
    "subagents/auditor/tools/element_trail.ts",
  ];

  const occurrences = urlVarOccurrences();
  const bypasses = occurrences
    .filter((o) => !o.gated && !NO_CREDENTIAL.has(`${o.file}: ${o.line}`))
    .map((o) => `${o.file}: ${o.line}`);

  // FAILS IF: any occurrence of either name in code is not a requireEndpointEnv argument, outside
  // the exact exempt lines — including a read added later, which the behavioural tests above
  // cannot know about.
  it("every read goes through requireEndpointEnv", () => {
    expect(bypasses).toEqual([]);
  });

  // Guards the scan itself: if it stops seeing the real call shape, the assertion above passes on
  // an empty list. Also pins that each exemption still matches a real line, so a stale one cannot
  // linger as a blanket allowance.
  it("the scan sees every known read", () => {
    expect(occurrences.filter((o) => o.gated).map((o) => o.file).sort()).toEqual(GATED_FILES);
    const exempt = occurrences.filter((o) => NO_CREDENTIAL.has(`${o.file}: ${o.line}`));
    expect(exempt.map((o) => `${o.file}: ${o.line}`).sort()).toEqual([...NO_CREDENTIAL].sort());
  });
});
