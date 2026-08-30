import { ValidateInResponseTo } from "@node-saml/node-saml";
import { describe, expect, it } from "vitest";
import { REPLAY_TTL_SECONDS } from "../../src/oauth/endpoints.js";
import {
  SAML_ACCEPTED_CLOCK_SKEW_MS,
  SAML_MAX_ASSERTION_AGE_MS,
  type SamlIdpRow,
  toSamlConfig,
} from "../../src/saml/config.js";

function fakeRow(overrides: Partial<SamlIdpRow> = {}): SamlIdpRow {
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

describe("toSamlConfig", () => {
  it("maps a SamlIdpRow to the SamlConfig shape node-saml expects", () => {
    const row = fakeRow();
    const config = toSamlConfig(row);

    expect(config.callbackUrl).toBe(row.acs_url);
    expect(config.entryPoint).toBe(row.idp_sso_url);
    expect(config.issuer).toBe(row.sp_entity_id);
    expect(config.idpCert).toEqual([row.idp_cert]);
    expect(config.audience).toBe(row.sp_entity_id);
    expect(config.identifierFormat).toBe(row.nameid_format);
    expect(config.wantAssertionsSigned).toBe(true);
    expect(config.validateInResponseTo).toBe(ValidateInResponseTo.never);
  });

  // node-saml accepts `idpCert: string | string[]`, and an array is how an overlap window is
  // expressed: during a rollover both the outgoing and the incoming cert are acceptable signers of
  // the same logical IdP. The mapping always produces an array so there is one code path, not a
  // one-vs-many branch that only the rare case exercises.
  it("carries both certs to node-saml when an overlap window is open", () => {
    const row = fakeRow({
      idp_cert_secondary: "-----BEGIN CERTIFICATE-----\nNEXT\n-----END CERTIFICATE-----",
    });

    const config = toSamlConfig(row);

    expect(config.idpCert).toEqual([row.idp_cert, row.idp_cert_secondary]);
  });

  it("omits an absent secondary cert rather than passing a null to node-saml", () => {
    const config = toSamlConfig(fakeRow({ idp_cert_secondary: null }));

    expect(config.idpCert).toEqual([fakeRow().idp_cert]);
    expect(config.idpCert).not.toContain(null);
  });
});

describe("assertion windows vs replay retention", () => {
  it("pins both SAML windows explicitly rather than inheriting node-saml defaults", () => {
    const config = toSamlConfig(fakeRow());

    // The windows node-saml applies are the constants on this file, not whatever its
    // constructor defaults to — a dependency upgrade cannot silently move what an
    // assertion must satisfy to be accepted.
    expect(config.acceptedClockSkewMs).toBe(SAML_ACCEPTED_CLOCK_SKEW_MS);
    expect(config.maxAssertionAgeMs).toBe(SAML_MAX_ASSERTION_AGE_MS);
  });

  // A consumed assertion is presentable until its IdP-issued NotOnOrAfter widened by the pinned
  // clock skew — only skew can extend presentability; MAX_AGE can only cap it early — and the
  // replay guard's retention must cover that whole presentable window or a captured assertion
  // replays after the guard has forgotten it. The window half beyond REPLAY_TTL_SECONDS is the
  // reaper's (AS_SAML_ASSERTION_MAX_SECONDS, in temper-services' as_reap_service); this binds the
  // numbers both sides can see, so widening a window here without raising retention fails this
  // test instead of re-opening replay. The sum is the conservative bound, and it is trivially
  // satisfied at today's pins (0 + 0 <= 600) — stated rather than hidden: it bites the day a
  // widening pushes past retention, not today.
  it("retention covers the widest assertion window the pinned windows can add", () => {
    const addedWindowSeconds = (SAML_ACCEPTED_CLOCK_SKEW_MS + SAML_MAX_ASSERTION_AGE_MS) / 1000;

    expect(REPLAY_TTL_SECONDS).toBeGreaterThanOrEqual(addedWindowSeconds);
  });
});
