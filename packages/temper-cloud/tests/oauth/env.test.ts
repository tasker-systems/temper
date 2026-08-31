import { afterEach, describe, expect, it } from "vitest";
import { servedAudiences } from "../../src/oauth/env.js";

/**
 * The served audience set is the ONE fact three readers must agree on — the authorize handler's
 * fail-closed check, the PRM's advertisement, and the refresh chain's write-time validation —
 * so its shape is pinned here: what is served, what the unset fallback collapses to, and that
 * an empty MCP_AUDIENCE means "absent", never a literal empty audience.
 */

describe("servedAudiences", () => {
  const original = { as: process.env.AS_AUDIENCE, mcp: process.env.MCP_AUDIENCE };

  afterEach(() => {
    if (original.as === undefined) delete process.env.AS_AUDIENCE;
    else process.env.AS_AUDIENCE = original.as;
    if (original.mcp === undefined) delete process.env.MCP_AUDIENCE;
    else process.env.MCP_AUDIENCE = original.mcp;
  });

  it("serves the instance audience and the MCP resource when both are configured", () => {
    process.env.AS_AUDIENCE = "https://inst.test/api";
    process.env.MCP_AUDIENCE = "https://inst.test/mcp";
    expect(servedAudiences()).toEqual(["https://inst.test/api", "https://inst.test/mcp"]);
  });

  it("collapses to the one instance audience when MCP_AUDIENCE is unset", () => {
    process.env.AS_AUDIENCE = "https://inst.test/api";
    delete process.env.MCP_AUDIENCE;
    expect(servedAudiences()).toEqual(["https://inst.test/api"]);
  });

  it("treats an empty MCP_AUDIENCE as absent, never as a literal empty audience", () => {
    process.env.AS_AUDIENCE = "https://inst.test/api";
    process.env.MCP_AUDIENCE = "  ";
    expect(servedAudiences()).toEqual(["https://inst.test/api"]);
  });

  it("names the missing variable when AS_AUDIENCE is unset rather than serving nothing", () => {
    delete process.env.AS_AUDIENCE;
    delete process.env.MCP_AUDIENCE;
    expect(() => servedAudiences()).toThrow(/AS_AUDIENCE/);
  });
});
