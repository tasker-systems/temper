export const meta = {
  name: 'code-quality-audit',
  description: 'Fan-out audit of the Rust crates against the code-quality best-practices lens; adversarially verify, then classify findings into a PR-sized work-breakdown.',
  whenToUse: 'When auditing the codebase against an opinionated best-practices rubric (the CQ-* lens, or any rubric-shaped doc) and you want a verified, classified, broken-up backlog rather than one mega-refactor PR.',
  phases: [
    { title: 'Audit', detail: 'one read-only auditor per crate area, emits rule-tagged findings', model: 'sonnet' },
    { title: 'Verify', detail: 'adversarial re-check of each area\'s findings, drops false positives', model: 'sonnet' },
    { title: 'Aggregate', detail: 'classify by rule/pattern, break into PR-sized chunks' },
  ],
}

// ─────────────────────────────────────────────────────────────────────────────
// Lens-parametrized: swap RUBRIC_DOC + RUBRIC + the rule_id enum for a SEC-* doc
// and this same harness runs a security best-practices sweep. `args` may override
// { rubricDoc, units } to retarget without editing the script.
// ─────────────────────────────────────────────────────────────────────────────

const RUBRIC_DOC = (args && args.rubricDoc) || 'docs/development/code-quality-best-practices.md'

const RULE_IDS = [
  'CQ-1', 'CQ-2', 'CQ-3', 'CQ-4', 'CQ-5', 'CQ-6', 'CQ-7',
  'CQ-8', 'CQ-9', 'CQ-10', 'CQ-11', 'CQ-12', 'CQ-13', 'CQ-14',
]

const RUBRIC = `
CQ-1  Single responsibility / fn length: a fn past ~60 lines, or one that reads "phase 1 … phase 2 …", or whose summary needs an "and". (A flat match over many variants is NOT a violation.)
CQ-2  Keys unique-by-construction, not loose markers: diff/dedup/join on a non-unique marker (e.g. origin_uri); identity keys not injected from one typed source on both sides of a boundary.
CQ-3  Parse don't validate: bare Uuid/String where a newtype belongs; revalidation scattered downstream; &String/&Vec<T> params instead of &str/&[T].
CQ-4  Names carry responsibility: a TYPE named Manager/Helper/Service/Factory/Util; a stringly-typed match "literal" over a bounded set the code owns. (Established MODULE names like services/backend are fine.)
CQ-5  Params structs: >5 domain params on a fn; #[expect(clippy::too_many_arguments)].
CQ-6  Error handling & escalation: .unwrap()/.expect() on a fallible runtime value in a library path; write-then-check (auth after mutation); panic for a recoverable condition; a softened/removed assertion.
CQ-7  Lint & suppression discipline: bare #[allow] (vs #[expect(reason=…)]); a public type without Debug; a magic constant with no explaining comment.
CQ-8  No "for now"/premature compat/abstraction: dead code kept "for compat"; a placeholder/"for now" workaround; a one-use abstraction.
CQ-9  Typed structs over inline JSON: serde_json::json!() for data with a known shape.
CQ-10 Shared types at boundaries: a hand-mirrored type duplicating a ts-rs-generated one.
CQ-11 Persistence is its own layer: inline sqlx::query!() in a handler/MCP tool/CLI action; persistence CRUD interleaved with behavior code.
CQ-12 Auth before writes / profile scoping: a data query not scoped via resources_visible_to/can_modify_resource; a mutation before its authz check.
CQ-13 SQL discipline: runtime query where a macro works (the ::vector unified_search is the allowed exception); multi-table JOINs copy-pasted across fns (→ view); filter predicates duplicated across queries (→ shared builder).
CQ-14 Testing: a removed/weakened assertion; a missing test-db/test-embed gate on #[sqlx::test]/embed tests; a direct-call test where a production-caller e2e is needed; a non-descriptive test name.
`.trim()

