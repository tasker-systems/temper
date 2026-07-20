import { describe, expect, it } from "vitest";

import {
  decideIdentity,
  isHumanPrincipal,
  notVaultedPrompt,
  revokedPrompt,
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
      auth: HUMAN_WITH_TEAM,
    });
  });

  it("accepts a 2-segment human principal (teamId absent)", () => {
    // The shape that breaks any `split(":")[2]` parse — it has no index 2.
    expect(decideIdentity(HUMAN_NO_TEAM)).toEqual({
      kind: "human",
      principalId: "slack:U024BE7LH",
      auth: HUMAN_NO_TEAM,
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
  it("carries the authorize URL", () => {
    expect(unlinkedPrompt("https://temperkb.io/authorize/abc123")).toContain(
      "https://temperkb.io/authorize/abc123",
    );
  });
});

describe("the broken-credential prompts", () => {
  it("each names the handle and offers the remedy", () => {
    // FAILS IF: either reply becomes a bare error with nothing the user can act on. The
    // handle is what makes it clear WHICH account is affected, and the remedy is the only
    // reason a user reads past the first line.
    for (const message of [notVaultedPrompt("j-cole-taylor"), revokedPrompt("j-cole-taylor")]) {
      expect(message).toContain("@j-cole-taylor");
      expect(message).toContain("temper slack disconnect");
    }
  });

  it("are DISTINCT from each other", () => {
    // FAILS IF: the two are collapsed into one shared string. "No credential is stored" and
    // "your access was revoked" are different facts — one is an incomplete link, the other a
    // deliberate withdrawal (possibly an admin deactivating the profile) — and a user who is
    // told the wrong one goes looking in the wrong place.
    expect(notVaultedPrompt("someone")).not.toBe(revokedPrompt("someone"));
    expect(revokedPrompt("someone").toLowerCase()).toContain("revoked");
    expect(notVaultedPrompt("someone").toLowerCase()).not.toContain("revoked");
  });

  it("say plainly that retrying will not help", () => {
    // FAILS IF: the not_vaulted copy loses its finality. Without it the user mentions again,
    // gets the identical message, and concludes the agent is simply broken — which is the
    // exact experience the pre-flight fork exists to prevent.
    expect(notVaultedPrompt("someone").toLowerCase()).toMatch(/won't fix/);
  });

  it("carry no URL, no task numbers and no dates", () => {
    // FAILS IF: someone "helpfully" pastes an authorize URL here. There is none to offer —
    // both states are reached from link-state's `linked` arm, which carries no
    // `authorize_url` — so any URL in this copy is invented. Also guards the copy rules: no
    // task numbers, no dates, no internal plans in user-facing text.
    for (const message of [notVaultedPrompt("someone"), revokedPrompt("someone")]) {
      expect(message).not.toContain("http");
      expect(message).not.toMatch(/\bT\d\b/);
      expect(message).not.toMatch(/\b20\d\d\b/);
    }
  });
});

describe("decideIdentity threads the caller's auth object through", () => {
  it("exposes attributes.user_id on the accepted arm, unparsed from principalId", () => {
    // The real SessionAuthContext carries decomposed Slack attributes
    // alongside principalId/principalType. This fixture is wider than
    // PrincipalLike to prove the generic threads it through intact.
    const auth = {
      principalId: "slack:T012AB3CD:U024BE7LH",
      principalType: "user",
      attributes: { user_id: "U024BE7LH", team_id: "T012AB3CD" },
    };

    const decision = decideIdentity(auth);

    if (decision.kind !== "human") throw new Error("expected a human decision");
    expect(decision.auth.attributes.user_id).toBe("U024BE7LH");
  });
});
