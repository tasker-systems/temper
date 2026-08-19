# Docs Restructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `docs/` contain only public documentation, so the Apidog sync is safe by construction rather than by configuration.

**Architecture:** Pure file relocation plus reference repair. Everything that is not public documentation moves to a new top-level `internal/`; `docs/cognitive-maps/` retires because temperkb.io supersedes it. Three of the moves are coupled to tooling that hardcodes the old path, and those couplings are repaired in the same task as their move. No application code changes.

**Tech Stack:** `git mv`, bash, ripgrep. No compilation involved — but `cargo make check` must stay green, because two of the moved paths are named by CI scripts.

**Spec:** [`docs/superpowers/specs/2026-08-19-docs-surface-rebuild-design.md`](../specs/2026-08-19-docs-surface-rebuild-design.md)

**This is plan 1 of 3.** It covers spec work-order steps 2, 3 and 4. Plan 2 covers step 6 (`reference/` generation) and step 8 (`docs-coverage`). Plan 3 covers steps 5 and 7 (the authoring passes). Step 1 (unpublish) is Pete's action in Apidog and is not in any plan.

## Global Constraints

- **Applied migrations are immutable.** 35 files under `migrations/` cite `docs/superpowers/...` paths in comments. They must **not** be edited — editing an applied migration trips the sqlx checksum and fails `db-migrate`. Their citations stay stale by decision; Task 1's `internal/README.md` records why.
- **Use `git mv`, never `mv` + `git add`.** Rename detection is what keeps 472 files reviewable as renames rather than as 472 deletions and 472 creations.
- **`docs/diagrams/` does not move.** `README.md` embeds four of those SVGs with `<img src="docs/diagrams/...">`. They are public assets and satisfy the invariant where they are.
- **The invariant being established:** everything in `docs/` is public; nothing else lives there.
- Commit after every task. Never batch two tasks into one commit.

---

### Task 1: Redirect the tooling and scaffold `internal/`

This task comes first for a reason: superpowers' `brainstorming` and `writing-plans` skills hard-code `docs/superpowers/{specs,plans}`. If the move lands before the redirect, the next session that writes a spec re-creates the directory this plan exists to remove.

**Files:**
- Create: `internal/README.md`
- Modify: `CLAUDE.md`
- Modify: `AGENTS.md`
- Modify: `.claude/skills/temper/guidance/fundamentals.md`

**Interfaces:**
- Consumes: nothing.
- Produces: the `internal/` directory and the stated preference every later task and future session relies on.

- [ ] **Step 1: Confirm the current stated location, so the edit is targeted**

```bash
grep -rn "docs/superpowers" CLAUDE.md AGENTS.md .claude/skills/temper/guidance/fundamentals.md
```

Expected: at least one hit in `fundamentals.md` (it names `docs/superpowers/specs/` as the spec location). `CLAUDE.md` and `AGENTS.md` may have none — that is fine; Step 3 adds the statement rather than editing one.

- [ ] **Step 2: Write `internal/README.md`**

```bash
mkdir -p internal
cat > internal/README.md <<'EOF'
# internal/

Engineering material that is **not** public documentation: process artifacts
(plans, specs, reviews), contributor guides, audits, and generated internal
projections.

## Why this directory exists

`docs/` is synced to the public documentation site. The invariant that keeps
that safe is structural rather than configured:

> **Everything in `docs/` is public. Nothing else lives there.**

An allowlist is configuration that has to be got right every time, and it was
got wrong: internal security audits were published because the tree contained
them, not because anyone chose to publish them. Moving non-public material out
of `docs/` means no configuration mistake can expose it.

See [the design spec](./superpowers/specs/2026-08-19-docs-surface-rebuild-design.md).

## Writing plans and specs

**Specs go in `internal/superpowers/specs/`. Plans go in `internal/superpowers/plans/`.**

The superpowers skills default to `docs/superpowers/...`; that default is wrong
for this repo and is overridden here, in `CLAUDE.md`, and in the temper skill's
`fundamentals.md`.

## Stale paths in applied migrations

35 files under `migrations/` cite `docs/superpowers/...` in header comments.
Those citations are **stale and will not be repaired.** An applied migration is
immutable — editing one, even a comment, changes its checksum and fails
`db-migrate`. To resolve such a citation, replace the `docs/` prefix with
`internal/`, or read it out of git history.
EOF
```

