import { createHmac } from "node:crypto";

/**
 * The account-link intent call: agent -> temper-api.
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
 * Ask temper for an authorize URL for this principal.
 *
 * `principalId` is passed WHOLE. It has 2-4 segments and must never be split.
 */
export async function requestAuthorizeUrl(principalId: string): Promise<string> {
  const baseUrl = requireEnv("TEMPER_API_URL");
  const secret = requireEnv("SLACK_LINK_SECRET");

  const body = JSON.stringify({ slack_principal_id: principalId });
  const { timestamp, signature } = signIntentRequest(secret, Date.now() / 1000, body);

  const res = await fetch(`${baseUrl.replace(/\/$/, "")}/internal/slack/link-intents`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Temper-Timestamp": timestamp,
      "X-Temper-Signature": signature,
    },
    body,
  });

  if (!res.ok) {
    throw new Error(`link-intents failed: ${res.status}`);
  }

  const json = (await res.json()) as { authorize_url: string };
  return json.authorize_url;
}

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`Missing required environment variable: ${name}`);
  return value;
}
