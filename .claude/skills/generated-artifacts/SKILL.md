---
name: generated-artifacts
description: Regenerating the router-derived artifacts (openapi.json, the temper-rb gem, temper-ts's schema.ts), the ts-rs TypeScript type trees, and the committed agent-skills/ projection. Use when a response DTO, route, ts-rs-derived Rust type, or shared skill template changes, or when `cargo make check` fails on openapi-check, openapi-rb-drift, openapi-ts-drift, ts-rs-drift, or skills-drift.
---

# Generated artifacts: OpenAPI SDKs, ts-rs type trees, and the agent-skills projection

Three independent codegen pipelines hang off the Rust source. All are gated by
`cargo make check`, and all fail in ways that read as "you forgot to regenerate"
when in fact you forgot to `git add` — or, for two of them, to `git commit`.

## OpenAPI + the temper-rb gem + temper-ts's `schema.ts` are all products of the router

A new/changed response DTO (a new field, a renamed type) restales **three** committed
artifacts: `openapi.json`, the generated Ruby gem under
`clients/temper-rb/lib/temper/generated`, and `clients/temper-ts/src/generated/schema.ts`
(emitted by `openapi-typescript`, pinned exactly — no caret — in temper-ts's
devDependencies).

```bash
cargo make openapi   # regenerates all three in one step
```

Gem regen needs Docker; the TS schema needs only Node.

`cargo make check` gates all three: `openapi-check` (spec), `openapi-rb-drift` (gem —
Docker-based, **skips** without Docker; the `test-ruby` CI job is the never-skipping
backstop), and `openapi-ts-drift` (schema — and unlike the gem's gate, this one **never
skips**: `openapi-typescript` needs only Node, so there is no environment in which
`cargo make check` would rather guess than check). Never assume that because one SDK's
gate is best-effort, the other is too — they have different skip semantics for different
reasons, and `openapi-ts-drift` is the strict one.

The generator pin + params for the gem live in one place —
`.github/scripts/generate-temper-rb.sh` — shared by cargo-make and the gem's Rakefile;
the TS equivalent is `.github/scripts/generate-temper-ts.sh`, shared by
`cargo make openapi-ts`, `check-temper-ts-drift.sh`, and the `test-agents-ts` CI job's
drift step. `detect-ci-scope.sh` carries `^openapi\.json$` in **both** `test-ruby`'s and
`test-agents-ts`'s trigger sets, for the identical reason: a contract change that does not
run the job whose gate catches the stale artifact is a gate that runs nowhere.
(`test-agents-ts` got this later than `test-ruby` did — the same rot the gem discovered in
`tests/contracts/`.)

### The drift gates compare against git, not against a fresh build — and they do not all want the same thing

Both `check-temper-rb-drift.sh` and `check-temper-ts-drift.sh` regenerate their artifact
and then run `git diff --exit-code` over it. So an artifact you have *just correctly
regenerated* still fails `cargo make check` while it sits unstaged — the error reads
"generated core/schema is out of date with openapi.json", which sounds like you forgot to
run `cargo make openapi` when in fact you need to `git add` its output. Stage the
regenerated files, then re-run `check`.

**`check-ts-rs-drift.sh` needs a `git commit`, not just a `git add`** — do not generalize
the paragraph above onto it. `git diff --exit-code` compares the worktree against the
*index*, so staging satisfies the two SDK gates. The ts-rs gate cannot use that form (a
newly derived, untracked `.ts` is invisible to it — that is the `slack_link.ts` incident
in its header), so it uses `git status --porcelain`, which reports **staged-vs-HEAD**
changes as well. A correctly regenerated tree therefore keeps failing `cargo make check`
after `git add` and only goes green once committed. The tell is a `M ` line — `M` in the
first column, space in the second — in the gate's own output: that is "staged, worktree
clean", i.e. the content is already right and the commit is what is missing.

The three gates having three different git comparisons is not an inconsistency to tidy;
each is the weakest form that still catches its artifact's failure mode. But it does mean
"stage it and re-run check" is only two-thirds of the story.

## `generate-ts-types` writes TWO trees, and both are gated

Besides temper-ui's `src/lib/types/generated/`, it emits
`packages/agent-workflows/mention/agent/generated/` — the mention agent is
workspace-isolated (not a bun `workspaces` member), so no import path reaches temper-ui's
tree and it needs its own copy. That export is **filtered to one type**
(`export_bindings_linkrefusal`) because ts-rs exports each type's transitive closure: two
files instead of the 36 an unfiltered crate export deposits. It also runs with
`ts-rs/import-esm`, since the agent is `moduleResolution: NodeNext` where an
extensionless relative import is TS2835 — a flag temper-ui neither needs nor gets, so the
two trees differ in import style **by design**.

`check-ts-rs-drift.sh` (task `ts-rs-drift`, in `cargo make check` and the `rust-quality`
CI job) regenerates every tree and fails on any difference. It derives the tree list from
main.toml's `TS_RS_EXPORT_DIR` lines, so a third consumer is covered with no edit — and
refuses to run over zero trees rather than passing having checked nothing. It uses
`git status`, not `git diff --exit-code`, because the diff form cannot see a **newly
generated untracked** file: temper-ui's `slack_link.ts` sat in exactly that state for a
full PR cycle. **This is the repo's only cross-LANGUAGE gate** — every other check lives
inside one language, which is how PR #498 merged with `tsc` clean and 79/79 tests green
while the agent spoke a retired wire contract. It does **not** cover the wire `status`
tags: those live in temper-api, which has no ts-rs (task
`019f910b-579b-74c2-bf05-702aaed0a011`).

## `agent-skills/` is a committed projection of the shared skill templates

`agent-skills/temper-knowledge-base/` is the installable skill for **MCP-only clients** (Claude
Desktop, claude.ai). It is not hand-maintained: `temper skill emit --path
agent-skills/temper-knowledge-base` regenerates it from `crates/temper-cli/templates/shared/*` (surface-
parameterized, `cli` vs `mcp`), `crates/temper-cli/skill-content/mcp/SKILL.md`, and — for
`references/frontmatter.md` — `temper_workflow`'s embedded JSON Schemas plus the
`ManagedMeta` wire type.

```bash
cargo run -p temper-cli -- skill emit --path agent-skills/temper-knowledge-base
cargo make skill-package   # …and wrap it as dist/temper-skill-v<ver>.zip
```

Gated by `skills-drift` (`check-skills-drift.sh`, in `cargo make check` and the
`rust-quality` CI job), with its harness in the pure-bash `guard-tests` job.

**Three things that will catch you out:**

- **It wants a `git commit`, not just a `git add`** — same `git status --porcelain` form
  as the ts-rs gate, for the same reason (a newly generated untracked file is invisible to
  `git diff`). The `M ` tell in the gate's own output means the same thing here.
- **It builds from source, never the installed `temper`.** This repo deliberately keeps a
  *stale* installed CLI for parity validation, so a gate shelling out to it would compare
  the committed projection against whatever is on PATH. Do not "simplify" it to `temper
  skill emit`.
- **Only the GENERATED files are covered.** `knowledge-base.md` (the MCP tool reference) is
  hand-written by design — the emit never touches it, so a false statement in it is invisible to
  this gate. A green run means "the generated part matches its source", not "the MCP skill is
  correct". The tool-NAME half of that gap is closed from the other side, by
  `temper-mcp`'s `every_tool_the_shipped_skill_names_exists_in_the_router` — the router is the
  authority and temper-cli does not depend on temper-mcp.

**Why the CLI skill is not projected too.** `temper skill install` bakes a per-user context
list into its router, so its render is one developer's and could never be committed. The MCP
skill has no such input, which is exactly what makes it pinnable — `emit` reads no config at
all (proven by the fact that it succeeds where `install` errors with "global config not
found"). If you ever add a config-derived value to the MCP tree, you have silently removed
its ability to be gated.

**Do not template a file that does not diverge.** `implementation-grounding.md`,
`plan-verification.md` and `subagent-guidance.md` name no command on either surface, so they
ship to both trees as the same bytes rather than as two renders of one template. A seam with
nothing on either side of it is cost without benefit. Conversely `cognitive-maps.md` and
`teams.md` are CLI-shaped throughout — they are **declared absent** from the MCP tree in its
SKILL.md rather than shipped wrong.

### Packaging it for upload

`cargo make skill-package` → `dist/temper-skill-v<ver>.zip` (gitignored; the release publishes it,
nothing commits it). Shared with the release workflow via
`.github/scripts/release/package-skill.sh`, the same one-definition pattern as
`generate-temper-rb.sh`. Its harness is `test-package-skill.sh` in `guard-tests`.

Three constraints the packager enforces because they fail late and unhelpfully otherwise:

- **The skill folder is the archive root** (`x.zip` → `temper-knowledge-base/` → `SKILL.md`).
  Files at the archive root are rejected by the uploader.
- **The folder name must equal SKILL.md's `name:`.** They live in two files, so a half-done rename
  uploads a skill whose folder and identity disagree.
- **The version is stamped at packaging time**, into the archive name and a `VERSION` file inside
  the bundle — never into the source tree. `update-version.sh` regenerates nothing, so a
  version-carrying tree would go stale on every `release-prepare` and red the drift gate.

Read [docs/guides/releasing.md](../../../docs/guides/releasing.md#the-agent-skill-bundle) before
touching the artifact's NAME: it is what keeps the bundle out of the manifest guard's glob.
