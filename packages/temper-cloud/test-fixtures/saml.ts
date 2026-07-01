// Test-fixture helper for building signed SAML 2.0 Responses.
//
// Ported from a hand-verified prototype (scratchpad/saml-fixture-test/makeSignedSamlResponse.mjs)
// that was empirically checked against node-saml@5.1.0's validatePostResponseAsync(). The signing
// logic below is preserved byte-for-byte from that prototype -- do not "simplify" it.
//
// Uses the PUBLIC xml-crypto + @xmldom/xmldom APIs (not node-saml internals).
import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { SignedXml } from "xml-crypto";

function isoNow(offsetMs = 0): string {
  return new Date(Date.now() + offsetMs).toISOString().replace(/\.\d+Z$/, "Z");
}

interface SignEnvelopedParams {
  xml: string;
  refXPath: string;
  afterXPath: string;
  privateKeyPem: string;
  publicCertPem: string;
}

// Sign `xml` (the element identified by `refXPath`, whole-document envelope),
// placing <ds:Signature> immediately after the element matched by `afterXPath`.
function signEnveloped({ xml, refXPath, afterXPath, privateKeyPem, publicCertPem }: SignEnvelopedParams): string {
  const sig = new SignedXml({
    privateKey: privateKeyPem,
    publicCert: publicCertPem,
    signatureAlgorithm: "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256",
    canonicalizationAlgorithm: "http://www.w3.org/2001/10/xml-exc-c14n#",
    getKeyInfoContent: SignedXml.getKeyInfoContent, // emit <X509Data> so idpCert-based verification path is realistic
  });
  sig.addReference({
    xpath: refXPath,
    transforms: [
      "http://www.w3.org/2000/09/xmldsig#enveloped-signature",
      "http://www.w3.org/2001/10/xml-exc-c14n#",
    ],
    digestAlgorithm: "http://www.w3.org/2001/04/xmlenc#sha256",
  });
  sig.computeSignature(xml, {
    location: { reference: afterXPath, action: "after" },
  });
  return sig.getSignedXml();
}

export interface MakeSignedSamlResponseParams {
  spEntityId: string;
  acsUrl: string;
  idpEntityId?: string;
  nameId: string;
  nameIdFormat?: string;
  attributes?: Record<string, string>;
  idpKeyPem: string;
  idpCertPem: string;
  assertionId?: string;
  responseId?: string;
  sessionIndex?: string;
  notBeforeOffsetMs?: number;
  notOnOrAfterOffsetMs?: number;
  issueInstant?: string;
}

export interface MakeSignedSamlResponseResult {
  samlResponseB64: string;
  signedResponseXml: string;
  assertionId: string;
  responseId: string;
}

/**
 * Build a signed SAML 2.0 Response (base64) that node-saml@5's
 * validatePostResponseAsync() will accept, given a SAML config with:
 *   { idpCert, callbackUrl, issuer, wantAssertionsSigned: true, validateInResponseTo: 'never', identifierFormat }
 *
 * IMPORTANT: node-saml defaults `wantAuthnResponseSigned` to `true` when not
 * explicitly set, so BOTH the <samlp:Response> and the <saml:Assertion> must
 * carry valid enveloped signatures -- not just the assertion. This helper
 * signs both, in the order a real IdP would (assertion first, then response).
 */
export function makeSignedSamlResponse({
  spEntityId,
  acsUrl,
  idpEntityId = "https://test-idp.example.com/metadata",
  nameId,
  nameIdFormat = "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
  attributes = {},
  idpKeyPem,
  idpCertPem,
  assertionId = `_${randomUUID()}`,
  responseId = `_${randomUUID()}`,
  sessionIndex = `_${randomUUID()}`,
  notBeforeOffsetMs = -60_000,
  notOnOrAfterOffsetMs = 5 * 60_000,
  issueInstant = isoNow(),
}: MakeSignedSamlResponseParams): MakeSignedSamlResponseResult {
  const notBefore = isoNow(notBeforeOffsetMs);
  const notOnOrAfter = isoNow(notOnOrAfterOffsetMs);
  const attributeXml = Object.entries(attributes)
    .map(
      ([name, value]) =>
        `<saml:Attribute Name="${name}" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:basic">` +
        `<saml:AttributeValue xsi:type="xs:string">${value}</saml:AttributeValue>` +
        `</saml:Attribute>`,
    )
    .join("");

  const assertionXml =
    `<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ` +
    `xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" ` +
    `ID="${assertionId}" IssueInstant="${issueInstant}" Version="2.0">` +
    `<saml:Issuer>${idpEntityId}</saml:Issuer>` +
    `<saml:Subject>` +
    `<saml:NameID Format="${nameIdFormat}">${nameId}</saml:NameID>` +
    `<saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer">` +
    `<saml:SubjectConfirmationData NotOnOrAfter="${notOnOrAfter}" Recipient="${acsUrl}"/>` +
    `</saml:SubjectConfirmation>` +
    `</saml:Subject>` +
    `<saml:Conditions NotBefore="${notBefore}" NotOnOrAfter="${notOnOrAfter}">` +
    `<saml:AudienceRestriction><saml:Audience>${spEntityId}</saml:Audience></saml:AudienceRestriction>` +
    `</saml:Conditions>` +
    `<saml:AuthnStatement AuthnInstant="${issueInstant}" SessionIndex="${sessionIndex}">` +
    `<saml:AuthnContext><saml:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport</saml:AuthnContextClassRef></saml:AuthnContext>` +
    `</saml:AuthnStatement>` +
    (attributeXml ? `<saml:AttributeStatement>${attributeXml}</saml:AttributeStatement>` : "") +
    `</saml:Assertion>`;

  const signedAssertionXml = signEnveloped({
    xml: assertionXml,
    refXPath: "//*[local-name(.)='Assertion']",
    afterXPath: "//*[local-name(.)='Assertion']/*[local-name(.)='Issuer']",
    privateKeyPem: idpKeyPem,
    publicCertPem: idpCertPem,
  });

  const responseXml =
    `<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ` +
    `xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ` +
    `ID="${responseId}" IssueInstant="${issueInstant}" Version="2.0" Destination="${acsUrl}">` +
    `<saml:Issuer>${idpEntityId}</saml:Issuer>` +
    `<samlp:Status><samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/></samlp:Status>` +
    signedAssertionXml +
    `</samlp:Response>`;

  const signedResponseXml = signEnveloped({
    xml: responseXml,
    refXPath: "//*[local-name(.)='Response']",
    afterXPath: "//*[local-name(.)='Response']/*[local-name(.)='Issuer']",
    privateKeyPem: idpKeyPem,
    publicCertPem: idpCertPem,
  });

  return {
    samlResponseB64: Buffer.from(signedResponseXml, "utf8").toString("base64"),
    signedResponseXml,
    assertionId,
    responseId,
  };
}

export function tamperSamlResponseB64(samlResponseB64: string): string {
  const xml = Buffer.from(samlResponseB64, "base64").toString("utf8");
  // Flip the NameID value -- content changes invalidate the digest without
  // touching the Signature block itself (so it still "looks" signed).
  const tampered = xml.replace(
    /(<saml:NameID[^>]*>)([^<]+)(<\/saml:NameID>)/,
    (_m, open: string, _val: string, close: string) => `${open}attacker@evil.example${close}`,
  );
  if (tampered === xml) {
    throw new Error("tamper: NameID not found, nothing was mutated");
  }
  return Buffer.from(tampered, "utf8").toString("base64");
}

export function loadIdpFixtureCert(path: string): string {
  return readFileSync(path, "utf8");
}
