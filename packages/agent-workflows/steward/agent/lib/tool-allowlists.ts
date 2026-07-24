/**
 * The two personas' MCP tool allow-lists, as data.
 *
 * They live here rather than inline in each `connections/temper.ts` for one reason: **the boundary
 * between the steward and the auditor is enforced by these lists, and a boundary nothing asserts is
 * a boundary nobody notices breaking.**
 *
 * The auditor runs as a declared subagent, and eve offers no way to start a session directly on a
 * subagent — so `schedules/auditor.ts` starts a **root steward session** which is instructed by
 * prompt to delegate immediately and call no temper tool itself. Everything downstream of that
 * delegation is prompt-strength. What makes the hop safe is not the prompt: it is that
 * `STEWARD_TOOLS` does **not** contain `record_citation_audit`, so no audit act can be emitted under
 * the steward credential however the root session is talked into behaving. That was true by
 * accident of two separately-authored lists; `tests/auditor.test.ts` now asserts it, and this module
 * is what gives it something to assert against.
 *
 * Neither list is a superset of the other, and that is deliberate in both directions — the steward
 * cannot audit, and the auditor cannot author (see each list's own notes).
 */

/**
 * The steward's 24 tools. The 9 excluded (region reads + genesis/admin/access) are
 * role-inappropriate for a steward; `record_citation_audit` is excluded for a stronger reason —
 * see the module doc.
 */
export const STEWARD_TOOLS: readonly string[] = [
  // Authored-4
  "create_resource",
  "assert_relationship",
  "facet_set",
  "fold_relationship",
  // Invocation envelope
  "invocation_open",
  "invocation_close",
  "invocation_show",
  "invocation_list",
  // Steward delta / watermark
  "steward_ingest_delta",
  "steward_advance_watermark",
  // Reads
  "search",
  "get_resource",
  "get_context",
  "list_contexts",
  "list_resources",
  "cogmap_read_charter",
  "describe_doc_type",
  "list_doc_types",
  "get_profile",
  // Mutations (delete_resource is soft-delete: flips is_active via a resource_deleted event)
  "update_resource",
  "update_resource_meta",
  "delete_resource",
  "retype_relationship",
  "reweight_relationship",
];

/**
 * The citation auditor's tools — READ + VERDICT and nothing else. The allow-list is the persona,
 * stated as capability rather than as instruction:
 *
 * - the **authored-4** (`create_resource`, `assert_relationship`, `facet_set`, `fold_relationship`)
 *   is absent — an auditor that can author findings is a citer, and spec §7's self-audit denial arm
 *   should never be the only thing standing between the two roles;
 * - `annotate_resource`, `update_*` and `delete_resource` are absent for the same reason;
 * - `steward_advance_watermark` is absent — the auditor completes ITS job, not the steward's.
 */
export const AUDITOR_TOOLS: readonly string[] = [
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
];
