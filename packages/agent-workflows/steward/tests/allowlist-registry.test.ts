import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { AUDITOR_TOOLS, STEWARD_TOOLS } from "../agent/lib/tool-allowlists.js";

/**
 * The registry witness: every tool name the steward/auditor surface uses must be a name the
 * temper-mcp server actually registers.
 *
 * The names come from parsing the server SOURCE — `crates/temper-mcp/src/service.rs`, the same
 * `#[tool_router]`..`#[tool_handler]` window the server's own guards read — never from a copy of
 * the lists under test. A fixture copied from an allow-list would witness nothing; this parser
 * reds the moment the server renames, merges, or drops a tool any of these names rely on, so a
 * consolidation can no longer strand an entry silently. An empty or malformed parse throws (it
 * can never pass vacuously), and the parse window itself is asserted below so a silent no-op is
 * not an option either.
 *
 * Field of view, stated: the two allow-lists, and `temper__`-prefixed tool mentions in the
 * agents' prompt docs — the prefix is the docs' convention for a literal tool call. Prose that
 * references a verb without the prefix ("fold the node") is out of view, as are the auditors'
 * eve-native REST tools under `subagents/auditor/tools/`, which are typed against their own
 * endpoint and never pass through these lists.
 */

const SERVICE_RS = join(import.meta.dirname, "../../../../crates/temper-mcp/src/service.rs");

function registryToolNames(): string[] {
  const source = readFileSync(SERVICE_RS, "utf8");
  // Line-anchored on purpose: service.rs's module doc and test comments mention both attribute
  // names in prose, so a bare `indexOf` can latch onto a sentence. The real attributes each sit
  // alone on their line inside the impl block; prose never does.
  const start = source.match(/^[ \t]*#\[tool_router\][ \t]*$/m)?.index;
  const end = source.match(/^[ \t]*#\[tool_handler\][ \t]*$/m)?.index;
  if (start === undefined || end === undefined || end < start) {
    throw new Error(
      "no #[tool_router]..#[tool_handler] impl window found in crates/temper-mcp/src/service.rs — " +
        "the registry parser and the server source have drifted; re-derive the parse before trusting any of these assertions",
    );
  }
  const names = source
    .slice(start, end)
    .split("#[tool(")
    .slice(1)
    .map((segment) => {
      const name = segment.match(/async fn (\w+)\(/)?.[1];
      if (!name) {
        throw new Error(
          "a #[tool( attribute in crates/temper-mcp/src/service.rs carries no following " +
            "`async fn name(` — the parser's shape assumption no longer holds",
        );
      }
      return name;
    });
  if (names.length === 0) {
    throw new Error(
      "the #[tool_router]..#[tool_handler] window in crates/temper-mcp/src/service.rs split into zero #[tool( segments — the parser and the server source have drifted",
    );
  }
  return names;
}

describe("every allow-list entry names a registered tool", () => {
  const registry = registryToolNames();

  // The parse anchor: a stable tool the registry has always carried. The membership assertions
  // below already red on an empty parse — this names the actual failure instead of a row of
  // confusing "not registered" entries pointing the wrong way.
  it("the parser read the registry (search is among the parsed names)", () => {
    expect(registry).toContain("search");
  });

  it("STEWARD_TOOLS resolves against the server registry", () => {
    for (const tool of STEWARD_TOOLS) {
      expect(
        registry,
        `"${tool}" is not registered by temper-mcp; the steward's eve filter matches exact names only, so this entry grants nothing`,
      ).toContain(tool);
    }
  });

  it("AUDITOR_TOOLS resolves against the server registry", () => {
    for (const tool of AUDITOR_TOOLS) {
      expect(
        registry,
        `"${tool}" is not registered by temper-mcp; the auditor's eve filter matches exact names only, so this entry grants nothing`,
      ).toContain(tool);
    }
  });
});

const PROMPT_DOCS = [
  "../agent/instructions.md",
  "../agent/skills/map-stewardship.md",
  "../agent/subagents/auditor/instructions.md",
] as const;

describe("the prompt docs call only tools that resolve", () => {
  const registry = registryToolNames();

  for (const doc of PROMPT_DOCS) {
    it(`${doc} — every temper__-prefixed name is a registered tool`, () => {
      const text = readFileSync(join(import.meta.dirname, doc), "utf8");
      const names = [...text.matchAll(/temper__([a-z_]+)/g)].map((match) => match[1]);

      // A doc whose tool mentions all vanished would make the loop above vacuous — assert the
      // scan saw the calls this doc is known to make, so the field of view cannot silently shrink.
      expect(
        names.length,
        `no temper__-prefixed tool calls found in ${doc} — the scanner's shape assumption no longer holds`,
      ).toBeGreaterThan(0);

      for (const name of names) {
        expect(
          registry,
          `"temper__${name}" in ${doc} does not resolve — the model will call it and the call will fail`,
        ).toContain(name);
      }
    });
  }
});