- [ ] **Step 3: State the preference where the skills will read it**

Append to `CLAUDE.md` (and mirror into `AGENTS.md`, which is the same content for other agents):

```bash
cat >> CLAUDE.md <<'EOF'

## Where specs and plans go

**Specs: `internal/superpowers/specs/`. Plans: `internal/superpowers/plans/`.**

Not `docs/`. `docs/` is synced to the public documentation site, and everything
in it is public — so process artifacts must not be written there. The
superpowers skills default to `docs/superpowers/...`; this instruction overrides
that default. See `internal/README.md`.
EOF
```

- [ ] **Step 4: Mirror it into `AGENTS.md`**

```bash
cat >> AGENTS.md <<'EOF'

## Where specs and plans go

**Specs: `internal/superpowers/specs/`. Plans: `internal/superpowers/plans/`.**

Not `docs/` — everything in `docs/` is public and synced to the documentation
site. See `internal/README.md`.
EOF
```

- [ ] **Step 5: Update the temper skill's fundamentals**

In `.claude/skills/temper/guidance/fundamentals.md`, find the "Design Spec Lifecycle" section and replace the `docs/superpowers/specs/` path with `internal/superpowers/specs/`. Both the prose and the fenced `cat` example mention it:

```bash
sed -i '' 's|docs/superpowers/specs/|internal/superpowers/specs/|g' \
  .claude/skills/temper/guidance/fundamentals.md
grep -n "superpowers/specs" .claude/skills/temper/guidance/fundamentals.md
```

Expected: every remaining hit reads `internal/superpowers/specs/`.

- [ ] **Step 6: Verify the redirect is stated in all three places**

```bash
grep -l "internal/superpowers/specs" CLAUDE.md AGENTS.md .claude/skills/temper/guidance/fundamentals.md
```

Expected: all three files listed.

- [ ] **Step 7: Commit**

```bash
git add internal/README.md CLAUDE.md AGENTS.md .claude/skills/temper/guidance/fundamentals.md
git commit -m "docs: point specs and plans at internal/, ahead of the move

The superpowers skills hard-code docs/superpowers/{specs,plans}. Stating the
override before moving anything stops the next session re-creating the
directory the move exists to remove."
```

---

### Task 2: Move `docs/superpowers/` to `internal/superpowers/`

**Files:**
- Move: `docs/superpowers/` (472 markdown files plus 6 non-markdown) → `internal/superpowers/`

**Interfaces:**
- Consumes: `internal/` from Task 1.
- Produces: `internal/superpowers/{plans,specs,reviews,handoffs,spikes}/`, the paths Task 6 repairs references to.

- [ ] **Step 1: Record the before-count, so the move can be verified rather than assumed**

```bash
find docs/superpowers -type f | wc -l
find docs/superpowers -type f -name '*.md' | wc -l
```

At the time of writing: `481` total, `474` markdown. **Do not treat those as the expected values** — this plan and its spec live in `docs/superpowers/specs/` and `docs/superpowers/plans/` and are themselves part of the count, so it moves as work proceeds. Record whatever the two commands print now; Step 3 compares against *that*, not against a number written here.

- [ ] **Step 2: Move the tree**

```bash
git mv docs/superpowers internal/superpowers
```

- [ ] **Step 3: Verify nothing was lost and git saw renames, not rewrites**

```bash
find internal/superpowers -type f | wc -l          # must equal Step 1's count
test ! -e docs/superpowers && echo "docs/superpowers is gone"
git diff --cached --diff-filter=R --name-only | wc -l   # renames detected
git diff --cached --diff-filter=D --name-only | wc -l   # must be 0
```

Expected: same count as Step 1; "docs/superpowers is gone"; a large rename count; **zero** deletions. A non-zero deletion count means rename detection failed — stop and investigate rather than committing.

- [ ] **Step 4: Confirm the build is unaffected**

