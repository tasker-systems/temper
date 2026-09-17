#!/usr/bin/env bash
# .github/scripts/test-check-public-surface-drift.sh
#
# Test harness for check-public-surface-drift.sh, in the same shape as every sibling
# guard test: the gate runs against a SYNTHETIC repo root in a temp dir (nothing here
# touches the working tree), every check class gets a manufactured instance of its own
# drift that must turn the gate red, and the wiring is asserted behaviourally — an
# uncommented CI invocation, placement inside the ungated guard-tests job, and
# reachability through the real detector. A gate that runs nowhere passes everywhere;
# a gate that checks nothing must refuse rather than report clean.
#
# Two things are derived from the gate under test, never restated here:
#   * the roadmap verb list (sourced out of the script) — a verb dropped from the gate
#     must surface as an untested case here, not keep "passing" against a stale copy.
#   * nothing else: the fixtures are this test's own, by construction synthetic.
#
#   bash .github/scripts/test-check-public-surface-drift.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-public-surface-drift.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

TREE=""
# make_tree NAME — a synthetic repo root holding the gate, a minimal CLI reference
# tree (one root page, two command pages), a README quoting them, a self-host page
# stating counts, and a vercel.json deriving DIFFERENT-shaped numbers than the real
# one on purpose: if the gate restated real counts instead of deriving, the clean
# case below would go red. Commits twice so HEAD~1 exists; internal/development/
# carries a VERB-FUL file from the base commit to prove the grandfather rule.
TREE=""
make_tree() {
    TREE="${WORK}/$1"
    mkdir -p "${TREE}/.github/scripts" "${TREE}/docs/reference/cli" \
             "${TREE}/docs/playbooks" "${TREE}/internal/development"
    cp "$GATE" "${TREE}/.github/scripts/"

    cat > "${TREE}/vercel.json" <<'EOF'
{
  "crons": [
    { "path": "/api/a", "schedule": "* * * * *" },
    { "path": "/api/b", "schedule": "* * * * *" }
  ],
  "functions": {
    "api/one.rs": { "memory": 3009 },
    "api/two.rs": { "memory": 3009 }
  },
  "routes": [
    { "handle": "filesystem" },
    { "src": "/x", "dest": "/api/one" },
    { "src": "/y", "dest": "/api/two" }
  ]
}
EOF

    cat > "${TREE}/docs/reference/cli/README.md" <<'EOF'
# CLI reference

Global options: `--help`, `--version`.
EOF

    cat > "${TREE}/docs/reference/cli/context.md" <<'EOF'
# `temper context`

Usage: temper context [OPTIONS] <COMMAND>

Commands:
  create          Create a new context on the server
  list            List the contexts you can see on the server

### `temper context create`

Usage: temper context create [OPTIONS] <NAME>

### `temper context list`

Usage: temper context list [OPTIONS]
EOF

    cat > "${TREE}/docs/reference/cli/resource.md" <<'EOF'
# `temper resource`

Usage: temper resource [OPTIONS] <COMMAND>

Commands:
  create          Create a resource
  update          Update a resource

### `temper resource create`

Usage: temper resource create [OPTIONS] [BODY]

- `--from <path>` — load the body from a file
- `--context <ref>` — home the resource in this context
EOF

    cat > "${TREE}/README.md" <<'EOF'
# temper

## Quick Start

```bash
temper context create myapp
temper resource create --from ~/docs/design.md --context @me/myapp
```
EOF

    cat > "${TREE}/docs/playbooks/self-host-temper.md" <<'EOF'
# Self-host temper

The authoritative routing, function and cron configuration is `vercel.json`.

Vercel cron (×2). The deployment declares 2 routes and 2 crons; 2 functions.
EOF

    # A verb-carrying file that predates every diff base this harness uses: the sweep
    # is diff-scoped, so this file must stay invisible to it (grandfather rule).
    echo "The drain operator runbook is planned for the next cycle." \
        > "${TREE}/internal/development/old-note.md"

    git -C "$TREE" init -q
    git -C "$TREE" -c user.email=test@example.com -c user.name=test add -A
    git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -qm base
    # Second commit so HEAD~1 exists and the default PSD_BASE resolves.
    echo "# scratch" > "${TREE}/scratch.md"
    git -C "$TREE" -c user.email=test@example.com -c user.name=test add -A
    git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -qm fixtures
}

