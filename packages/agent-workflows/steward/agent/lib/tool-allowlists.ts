/**
 * The two personas' MCP tool allow-lists, as data.
 *
 * They live here rather than inline in each `connections/temper.ts` for one reason: **the boundary
 * between the steward and the auditor is enforced by these lists, and a boundary nothing asserts is
 * a boundary nobody notices breaking.**
 *
 * Every name below must resolve against the server's live tool registry, and that is asserted, not
 * assumed: `tests/allowlist-registry.test.ts` parses the registered names out of the server source
 * (`crates/temper-mcp/src/service.rs`) and fails on any name the registry does not carry. The lists
 * hold the registry's **consolidation grain** — one tool with an `action`/`view` discriminator in
 * place of the families it merged (the four relationship verbs → `relationship`; the envelope's
 * open/close → `invocation_manage` and show/list → `invocation_read`; the context reads →
 * `context_read`; the cogmap reads → `cogmap_read`; the doc-type reads → `describe_schema`). That
 * grain is deliberate: the steward keeps its role's full loop — charter read, edge verbs, envelope —
 * and nothing is granted here beyond what consolidation merged into a tool one of these verbs
 * already used. A consolidation that renames or re-merges a tool these lists rely on now fails CI
 * instead of stranding entries.
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
 * The steward's tools, at the registry's consolidation grain. The excluded tools — the region
 * materialize writes (`cogmap_materialize`, `context_materialize`), genesis/admin/access, and
 * `record_citation_audit` — are role-inappropriate for a steward; `record_citation_audit` is
 * excluded for a stronger reason — see the module doc.
 *
 * `get_profile` was dropped rather than mapped: the registry no longer registers it, and identity
 * is the credential's — the server binds the authenticated principal to every act server-side.
 */
export const STEWARD_TOOLS: readonly string[] = [
  // Authored-4: create, edge-author (assert/fold/retype/reweight via `action`), facet.
  "create_resource",
  "relationship",
  "facet_set",
  // Invocation envelope: open/close via `action`, show/list via `view`.
  "invocation_manage",
  "invocation_read",
  // Steward delta / watermark
  "steward_ingest_delta",
  "steward_advance_watermark",
  // Reads
  "search",
  "get_resource",
  "context_read",
  "list_resources",
  "cogmap_read",
  "describe_schema",
  // Mutations (delete_resource is soft-delete: flips is_active via a resource_deleted event)
  "update_resource",
  "update_resource_meta",
  "delete_resource",
];

/**
 * The citation auditor's tools — READ + VERDICT and nothing else. The allow-list is the persona,
 * stated as capability rather than as instruction:
 *
 * - the **authored-4** (`create_resource`, `relationship`, `facet_set` — four authoring verbs over
 *   three tools since consolidation) is absent — an auditor that can author findings is a citer, and
 *   spec §7's self-audit denial arm should never be the only thing standing between the two roles;
 * - `annotate_resource`, `update_*` and `delete_resource` are absent for the same reason;
 * - `steward_advance_watermark` is absent — the auditor completes ITS job, not the steward's
 *   (`complete_audit_job`, the auditor's own closing act, is an eve-native REST tool under
 *   `subagents/auditor/tools/`, not an MCP tool — the registry witness deliberately does not
 *   see it).
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
  // The map's telos, so a citation is weighed against what the map is for (`view: "charter"`
  // among the six read views the consolidated tool carries).
  "cogmap_read",
  // The invocation envelope (§5.4 — it needs nothing new): open/close via `invocation_manage`,
  // show via `invocation_read`. The consolidated read tool also carries the `list` view — not
  // separately grantable, and accepted: envelopes are the auditor's own accountability record.
  "invocation_manage",
  "invocation_read",
];