Nothing under `docs/` is compiled into Rust (no `include_str!` targets it) and CI's docs classification is an extension test rather than a path test, so this move cannot move a build outcome. Confirm the first half:

```bash
grep -rn 'include_str!' crates/ --include='*.rs' | grep -i "docs/" || echo "no docs/ file is compiled in"
```

Expected: `no docs/ file is compiled in`.

- [ ] **Step 5: Commit**

```bash
git commit -m "docs: move docs/superpowers to internal/superpowers

472 markdown files of plans and specs, which are process artifacts rather
than documentation. This alone removes 462 pages from the published site
when it merges, since Apidog syncs docs/ from main."
```

---

### Task 3: Move the remaining non-public material to `internal/`

`docs/specs/`, `docs/experiments/` and `docs/api/` were absent from the spec's move table; they are process and design material and belong here. `docs/registers/` is deliberately **excluded** — it is coupled to tooling and is Task 4.

**Files:**
- Move: `docs/development/` (5), `docs/agents/` (6), `docs/code-reviews/` (5), `docs/security/` (1), `docs/decisions/` (1), `docs/research/` (3), `docs/specs/` (2), `docs/experiments/` (3 files), `docs/api/` (1 file) → `internal/`

**Interfaces:**
- Consumes: `internal/` from Task 1.
- Produces: `internal/{development,agents,code-reviews,security,decisions,research,specs,experiments,api}/`.

- [ ] **Step 1: Record before-counts**

```bash
for d in development agents code-reviews security decisions research specs experiments api; do
  printf "%-14s %s\n" "$d" "$(find "docs/$d" -type f | wc -l | tr -d ' ')"
done
```

- [ ] **Step 2: Move each directory**

```bash
for d in development agents code-reviews security decisions research specs experiments api; do
  git mv "docs/$d" "internal/$d"
done
```

- [ ] **Step 3: Verify the counts survived and nothing was deleted**

```bash
for d in development agents code-reviews security decisions research specs experiments api; do
  printf "%-14s %s\n" "$d" "$(find "internal/$d" -type f | wc -l | tr -d ' ')"
done
git diff --cached --diff-filter=D --name-only | wc -l
```

Expected: counts match Step 1 exactly; deletion count is **0**.

- [ ] **Step 4: Confirm what remains in `docs/` is only public material**

```bash
ls docs/
```

Expected exactly: `auth`, `cognitive-maps`, `diagrams`, `guides`, `registers`, and the 11 loose `.md` files. `cognitive-maps` goes in Task 5; `registers` in Task 4; `auth`, `guides` and the loose files are re-homed by plan 3.

- [ ] **Step 5: Commit**

```bash
git commit -m "docs: move contributor and process material to internal/

Contributor docs are not a public audience: development, agents,
code-reviews, security, decisions, research, specs, experiments and the
hand-written query.openapi.yaml design contract. The published API
reference is generated from openapi.json, not from this file."
```

---

### Task 4: Move `docs/registers/` and repoint the tooling that writes it

`docs/registers/coverage.yaml` is a generated projection whose path is hardcoded in three places. Moving the file without all three leaves the drift gate comparing against a path that no longer exists.

**Files:**
- Move: `docs/registers/coverage.yaml` → `internal/registers/coverage.yaml`
- Modify: `tools/register_projection/__main__.py` (the `DEFAULT_OUT` constant and the usage docstring)
- Modify: `.github/scripts/check-register-coverage-drift.sh` (the `ARTIFACT` variable)
- Modify: `tools/cargo-make/main.toml` (the task description)

**Interfaces:**
- Consumes: `internal/` from Task 1.
- Produces: `internal/registers/coverage.yaml`, regenerable and still drift-gated.

- [ ] **Step 1: Find every hardcoded occurrence, so none is missed**

```bash
grep -rn "docs/registers" tools/ .github/scripts/ Makefile.toml 2>/dev/null
```

Expected: 5 hits — `__main__.py` (docstring line ~4 and `DEFAULT_OUT` line ~31), `check-register-coverage-drift.sh` (comment line ~3 and `ARTIFACT` line ~38), and `tools/cargo-make/main.toml` (description line ~60).

