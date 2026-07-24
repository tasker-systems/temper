import { defineAgent } from "eve";

import { gatewayModelOptions, resolveAuditorModelConfig } from "../../lib/model-config.js";

const model = resolveAuditorModelConfig();

/**
 * The citation auditor (Set 5), as a DECLARED SUBAGENT rather than a second schedule on the root
 * agent — and the reason is the isolation boundary, not tidiness.
 *
 * A declared subagent inherits none of the root's authored slots (eve `docs/subagents.mdx`, "The
 * isolation boundary"): its instructions, connections and tools are only what is authored under this
 * directory. That is exactly the property spec §5.2/§5.3 need. It gives the auditor:
 *
 * - its own **model**, the one lever that genuinely attacks shared trained priors (§5.3), which eve
 *   resolves per declared agent at build time and offers no per-session override for;
 * - its own **temper connection** under its own machine credential (§5.2), unreachable from a
 *   steward session — so "assessed by another party" is enforced by what the two sessions can call,
 *   not by what a prompt asks them not to;
 * - its own **tool surface**: reads, the element trail, and `record_citation_audit`. The authored-4
 *   is absent, because an auditor that can author findings is a citer.
 *
 * It lives beside the steward in one Eve project because code colocation does not create epistemic
 * dependence — the two have separate instructions, separate success criteria, and never read one
 * another. A sibling Eve project was considered and rejected as cost without benefit (§5.3).
 */
export default defineAgent({
  model: model.primary,
  modelOptions: gatewayModelOptions(model.fallbacks),
  description:
    "Citation auditor: weighs whether each source cited by a finding carries the connection the citation claims, and records a signed defensibility verdict per (block, source) citation. It never assesses whether a claim is true.",
});
