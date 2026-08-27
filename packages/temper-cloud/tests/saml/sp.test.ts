import { fileURLToPath } from "node:url";
import type { Profile } from "@node-saml/node-saml";
import { describe, expect, it } from "vitest";
import { type SamlIdpRow, toSamlConfig } from "../../src/saml/config.js";
import {
  buildSpMetadata,
  extractGroups,
  groupProvisioningConfigured,
  mapProfileToClaims,
  validateAssertion,
} from "../../src/saml/sp.js";
import {
  loadIdpFixtureCert,
  makeSignedSamlResponse,
  tamperSamlResponseB64,
} from "../../test-fixtures/saml.js";

const CERTS_DIR = fileURLToPath(new URL("../../test-fixtures/certs/", import.meta.url));
const idpCertPem = loadIdpFixtureCert(`${CERTS_DIR}idp-cert.pem`);
const idpKeyPem = loadIdpFixtureCert(`${CERTS_DIR}idp-key.pem`);
// The incoming key of a rollover: a *different* signer, same logical IdP.
const nextCertPem = loadIdpFixtureCert(`${CERTS_DIR}idp-cert-secondary.pem`);
const nextKeyPem = loadIdpFixtureCert(`${CERTS_DIR}idp-key-secondary.pem`);

function fakeIdp(overrides: Partial<SamlIdpRow> = {}): SamlIdpRow {
  return {
    idp_key: "primary",
    is_active: true,
    idp_cert: "-----BEGIN CERTIFICATE-----\nFAKE\n-----END CERTIFICATE-----",
    idp_cert_secondary: null,
    idp_sso_url: "https://idp.example.com/sso",
    idp_entity_id: "https://idp.example.com/entity",
    sp_entity_id: "https://temper.example.com/sp",
    acs_url: "https://temper.example.com/api/saml/acs",
    nameid_format: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
    email_attr: "email",
    stable_id_attr: "uid",
    groups_attr: null,
    created: "2026-07-01T00:00:00.000Z",
    updated: "2026-07-01T00:00:00.000Z",
    ...overrides,
  };
}

function fakeSignedIdp(overrides: Partial<SamlIdpRow> = {}): SamlIdpRow {
  return fakeIdp({ idp_cert: idpCertPem, ...overrides });
}

describe("mapProfileToClaims", () => {
  it("uses the persistent NameID as sub and reads email from the email attribute", () => {
    const profile = {
      nameID: "persistent-id-123",
      nameIDFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
      attributes: { email: "alice@example.com" },
    } as unknown as Profile;
    const idp = fakeIdp();

    const claims = mapProfileToClaims(profile, idp);

    expect(claims.sub).toBe("persistent-id-123");
    expect(claims.email).toBe("alice@example.com");
    expect(claims.email_verified).toBe(true);
  });

  it("falls back to the stable-id attribute for sub when NameID is transient", () => {
    const profile = {
      nameID: "transient-id-abc",
      nameIDFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:transient",
      attributes: { uid: "stable-uid-456", email: "bob@example.com" },
    } as unknown as Profile;
    const idp = fakeIdp();

    const claims = mapProfileToClaims(profile, idp);

    expect(claims.sub).toBe("stable-uid-456");
    expect(claims.email).toBe("bob@example.com");
    expect(claims.email_verified).toBe(true);
  });

  it("throws when NameID is transient and no stable-id attribute is present", () => {
    const profile = {
      nameID: "transient-id-xyz",
      nameIDFormat: "urn:oasis:names:tc:SAML:2.0:nameid-format:transient",
      attributes: { email: "carol@example.com" },
    } as unknown as Profile;
    const idp = fakeIdp();

    expect(() => mapProfileToClaims(profile, idp)).toThrow(
      /no persistent NameID and no stable-id attribute 'uid'/,
    );
  });
});

