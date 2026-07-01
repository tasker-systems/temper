import type { Profile } from "@node-saml/node-saml";
import type { MintedClaims } from "../oauth/mint.js";
import type { SamlIdpRow } from "./config.js";

const PERSISTENT_SUFFIX = ":persistent";
const EMAIL_SUFFIX = ":emailAddress";

/**
 * Reads a single assertion attribute as a string. node-saml types `profile.attributes` as
 * `unknown` (index signature), so callers must narrow it first. A multi-valued attribute is
 * exposed as a string[]; we take the first element. Returns undefined if absent/empty.
 */
function readAttr(attrs: Record<string, unknown>, name: string): string | undefined {
  const value = attrs[name];
  const scalar = Array.isArray(value) ? value[0] : value;
  if (scalar === undefined || scalar === null || scalar === "") {
    return undefined;
  }
  return String(scalar);
}

/** Pure mapping from a validated SAML assertion profile to the claims we mint into a token. */
export function mapProfileToClaims(profile: Profile, idp: SamlIdpRow): MintedClaims {
  const attrs = (profile.attributes ?? {}) as Record<string, unknown>;
  const nameIDFormat = profile.nameIDFormat ?? "";

  const sub = nameIDFormat.endsWith(PERSISTENT_SUFFIX)
    ? profile.nameID
    : readAttr(attrs, idp.stable_id_attr);
  if (!sub) {
    throw new Error(
      `SAML profile has no persistent NameID and no stable-id attribute '${idp.stable_id_attr}'`,
    );
  }

  let email = readAttr(attrs, idp.email_attr);
  if (!email && nameIDFormat.endsWith(EMAIL_SUFFIX)) {
    email = profile.nameID;
  }
  if (!email) {
    throw new Error(`SAML profile has no email attribute '${idp.email_attr}'`);
  }

  return {
    sub,
    email,
    // A validly signed SAML assertion from the configured IdP is treated as an authoritative,
    // pre-verified identity source, so the email it carries is considered verified.
    email_verified: true,
  };
}