// Audit units — "by area", one auditor per unit. Big crates split by module dir so
// each auditor's scope stays tractable. `topLevel` means the *.rs files directly under
// the crate's src/ (not in a subdir already covered by another unit).
const DEFAULT_UNITS = [
  { key: 'cli-actions', paths: ['crates/temper-cli/src/actions/'] },
  { key: 'cli-commands', paths: ['crates/temper-cli/src/commands/'] },
  { key: 'cli-rest', paths: ['crates/temper-cli/src/cloud_backend/', 'crates/temper-cli/src/output/', 'crates/temper-cli/src/templates/'], topLevel: 'crates/temper-cli/src/' },
  { key: 'substrate-core', paths: [], topLevel: 'crates/temper-substrate/src/' },
  { key: 'substrate-readback', paths: ['crates/temper-substrate/src/readback/'] },
  { key: 'substrate-scenario', paths: ['crates/temper-substrate/src/scenario/'] },
  { key: 'workflow-frontmatter', paths: ['crates/temper-workflow/src/frontmatter/'] },
  { key: 'workflow-ops', paths: ['crates/temper-workflow/src/operations/', 'crates/temper-workflow/src/types/'], topLevel: 'crates/temper-workflow/src/' },
  { key: 'api-handlers', paths: ['crates/temper-api/src/handlers/'] },
  { key: 'api-services', paths: ['crates/temper-api/src/services/'] },
  { key: 'api-backend', paths: ['crates/temper-api/src/backend/', 'crates/temper-api/src/middleware/'], topLevel: 'crates/temper-api/src/' },
  { key: 'client', paths: [], topLevel: 'crates/temper-client/src/' },
  { key: 'core', paths: [], topLevel: 'crates/temper-core/src/' },
  { key: 'mcp', paths: [], topLevel: 'crates/temper-mcp/src/' },
  { key: 'ingest-agents', paths: [], topLevel: 'crates/temper-ingest/src/' },
  { key: 'agents', paths: [], topLevel: 'crates/temper-agents/src/' },
]

const UNITS = (args && Array.isArray(args.units) && args.units.length) ? args.units : DEFAULT_UNITS

function scopeLines(u) {
  const lines = (u.paths || []).map((p) => `  - ${p} (recurse)`)
  if (u.topLevel) lines.push(`  - ${u.topLevel} — ONLY the *.rs files directly in this dir (subdirs are covered by other units)`)
  return lines.join('\n')
}

const findingProps = {
  rule_id: { type: 'string', enum: RULE_IDS },
  file: { type: 'string' },
  span: { type: 'string', description: 'line number or range, e.g. "120-180"' },
  severity: { type: 'string', enum: ['high', 'medium', 'low'] },
  effort: { type: 'string', enum: ['S', 'M', 'L'] },
  pattern_tag: { type: 'string', description: 'short kebab slug grouping similar findings repo-wide' },
  evidence: { type: 'string', description: '1-2 line excerpt or precise description' },
  why: { type: 'string' },
  suggested_fix: { type: 'string' },
}

const FINDINGS_SCHEMA = {
  type: 'object',
  properties: {
    unit: { type: 'string' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: findingProps,
        required: ['rule_id', 'file', 'span', 'severity', 'effort', 'pattern_tag', 'why'],
      },
    },
  },
  required: ['unit', 'findings'],
}

const VERIFY_SCHEMA = {
  type: 'object',
  properties: {
    results: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          ...findingProps,
          verdict: { type: 'string', enum: ['real', 'false_positive', 'justified'] },
          verify_note: { type: 'string' },
        },
        required: ['rule_id', 'file', 'verdict', 'verify_note'],
      },
    },
  },
  required: ['results'],
}

const REPORT_SCHEMA = {
  type: 'object',
  properties: {
    summary: { type: 'string', description: '3-5 sentences: overall health, dominant patterns, systemic vs one-off split' },
    by_rule: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          rule_id: { type: 'string', enum: RULE_IDS },
          count: { type: 'number' },
          systemic: { type: 'boolean', description: 'recurs across ≥3 files/units' },
          note: { type: 'string' },
        },
        required: ['rule_id', 'count', 'systemic'],
      },
    },
    hotspots: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          file: { type: 'string' },
          count: { type: 'number' },
          rule_ids: { type: 'array', items: { type: 'string' } },
        },
        required: ['file', 'count'],
      },
    },
    work_breakdown: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          title: { type: 'string' },
          rule_ids: { type: 'array', items: { type: 'string' } },
          pattern_tags: { type: 'array', items: { type: 'string' } },
          files: { type: 'array', items: { type: 'string' } },
          rationale: { type: 'string' },
          est_effort: { type: 'string', enum: ['S', 'M', 'L', 'XL'] },
          pr_scope_note: { type: 'string', description: "what's in and out of this PR-sized chunk" },
          priority: { type: 'string', enum: ['high', 'medium', 'low'] },
        },
        required: ['title', 'rationale', 'est_effort', 'priority'],
      },
    },
  },
  required: ['summary', 'by_rule', 'work_breakdown'],
}

function auditPrompt(u) {
  return `You are auditing part of the Temper Rust codebase against the project's CODE-QUALITY BEST-PRACTICES lens. This is a READ-ONLY audit — do NOT edit any files.

AUDIT UNIT: ${u.key}
SCOPE — audit ONLY these paths:
${scopeLines(u)}

First read the full rubric (rationale + worked examples):
  ${RUBRIC_DOC}

Then audit every Rust file in scope. Rule index — cite exactly ONE rule_id per finding:

${RUBRIC}

METHOD:
- Use grep + targeted reads. Be thorough but precise: report only REAL violations you can point to with file + line span.
- An empty findings list is a valid, HONEST result. Do NOT invent findings to seem thorough.
- Do NOT report nits the rubric doesn't cover. When unsure whether something clears the bar, leave it for the verifier — but prefer precision over volume.
- pattern_tag: a short kebab slug grouping similar findings across the repo (e.g. "phase-bundling-fn", "bare-allow", "inline-sql-in-surface", "stringly-typed-match", "bare-uuid-param"). Reuse obvious tags so aggregation can group them.
- severity: high = correctness-adjacent or an invariant breach (CQ-9..CQ-13); medium = a clear smell; low = minor.
- effort: S (<30min), M (a focused change), L (multi-file/structural).

Return structured findings (rule_id, file, span, severity, effort, pattern_tag, evidence, why, suggested_fix).`
}

