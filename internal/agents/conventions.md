# Branch, Commit & Feature Flag Conventions

> Shared agent guidance — the source of truth for `AGENTS.md` and `CLAUDE.md`.

## Branch and Commit Conventions

These patterns are observed in recent history rather than rigidly enforced. Match the existing style when in doubt.

### Branch naming

`<initials>/<scope>` — current author uses `jct/<scope>` with kebab-case scope. Examples: `jct/wave1-phase3a-dbbackend-foundation`, `jct/post-cloud-only-qol-trivial-trio`. Keep scopes terse but specific enough to disambiguate parallel branches.

### Commit and PR title prefixes

| Prefix | Use for |
|--------|---------|
| `wave N phase X[a]:` or `Wave N Phase X:` | Numbered phases inside a multi-PR feature plan |
| `cloud-only(<scope>):` | Commits in a multi-chunk migration; `<scope>` is the chunk or PR-letter |
| `QoL:` | Polish, ergonomics, dead-code drops, small cleanups |
| `post-PR-<n>:` | Follow-up to review feedback on PR #n that didn't land inline |
| `audit:` | Output of an audit sweep — rationalization comments, threading fixes |
| `fix(<scope>):` / `refactor(<scope>):` / `docs(<scope>):` / `test:` / `chore:` / `mcp:` | Conventional-Commits style for narrow scoped changes |

Self-contained features sometimes use a plain narrative title with no prefix (e.g. "Limb 1 — relationship events + edge projection", "Add offline_access scope and refresh_token grant support"). That's fine when the PR is its own story; reach for a prefix when the change is one beat of a longer arc.

### Bundling fixes into the PR that surfaced them

If a fix's story is "this PR's tests / new code path surfaced a pre-existing bug," bundle it into the same PR rather than extracting. The narrative stays cohesive: one PR, one explanation. Examples in history: PR #69 bundled the empty-body dedup fix into Phase 3a's PR because workspace feature unification first exposed it under that test suite.

Conversely, if the fix is unrelated to the PR's narrative — even if you noticed it while working — extract it. Mixed-narrative PRs are harder to review and harder to revert.

## Feature Flags

Rust crates use feature flags to gate heavy dependencies:
- `test-db` — enables database integration tests (temper-api, tests/e2e)
- `test-embed` — enables embedding tests (temper-ingest)
- `embed` / `extract` — gates ONNX and kreuzberg dependencies (temper-ingest)
- `web-api` — enables utoipa OpenAPI derives (temper-core)
- `typescript` — enables ts-rs type generation (temper-core)
- `mcp` — enables schemars JsonSchema derives for MCP tool parameters (temper-core)
- `artifact-tests` — enables temper-substrate's **scenario write-path** integration tests (bootseed, seed/scenario load + roundtrip + equivalence, charter, content, ledger, replay) plus ONNX. Tests run on ephemeral `public`-schema databases via `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` — each test gets its own isolated database. CI runs it in its own **Substrate Artifact Tests** job (a distinct feature set, so it cannot fold into the `--workspace` integration run); run locally with **`cargo make test-artifacts`**. temper-substrate's pure core tests (affinity, cluster) are ungated and run in CI.
- `scenario-schema` — enables `schemars::JsonSchema` derives for temper-substrate's **two** JSON-Schema snapshot suites: `tests/scenario_schema.rs` (the scenario YAML model) and `tests/payload_schema.rs` (the **event payload wire contract** — the boot-seed stamps those fixtures into `kb_event_types.payload_schema`, so repo == registry == Rust types). Runs in the **Unit** CI job and via **`cargo make test-schema`** (which `cargo make test` depends on). Regenerate with `UPDATE_SCHEMA=1 cargo make test-schema`.

  > **Run it package-scoped — `-p temper-substrate`, never `--workspace`.** Feature unification changes the emitted schema; `-p` is what the regen emits and what the boot-seed stamps. See [crates/temper-substrate/CLAUDE.md](../../crates/temper-substrate/CLAUDE.md).
## PR descriptions

Reviewers here are the author, agents, and anyone reading a public repository. Write for someone who
will read the diff — not for someone reconstructing the session.

**Four sections, and `Approach` is optional:** what changed, why, why this shape rather than the
obvious alternative, and how it was verified. `.github/pull_request_template.md` carries the prompts.

**The code carries the detail.** A rationale long enough to need a PR essay belongs in the migration
header or the doc comment, next to the thing it explains, where it survives the PR being merged and
forgotten. Link to it; don't restate it. A description that can be read in under a minute and a
codebase that explains itself are the same goal approached from two ends.

**No session narrative.** What was tried first, which probe was wrong, what an agent reported, how
many rounds it took. That belongs in the session note. A PR description is a claim about the code,
not a record of the work.

**A description is not a disclosure surface.** Specs, plans, and gap inventories live in
`tasker-systems/temper-artifacts` — that is why they were moved there, and why
`.github/scripts/check-no-process-artifacts.sh` keeps them out of this tree. The same reasoning
applies to the description itself, plus: no production identifiers, no tenant data, no operational
state of a running deployment. This is not obscurity as a control; it is declining to publish a map
of where to push. Reference a task by id when a reviewer needs the context.

**Scope statements are the exception, and they are welcome.** "Cost only; behavior unchanged", "the
read path is untouched", "this leaves X alone deliberately" — reviewers infer coverage from silence,
so one line naming what a change does *not* do is worth a paragraph of caveats. That is a statement
about this PR's boundary, which is different from an inventory of what remains weak.

## Migration comments

Migrations in this repo argue for themselves, and that is right — the reasoning is what stops the
next edit from removing a load-bearing clause. But an eighty-line header above ninety lines of SQL
makes the change itself hard to find, and a header nobody scrolls past is worse than a short one.

**Three places, three different readers. Do not say the same thing in all of them.**

| Where | Reader | Carries |
|---|---|---|
| File header | someone reading the **diff** | why this migration exists; what a later edit must not break |
| `COMMENT ON …` | someone at `\d`/`\df+` with **no git access** | what the object does *now*, and what changed it |
| `declare_migration` reason | the **ledger**, and deploy-skew review | the `additive` / `shape-breaking` argument |

Most bloat is the same paragraph appearing in all three. Write it once, in the one whose reader needs
it.

**Keep the header near 25 lines.** If the argument genuinely needs more, it is a spec: put it in
`temper-artifacts` and leave a one-line pointer. In the header, prefer:

- **The constraint over the story.** `MATERIALIZED IS LOAD-BEARING, NOT STYLE` earns its lines. How
  the problem was discovered does not.
- **The number over the evidence.** "97% of the runtime, measured" — not the pasted `EXPLAIN` plan.
  Plans, corpus statistics and query output belong in the spec.
- **A version citation over a restatement.** `20260727000010` already argues why `scored` is
  materialized; cite it. The repo's existing habit of naming migration versions inline is what makes
  this work, and it is cheaper than paraphrasing.
- **No enumerated future work.** "Not fixed here, and each is its own task" is a gap inventory in a
  public tree. A single caveat naming a real trap next to the code it traps is fine; a list is a
  backlog, and backlogs live in the vault.
