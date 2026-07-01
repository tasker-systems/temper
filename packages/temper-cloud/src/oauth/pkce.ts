import { createHash, timingSafeEqual } from "node:crypto";

/**
 * Verifies a PKCE S256 code_verifier/code_challenge pair.
 *
 * Computes base64url(sha256(verifier)) and compares it to the supplied
 * challenge in constant time. Returns false (rather than throwing) when
 * the lengths differ, since timingSafeEqual requires equal-length buffers.
 */
export function verifyPkceS256(verifier: string, challenge: string): boolean {
  const computed = createHash("sha256").update(verifier).digest("base64url");
  const computedBuf = Buffer.from(computed);
  const challengeBuf = Buffer.from(challenge);

  if (computedBuf.length !== challengeBuf.length) {
    return false;
  }

  return timingSafeEqual(computedBuf, challengeBuf);
}