function verifyPrompt(u, findings) {
  return `You are an ADVERSARIAL VERIFIER for a code-quality audit of Temper. For each candidate finding below, independently re-open the cited code and decide: REAL violation, FALSE_POSITIVE, or JUSTIFIED deviation (real pattern but defensible here).

Default to SKEPTICISM — best-practice findings are opinion-laden. Specifically:
- An established MODULE name (services/backend/handlers) is NOT a CQ-4 weasel-word; only a poorly-named TYPE is.
- A long FLAT match/dispatch over many variants is NOT a CQ-1 violation; bundled sequential phases are.
- A runtime sqlx query forced by a ::vector cast or dynamic ORDER BY is NOT a CQ-13 violation.
- An auth check that IS present before the write is NOT a CQ-12 violation.
- .expect() guarding a genuine just-established invariant (with a reason message) is NOT a CQ-6 violation.

Read the rubric for the bar: ${RUBRIC_DOC}

CANDIDATE FINDINGS (unit ${u.key}):
${JSON.stringify(findings, null, 2)}

Open each cited file/span and confirm. Return results: the SAME finding objects, each augmented with:
- verdict: 'real' | 'false_positive' | 'justified'
- verify_note: one line on why.
You may correct file/span/severity if the original got them wrong. Be decisive.`
}

function aggregatePrompt(confirmed) {
  return `You are synthesizing a code-quality audit of Temper's Rust crates into an ACTIONABLE work-breakdown. The explicit goal: NOT one giant refactor PR — break the work into independently reviewable, PR-sized chunks.

CONFIRMED FINDINGS (already adversarially verified as real — ${confirmed.length} total):
${JSON.stringify(confirmed, null, 2)}

Produce a structured report:
- summary: 3-5 sentences — overall health, the dominant patterns, the systemic-vs-one-off split.
- by_rule: one row per CQ rule that has ≥1 finding — { rule_id, count, systemic (true if it recurs across ≥3 files/units), note }.
- hotspots: the files with the most findings — { file, count, rule_ids }.
- work_breakdown: the actual plan. Each chunk = ONE coherent, PR-sized unit of work, independently reviewable and revertable:
  { title, rule_ids, pattern_tags, files, rationale, est_effort (S/M/L/XL), pr_scope_note (what's in/out), priority }.
  - Group by PATTERN when the fix is mechanical and repo-wide (e.g. "replace bare #[allow] with #[expect(reason=…)] across N files").
  - Group by AREA when the fix needs subsystem context (e.g. "decompose the 3 oversized fns in temper-cli/src/commands/resource.rs").
  - Prefer MANY SMALL chunks over a few large ones. Order by priority, then value.`
}

// ── Run ──────────────────────────────────────────────────────────────────────
log(`Code-quality audit: ${UNITS.length} units against ${RUBRIC_DOC}`)

phase('Audit')
const perUnit = await pipeline(
  UNITS,
  (u) => agent(auditPrompt(u), { label: `audit:${u.key}`, phase: 'Audit', schema: FINDINGS_SCHEMA, agentType: 'Explore', model: 'sonnet' }),
  (audit, u) => {
    const findings = (audit && audit.findings) || []
    if (!findings.length) return { unit: u.key, results: [] }
    return agent(verifyPrompt(u, findings), { label: `verify:${u.key}`, phase: 'Verify', schema: VERIFY_SCHEMA, agentType: 'Explore', model: 'sonnet' })
      .then((v) => ({ unit: u.key, results: (v && v.results) || [] }))
  },
)

const confirmed = perUnit
  .filter(Boolean)
  .flatMap((p) => (p.results || []).filter((r) => r.verdict === 'real'))

log(`${confirmed.length} confirmed findings across ${UNITS.length} units → aggregating`)

phase('Aggregate')
const report = confirmed.length
  ? await agent(aggregatePrompt(confirmed), { label: 'aggregate', phase: 'Aggregate', schema: REPORT_SCHEMA })
  : { summary: 'No confirmed findings — the audited crates are clean against the current lens.', by_rule: [], hotspots: [], work_breakdown: [] }

return { rubric_doc: RUBRIC_DOC, unit_count: UNITS.length, confirmed_count: confirmed.length, confirmed, report }
