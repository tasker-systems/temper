import type { Profile } from "@node-saml/node-saml";
import { describe, expect, it } from "vitest";
import type { SamlIdpRow } from "../../src/saml/config.js";
import { mapProfileToClaims } from "../../src/saml/sp.js";

function fakeIdp(overrides: Partial<SamlIdpRow> = {}): SamlIdpRow {
  return {
    idp_key: "primary",
    is_active: true,
    idp_cert: "-----BEGIN CERTIFICATE-----\nFAKE\n-----END CERTIFICATE-----",
    idp_sso_url: "https://idp.example.com/sso",
    idp_entity_id: "https://idp.example.com/entity",
    sp_entity_id: "https://temper.example.com/sp",
    acs_url: "https://temper.example.com/api/saml/acs",
    nameid_format: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
    email_attr: "email",
    stable_id_attr: "uid",
    created: "2026-07-01T00:00:00.000Z",
    updated: "2026-07-01T00:00:00.000Z",
    ...overrides,
  };
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