# run_case NAME EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_case() {
    local name="$1" expected="$2" needle="${3:-}"
    local out actual=0
    out="$(cd "$TREE" && bash .github/scripts/check-public-surface-drift.sh 2>&1)" || actual=$?

    if [ "$actual" != "$expected" ]; then
        echo "  FAIL: ${name} — expected exit ${expected}, got ${actual}"
        echo "        output: ${out}"
        FAIL=$((FAIL + 1))
        return 0
    fi
    if [ -n "$needle" ] && ! echo "$out" | grep -qF -e "$needle"; then
        echo "  FAIL: ${name} — exit ${actual} was right but the message did not mention '${needle}'"
        echo "        output: ${out}"
        FAIL=$((FAIL + 1))
        return 0
    fi
    echo "  PASS: ${name}"
    PASS=$((PASS + 1))
}

echo "Running check-public-surface-drift.sh tests..."
echo ""

# --- POSITIVE: the clean synthetic tree passes ---

make_tree clean
run_case "clean synthetic tree: passes" 0 "public-surface drift: clean"
run_case "clean tree sweep reports the grandfathered file invisible" 0 "0 new file(s)"

# A tree whose numbers differ from the real repo's proves the counts derive rather
# than restate (the gate checked this tree's 2/2/2 here, never the real 10/22/3).
run_case "clean tree counts derive from THIS tree's vercel.json" 0 "4 stated figures checked"

# --- NEGATIVE: one manufactured bite per check class ---

make_tree invented-command
sed -i '' -e 's/^temper context create myapp$/temper context add myapp/' "${TREE}/README.md" 2>/dev/null \
    || sed -i 's/^temper context create myapp$/temper context add myapp/' "${TREE}/README.md"
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -aqm bite
run_case "invented subcommand: FAILS" 1 "neither a documented subcommand nor a positional argument of 'context'"

make_tree invented-flag
sed -i '' -e 's/^temper context create myapp$/temper context create --bogus myapp/' "${TREE}/README.md" 2>/dev/null \
    || sed -i 's/^temper context create myapp$/temper context create --bogus myapp/' "${TREE}/README.md"
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -aqm bite
run_case "invented flag: FAILS" 1 "--bogus"

make_tree continuation-flag
printf '```bash\ntemper context create \\\n  --bogus myapp\n```\n' >> "${TREE}/README.md"
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -aqm bite
run_case "flag on a \\-continued line: still FAILS (continuations join)" 1 "--bogus"

make_tree page-count-mutated
sed -i '' -e 's/2 crons/3 crons/' "${TREE}/docs/playbooks/self-host-temper.md" 2>/dev/null \
    || sed -i 's/2 crons/3 crons/' "${TREE}/docs/playbooks/self-host-temper.md"
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -aqm bite
run_case "stated count mutated: FAILS against the authority" 1 "stated 3 crons vs vercel.json's 2"

make_tree authority-mutated
# The mutation runs with $TREE as its CWD — a CWD-relative open from the harness
# root would plant the probe in the REAL repo, which is exactly the accident the
# exemplar guard test's header warns about.
(
    cd "$TREE" && python3 - <<'PYEOF'
import json
with open("vercel.json") as f: m = json.load(f)
m["crons"].append({"path": "/api/c", "schedule": "* * * * *"})
with open("vercel.json", "w") as f: json.dump(m, f)
PYEOF
)
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -aqm bite
run_case "authority count mutated: FAILS the now-stale page" 1 "crons"

make_tree missing-authority
rm "${TREE}/vercel.json"
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -aqm bite
run_case "vercel.json absent: refuses rather than checking nothing" 1 "refusing to check nothing"

make_tree missing-tree
rm -r "${TREE}/docs/reference/cli"
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -aqm bite
run_case "cli reference tree absent: refuses rather than checking nothing" 1 "refusing to check nothing"

# --- NEGATIVE: the roadmap sweep — new files only, every verb, case-insensitively ---

make_tree roadmap-new-file
printf 'The reconciliation job is planned for later.\n' > "${TREE}/internal/development/new-note.md"
git -C "$TREE" -c user.email=test@example.com -c user.name=test add -A
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -qm bite
run_case "new internal/ file with a roadmap verb: FAILS" 1 "roadmap sweep"

make_tree roadmap-clean-file
printf 'The reconciliation job runs hourly.\n' > "${TREE}/internal/development/new-note.md"
git -C "$TREE" -c user.email=test@example.com -c user.name=test add -A
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -qm bite
run_case "new internal/ file without roadmap verbs: passes" 0 "public-surface drift: clean"

make_tree roadmap-design-system
mkdir -p "${TREE}/design-system/docs"
printf 'Ships when it ships.\n' > "${TREE}/design-system/docs/note.md"
git -C "$TREE" -c user.email=test@example.com -c user.name=test add -A
git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -qm bite
run_case "new design-system/ file with a roadmap verb: FAILS" 1 "roadmap sweep"

