import { ValidateInResponseTo } from "@node-saml/node-saml";
import { describe, expect, it } from "vitest";
import { type SamlIdpRow, toSamlConfig } from "../../src/saml/config.js";

function fakeRow(overrides: Partial<SamlIdpRow> = {}): SamlIdpRow {
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
    groups_attr: null,
    created: "2026-07-01T00:00:00.000Z",
    updated: "2026-07-01T00:00:00.000Z",
    ...overrides,
  };
}

describe("toSamlConfig", () => {
  it("maps a SamlIdpRow to the SamlConfig shape node-saml expects", () => {
    const row = fakeRow();
    const config = toSamlConfig(row);

    expect(config.callbackUrl).toBe(row.acs_url);
    expect(config.entryPoint).toBe(row.idp_sso_url);
    expect(config.issuer).toBe(row.sp_entity_id);
    expect(config.idpCert).toBe(row.idp_cert);
    expect(config.audience).toBe(row.sp_entity_id);
    expect(config.identifierFormat).toBe(row.nameid_format);
    expect(config.wantAssertionsSigned).toBe(true);
    expect(config.validateInResponseTo).toBe(ValidateInResponseTo.never);
  });
});