- [ ] **Step 2: Move the file**

```bash
git mv docs/registers internal/registers
```

- [ ] **Step 3: Repoint all three consumers**

```bash
sed -i '' 's|docs/registers/coverage\.yaml|internal/registers/coverage.yaml|g' \
  tools/register_projection/__main__.py \
  .github/scripts/check-register-coverage-drift.sh \
  tools/cargo-make/main.toml
grep -rn "docs/registers" tools/ .github/scripts/ Makefile.toml || echo "no stale references remain"
```

Expected: `no stale references remain`.

- [ ] **Step 4: Regenerate and confirm the projection writes to the new path**

```bash
uv run --project tools register-projection --repo-root . --out internal/registers/coverage.yaml
git diff --stat internal/registers/coverage.yaml
```

Expected: the command succeeds. An empty diff is the good outcome — it means the projection reproduces the moved file byte-for-byte. A non-empty diff means the projection's content changed for an unrelated reason; inspect before continuing.

- [ ] **Step 5: Run the drift gate against the new path**

```bash
bash .github/scripts/check-register-coverage-drift.sh; echo "EXIT: $?"
```

Expected: `EXIT: 0`. A non-zero exit here means Step 3 missed a path — this gate is the test for this task.

- [ ] **Step 6: Commit**

```bash
git add -A tools/ .github/scripts/check-register-coverage-drift.sh internal/registers
git commit -m "chore: move the register-coverage projection to internal/

It is a generated internal projection of outcome-register evidence, not
public documentation. The generator, the drift gate and the cargo-make
description all hardcoded the old path and move with it."
```

---

### Task 5: Retire `docs/cognitive-maps/`

`[decided — 2026-08-19, Pete]` temperkb.io is the source. All twelve files have live, richer Svelte counterparts; the flip described in `site-ia.md` was executed against the Svelte pages, not the markdown.

**Files:**
- Delete: `docs/cognitive-maps/` (12 markdown files)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing. Task 6 repairs the 8 inbound references this creates.

- [ ] **Step 1: Re-verify every file has a counterpart before deleting**

Do not skip this. It is the check that makes the deletion safe rather than assumed.

```bash
find "packages/temper-ui/src/routes/(public)/cognitive-maps" -name "+page.svelte" | wc -l
find "packages/temper-ui/src/routes/(public)/operating" -name "+page.svelte" | wc -l
```

Expected: `9` under cognitive-maps (movements 1–6, the bridge `operating-temper`, `the-set`, and the index) and `5` under operating (index + `deployment`, `governance-and-administration`, `observability-and-audit`, `insights`). Together these cover all twelve markdown files: 01–06, 07, 07a–07d, and README.

- [ ] **Step 2: Delete the tree**

```bash
git rm -r docs/cognitive-maps
```

- [ ] **Step 3: Verify it is gone and recoverable**

```bash
test ! -e docs/cognitive-maps && echo "retired"
git log --oneline -1 -- docs/cognitive-maps/README.md
```

Expected: `retired`, and a commit hash proving history still reaches the deleted files.

- [ ] **Step 4: Commit**

```bash
git commit -m "docs: retire docs/cognitive-maps; temperkb.io is the source

Twelve markdown files, each hand-duplicated as a richer Svelte page:
movements 1-6 and the set index under (public)/cognitive-maps, movement 7
as the bridge site-ia.md specced, and 07a-07d as (public)/operating's four
children. site-ia.md's flip was executed against the Svelte pages, so the
markdown was the copy that would rot. Folded, not forgotten - history keeps
them."
```

---

### Task 6: Repair the editable inbound references

117 files outside `docs/superpowers/` reference it, plus smaller counts for the other moved directories. 35 of those 117 are applied migrations and are **immutable** — they stay stale by decision.

**Files:**
- Modify: every non-migration file citing a moved path (~82 for superpowers, plus ~55 across the other moved directories)
- Modify: `.github/scripts/test-detect-ci-scope.sh:62` (a test fixture naming a moved path)

**Interfaces:**
- Consumes: the new paths from Tasks 2–5.
- Produces: a tree where every live reference resolves.

