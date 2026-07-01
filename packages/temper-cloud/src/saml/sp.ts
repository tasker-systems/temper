import { type Profile, SAML } from "@node-saml/node-saml";
import type { MintedClaims } from "../oauth/mint.js";
import { type SamlIdpRow, toSamlConfig } from "./config.js";

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

/** Builds the IdP-initiated SP login redirect URL, carrying our opaque relay state. */
export async function buildLoginRedirect(idp: SamlIdpRow, relayState: string): Promise<string> {
  return new SAML(toSamlConfig(idp)).getAuthorizeUrlAsync(relayState, undefined, {});
}

/**
 * The shape node-saml's `Profile.getAssertion()` loosely types as `Record<string, unknown> | null`.
 * We only need the assertion's `ID` attribute (under xml2js's `$` attribute bag) to extract the
 * assertion ID for the replay guard.
 */
interface AssertionIdContainer {
  Assertion?: { $?: { ID?: string } };
}

/**
 * Validates a SAML Response (both the Response and Assertion signatures, per node-saml's default
 * `wantAuthnResponseSigned`), and extracts the assertion ID for replay-guarding.
 * Throws on bad signature, audience mismatch, expired assertion, or missing assertion ID --
 * callers should let these propagate.
 */
export async function validateAssertion(
  idp: SamlIdpRow,
  samlResponseB64: string,
): Promise<{ profile: Profile; assertionId: string }> {
  const { profile } = await new SAML(toSamlConfig(idp)).validatePostResponseAsync({
    SAMLResponse: samlResponseB64,
  });
  if (!profile) {
    throw new Error("SAML validation returned no profile");
  }
  const assertion = profile.getAssertion?.() as AssertionIdContainer | null | undefined;
  const assertionId = assertion?.Assertion?.$?.ID;
  if (!assertionId) {
    throw new Error("SAML assertion missing ID");
  }
  return { profile, assertionId };
}

/** Builds the SP metadata XML document that an IdP administrator loads to configure trust. */
export function buildSpMetadata(idp: SamlIdpRow): string {
  // First arg is the decryption cert (nullable) -- we don't support encrypted assertions, so null.
  return new SAML(toSamlConfig(idp)).generateServiceProviderMetadata(null);
}
