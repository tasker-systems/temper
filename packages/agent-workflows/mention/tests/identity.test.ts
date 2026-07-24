import { describe, expect, it, vi } from "vitest";

import type { Refusal } from "../agent/generated/admission.js";
import {
  decideIdentity,
  isHumanPrincipal,
  linkVanishedPrompt,
  notVaultedPrompt,
  pendingApprovalPrompt,
  standingRefusedPrompt,
  standingReply,
  unknownStandingPrompt,
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

/** Every reply reachable from link-state's `linked` arm — i.e. every mint refusal. */
const refusalPrompts = (handle: string) => [
  notVaultedPrompt(handle),
  standingRefusedPrompt(handle),
  pendingApprovalPrompt(handle),
  unknownStandingPrompt(handle),
  linkVanishedPrompt(handle),
];

describe("the refusal prompts", () => {
  it("each names the handle", () => {
    // FAILS IF: a reply becomes a bare error with nothing identifying in it. The handle is
    // what makes it clear WHICH account is affected — a user with two Slack identities
    // otherwise cannot tell which one the agent is refusing.
    for (const message of refusalPrompts("j-cole-taylor")) {
      expect(message).toContain("@j-cole-taylor");
    }
  });

  it("are all DISTINCT from one another", () => {
    // FAILS IF: any two are collapsed into a shared string. They are separate messages
    // precisely because they carry separate REMEDIES; a shared "something went wrong" — the
    // tempting simplification — trips this even though each reply still "says something".
    const replies = refusalPrompts("someone");
    expect(new Set(replies).size).toBe(replies.length);
  });

  // THE REGRESSION GUARD FOR THE FALSE REMEDY. This is the bug the linked-identity state
  // machine exists to end: the old flat `revoked` arm told every refused human to re-link,
  // including ones whose standing an admin had denied. Re-linking cannot move principal
  // standing, so that advice sent them round a loop with no exit.
  //
  // FAILS IF: the re-link remedy leaks back into any standing reply.
  it("offers re-link ONLY where re-linking is the actual remedy", () => {
    // The one state re-linking fixes: a link with no vaulted credential.
    expect(notVaultedPrompt("someone")).toContain("temper slack disconnect");

    // The states only an admin can fix. `disconnect` must appear in NEITHER, and the denial
    // copy must say so out loud rather than merely omitting it.
    for (const message of [standingRefusedPrompt("someone"), pendingApprovalPrompt("someone")]) {
      expect(message).not.toContain("temper slack disconnect");
    }
    expect(standingRefusedPrompt("someone").toLowerCase()).toContain("reconnecting won't change");
    expect(standingRefusedPrompt("someone").toLowerCase()).toContain("admin");
  });

  it("says plainly, where true, that retrying will not help", () => {
    // FAILS IF: the not_vaulted copy loses its finality. Without it the user mentions again,
    // gets the identical message, and concludes the agent is simply broken — which is the
    // exact experience the pre-flight fork exists to prevent.
    expect(notVaultedPrompt("someone").toLowerCase()).toMatch(/won't fix/);

    // ...and the converse, which is not symmetry for its own sake: `not_linked` is the ONE
    // refusal where mentioning again genuinely works, because the next link-state read takes
    // the `unlinked` arm and hands out a fresh authorize URL.
    expect(linkVanishedPrompt("someone").toLowerCase()).toContain("mention me again");
  });

  it("carry no URL, no task numbers and no dates", () => {
    // FAILS IF: someone "helpfully" pastes an authorize URL here. There is none to offer —
    // every one of these is reached from link-state's `linked` arm, which carries no
    // `authorize_url` — so any URL in this copy is invented. Also guards the copy rules: no
    // task numbers, no dates, no internal plans in user-facing text.
    for (const message of refusalPrompts("someone")) {
      expect(message).not.toContain("http");
      expect(message).not.toMatch(/\bT\d\b/);
      expect(message).not.toMatch(/\b20\d\d\b/);
    }
  });
});

describe("standingReply maps the six admit refusals onto three remedies", () => {
  // FAILS IF: a kind is re-pointed at the wrong remedy — the single decision this whole
  // change turns on. Table-driven so a seventh kind added to the union without a decision
  // here shows up as a compile error at `standingReply`, not a silently missing row.
  it.each(["denied", "no_standing", "revoked", "deactivated"] as const)(
    "routes %s to the admin-decision reply",
    (kind) => {
      expect(standingReply({ kind }, "someone")).toBe(standingRefusedPrompt("someone"));
    },
  );

  it("routes requested to the do-nothing reply, NOT the go-ask-someone one", () => {
    // FAILS IF: `requested` folds into the denied group. Telling a user whose request is
    // already on file to "ask an admin to approve you" invites a duplicate request.
    expect(standingReply({ kind: "requested" }, "someone")).toBe(pendingApprovalPrompt("someone"));
    expect(pendingApprovalPrompt("someone").toLowerCase()).toContain("nothing more is needed");
  });

  // FAILS IF: a transition-machine refusal is given user-facing copy, or falls through to
  // `undefined`. These three are unreachable through the mint — `resolve` delegates standing to
  // `admit`, and `only_admit_reachable_refusals_ever_surface` panics in Rust if one escapes — so
  // reaching them means OUR invariant broke, not that the user did anything. They are handled
  // explicitly rather than by a `default:` so a tenth `Refusal` variant is a compile error.
  it.each(["illegal_transition", "insufficient_authority", "no_prior_standing"] as const)(
    "routes the unreachable %s to the our-fault reply and logs it",
    (kind) => {
      const spy = vi.spyOn(console, "error").mockImplementation(() => {});

      // Cast: these arms carry extra fields, and the point of the test is the KIND dispatch.
      const message = standingReply({ kind } as unknown as Refusal, "someone");

      expect(message).toBe(unknownStandingPrompt("someone"));
      expect(spy).toHaveBeenCalledWith(expect.stringMatching(/admit cannot produce/), { kind });
      spy.mockRestore();
    },
  );

  it("routes an unrecognized standing to the our-fault reply, and LOGS the raw value", () => {
    // FAILS IF: the raw value is swallowed. This state means temper stored a standing this
    // build has never heard of — a version skew — and the raw string is the only evidence
    // that says which one, so dropping it makes the skew undiagnosable from the outside.
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});

    const message = standingReply({ kind: "unrecognized_standing", raw: "quarantined" }, "someone");

    expect(message).toBe(unknownStandingPrompt("someone"));
    expect(spy).toHaveBeenCalledWith(expect.any(String), { raw: "quarantined" });
    spy.mockRestore();
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
