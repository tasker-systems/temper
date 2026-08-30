import { type SamlConfig, ValidateInResponseTo } from "@node-saml/node-saml";
import type { NeonClient } from "../db.js";
import { logger } from "../logger.js";

/** Mirrors the `kb_saml_idp` table columns exactly (Task 2.1 migration). */
export interface SamlIdpRow {
  idp_key: string;
  is_active: boolean;
  idp_cert: string;
  /**
   * A second acceptable signing certificate for the SAME logical IdP, non-null only while a
   * signing-key rollover is in progress. Order carries no meaning: an assertion signed by either
   * cert validates, which is what lets the operator add the incoming cert, watch logins keep
   * working, let the IdP cut over on its own schedule, and only then drop the outgoing one.
   */
  idp_cert_secondary: string | null;
  idp_sso_url: string;
  idp_entity_id: string;
  sp_entity_id: string;
  acs_url: string;
  nameid_format: string;
  email_attr: string;
  stable_id_attr: string;
  groups_attr: string | null;
  created: string;
  updated: string;
}

/** A PEM certificate block, as node-saml requires it: real newlines, header and footer intact. */
const PEM_CERT = /-----BEGIN CERTIFICATE-----\r?\n[\s\S]*?-----END CERTIFICATE-----/g;

/**
 * The certificates that may have signed an assertion from this IdP, outgoing first.
 *
 * This widens the accepted signer set by exactly the certificates held in this one row. It is
 * deliberately NOT a relaxation of which IdP is active: a cert belonging to some other IdP, or to
 * none, is no more acceptable during a rollover than outside one.
 *
 * **One unusable slot must never veto a usable one.** node-saml resolves the whole `idpCert` array
 * before verifying anything and throws on the first entry it cannot parse, so a malformed secondary
 * refuses assertions the *primary* signed - the exact outage an overlap window exists to prevent.
 * Entries are matched against `PEM_CERT` and dropped when they do not parse. Dropping is not the
 * only signal: a CHECK constraint refuses the write, so an operator hears about a bad paste when
 * they stage it rather than when the IdP cuts over.
 *
 * A slot holding a PEM *bundle* contributes every certificate in it, because node-saml reads only
 * the first block of a multi-cert string - an operator pasting an IdP-exported chain would
 * otherwise get a slot that looks configured and silently honours one certificate of several.
 */
function certsFor(row: SamlIdpRow): string[] {
  const certs: string[] = [];
  for (const [slot, raw] of [
    ["idp_cert", row.idp_cert],
    ["idp_cert_secondary", row.idp_cert_secondary],
  ] as const) {
    const trimmed = raw?.trim() ?? "";
    if (trimmed === "") {
      continue;
    }
    const blocks = trimmed.match(PEM_CERT);
    if (blocks) {
      certs.push(...blocks);
    } else if (slot === "idp_cert") {
      // The primary is passed through exactly as it always was. node-saml accepts shapes this
      // regex does not match - bare base64 among them - and an instance already running on one of
      // them must keep running. Tightening what the PRIMARY may hold is a separate change with its
      // own migration story; it is not something a rollover feature gets to do in passing.
      certs.push(trimmed);
    } else {
      logger.warn(
        { slot },
        "SAML: ignoring a certificate slot that does not hold a PEM certificate block - check for an escaped newline, or paste the PEM with real line breaks",
      );
    }
  }
  return certs;
}

/**
 * `certsFor`, refusing a row that yields no usable certificate at all.
 *
 * node-saml's own `assertRequired` rejects a null or empty-string `idpCert` but accepts an empty
 * ARRAY, and then fails every assertion with a generic bad-signature error - so a row with no
 * usable certificate would present as "the IdP is signing wrongly" on every login rather than as
 * the configuration fault it is. Failing here names the actual problem.
 */