describe("validateAssertion", () => {
  it("validates a genuinely-signed SAML Response end-to-end through node-saml", async () => {
    const idp = fakeSignedIdp();
    const { samlResponseB64, assertionId } = makeSignedSamlResponse({
      spEntityId: idp.sp_entity_id,
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: {
        [idp.email_attr]: "jane@example.com",
        [idp.stable_id_attr]: "stable-123",
      },
      idpKeyPem,
      idpCertPem,
    });

    const result = await validateAssertion(idp, samlResponseB64);

    expect(result.assertionId).toBe(assertionId);
    expect(result.profile.nameID).toBe("the-persistent-id");

    const claims = mapProfileToClaims(result.profile, idp);
    expect(claims.sub).toBe("the-persistent-id");
    expect(claims.email).toBe("jane@example.com");
    expect(claims.email_verified).toBe(true);
  });

  it("rejects a tampered SAML Response", async () => {
    const idp = fakeSignedIdp();
    const { samlResponseB64 } = makeSignedSamlResponse({
      spEntityId: idp.sp_entity_id,
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: {
        [idp.email_attr]: "jane@example.com",
        [idp.stable_id_attr]: "stable-123",
      },
      idpKeyPem,
      idpCertPem,
    });
    const tamperedB64 = tamperSamlResponseB64(samlResponseB64);

    await expect(validateAssertion(idp, tamperedB64)).rejects.toThrow();
  });

  it("rejects a Response whose outer <samlp:Response> is unsigned (assertion-only signature)", async () => {
    const idp = fakeSignedIdp();
    const { samlResponseB64 } = makeSignedSamlResponse({
      spEntityId: idp.sp_entity_id,
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: {
        [idp.email_attr]: "jane@example.com",
        [idp.stable_id_attr]: "stable-123",
      },
      idpKeyPem,
      idpCertPem,
      signResponse: false,
    });

    await expect(validateAssertion(idp, samlResponseB64)).rejects.toThrow();
  });

  it("rejects an assertion whose audience doesn't match the SP's entity id", async () => {
    const idp = fakeSignedIdp();
    const { samlResponseB64 } = makeSignedSamlResponse({
      spEntityId: "https://WRONG.example/meta",
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: {
        [idp.email_attr]: "jane@example.com",
        [idp.stable_id_attr]: "stable-123",
      },
      idpKeyPem,
      idpCertPem,
    });

    await expect(validateAssertion(idp, samlResponseB64)).rejects.toThrow();
  });

  it("rejects an expired assertion", async () => {
    const idp = fakeSignedIdp();
    const { samlResponseB64 } = makeSignedSamlResponse({
      spEntityId: idp.sp_entity_id,
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: {
        [idp.email_attr]: "jane@example.com",
        [idp.stable_id_attr]: "stable-123",
      },
      idpKeyPem,
      idpCertPem,
      notBeforeOffsetMs: -600_000,
      notOnOrAfterOffsetMs: -300_000,
    });

    await expect(validateAssertion(idp, samlResponseB64)).rejects.toThrow();
  });
});

describe("buildSpMetadata", () => {
  it("returns SP metadata XML containing the SP entity id", () => {
    const idp = fakeSignedIdp();

    const metadata = buildSpMetadata(idp);

    expect(metadata.length).toBeGreaterThan(0);
    expect(metadata).toContain("<EntityDescriptor");
    expect(metadata).toContain(idp.sp_entity_id);
  });
});

describe("groupProvisioningConfigured", () => {
  /**
   * The predicate exists to separate two facts `extractGroups` deliberately answers `null` to
   * alike. Asserted here because the ACS branches on it: true means an assertion carrying no groups
   * is worth reporting, false means there was never anything to report.
   */
  it("is false for an authentication-only IdP and true once groups_attr is set", () => {
    expect(groupProvisioningConfigured(fakeIdp({ groups_attr: null }))).toBe(false);
    expect(groupProvisioningConfigured(fakeIdp({ groups_attr: "groups" }))).toBe(true);
  });
});

