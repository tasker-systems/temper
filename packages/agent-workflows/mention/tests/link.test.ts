import { createHmac } from "node:crypto";
import { describe, expect, it } from "vitest";

import { signIntentRequest } from "../agent/lib/link.js";

describe("signIntentRequest", () => {
  it("signs HMAC-SHA256 over `{timestamp}.{body}` as lowercase hex", () => {
    const body = JSON.stringify({ slack_principal_id: "slack:T1:U1" });
    const { timestamp, signature } = signIntentRequest("s3cret", 1_700_000_000, body);

    expect(timestamp).toBe("1700000000");
    // The known-answer check: this MUST match temper_core::internal_sig::sign.
    const expected = createHmac("sha256", "s3cret")
      .update(`1700000000.${body}`)
      .digest("hex");
    expect(signature).toBe(expected);
    expect(signature).toMatch(/^[0-9a-f]{64}$/);
  });
});
