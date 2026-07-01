import { describe, expect, it } from "vitest";
import { verifyPkceS256 } from "../../src/oauth/pkce.js";

describe("verifyPkceS256", () => {
  it("accepts a matching S256 verifier/challenge pair", () => {
    // challenge = base64url(sha256(verifier)); precomputed for this verifier.
    const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    expect(verifyPkceS256(verifier, challenge)).toBe(true);
  });

  it("rejects a wrong challenge", () => {
    const challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    expect(verifyPkceS256("wrong-verifier", challenge)).toBe(false);
  });

  it("rejects a challenge of a different length without throwing", () => {
    const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    expect(verifyPkceS256(verifier, "short")).toBe(false);
  });
});