describe("extractGroups", () => {
  const profileWith = (attrs: Record<string, unknown>): Profile =>
    ({ attributes: attrs }) as unknown as Profile;

  it("returns null (no signal) when groups_attr is not configured", () => {
    expect(
      extractGroups(profileWith({ groups: ["a"] }), fakeIdp({ groups_attr: null })),
    ).toBeNull();
  });

  it("returns null (no signal) when the named attribute is absent from the assertion", () => {
    // Transient IdP misconfig: groups configured, but this assertion omitted the attribute.
    expect(
      extractGroups(profileWith({ other: ["a"] }), fakeIdp({ groups_attr: "groups" })),
    ).toBeNull();
  });

  it("reads a multi-valued attribute", () => {
    expect(
      extractGroups(profileWith({ groups: ["a", "b"] }), fakeIdp({ groups_attr: "groups" })),
    ).toEqual(["a", "b"]);
  });

  it("coerces a single-valued attribute to a one-element array", () => {
    expect(
      extractGroups(profileWith({ groups: "solo" }), fakeIdp({ groups_attr: "groups" })),
    ).toEqual(["solo"]);
  });

  it("returns [] (genuine empty signal) when the attribute is present but empty", () => {
    // Attribute present with no values → real "in no groups now" → caller DOES reconcile/revoke.
    expect(extractGroups(profileWith({ groups: [] }), fakeIdp({ groups_attr: "groups" }))).toEqual(
      [],
    );
  });
});

/**
 * The three states an IdP signing-key rollover passes through, in order. The middle state is the
 * whole point: it is the only one in which both the outgoing and the incoming key are acceptable,
 * and it is what lets each step of the operator procedure be taken independently and reversed.
 *
 * `nextKeyPem`/`nextCertPem` stand for the incoming key. In the only-old and only-new states it is
 * simultaneously the "signer configured for no active IdP" case, which is why the rejection
 * assertions below are not a separate fixture: an unconfigured signer and a not-yet-configured one
 * are the same thing to the SP, and must stay that way.
 */
describe("validateAssertion across an IdP signing-key rollover", () => {
  function signedBy(idp: SamlIdpRow, keyPem: string, certPem: string) {
    return makeSignedSamlResponse({
      spEntityId: idp.sp_entity_id,
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: {
        [idp.email_attr]: "jane@example.com",
        [idp.stable_id_attr]: "stable-123",
      },
      idpKeyPem: keyPem,
      idpCertPem: certPem,
    }).samlResponseB64;
  }

  // State 1 — only the outgoing cert is configured. The incoming key has not been added yet.
  it("only-old: accepts the outgoing signer and rejects the incoming one", async () => {
    const idp = fakeIdp({ idp_cert: idpCertPem, idp_cert_secondary: null });

    await expect(
      validateAssertion(idp, signedBy(idp, idpKeyPem, idpCertPem)),
    ).resolves.toBeTruthy();
    await expect(validateAssertion(idp, signedBy(idp, nextKeyPem, nextCertPem))).rejects.toThrow();
  });

  // State 2 — the overlap window. Both keys sign acceptably, so the IdP may cut over at any moment
  // without coordination, and may cut back.
  it("both: accepts an assertion signed by either configured cert", async () => {
    const idp = fakeIdp({ idp_cert: idpCertPem, idp_cert_secondary: nextCertPem });

    await expect(
      validateAssertion(idp, signedBy(idp, idpKeyPem, idpCertPem)),
    ).resolves.toBeTruthy();
    await expect(
      validateAssertion(idp, signedBy(idp, nextKeyPem, nextCertPem)),
    ).resolves.toBeTruthy();
  });

  // State 3 — the outgoing cert has been removed. Acceptance of it must stop at that moment, not
  // at some later cache expiry: `loadActiveIdp` reads the row per request, so the write IS the
  // revocation.
  it("only-new: removing the outgoing cert immediately stops accepting assertions signed by it", async () => {
    const idp = fakeIdp({ idp_cert: nextCertPem, idp_cert_secondary: null });

    await expect(
      validateAssertion(idp, signedBy(idp, nextKeyPem, nextCertPem)),
    ).resolves.toBeTruthy();
    await expect(validateAssertion(idp, signedBy(idp, idpKeyPem, idpCertPem))).rejects.toThrow();
  });

  // Adding rollover support must not make an unknown signer acceptable. A cert that is configured
  // for NO active IdP is rejected whether or not an overlap window is open — the overlap widens the
  // accepted set by exactly one named cert, never by "some other valid-looking certificate".
  it("rejects a signer configured for no active IdP, including mid-overlap", async () => {
    const overlapping = fakeIdp({ idp_cert: idpCertPem, idp_cert_secondary: idpCertPem });

    await expect(
      validateAssertion(overlapping, signedBy(overlapping, nextKeyPem, nextCertPem)),
    ).rejects.toThrow();
  });
});

