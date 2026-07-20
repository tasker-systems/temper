import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { TEMPER_READ_TOOLS, getTemperToken } from "../agent/lib/mcp-auth.js";
import type { MintOutcome } from "../agent/lib/mint.js";

/**
 * `getTemperToken` is tested through the mint MODULE boundary rather than through `fetch`:
 * `mint.ts` has its own suite for the wire format, and what matters here is which principal
 * string crosses that boundary and how each of the three outcomes is turned into a token or
 * a failure.
 */
const requestMintedToken = vi.hoisted(() => vi.fn<(p: string) => Promise<MintOutcome>>());
vi.mock("../agent/lib/mint.js", () => ({ requestMintedToken }));

describe("getTemperToken", () => {
  /** The 3-segment human shape eve mints for a Slack user in a workspace. */
  const PRINCIPAL = "slack:T012AB3CD:U024BE7LH";

  beforeEach(() => {
    requestMintedToken.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  // FAILS IF: getToken passes anything other than `principal.id` to mint — a parse of it, a
  // `principal.issuer`-prefixed composite, an `attributes.user_id`, or a hardcoded value.
  // The equality "principal.id === the SessionAuthContext principalId that link-state and
  // the mint route are keyed on" is what makes translation unnecessary and any translation
  // wrong; this is the test that notices someone adding one.
  it("passes principal.id to mint VERBATIM, whole and unparsed", async () => {
    requestMintedToken.mockResolvedValue({
      status: "token",
      access_token: "at-1",
      expires_at_ms: 1_784_505_600_000,
    });

    await getTemperToken({
      principal: {
        type: "user",
        id: PRINCIPAL,
        issuer: "slack:T012AB3CD",
        attributes: { user_id: "U024BE7LH" },
      },
    });

    expect(requestMintedToken).toHaveBeenCalledTimes(1);
    expect(requestMintedToken).toHaveBeenCalledWith(PRINCIPAL);
  });

  // FAILS IF: expiresAt is dropped, or rescaled. eve's TokenResult.expiresAt is epoch
  // MILLISECONDS; a `/ 1000` would still "have a value" while claiming the token expired in
  // 1970, so the runtime would re-mint on every single tool call.
  it("returns the token with expiresAt as epoch milliseconds, unscaled", async () => {
    const EXPIRES_AT_MS = 1_784_505_600_000;
    requestMintedToken.mockResolvedValue({
      status: "token",
      access_token: "at-1",
      expires_at_ms: EXPIRES_AT_MS,
    });

    const result = await getTemperToken({ principal: { type: "user", id: PRINCIPAL } });

    expect(result).toEqual({ token: "at-1", expiresAt: EXPIRES_AT_MS });
    expect(new Date(result.expiresAt as number).getUTCFullYear()).toBe(2026);
  });

  // FAILS IF: `not_vaulted` resolves instead of throwing — i.e. getToken FAILS OPEN and the
  // connection calls the MCP server with an empty/undefined bearer. `retryable: false` is
  // asserted because a retryable failure invites eve to re-prompt for an interactive flow
  // that does not exist here.
  it("fails closed and terminally on not_vaulted", async () => {
    requestMintedToken.mockResolvedValue({ status: "not_vaulted" });

    await expect(
      getTemperToken({ principal: { type: "user", id: PRINCIPAL } }),
    ).rejects.toMatchObject({ reason: "not_vaulted", retryable: false });
  });

  // FAILS IF: `revoked` fails open, or is collapsed onto the not_vaulted reason. The reason
  // codes stay distinct because they surface on the stream event and the failed tool result.
  it("fails closed and terminally on revoked, with its own reason code", async () => {
    requestMintedToken.mockResolvedValue({ status: "revoked" });

    await expect(
      getTemperToken({ principal: { type: "user", id: PRINCIPAL } }),
    ).rejects.toMatchObject({ reason: "revoked", retryable: false });
  });

  // FAILS IF: a failure carries the credential. The error message reaches logs and the model
  // as a failed tool result, so it must never quote a token — and on these arms there is no
  // token, so the guard is against a future arm that has one being logged the same way.
  it("never puts an access token in the failure message", async () => {
    requestMintedToken.mockResolvedValue({ status: "revoked" });

    await expect(
      getTemperToken({ principal: { type: "user", id: PRINCIPAL } }),
    ).rejects.not.toThrow(/at-/);
  });
});

describe("TEMPER_READ_TOOLS", () => {
  // FAILS IF: any tool is added to the allow-list. Asserted as an EXACT list, not a
  // "does not contain create_resource" spot-check, because the failure mode is a name nobody
  // thought to spot-check. Writes are out of scope until the read-only-member-can-create
  // authorization bug is fixed, so an addition here must be a deliberate, reviewed edit to
  // BOTH the list and this test.
  it("is exactly the read-only surface", () => {
    expect([...TEMPER_READ_TOOLS]).toEqual([
      "search",
      "get_resource",
      "get_context",
      "list_contexts",
      "list_resources",
      "cogmap_read_charter",
      "describe_doc_type",
      "list_doc_types",
      "get_profile",
    ]);
  });

  // FAILS IF: a mutating tool slips in under a name the exact-list test above was updated to
  // accept without thought. Names the write families from temper-mcp's service.rs directly,
  // so it keeps biting even if someone "fixes" the list assertion by pasting the new value.
  it("contains no tool from a mutating family", () => {
    const MUTATING = [
      "create_",
      "update_",
      "delete_",
      "assert_",
      "retype_",
      "reweight_",
      "fold_",
      "facet_set",
      "annotate_",
      "ingest_",
      "cogmap_bind",
      "cogmap_unbind",
      "cogmap_create",
      "cogmap_grant",
      "cogmap_revoke",
      "resource_grant",
      "resource_revoke",
      "share_context",
      "unshare_context",
      "transfer_context",
      "accept_invitation",
      "decline_invitation",
      "steward_advance_watermark",
      "invocation_open",
      "invocation_close",
    ];

    for (const tool of TEMPER_READ_TOOLS) {
      for (const prefix of MUTATING) {
        expect(tool.startsWith(prefix), `${tool} matches the mutating name ${prefix}`).toBe(
          false,
        );
      }
    }
  });
});

describe("the temper connection definition", () => {
  // FAILS IF: the connection stops using TEMPER_READ_TOOLS (e.g. someone inlines a list that
  // then drifts), stops being user-scoped, or stops routing through getTemperToken. The
  // allow-list tests above are worthless if the connection does not actually use the list.
  // Imported dynamically because the module reads TEMPER_MCP_URL at load.
  it("wires the read-only list, user scoping and the minting getToken", async () => {
    vi.stubEnv("TEMPER_MCP_URL", "https://temper.test/api/mcp");
    const connection = (await import("../agent/connections/temper.js")).default;
    vi.unstubAllEnvs();

    expect(connection.url).toBe("https://temper.test/api/mcp");
    expect(connection.tools).toEqual({ allow: TEMPER_READ_TOOLS });
    expect(connection.auth).toEqual({ principalType: "user", getToken: getTemperToken });
  });
});