function requireCertsFor(row: SamlIdpRow): string[] {
  const certs = certsFor(row);
  if (certs.length === 0) {
    throw new Error(
      `kb_saml_idp row '${row.idp_key}' holds no usable signing certificate - idp_cert must contain a PEM certificate block`,
    );
  }
  return certs;
}

/**
 * Clock-skew tolerance applied to the IdP's timestamps, in milliseconds — deliberately ZERO.
 *
 * **A security-relevant value is chosen rather than inherited.** node-saml's own default is also 0,
 * and that coincidence is why the explicit pin exists: while the two agree there is nothing on this
 * file to review, and a library release that moved its default would silently move ours with it.
 * Zero means the IdP's `NotBefore`/`NotOnOrAfter` are taken at face value — no clock disagreement
 * between the IdP and this host is forgiven, and no expired assertion lingers one millisecond past
 * the window the IdP itself issued. Widening this is a posture change, not a fix: it extends how
 * long a consumed assertion stays presentable, which the replay guard's retention must then cover
 * (see `REPLAY_TTL_SECONDS` in `../oauth/endpoints.ts` and the coupling test that binds them).
 */
export const SAML_ACCEPTED_CLOCK_SKEW_MS = 0;

/**
 * Cap on an assertion's age beyond its IdP-issued window, in milliseconds — deliberately ZERO.
 *
 * Zero is node-saml's "no additional cap": the assertion expires exactly at its `NotOnOrAfter` and
 * never a moment sooner, so an IdP issuing five-minute assertions gets five minutes and an operator
 * shortening the IdP's window needs no change here. Pinning it states the choice; the alternative —
 * an operator-set cap stricter than `NotOnOrAfter` — would refuse assertions the IdP considers
 * valid, an availability failure that would present as "the IdP is signing wrongly".
 */
export const SAML_MAX_ASSERTION_AGE_MS = 0;

/** Pure mapping from the persisted IdP row to the node-saml SP config. */
export function toSamlConfig(row: SamlIdpRow): SamlConfig {
  return {
    callbackUrl: row.acs_url,
    entryPoint: row.idp_sso_url,
    issuer: row.sp_entity_id,
    // node-saml accepts `string | string[]`. Always an array, even for the one-cert steady state,
    // so signature verification has a single code path rather than a rare branch that only an
    // in-progress rollover exercises.
    idpCert: requireCertsFor(row),
    audience: row.sp_entity_id,
    identifierFormat: row.nameid_format,
    // The two assertion windows are pinned constants above, not inherited defaults — the same
    // register as `wantAuthnResponseSigned` below: what an assertion must satisfy to be accepted
    // is a reviewable choice on this file, never something a dependency upgrade decides.
    acceptedClockSkewMs: SAML_ACCEPTED_CLOCK_SKEW_MS,
    maxAssertionAgeMs: SAML_MAX_ASSERTION_AGE_MS,
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

/**
 * Loads the single active IdP configuration row, or null if none is active.
 *
 * `idp_cert_secondary` is read through `to_jsonb` rather than named as a column, and that is
 * load-bearing rather than stylistic. Migrations here are a deploy step, not a startup step, and
 * the self-host playbooks tell an operator to deploy before migrating - so this binary must expect
 * to run against a schema that has no such column. A named column reference raises 42703 there, and
 * since the SAML ACS is the only human authentication door on an AS-mode instance, that is every
 * login failing. `to_jsonb(t) ->> '<absent key>'` yields NULL instead, which `certsFor` reads as
 * "no overlap window" - the correct answer on a schema that cannot hold one. Once migration
 * 20260827000010 is applied everywhere this may become a plain column reference.
 */
export async function loadActiveIdp(db: NeonClient): Promise<SamlIdpRow | null> {
  const rows = await db`SELECT idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id,
    sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr, created, updated,
    to_jsonb(t) ->> 'idp_cert_secondary' AS idp_cert_secondary
    FROM kb_saml_idp t WHERE is_active = true LIMIT 1`;
  return rows.length > 0 ? (rows[0] as SamlIdpRow) : null;
}