- [ ] **Step 1: Enumerate what needs repair, excluding migrations**

```bash
grep -rIl -E "docs/(superpowers|development|agents|code-reviews|security|decisions|research|specs|experiments|api|registers|cognitive-maps)" \
  --exclude-dir=node_modules --exclude-dir=target --exclude-dir=.git . \
  | grep -v '^migrations/' | sort > /tmp/docs-refs-to-fix.txt
wc -l < /tmp/docs-refs-to-fix.txt
```

- [ ] **Step 2: Rewrite the moved prefixes**

`cognitive-maps` is excluded from this rewrite — it was deleted, not moved, so its references need judgement rather than a prefix swap. Step 4 handles them.

```bash
while read -r f; do
  sed -i '' -E 's|docs/(superpowers\|development\|agents\|code-reviews\|security\|decisions\|research\|specs\|experiments\|api\|registers)/|internal/\1/|g' "$f"
done < /tmp/docs-refs-to-fix.txt
```

- [ ] **Step 3: Verify no live non-migration reference to a moved path survives**

```bash
grep -rIn -E "docs/(superpowers|development|agents|code-reviews|security|decisions|research|specs|experiments|api|registers)/" \
  --exclude-dir=node_modules --exclude-dir=target --exclude-dir=.git . \
  | grep -v '^migrations/' | head
echo "--- migrations (expected to remain stale) ---"
grep -rIl "docs/superpowers" migrations/ | wc -l
```

Expected: the first command prints nothing. The second prints `35` — those are the immutable citations, and their presence is correct.

- [ ] **Step 4: Repair the cognitive-maps references by hand**

```bash
grep -rIn "docs/cognitive-maps" --exclude-dir=node_modules --exclude-dir=target --exclude-dir=.git .
```

Each hit is a link to a retired file. Repoint it at the temperkb.io page that superseded it, using the mapping in the spec's retirement section. Do not prefix-swap these — there is no `internal/cognitive-maps/`.

- [ ] **Step 5: Confirm the CI scope guard still passes**

`test-detect-ci-scope.sh` feeds a `docs/superpowers/...` path as a fixture. Step 2 rewrote it; this confirms the guard still agrees with itself.

```bash
bash .github/scripts/test-detect-ci-scope.sh; echo "EXIT: $?"
```

Expected: `EXIT: 0`.

- [ ] **Step 6: Run the full check**

```bash
cargo make check 2>&1 | tail -30
```

Expected: green. This is the task's real test — it exercises `check-register-coverage-drift`, `skills-drift`, and the audit scripts whose header comments Step 2 rewrote.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "docs: repoint references at internal/

Rewrites the moved prefixes across ~82 non-migration files and repairs the
cognitive-maps links by hand against their temperkb.io successors.

The 35 applied migrations citing docs/superpowers are deliberately NOT
touched: an applied migration is immutable, and editing one - even a
comment - changes its checksum and fails db-migrate. internal/README.md
records how to resolve those stale citations."
```

---

### Task 7: Assert the invariant

The point of the whole plan is one property. This task states it as a runnable check so a later change cannot quietly break it.

**Files:**
- Create: `.github/scripts/check-docs-public-only.sh`
- Modify: `tools/cargo-make/main.toml` (register the task)

**Interfaces:**
- Consumes: the finished tree from Tasks 2–6.
- Produces: `check-docs-public-only`, a gate plan 2 extends.

- [ ] **Step 1: Write the check**

It follows `register-coverage.py`'s discipline: it asserts its scan found something, so an empty scan cannot pass vacuously.

```bash
cat > .github/scripts/check-docs-public-only.sh <<'EOF'
#!/usr/bin/env bash
# Fail if docs/ contains anything that is not public documentation.
#
# WHY: docs/ is synced to the public documentation site. The safety property is
# structural, not configured — "everything in docs/ is public, nothing else
# lives there" — because the alternative, an allowlist, was got wrong once and
# published internal security audits.
#
# This asserts the ABSENCE of known-internal directory names. It cannot judge
# whether a given page is fit to publish; it only catches the failure mode that
# actually occurred, which was a whole internal tree sitting under docs/.
set -euo pipefail
cd "$(dirname "$0")/../.."

