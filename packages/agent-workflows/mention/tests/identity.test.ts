import { describe, expect, it } from "vitest";

import {
  decideIdentity,
  isHumanPrincipal,
  unlinkedPrompt,
  type PrincipalLike,
} from "../agent/lib/identity.js";

/**
 * Principal fixtures mirroring the four shapes eve's Slack channel actually
 * mints (verified against `eve/dist/src/public/channels/slack/auth.js` at
 * 0.18.1). The whole point of these tests is that the varying segment count
 * never reaches our logic as a parse.
 */
const HUMAN_WITH_TEAM: PrincipalLike = {
  principalId: "slack:T012AB3CD:U024BE7LH",
  principalType: "user",
};
const HUMAN_NO_TEAM: PrincipalLike = {
  principalId: "slack:U024BE7LH",
  principalType: "user",
};
const BOT_WITH_TEAM: PrincipalLike = {
  principalId: "slack:T012AB3CD:bot:B024BE7LH",
  principalType: "service",
};
const BOT_NO_TEAM: PrincipalLike = {
  principalId: "slack:bot:B024BE7LH",
  principalType: "service",
};

describe("decideIdentity", () => {
  it("accepts a 3-segment human principal (teamId present)", () => {
    expect(decideIdentity(HUMAN_WITH_TEAM)).toEqual({
      kind: "human",
      principalId: "slack:T012AB3CD:U024BE7LH",
    });
  });

  it("accepts a 2-segment human principal (teamId absent)", () => {
    // The shape that breaks any `split(":")[2]` parse — it has no index 2.
    expect(decideIdentity(HUMAN_NO_TEAM)).toEqual({
      kind: "human",
      principalId: "slack:U024BE7LH",
    });
  });

  it("rejects a bot principal (principalType 'service')", () => {
    expect(decideIdentity(BOT_WITH_TEAM)).toEqual({
      kind: "rejected",
      reason: "not-human",
    });
  });

  it("rejects a bot principal even without a teamId", () => {
    expect(decideIdentity(BOT_NO_TEAM)).toEqual({
      kind: "rejected",
      reason: "not-human",
    });
  });

  it("rejects a null auth (message carried no author)", () => {
    expect(decideIdentity(null)).toEqual({ kind: "rejected", reason: "no-auth" });
  });

  it("rejects an unrecognized principalType rather than admitting it", () => {
    // Fail-closed: the gate is `=== "user"`, not `!== "service"`, so a
    // principalType eve adds later is refused until we consider it.
    const decision = decideIdentity({
      principalId: "slack:T012AB3CD:X999",
      principalType: "workflow",
    });
    expect(decision).toEqual({ kind: "rejected", reason: "not-human" });
  });
});

describe("principalId is opaque", () => {
  it.each([
    ["3-segment human", HUMAN_WITH_TEAM],
    ["2-segment human", HUMAN_NO_TEAM],
  ])("passes a %s principalId through WHOLE and unparsed", (_label, auth) => {
    const decision = decideIdentity(auth);

    if (decision.kind !== "human") throw new Error("expected a human decision");
    // Identical string, not a reconstruction: no segment was dropped, reordered,
    // or re-joined on the way through.
    expect(decision.principalId).toBe(auth.principalId);
  });

  it("does not mistake a 2-segment id's user for a team id", () => {
    // A `slack:<team>:<user>` parse applied to `slack:<user>` would key this
    // human by "U024BE7LH" as if it were a team. Whole-string identity means
    // the two shapes can never collide with each other.
    const withTeam = decideIdentity(HUMAN_WITH_TEAM);
    const noTeam = decideIdentity(HUMAN_NO_TEAM);

    if (withTeam.kind !== "human" || noTeam.kind !== "human") {
      throw new Error("expected human decisions");
    }
    expect(withTeam.principalId).not.toBe(noTeam.principalId);
  });

  it("preserves an unfamiliar principalId shape verbatim", () => {
    // Robustness against eve minting a shape we have not seen: we store it, we
    // do not interpret it.
    const exotic = "slack:T1:enterprise:E9:U5";
    const decision = decideIdentity({
      principalId: exotic,
      principalType: "user",
    });

    if (decision.kind !== "human") throw new Error("expected a human decision");
    expect(decision.principalId).toBe(exotic);
  });
});

describe("isHumanPrincipal", () => {
  it("is true only for principalType 'user'", () => {
    expect(isHumanPrincipal(HUMAN_WITH_TEAM)).toBe(true);
    expect(isHumanPrincipal(HUMAN_NO_TEAM)).toBe(true);
    expect(isHumanPrincipal(BOT_WITH_TEAM)).toBe(false);
    expect(isHumanPrincipal(BOT_NO_TEAM)).toBe(false);
  });
});

describe("unlinkedPrompt", () => {
  it("echoes the principalId whole", () => {
    expect(unlinkedPrompt("slack:T012AB3CD:U024BE7LH")).toContain(
      "slack:T012AB3CD:U024BE7LH",
    );
  });

  it("echoes a teamless principalId whole", () => {
    expect(unlinkedPrompt("slack:U024BE7LH")).toContain("slack:U024BE7LH");
  });
});
