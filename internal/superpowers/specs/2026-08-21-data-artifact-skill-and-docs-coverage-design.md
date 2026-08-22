# Design spec: Data artifact skill and docs coverage

**Status:** design, for approval.
**Task:** [Update skill files and docs playbooks for data artifacts](./01a0265e-85e4-7450-b279-7fedbce59555)
**Goal:** [Structured data is stored as structured data](./01a02163-ba6a-7b00-91f5-5f416e43f4f6)

## Problem statement

The data artifact surface is shipped and exercised end-to-end: substrate → service → API →
client → MCP → CLI. An e2e test (PR #750, merged) closes the loop, and a live
`temper data-artifact commit` → `show` round-trip has been executed against the production DB.

But no skill file, agent-skills projection file, or public docs page teaches an agent how or
why to use data artifacts. The `data-artifact` CLI commands appear only in generated
`--help` text (`docs/reference/cli/data-artifact.md`) and the installed CLI skill's `reference.md`
command table — syntax only, no teaching. Agents do not know the surface exists, what it is
for, or when to reach for it instead of writing fenced JSON into a resource body.

The practice this replaces — agents writing structured data into fenced code blocks inside
resource body markdown — is never explicitly taught either. It persists because there is no
alternative, not because a skill told an agent to do it. The design spec for data artifacts
(`internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md`) is the only
document that names the practice and its three failure modes (btree ceiling, chunker shreds
fences at comment lines, fragments embedded as semantic noise).

## Chosen approach: new shared template + docs playbook

A new shared template file, `crates/temper-cli/templates/shared/data-artifacts.md`, projects to
both skill trees (CLI and MCP) via `temper skill emit`. It sits alongside
`outcome-registers.md` and `memories.md` as a principles-and-practice file — not a command
reference, but a "when to reach for this and why it exists" guide.

A docs playbook page in `docs/playbooks/` provides the worked example: an agent session commits
structured output as a data artifact, and a later session retrieves it and grounds on it.

Cross-references in `knowledge-base.md` (both surfaces) point agents at the new file when they
are about to store structured data.

### Why a shared template

- One source, two projections — `temper skill emit` writes to `agent-skills/temper-knowledge-base/`,
  and `temper skill install` writes to `~/.claude/skills/temper/`.
- The `skills-drift` gate compares the committed projection against source, so a new generated
  file is covered automatically.
- The teaching content is surface-agnostic: the principles (writer ≠ reader, corpus protection,
  selection vocabulary) do not depend on CLI vs MCP. The command table in `reference.md` already
  carries the CLI syntax; the MCP equivalent lives in `knowledge-base.md`'s tool reference.

## Components affected

### 1. New shared template: `crates/temper-cli/templates/shared/data-artifacts.md`

**Responsibility:** teach agents when to commit a data artifact, why the surface exists, and
the selection vocabulary — not the command syntax (which `reference.md` and `knowledge-base.md`
already cover).

**Content outline:**

- **The problem this solves.** Structured data produced by one session is read by a later,
  unrelated session. Today that data lives in fenced code blocks inside resource bodies, and the
  system treats it as prose: the chunker splits it at its own comment lines, embeds the
  fragments into a search corpus built for sentences, and hands the next reader a reassembly
  puzzle. Data artifacts give structured data a home of its own.

- **The why-anchor.** This protects the *next session's* attention. The writer and the reader
  are different actors separated in time — they share no context, cannot negotiate, and the
  reader has only what was stored. Data artifacts exist so that what survives the gap is the
  data itself, whole, not a fence inside prose that the system shredded.

- **When to commit a data artifact vs. writing into a resource body.**
  - **Commit a data artifact when:** the content is structured (JSON, YAML, a computation
    output, a measurement, a query plan), the reader is a later session that needs it whole,
    and embedding it into the search corpus would be semantic noise.
  - **Write into the resource body when:** the content is prose (session notes, problem
    statements, design rationale), the reader is a human or an agent reading for
    understanding, and the content belongs in the searchable corpus.
  - **The trap:** data artifacts are not "just another way to store JSON." The distinction is
    not about format — it is about whether the content is data that a later session must
    retrieve whole, or prose that a reader must understand. A fenced JSON block in a session
    note is prose with a shape; a data artifact is the shape without the prose.

- **The selection vocabulary.** When a resource owns several artifacts of one family, a reader
  determines which to take from the stored record alone:
  - **`kind`** — the bare family name (e.g. `"measurement"`, `"query-plan"`). Free-form, but
    must be consistent within a family.
  - **`intent`** — `current`, `member`, or `pinned`. A closed vocabulary — the system refuses
    an unrecognized intent and the refusal carries the vocabulary.
  - **`precedence`** — ordering among peers. Meaningful for `member`; carried for all.
  - **`supersedes`** — artifact IDs this commit replaces. The system folds superseded
    artifacts out of the default list; `--include-folded` returns them.

- **Shape state.** Every artifact carries a `shape_state`:
  - `never_declared` — no shape has been declared for this family (today, this is the only
    state; the shape registry is future work).
  - `declared_and_satisfied`, `declared_and_not_satisfied`, `declared_and_not_yet_checked` —
    future states, dependent on the shape registry.

  The absence of a shape is a first-class state, not a degraded one. `persistence-never-
  requires-a-prior-declaration` is a clause in the goal register: an actor can commit
  structured data without first declaring its shape, and that is a first-class act.

- **What data artifacts are NOT.**
  - Not searchable. Data artifacts are never found by resemblance or text match. They are
    reached only through the resource that owns them.
  - Not embedded. Data artifact content never enters the search corpus.
  - Not a replacement for `open_meta`. `open_meta` is for small, key-scoped metadata;
    data artifacts are for structured content of any size.

### 2. SKILL.md router rows

Both the MCP SKILL.md (`crates/temper-cli/skill-content/mcp/SKILL.md`) and the CLI SKILL.md
(generated by `temper skill install`) get a row in the supporting-files table pointing to
`data-artifacts.md`.

MCP SKILL.md table addition:
```
| `data-artifacts.md` | **Before storing structured data** — when to commit a data artifact vs. writing into a resource body, and the selection vocabulary |
```

CLI SKILL.md supporting-files list addition:
```
- `data-artifacts.md` — When to commit a data artifact vs. writing into a resource body, selection vocabulary, shape state
```

### 3. Cross-reference in `knowledge-base.md`

Both the MCP `knowledge-base.md` (`agent-skills/temper-knowledge-base/knowledge-base.md`) and
the CLI `knowledge-base.md` (`~/.claude/skills/temper/knowledge-base.md`) get a note in the
resource-writing section pointing agents at `data-artifacts.md` when the content they are
about to store is structured data rather than prose.

The MCP `knowledge-base.md` is hand-written by design (the generated-artifacts skill
documents this: "knowledge-base.md is hand-written by design — the emit never touches it").
So this cross-reference is a manual edit to the hand-written file, not a template change.

The CLI `knowledge-base.md` lives in `~/.claude/skills/temper/knowledge-base.md` and is also
hand-written (not generated from a template — it is CLI-specific).

### 4. Docs playbook: `docs/playbooks/commit-structured-data-as-an-artifact.md`

**Responsibility:** a worked example walking through the commit → show round-trip, framed by
the "writer and reader are different sessions separated in time" narrative spine.

**Content outline:**

- **Outcome:** by the end of this playbook you will have committed structured output as a data
  artifact owned by a resource, retrieved it whole in a later session, and understood when to
  reach for this instead of writing fenced JSON into a resource body.
- **Prerequisites:** Temper installed, authenticated, familiar with contexts and refs.
- **The problem:** why fenced JSON in resource bodies fails (chunker, corpus pollution, no
  shape) — brief, linking to the concepts page if one exists, or self-contained.
- **Commit a data artifact:** `temper data-artifact commit` with a real example (e.g. a
  measurement JSON committed to a task).
- **Retrieve it later:** `temper data-artifact show` and `temper data-artifact list`, showing
  byte-identical retrieval.
- **Selection among many:** `--kind`, `--intent`, `--include-folded`, supersedes folding.
- **When not to use this:** prose belongs in the resource body; data artifacts are for
  structured content a later session must retrieve whole.
- **Further reading:** links to CLI reference, concepts pages.

### 5. Regeneration and gating

After writing the template and SKILL.md changes:

```bash
cargo run -p temper-cli -- skill emit --path agent-skills/temper-knowledge-base
temper skill install --target opencode
```

The `skills-drift` gate (`cargo make check`) compares the committed projection against source.
The `docs-coverage` gate checks link integrity in `docs/`.

The CLI `reference.md` already has the data-artifact command rows (lines 40-42). No change
needed there.

## Key decisions and trade-offs accepted

1. **New shared template over inline in `knowledge-base.md`.** A dedicated file gets its own
   read trigger in the router and does not bloat the tool reference. The teaching content is
   principles-and-practice, not tool syntax — the same shape as `outcome-registers.md` and
   `memories.md`.

2. **Surface-agnostic template.** The teaching content does not name CLI commands or MCP tools
   directly — those live in `reference.md` (CLI) and `knowledge-base.md` (MCP). The template
   teaches *when* and *why*, not *how to type the command*. This is what makes it shareable
   across both surfaces.

3. **`knowledge-base.md` cross-references are hand-written, not templated.** The
   generated-artifacts skill explicitly documents that `knowledge-base.md` is hand-written by
   design. The cross-reference is a small addition to the existing resource-writing section.

4. **No concepts page in `docs/concepts/`.** The playbook is self-contained — the "why" is
   short enough to carry inline. A separate concepts page would split the narrative across two
   pages with no read-order guarantee. If the concept grows, a page can be split out later.

5. **The trap is stated explicitly in the template.** "Data artifacts are not 'just another way
   to store JSON.'" The distinction is about whether the content is data a later session must
   retrieve whole, or prose a reader must understand — not about format. This is the
   why-anchor from the goal register, restated for the skill audience.

## Open questions and risks

- **Shape registry is out of scope.** The template mentions `never_declared` as the only live
  shape state and names the future states, but does not teach how to declare a shape — that
  surface does not exist yet. The registry tenancy question (global vs team-scoped vs
  context-scoped) is the blocking decision for two uncovered clauses and must be ruled before
  the registry is built. This task does not touch it.

- **No existing fenced-JSON-as-data teaching to update.** The explore search found no skill
   file or docs page that explicitly teaches the fenced-JSON practice. The task's acceptance
   criterion ("existing skill sections that teach fenced-JSON-as-data reference the data
   artifact alternative") is satisfied vacuously for the skill surface — no section teaches it.
   The `knowledge-base.md` cross-reference is the proactive version: it teaches agents to reach
   for data artifacts *before* they default to fencing JSON in a body.

- **CLI SKILL.md is generated, not hand-written.** `temper skill install` generates the CLI
   SKILL.md from the source templates plus per-user context config. The supporting-files list
   in the CLI SKILL.md is part of the generated output, so adding `data-artifacts.md` to the
   template source is what propagates it. Verify after `temper skill install` that the row
   appears.