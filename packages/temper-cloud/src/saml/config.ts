import { type SamlConfig, ValidateInResponseTo } from "@node-saml/node-saml";
import type { NeonClient } from "../db.js";

/** Mirrors the `kb_saml_idp` table columns exactly (Task 2.1 migration). */
export interface SamlIdpRow {
  idp_key: string;
  is_active: boolean;
  idp_cert: string;
  idp_sso_url: string;
  idp_entity_id: string;
  sp_entity_id: string;
  acs_url: string;
  nameid_format: string;
  email_attr: string;
  stable_id_attr: string;
  created: string;
  updated: string;
}

/** Pure mapping from the persisted IdP row to the node-saml SP config. */
export function toSamlConfig(row: SamlIdpRow): SamlConfig {
  return {
    callbackUrl: row.acs_url,
    entryPoint: row.idp_sso_url,
    issuer: row.sp_entity_id,
    idpCert: row.idp_cert,
    audience: row.sp_entity_id,
    identifierFormat: row.nameid_format,
    wantAssertionsSigned: true,
    // node-saml defaults this to true already, but pin it explicitly so the "both the Response and
    // the Assertion must be signed" guarantee is a local, reviewable invariant rather than an
    // inherited library default that could silently change.
    wantAuthnResponseSigned: true,
    // We mint our own opaque relay_state per flow (kb_oauth_flow.relay_state) rather than relying
    // on node-saml's InResponseTo bookkeeping, so InResponseTo validation is not applicable here.
    validateInResponseTo: ValidateInResponseTo.never,
  };
}

/** Loads the single active IdP configuration row, or null if none is active. */
export async function loadActiveIdp(db: NeonClient): Promise<SamlIdpRow | null> {
  const rows = await db`SELECT idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id,
    sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, created, updated
    FROM kb_saml_idp WHERE is_active = true LIMIT 1`;
  return rows.length > 0 ? (rows[0] as SamlIdpRow) : null;
}