FORBIDDEN='superpowers development agents code-reviews security decisions research specs experiments registers'

# (a) The scan must find something. A docs/ that does not exist, or is empty,
# would satisfy every assertion below while checking nothing.
count=$(find docs -type f 2>/dev/null | wc -l | tr -d ' ')
if [ "$count" -eq 0 ]; then
    echo "FAIL: docs/ has no files — refusing to report clean on an empty scan." >&2
    exit 1
fi

# (b) No forbidden directory may exist under docs/.
failed=0
for d in $FORBIDDEN; do
    if [ -e "docs/$d" ]; then
        echo "FAIL: docs/$d exists — internal material belongs in internal/." >&2
        failed=1
    fi
done

[ "$failed" -eq 0 ] && echo "OK: docs/ holds $count files, none in an internal tree."
exit "$failed"
EOF
chmod +x .github/scripts/check-docs-public-only.sh
```

- [ ] **Step 2: Verify it passes on the restructured tree**

```bash
bash .github/scripts/check-docs-public-only.sh; echo "EXIT: $?"
```

Expected: `OK: docs/ holds N files, none in an internal tree.` and `EXIT: 0`.

- [ ] **Step 3: Verify it actually fails when the invariant is broken**

A gate that has never failed is not known to work.

```bash
mkdir -p docs/security && touch docs/security/probe.md
bash .github/scripts/check-docs-public-only.sh; echo "EXIT: $?"
rm -rf docs/security
```

Expected: `FAIL: docs/security exists` and `EXIT: 1`. Then confirm it returns to green:

```bash
bash .github/scripts/check-docs-public-only.sh; echo "EXIT: $?"
```

Expected: `EXIT: 0`. Confirm `git status --short` shows no leftover `docs/security`.

- [ ] **Step 4: Register it with cargo-make**

Two edits in `tools/cargo-make/main.toml`. First, define the task — it follows the shape of the neighbouring `register-coverage-drift` task at around line 60:

```toml
[tasks.docs-public-only]
description = "Fail if docs/ contains internal material — the publish-safety invariant"
script = ["bash ${CARGO_MAKE_WORKING_DIRECTORY}/.github/scripts/check-docs-public-only.sh"]
```

Second, add it to the `check` task so it actually runs. `[tasks.check]` is at line 16 and composes via a `dependencies` array starting at line 27 (`"rust-fmt-check"`, `"rust-clippy"`, `"rust-docs"`, `"rust-machete"`, …). Append the new task name to that array:

```toml
dependencies = [
  "rust-fmt-check",
  # … existing entries unchanged …
  "docs-public-only",
]
```

Confirm the edit took:

```bash
grep -n "docs-public-only" tools/cargo-make/main.toml
```

Expected: two hits — the `[tasks.docs-public-only]` definition and the entry in `check`'s dependencies. **One hit means the gate is defined but never runs**, which is the failure this step exists to prevent.

- [ ] **Step 5: Confirm it runs as part of check**

```bash
cargo make docs-public-only; echo "EXIT: $?"
cargo make check 2>&1 | tail -20
```

Expected: both green.

- [ ] **Step 6: Commit**

```bash
git add .github/scripts/check-docs-public-only.sh tools/cargo-make/main.toml
git commit -m "ci: gate the docs-are-public-only invariant

Asserts no internal tree has reappeared under docs/, and asserts its own
scan found something so an empty docs/ cannot pass vacuously. Verified to
fail by planting a probe directory, not only to pass."
```

---

## What this plan does not do

- **Unpublishing.** Pete's action in Apidog. Because Apidog syncs `main`, merging this plan is what drops the 462 plan/spec pages; unpublishing is what removes the security audits *before* then.
- **Re-homing `docs/guides/`, `docs/auth/` and the 11 loose root files** into `playbooks/`/`concepts/`/`reference/`. Those need the target structure to exist first — plan 3.
- **Triage of the moved artifacts.** They move wholesale; per-file triage is its own later session, by decision.
- **`docs/diagrams/`.** Stays put: `README.md` embeds four of those SVGs, and they are public assets.
