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
    requestMintedToken.mockResolvedValue({ status: "refused", reason: "not_vaulted" });

    await expect(
      getTemperToken({ principal: { type: "user", id: PRINCIPAL } }),
    ).rejects.toMatchObject({ reason: "not_vaulted", retryable: false });
  });

  // FAILS IF: a standing refusal fails open, or is collapsed onto the not_vaulted reason.
  // The reason codes stay distinct because they surface on the stream event and the failed
  // tool result — and because "re-link" and "an admin must approve you" are different
  // instructions, which is the whole point of the typed refusal.
  it("fails closed and terminally on a standing refusal, carrying the KIND in the reason", async () => {
    requestMintedToken.mockResolvedValue({
      status: "refused",
      reason: "standing",
      refusal: { kind: "denied" },
    });

    await expect(
      getTemperToken({ principal: { type: "user", id: PRINCIPAL } }),
    ).rejects.toMatchObject({ reason: "standing:denied", retryable: false });
  });

  // FAILS IF: the two standing kinds collapse to one reason code. A deactivated principal and
  // a denied one are the same remedy for the USER (ask an admin) but different facts for
  // whoever reads the log, and this is the only place that distinction survives.
  it("keeps standing kinds distinct in the reason code", async () => {
    requestMintedToken.mockResolvedValue({
      status: "refused",
      reason: "standing",
      refusal: { kind: "deactivated" },
    });

    await expect(
      getTemperToken({ principal: { type: "user", id: PRINCIPAL } }),
    ).rejects.toMatchObject({ reason: "standing:deactivated", retryable: false });
  });

  // FAILS IF: `not_linked` is treated as retryable, or as an error rather than a refusal. It
  // is reachable here as a race — the link was removed between the pre-flight and this call.
  it("fails closed and terminally on not_linked", async () => {
    requestMintedToken.mockResolvedValue({ status: "refused", reason: "not_linked" });

    await expect(
      getTemperToken({ principal: { type: "user", id: PRINCIPAL } }),
    ).rejects.toMatchObject({ reason: "not_linked", retryable: false });
  });

  // FAILS IF: the minted token escapes anywhere other than the returned `TokenResult`.
  //
  // The `token` arm is the ONLY arm where a credential is in scope, so it is the only arm
  // where this property can be violated — and therefore the only arm worth mocking. Adding a
  // `console.log(outcome)` for debugging, or widening any thrown message to interpolate the
  // outcome, makes this go red.
  it("lets the access token reach the return value and NOTHING else", async () => {
    const SECRET = "at-live-do-not-log-9f3c2b";
    const spies = (["log", "info", "warn", "error", "debug"] as const).map((level) =>
      vi.spyOn(console, level).mockImplementation(() => {}),
    );
    requestMintedToken.mockResolvedValue({
      status: "token",
      access_token: SECRET,
      expires_at_ms: 1_784_505_600_000,
    });

    const result = await getTemperToken({ principal: { type: "user", id: PRINCIPAL } });

    // The token DID come back — without this the assertions below would pass vacuously on a
    // function that simply never produced a token at all.
    expect(result.token).toBe(SECRET);

    for (const spy of spies) {
      for (const call of spy.mock.calls) {
        expect(JSON.stringify(call) ?? "").not.toContain(SECRET);
      }
    }
  });

  // FAILS IF: a thrown failure interpolates the mint outcome. Error messages reach logs and
  // the model as a failed tool result. The mock carries a token on an arm that must throw,
  // which is exactly the shape a future "token but unusable" variant would have.
  it("never puts an access token in a failure message", async () => {
    const SECRET = "at-live-do-not-throw-4e1a";
    requestMintedToken.mockResolvedValue({
      status: "unrecognized_future_status",
      access_token: SECRET,
    } as unknown as MintOutcome);

    await expect(getTemperToken({ principal: { type: "user", id: PRINCIPAL } })).rejects.toThrow(
      expect.objectContaining({ message: expect.not.stringContaining(SECRET) }),
    );
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
  //
  // The list is split in two because PREFIX MATCHING IS NOT ENOUGH, in both directions:
  //
  //   - It under-matches when the mutating verb is not at the FRONT of the name. `facet_set` as a
  //     prefix cannot reach `edge_facet_set`, which is the same act aimed at an edge. That is how
  //     four write tools sat uncovered here; see MUTATING_EXACT.
  //   - It over-matches when a write's name is a prefix of a read's. `cogmap_materialize` is a
  //     write ("Requires cogmap-write"); `cogmap_materialize_delta` READS the delta it acts on. A
  //     prefix entry for the first would condemn the second and block a legitimate future grant.
  //
  // So: families whose verb leads go in MUTATING_PREFIXES; everything else is matched whole.
  it("contains no tool from a mutating family", () => {
    const MUTATING_PREFIXES = [
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
      "rename_",
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

    // Whole-name matches: writes no prefix above can reach. Each was verified against its
    // `#[tool(description = ...)]` in crates/temper-mcp/src/service.rs.
    const MUTATING_EXACT = [
      "edge_facet_set", // `facet_set` on an edge — the `facet_set` prefix cannot see it
      "record_citation_audit", // appends a signed audit event
      "cogmap_materialize", // re-materializes regions; "Requires cogmap-write"
      "context_materialize", // re-forms a context's regions
    ];

    for (const tool of TEMPER_READ_TOOLS) {
      for (const prefix of MUTATING_PREFIXES) {
        expect(tool.startsWith(prefix), `${tool} matches the mutating name ${prefix}`).toBe(
          false,
        );
      }
      for (const name of MUTATING_EXACT) {
        expect(tool === name, `${tool} is the mutating tool ${name}`).toBe(false);
      }
    }
  });

  // FAILS IF: nothing, today — this is a NOTE, kept as an executable one so it cannot rot into a
  // comment nobody rereads. `ingest_blocks` is a READ ("Read back the segments that have landed
  // for an in-progress segmented ingest"), but the `ingest_` family prefix condemns it. That
  // over-match is deliberate and currently free, because `ingest_blocks` is out of the allow-list
  // for uncertainty anyway. What clears it, if it is ever wanted: move it to a whole-name
  // exemption, the same way MUTATING_EXACT resolves the cogmap_materialize/_delta pair. Recorded
  // so the next person hits a stated decision rather than an apparent bug.
  it("over-matches ingest_blocks, a read, and that is the known and bounded cost", () => {
    expect("ingest_blocks".startsWith("ingest_")).toBe(true);
    expect([...TEMPER_READ_TOOLS]).not.toContain("ingest_blocks");
  });

  // DECLARED REMAINDER — deliberately a comment and NOT a test. Both lists above are
  // hand-maintained, so they bite on a careless ADDITION TO THE ALLOW-LIST and are blind to a NEW
  // WRITE TOOL added to temper-mcp. That blindness is exactly how `rename_context` came to be
  // unnamed here: PR #594 added the tool, and nothing in this package had any reason to notice.
  //
  // A census assertion — "every tool in service.rs is classified read or write" — WOULD bite on
  // addition, and unlike the analogous Rust case it is even reachable: all 62 tools live in one
  // file and a vitest node environment can read it. It is deliberately not done. It would put a
  // 62-name census of the whole MCP surface into a workspace-isolated package that uses 9 of them,
  // and charge every unrelated tool addition a maintenance visit here. The narrower guard is the
  // exact-list assertion above, which already forces deliberation on the only edit that can
  // actually widen this agent's reach.
  //
  // It is a comment rather than an `it(...)` on purpose: a tautological test would put a passing
  // row in the report for something nothing checks, which is a remainder reading as coverage —
  // the failure this file's whole style exists to avoid.
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
