import { defineMcpClientConnection } from "eve/connections";
import { never } from "eve/tools/approval";

import { AUDITOR_CREDENTIALS, mintAuditorM2mToken, requireEnv } from "../../../lib/temper-auth.js";

/**
 * The citation auditor's sole seam to temper-mcp — **its own connection under its own machine
 * credential**, and the reason it is not the root's is the whole of spec §5.2.
 *
 * The chain is `client_id → kb_machine_clients → profile → kb_entities.profile_id →
 * kb_events.emitter_entity_id`: one credential is one emitter entity. A steward and an auditor
 * sharing a machine client would emit acts the ledger cannot tell apart, and the self-declared
 * `AgentAuthorship.persona` cannot rescue that — it is invisible to projections by design. "Assessed
 * by another party" would degrade to "asserted by the same party wearing a label." Because a declared
 * subagent inherits none of the root's connections, this file is also what makes the boundary
 * structural: a steward session cannot reach the auditor's credential, and this session cannot reach
 * the steward's.
 *
 * There is no Vercel Connect branch here, unlike the root's connection. A Connect connector is
 * scoped to the DEPLOYMENT, not to a principal, so falling back to it would authenticate the auditor
 * as the deployment — i.e. as the steward — silently reintroducing the collapse above. If
 * `TEMPER_AUDITOR_M2M_CLIENT_ID` is absent, this falls to a static `TEMPER_AUDITOR_TOKEN` (`eve dev`)
 * and otherwise throws.
 *
 * The auditor's principal is provisioned with team membership and **read-only** cogmap reach:
 * `--cogmap <ref>` defaults to WRITE, which makes `cogmap_authorable_by_profile` true for every
 * finding homed in the map, classifies the auditor as `AuditAuthority::Author`, and 404s every audit
 * it attempts — with nothing failing at deploy time. See `docs/auth/machine-token-contract.md` §C.
 *
 * The allow-list is the persona, stated as capability rather than as instruction. It is READ +
 * VERDICT and nothing else:
 * - the **authored-4** (`create_resource`, `assert_relationship`, `facet_set`, `fold_relationship`)
 *   is absent — an auditor that can author findings is a citer, and §7's self-audit denial arm
 *   should never be the only thing standing between the two roles;
 * - `annotate_resource`, `update_*` and `delete_resource` are absent for the same reason;
 * - `steward_advance_watermark` is absent — the auditor completes ITS job, not the steward's.
 *
 * Approval is `never()` — the auditor is autonomous and audited by the invocation envelope, exactly
 * as the steward is.
 */
export default defineMcpClientConnection({
  url: requireEnv("TEMPER_MCP_URL"),
  description:
    "Temper knowledge base: a finding's citations (get_block_provenance), the cited sources themselves, and the append-only citation-audit trail this agent writes its verdicts to.",
  auth: process.env[AUDITOR_CREDENTIALS.clientId]
    ? { getToken: mintAuditorM2mToken }
    : { getToken: async () => ({ token: requireEnv(AUDITOR_CREDENTIALS.staticToken) }) },
  approval: never(),
  tools: {
    allow: [
      // The verdict — the only write this agent has.
      "record_citation_audit",
      // The citations under audit.
      "get_block_provenance",
      // Reading the finding, its cited sources, and its lineage — the §3.3 material.
      "get_resource",
      "resource_lineage",
      "search",
      // The map's telos, so a citation is weighed against what the map is for.
      "cogmap_read_charter",
      // The invocation envelope (§5.4 — it needs nothing new).
      "invocation_open",
      "invocation_close",
      "invocation_show",
      // Identity self-check: which principal am I actually authoring as.
      "get_profile",
    ],
  },
});
