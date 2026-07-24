import { createHmac } from "node:crypto";

/**
 * The account-link state call: agent -> temper-api.
 *
 * The signature covers THIS server-to-server call, not the URL the user clicks. temper's
 * `internal_sig` skew window is 30 seconds and a human clicks a Slack link minutes later,
 * so signing the user-facing URL would force that gate open. What the user gets is the
 * IdP's own authorize URL with an opaque, single-use, DB-backed state.
 *
 * Scheme (must match `temper_core::internal_sig::sign` byte for byte):
 *   HMAC-SHA256(secret, "{unix_timestamp}.{raw_body}") -> lowercase hex
 */
export function signIntentRequest(
  secret: string,
  timestampSecs: number,
  body: string,
): { timestamp: string; signature: string } {
  const timestamp = String(Math.floor(timestampSecs));
  const signature = createHmac("sha256", secret)
    .update(`${timestamp}.${body}`)
    .digest("hex");
  return { timestamp, signature };
}

/**
 * What temper says about this Slack principal — mirrors the Rust
 * `SlackLinkStateResponse` (`#[serde(tag = "status")]`, snake_case).
 *
 * A discriminated union rather than two nullable fields, for the same reason the Rust side
 * is one: "linked with an authorize URL" and "unlinked with a handle" are not states, and a
 * shape that can express them is a shape someone eventually reads wrong. Branching on
 * `status` makes the compiler insist both arms are handled.
 */
export type LinkState =
  | { readonly status: "linked"; readonly handle: string }
  | { readonly status: "unlinked"; readonly authorize_url: string };

/**
 * Ask temper what to say to this principal: already linked, or here is a fresh URL.
 *
 * The question is deliberately "what do I say?" and not "mint me a URL". Asking for a URL
 * unconditionally re-prompted an already-linked user on every mention and minted a junk
 * intent row each time — the server now answers the real question, and only the unlinked
 * arm costs a write.
 *
 * `principalId` is passed WHOLE. It has 2-4 segments and must never be split.
 */
export async function requestLinkState(principalId: string): Promise<LinkState> {
  // Asserted on BOTH signed calls, not just the mint. link-state runs on every mention while the
  // mint runs only on the `linked` arm, so checking here is what makes a never-linked workspace's
  // collision visible at all — and the check is cheap enough that the earliest caller should carry
  // it rather than the most privileged one.
  assertSlackSecretsDistinct();

  const baseUrl = requireEnv("TEMPER_API_URL");
  const secret = requireEnv("SLACK_LINK_SECRET");

  const body = JSON.stringify({ slack_principal_id: principalId });
  const { timestamp, signature } = signIntentRequest(secret, Date.now() / 1000, body);

  const res = await fetch(`${baseUrl.replace(/\/$/, "")}/internal/slack/link-state`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Temper-Timestamp": timestamp,
      "X-Temper-Signature": signature,
    },
    body,
  });

  if (!res.ok) {
    throw new Error(`link-state failed: ${res.status}`);
  }

  return (await res.json()) as LinkState;
}

/**
 * Exported for `mint.ts`, which already imports `signIntentRequest` from here — sharing the
 * existing dependency beats a third module holding four lines, and beats a copy that could
 * drift in its error text.
 */
export function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`Missing required environment variable: ${name}`);
  return value;
}

/**
 * Refuse to make a signed call when this deployment's two Slack secrets hold the SAME value.
 *
 * The split is the security property, not tidiness: `SLACK_LINK_SECRET` gates an endpoint that
 * answers a question, `SLACK_MINT_SECRET` gates one that hands back a token carrying a human's
 * entire temper reach. One value for both means possession of the cheap capability already is the
 * expensive one. temper-api enforces the same invariant across all five of its shared secrets at
 * boot (`check_secret_distinctness`, `crates/temper-services/src/config.rs`) — this is the half of
 * it that lives where the agent's own copies are read.
 *
 * **This agent is a SEPARATE Vercel deployment with its own environment**, which is the entire
 * reason a server-side check does not cover it. Note the asymmetry in how the two failures present:
 * if only the agent collides, its two calls cannot both authenticate and the mismatch surfaces as a
 * 401 on every mention. If BOTH sides are set to the same colliding value, everything works and the
 * privilege split is simply gone — the silent case, and the one this exists for.
 *
 * Checked at call time rather than at module load, matching how every other variable here is read
 * (see CLAUDE.md, "read at request time"): a module-load throw is what `lib/mcp-auth.ts` exists to
 * avoid, since it makes a plain `import` fail in a test process. The cost is that a colliding
 * deployment is caught on its first mention rather than at deploy — acceptable, because a
 * correctly-paired temper-api refuses to boot at all in that configuration.
 *
 * Absence is not a collision: an unset variable is `requireEnv`'s business, not this function's.
 */
export function assertSlackSecretsDistinct(): void {
  const link = process.env.SLACK_LINK_SECRET;
  const mint = process.env.SLACK_MINT_SECRET;
  if (link && mint && link === mint) {
    throw new Error(
      "SLACK_LINK_SECRET and SLACK_MINT_SECRET hold the same value. link-state answers " +
        "'is this principal linked?'; mint hands back that human's entire reach. Sharing one " +
        "value makes the cheap capability yield the expensive one. Give each its own.",
    );
  }
}
