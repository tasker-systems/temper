# Conventions

The shared engineering baseline is the projected
[tasker-systems-conventions](../../.claude/skills/tasker-systems-conventions/SKILL.md) skill. It has
no branch, commit or PR rules; these are temper's own, and all are convention unless a gate is
named.

## Branches and commits

- Branches are `jct/<kebab-scope>`.
- Titles use Conventional Commits (`fix(<scope>):`, `docs:`, `refactor:`, `test:`, `chore:`), or a
  plain narrative title when the PR is its own story.
- A fix the PR's own work surfaced goes in that PR; an unrelated fix gets its own. *Why:* one PR,
  one story, one revert.

## PR descriptions

What changed, why, why this shape (optional), and how it was verified —
`.github/pull_request_template.md` carries the prompts. Write for someone reading the diff:

- **No session narrative.** That belongs in the session note.
- **Detail lives in the code** — a migration header or doc comment, linked, not restated.
- **No disclosure.** No production identifiers, tenant data or deployment state; specs and gap
  inventories stay in `temper-artifacts`. *(gated for trees: `check-no-process-artifacts.sh`)*
- **Scope statements are welcome** ("the read path is untouched"): reviewers infer coverage from
  silence.

## Migration comments

Three places, three readers — say each thing once:

| Where | Reader | Carries |
|---|---|---|
| File header (~25 lines) | the diff reader | why it exists; what a later edit must not break |
| `COMMENT ON …` | someone at `\d` without git | what the object does now |
| `declare_migration` reason | the ledger and deploy review | the additive / shape-breaking argument |

Prefer the constraint over the story, the number over the evidence, and a migration-version
citation over a restatement. A longer argument is a spec in `temper-artifacts`; no enumerated
future work in a public tree.

## Feature-gated tests

Features are declared in each crate's `Cargo.toml`. The traps:

- `cargo make test-e2e` enables only `test-db`, so it compiles out every `test-embed` test; CI
  enables both. Use `cargo make test-e2e-embed` when touching ingest or push-body code.
- `scenario-schema` snapshots must run `-p temper-substrate`, never `--workspace`: feature
  unification changes the emitted schema. See `crates/temper-substrate/CLAUDE.md`.
- `artifact-tests` runs in its own CI job (`cargo make test-artifacts`) because its feature set
  cannot fold into the workspace run.