# Every verb in the gate's list, derived — not restated — from the gate itself.
VERB_LINE="$(grep -E '^ROADMAP_VERBS=' "$GATE")"
eval "$VERB_LINE"
if [ -z "${ROADMAP_VERBS:-}" ]; then
    echo "  FAIL: could not read ROADMAP_VERBS out of ${GATE} — the per-verb cases below"
    echo "        would silently check nothing."
    FAIL=$((FAIL + 1))
else
    IFS='|' read -r -a VERBS <<< "$ROADMAP_VERBS"
    for verb in "${VERBS[@]}"; do
        make_tree "verb-${verb// /-}"
        mkdir -p "${TREE}/internal/development"
        capitalised="$(printf '%s' "$verb" | cut -c1 | tr '[:lower:]' '[:upper:]')$(printf '%s' "$verb" | cut -c2-)"
        printf 'Subject: %s\n' "$capitalised" > "${TREE}/internal/development/v.md"
        git -C "$TREE" -c user.email=test@example.com -c user.name=test add -A
        git -C "$TREE" -c user.email=test@example.com -c user.name=test commit -qm bite
        run_case "verb '${verb}' (capitalised): FAILS" 1 "roadmap sweep"
    done
fi

# --- WIRING: a gate that runs nowhere passes everywhere ---

assert_uncommented() {
    local name="$1" file="$2" needle="$3"
    if grep -F "$needle" "${REPO_ROOT}/${file}" | grep -qvE '^[[:space:]]*#'; then
        echo "  PASS: ${name}"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: ${name} — '${needle}' absent or only present commented-out in ${file}"
        FAIL=$((FAIL + 1))
    fi
}

assert_uncommented "the gate runs in code-quality.yml, on a live (uncommented) line" \
    ".github/workflows/code-quality.yml" \
    "bash .github/scripts/check-public-surface-drift.sh"

assert_uncommented "the guard test runs in code-quality.yml too" \
    ".github/workflows/code-quality.yml" \
    "bash .github/scripts/test-check-public-surface-drift.sh"

wf="${REPO_ROOT}/.github/workflows/code-quality.yml"
guard_line="$(grep -n '^  guard-tests:' "$wf" | head -1 | cut -d: -f1 || true)"
gate_line="$(grep -n 'bash .github/scripts/check-public-surface-drift.sh' "$wf" \
    | grep -vE ':[[:space:]]*#' | head -1 | cut -d: -f1 || true)"
if [ -n "$guard_line" ] && [ -n "$gate_line" ] && [ "$gate_line" -gt "$guard_line" ]; then
    echo "  PASS: the gate runs inside guard-tests, the job no input can switch off"
    PASS=$((PASS + 1))
else
    echo "  FAIL: the gate is not inside guard-tests (guard-tests at line '${guard_line}',"
    echo "        gate at line '${gate_line}') — reaching it may now require run-rust-quality"
    FAIL=$((FAIL + 1))
fi

# Reachability through the REAL detector. The gate's own failure modes are a
# README edit (cli-claims), a design-system/ new file (the sweep), and an
# internal/ new file (the sweep) — every one must summon code-quality. A README
# change must stay DOCS_ONLY: the gate is reachable precisely so a docs-only PR
# need not conscript the heavy pipeline.
verdict="$(echo 'README.md' | bash "${REPO_ROOT}/.github/scripts/detect-ci-scope.sh" --stdin 2>/dev/null || true)"
if echo "$verdict" | grep -qE '^RUN_CODE_QUALITY=true'; then
    echo "  PASS: a README.md change invokes code-quality (the cli-claims check is reachable)"
    PASS=$((PASS + 1))
else
    echo "  FAIL: a README.md change does not invoke code-quality — unreachable on its own failure mode"
    FAIL=$((FAIL + 1))
fi
if echo "$verdict" | grep -qE '^DOCS_ONLY=true'; then
    echo "  PASS: a README.md change stays docs-only — the heavy pipeline stays off"
    PASS=$((PASS + 1))
else
    echo "  FAIL: a README.md change no longer scopes as docs-only — the gate now costs the heavy pipeline"
    FAIL=$((FAIL + 1))
fi

for probe in 'design-system/docs/new.md' 'internal/development/new.md' 'docs/guides/x.md'; do
    verdict="$(echo "$probe" | bash "${REPO_ROOT}/.github/scripts/detect-ci-scope.sh" --stdin 2>/dev/null || true)"
    if echo "$verdict" | grep -qE '^RUN_CODE_QUALITY=true'; then
        echo "  PASS: ${probe} invokes code-quality (guard-tests reachable)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: ${probe} does not invoke code-quality — a gate class is unreachable"
        FAIL=$((FAIL + 1))
    fi
done

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
