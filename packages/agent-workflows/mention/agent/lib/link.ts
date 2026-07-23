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
