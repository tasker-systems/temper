import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { requestLinkState, requireEndpointEnv } from "../agent/lib/link.js";
import { requestMintedToken } from "../agent/lib/mint.js";

/**
 * `TEMPER_API_URL` and `TEMPER_MCP_URL` are env-chosen and carry credentials: the signed link-state
 * and mint calls (the mint's response is a human's access token) and the connection that carries
 * that token. `requireEndpointEnv` is the one gate; these tests pin the gate, its wiring at each
 * site, and that no read of either variable goes around it.
 */

const AGENT_DIR = fileURLToPath(new URL("../agent/", import.meta.url));
const PRINCIPAL = "slack:T012AB3CD:U024BE7LH";

beforeEach(() => {
  vi.stubEnv("SLACK_LINK_SECRET", "link-s3cret");
  vi.stubEnv("SLACK_MINT_SECRET", "mint-s3cret");
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("requireEndpointEnv", () => {
  // FAILS IF: plaintext http to a non-loopback host is accepted, or the refusal stops naming the
  // variable the operator has to fix.
  it.each(["TEMPER_API_URL", "TEMPER_MCP_URL"])("refuses non-loopback http in %s, naming it", (name) => {
    vi.stubEnv(name, "http://temper.example.com/api");
    expect(() => requireEndpointEnv(name)).toThrow(new RegExp(`^${name} is plaintext http`));
  });

  // FAILS IF: the gate over-reaches and breaks the local-development case, or https.
  it.each(["http://127.0.0.1:8080", "http://localhost:3000/", "https://temperkb.io/api/mcp"])(
    "passes %s through unchanged",
    (value) => {
      vi.stubEnv("TEMPER_API_URL", value);
      expect(requireEndpointEnv("TEMPER_API_URL")).toBe(value);
    },
  );
});

describe("the signed calls refuse a plaintext TEMPER_API_URL before sending", () => {
  const CALLS = { "link-state": requestLinkState, mint: requestMintedToken } as const;

  // FAILS IF: either call builds its request without the gate. The refusal must come before the
  // fetch — the request carries an HMAC signature, and the mint's response is a human's token.
  it.each(Object.entries(CALLS))("%s", async (_label, call) => {
    vi.stubEnv("TEMPER_API_URL", "http://temper.example.com");
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    await expect(call(PRINCIPAL)).rejects.toThrow(/^TEMPER_API_URL is plaintext http/);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  // FAILS IF: the gate breaks local development against a temper on this machine.
  it.each(Object.entries(CALLS))("%s still sends to loopback http", async (_label, call) => {
    vi.stubEnv("TEMPER_API_URL", "http://127.0.0.1:8080/");
    const fetchMock = vi.fn(
      async () => new Response(JSON.stringify({ status: "linked", handle: "h" }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await call(PRINCIPAL).catch(() => {}); // the mint rejects this body's shape; only the send matters
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(String((fetchMock.mock.calls[0] as unknown[])[0])).toMatch(/^http:\/\/127\.0\.0\.1:8080\/internal\//);
  });
});

describe("the temper connection refuses a plaintext TEMPER_MCP_URL at load", () => {
  beforeEach(() => {
    vi.resetModules();
  });

  // FAILS IF: the connection reads its URL without the gate. It carries the mentioning human's
  // minted token, so this is the most privileged of the three sites.
  it("refuses non-loopback http", async () => {
    vi.stubEnv("TEMPER_MCP_URL", "http://temper.example.com/api/mcp");
    await expect(import("../agent/connections/temper.js")).rejects.toThrow(/^TEMPER_MCP_URL is plaintext http/);
  });

  it("still loads on loopback http", async () => {
    vi.stubEnv("TEMPER_MCP_URL", "http://127.0.0.1:3000/api/mcp");
    const connection = (await import("../agent/connections/temper.js")).default;
    expect(connection.url).toBe("http://127.0.0.1:3000/api/mcp");
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

describe("no read of TEMPER_API_URL / TEMPER_MCP_URL bypasses the gate", () => {
  /**
   * Readers allowed to go straight to `process.env`, because neither puts a credential on the URL:
   * telemetry takes the MCP URL as a span-matching hint, and the citations instruction uses the API
   * URL only to build links the model shows the user.
   */
  const NO_CREDENTIAL = new Set(["instrumentation.ts", "instructions/citations.ts"]);

  const gated: string[] = [];
  const bypasses: string[] = [];
  for (const [file, src] of agentSources()) {
    for (const m of src.matchAll(/(\w+)\(\s*["'](TEMPER_(?:API|MCP)_URL)["']/g)) {
      (m[1] === "requireEndpointEnv" ? gated : bypasses).push(`${file}: ${m[1]}("${m[2]}")`);
    }
    for (const m of src.matchAll(/process\.env(?:\.|\[\s*["'])(TEMPER_(?:API|MCP)_URL)/g)) {
      if (!NO_CREDENTIAL.has(file)) bypasses.push(`${file}: process.env.${m[1]}`);
    }
  }

  // FAILS IF: any call site reads either variable through anything but the gate — including one
  // added later, which the behavioural tests above cannot know about.
  it("every read goes through requireEndpointEnv", () => {
    expect(bypasses).toEqual([]);
  });

  // Guards the scan itself: if the pattern stops matching the real call shape, the assertion above
  // passes on an empty list.
  it("the scan sees every known credential-carrying read", () => {
    expect(gated.map((g) => g.split(":")[0]).sort()).toEqual([
      "connections/temper.ts",
      "lib/link.ts",
      "lib/mint.ts",
    ]);
  });
});
