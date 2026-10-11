import { readdirSync, readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
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

  // FAILS IF: `*.localhost` is treated as loopback. On a server runtime the resolver may send it to
  // DNS, so it is not known to stay on the machine.
  it.each(["http://foo.localhost:3000", "http://temper.localhost./api", "http://localhost.:3000"])("refuses %s", (value) => {
    vi.stubEnv("TEMPER_API_URL", value);
    expect(() => requireEndpointEnv("TEMPER_API_URL")).toThrow(/^TEMPER_API_URL is plaintext http/);
  });

  // FAILS IF: the refusal names a remedy this agent does not have (an opt-out it does not expose, a
  // client_secret it does not hold), or echoes a credential written into the URL.
  it.each(["http://temper.example.com", "http://user:tok3n@temper.example.com", "htps://user:tok3n@x.com"])(
    "refuses %s without a false remedy or the credential in the message",
    (value) => {
      vi.stubEnv("TEMPER_API_URL", value);
      let message = "";
      try {
        requireEndpointEnv("TEMPER_API_URL");
      } catch (err) {
        message = (err as Error).message;
      }
      expect(message).toMatch(/^TEMPER_API_URL /);
      expect(message).not.toMatch(/allowInsecureHttp|client_secret|tok3n/);
    },
  );

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

describe("the signed calls never follow a redirect", () => {
  const servers: Server[] = [];

  /** Start a loopback server; resolves to its origin. */
  async function serve(handler: Parameters<typeof createServer>[1]): Promise<string> {
    const server = createServer(handler);
    servers.push(server);
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    return `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  }

  afterEach(async () => {
    await Promise.all(servers.splice(0).map((s) => new Promise((r) => s.close(r))));
  });

  const CALLS = { "link-state": requestLinkState, mint: requestMintedToken } as const;

  // FAILS IF: either call follows a redirect. `requireEndpointEnv` only vets the configured URL;
  // a followed 307 resends the signature headers and body to wherever it points (fetch strips only
  // `Authorization` cross-origin), and hands back that host's response — the mint's is a token.
  it.each(Object.entries(CALLS))("%s", async (_label, call) => {
    const elsewhere: string[] = [];
    const target = await serve((req, res) => {
      elsewhere.push(String(req.headers["x-temper-signature"]));
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ status: "linked", handle: "h" }));
    });
    const origin = await serve((req, res) => {
      res.writeHead(307, { location: `${target}${req.url}` });
      res.end();
    });
    vi.stubEnv("TEMPER_API_URL", origin);

    await expect(call(PRINCIPAL)).rejects.toThrow();
    expect(elsewhere).toEqual([]);
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

/**
 * Every occurrence of either variable name in CODE under `agent/` (line-leading block comments and
 * `//` lines stripped), as `{ file, line, gated }`. Only block comments that START a line are
 * stripped, so a `/*` inside a string (a glob, say) cannot swallow the code after it. Token-level on purpose: a call-shape regex misses
 * `` process.env[`TEMPER_API_URL`] `` and `const { TEMPER_API_URL } = process.env`, and a new read
 * in either shape would then pass. Still blind to a name computed at runtime (`env[name]`).
 */
function urlVarOccurrences(): Array<{ file: string; line: string; gated: boolean }> {
  const out: Array<{ file: string; line: string; gated: boolean }> = [];
  for (const [file, src] of agentSources()) {
    const code = src.replace(/^[ \t]*\/\*[\s\S]*?\*\//gm, "");
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
   * Exact lines allowed to read straight from `process.env`, because neither puts a credential on
   * the URL: telemetry takes the MCP URL as a span-matching hint, and the citations instruction uses
   * the API URL only to build links the model shows the user. Exact lines, not whole files, so a
   * credential use added to either file is still caught.
   */
  const NO_CREDENTIAL = new Set([
    "instrumentation/otlp.ts: spanProcessors: otlpSpanProcessors({ mcpEndpoint: process.env.TEMPER_MCP_URL }),",
    "instructions/citations.ts: markdown: citationLinkInstruction(process.env.TEMPER_API_URL),",
  ]);
  const GATED_FILES = ["connections/temper.ts", "lib/link.ts", "lib/mint.ts"];

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