/**
 * node-saml resolves every entry of `idpCert` before it verifies anything, and throws on the first
 * one it cannot parse. So a slot holding something that is not a certificate does not merely fail
 * to help — it refuses assertions the OTHER slot legitimately signed. That turns staging an
 * incoming cert, the step of a rollover that is supposed to change nothing, into a total
 * authentication outage.
 *
 * The escaped-newline case is the one that matters in practice: a SQL literal written
 * `'-----BEGIN CERTIFICATE-----\n...'` stores a literal backslash-n under the default
 * `standard_conforming_strings`, which is exactly the shape node-saml rejects.
 */
describe("a malformed slot never vetoes a usable one", () => {
  const unusable: Array<[string, string]> = [
    [
      "an escaped-newline PEM, as a plain SQL literal stores it",
      "-----BEGIN CERTIFICATE-----\\nMIIC\\n-----END CERTIFICATE-----",
    ],
    ["whitespace only", "   "],
    ["a PEM with no line breaks at all", idpCertPem.replace(/\n/g, "")],
    ["unrelated text", "not a certificate"],
  ];

  for (const [label, junk] of unusable) {
    it(`still accepts the primary's signature when the secondary holds ${label}`, async () => {
      const idp = fakeIdp({ idp_cert: idpCertPem, idp_cert_secondary: junk });
      const signed = makeSignedSamlResponse({
        spEntityId: idp.sp_entity_id,
        acsUrl: idp.acs_url,
        nameId: "the-persistent-id",
        nameIdFormat: idp.nameid_format,
        attributes: { [idp.email_attr]: "jane@example.com", [idp.stable_id_attr]: "stable-123" },
        idpKeyPem,
        idpCertPem,
      }).samlResponseB64;

      await expect(validateAssertion(idp, signed)).resolves.toBeTruthy();
    });
  }

  // Dropping an unusable slot must not drop the guarantee: the junk is not a signer either.
  it("and the discarded slot confers nothing on whoever holds its key", async () => {
    const idp = fakeIdp({ idp_cert: idpCertPem, idp_cert_secondary: "   " });
    const signedByOther = makeSignedSamlResponse({
      spEntityId: idp.sp_entity_id,
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: { [idp.email_attr]: "jane@example.com", [idp.stable_id_attr]: "stable-123" },
      idpKeyPem: nextKeyPem,
      idpCertPem: nextCertPem,
    }).samlResponseB64;

    await expect(validateAssertion(idp, signedByOther)).rejects.toThrow();
  });

  // A row with nothing usable is a configuration fault and says so, rather than presenting as
  // every assertion having a bad signature.
  it("names the configuration fault when no slot holds a certificate", () => {
    expect(() => toSamlConfig(fakeIdp({ idp_cert: "   ", idp_cert_secondary: null }))).toThrow(
      /no usable signing certificate/,
    );
  });

  // An IdP-exported chain in one slot must contribute every certificate in it: node-saml reads only
  // the first block of a multi-cert string, so a leaf that is not first would validate nothing.
  it("honours every certificate in a PEM bundle, not just the first", async () => {
    const idp = fakeIdp({ idp_cert: `${nextCertPem}\n${idpCertPem}`, idp_cert_secondary: null });
    const signedBySecond = makeSignedSamlResponse({
      spEntityId: idp.sp_entity_id,
      acsUrl: idp.acs_url,
      nameId: "the-persistent-id",
      nameIdFormat: idp.nameid_format,
      attributes: { [idp.email_attr]: "jane@example.com", [idp.stable_id_attr]: "stable-123" },
      idpKeyPem,
      idpCertPem,
    }).samlResponseB64;

    await expect(validateAssertion(idp, signedBySecond)).resolves.toBeTruthy();
  });
});
