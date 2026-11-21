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

## The dev roadmap

temper's roadmap is a dated snapshot in `+temper-community/dev-roadmap`, drawn from the goals in
`+temper-dev/core` and the work items in `+temper-dev/core-workflows` by `tools/roadmap/snapshot.sh`
(`--dry-run` to print it; ROAD-1 to ROAD-6 in the conventions baseline's `doctrine/roadmap.md`).
Its terms, per Temper decision 01a1ecb9:

- **Claimed:** the work item carries `open_meta.claim_id`. **Ready:** `open_meta.state: "ready"`.
  **Filed:** `temper-stage` is `backlog` or `in-progress`. **Open goal:** `temper-status` is
  `active` or `paused`.
- **The grouping key (ROAD-3):** `open_meta.area`, one per open goal, from this vocabulary, in
  this order: `identity-and-access`, `surfaces`, `reliability`, `release-and-deploy`,
  `reads-and-search`, `interop`, `knowledge-model`, `records-and-ledger`, `privacy`,
  `ingest-and-writes`, `process`, `ui`.
- **The public line (ROAD-6):** the roadmap's readers are wider than the goals' readers, so a goal
  is shown only under its owner-written `open_meta.public_title`, with `open_meta.public_outcome`
  as the line under it, both stating the outcome and not a current weakness. A goal missing either
  is withheld and counted, never named. No work item is named.

`tools/roadmap/test.sh` checks the vocabulary above against the tool.
