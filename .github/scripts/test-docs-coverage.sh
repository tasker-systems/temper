#!/usr/bin/env bash
# .github/scripts/test-docs-coverage.sh
#
# Test harness for scripts/docs-coverage.py.
#
# Every case runs against a throwaway docs tree built below, never the real one — the script
# takes `--docs`, so no seams are needed for that. `DOCS_COVERAGE_LLMS_URL` is the one harness
# seam, used to point the publish check at an unreachable host and assert the UNKNOWN path
# rather than testing it by unplugging the network. No CI job sets it.
#
# Two properties matter more than the rest and each has a case here:
#
#   * --strict fails on dangling links and on NOTHING ELSE. Orphans, escaping links and
#     unclaimed reference subtrees are reported and must leave the exit code at 0. A --strict
#     that quietly grew a second failure mode would start failing on legitimate shapes, which
#     is how a check gets switched off.
#   * A tree the script cannot read must REFUSE, never report zero. A confident "0 dangling
#     links" derived from having looked at nothing is worse than no check at all.
#
#   bash .github/scripts/test-docs-coverage.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TOOL="${REPO_ROOT}/scripts/docs-coverage.py"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# A minimal well-formed tree: an index, one door, one guide reached through it.
make_tree() {
    local root="$1"
    rm -rf "$root"
    mkdir -p "$root/doors" "$root/guides"
    printf '# Docs\n\n- [Using](./doors/for-users.md)\n' >"$root/index.md"
    printf '# Using\n\n- [Install](../guides/install.md)\n' >"$root/doors/for-users.md"
    printf '# Install\n' >"$root/guides/install.md"
}

# run_case NAME DOCS_DIR EXTRA_ARGS EXPECTED_EXIT [NEEDLE]
run_case() {
    local name="$1" docs="$2" extra="$3" expected_exit="$4" needle="${5:-}"
    local output actual_exit
    set +e
    # shellcheck disable=SC2086 # $extra is a deliberate argument list
    output="$(python3 "$TOOL" --docs "$docs" --no-network $extra 2>&1)"
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "  FAIL: ${name}"
        echo "    expected exit=${expected_exit} actual=${actual_exit}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    if [ -n "$needle" ] && ! printf '%s' "$output" | grep -qF -- "$needle"; then
        echo "  FAIL: ${name}"
        echo "    exit matched but expected text missing: ${needle}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    echo "  ok: ${name}"
    PASS=$((PASS + 1))
}

echo "test-docs-coverage"

# --- BASELINE: without a green case, every red below proves nothing ---
T="${WORK}/clean"; make_tree "$T"
run_case "a clean tree passes --strict" "$T" "--strict" 0 "no orphans"

# --- the one --strict failure ---
T="${WORK}/dangling"; make_tree "$T"
printf '# Install\n\n[gone](./nowhere.md)\n' >"$T/guides/install.md"
run_case "a dangling link fails --strict" "$T" "--strict" 1 "DANGLING"
run_case "the same tree passes WITHOUT --strict" "$T" "" 0 "DANGLING"

# --- fence-awareness: the false positive that would have made --strict permanently red ---
#
# A real page (docs/guides/operational-memory.md) shows a rendered MEMORY.md inside a fence,
# and that sample contains [title](<uuid>) links to vault resources. They are illustration, not
# navigation, and no edit could ever make them resolve.
T="${WORK}/fenced"; make_tree "$T"
cat >"$T/guides/install.md" <<'EOF'
# Install

```markdown
- [feature unification pulls in ort](019fc011-e40f-72b2-be4b-598ec2f68c71)
- [a second one](019fc010-c86e-7ed0-94e7-b36bf255d36c)
```
EOF
run_case "links inside a fenced block are illustration, not dangling" "$T" "--strict" 0 "none"

# A fenced block must not swallow the REST of the file either — an unbalanced strip would
# silently stop finding real links after the first fence, which reads as a clean tree.
T="${WORK}/afterfence"; make_tree "$T"
cat >"$T/guides/install.md" <<'EOF'
# Install

```text
[not a link](./nowhere-in-here.md)
```

[but this one is](./nowhere-after.md)
EOF
run_case "a real dangling link AFTER a fence is still found" "$T" "--strict" 1 "nowhere-after.md"

# --- reported, never adjudicated ---
T="${WORK}/orphan"; make_tree "$T"
printf '# Lonely\n' >"$T/guides/orphan.md"
run_case "an orphan is reported and does NOT fail --strict" "$T" "--strict" 0 "ORPHAN"

T="${WORK}/escaping"; make_tree "$T"
printf '# outside\n' >"${WORK}/escaping-target.md"
printf '# Install\n\n[out](../../escaping-target.md)\n' >"$T/guides/install.md"
run_case "a link escaping the tree is reported and does NOT fail --strict" "$T" "--strict" 0 "ESCAPES"

T="${WORK}/unclaimed"; make_tree "$T"
mkdir -p "$T/reference/invented"
printf '# whatever\n' >"$T/reference/invented/page.md"
run_case "an unclaimed reference subtree is reported, not adjudicated" "$T" "--strict" 0 "UNCLAIMED"

T="${WORK}/unmarked"; make_tree "$T"
mkdir -p "$T/reference/cli"
printf '# hand-written\n' >"$T/reference/cli/sneaked-in.md"
run_case "a reference page with no GENERATED marker is reported" "$T" "--strict" 0 "UNMARKED"

# --- refuse to report zero ---
run_case "a docs/ that does not exist: REFUSES (exit 2), never reports clean" \
    "${WORK}/no-such-dir" "--strict" 2 "does not exist"

T="${WORK}/empty"; rm -rf "$T"; mkdir -p "$T"
run_case "a docs/ with no markdown: REFUSES rather than reporting zero" "$T" "--strict" 2 \
    "contains no markdown pages"

T="${WORK}/noindex"; make_tree "$T"; rm "$T/index.md"
run_case "a missing index.md: REFUSES rather than calling every page an orphan" "$T" "--strict" 2 \
    "index.md is missing"

# The subtlest of the three: index.md exists but the parser gets nothing out of it. Every page
# would report as an orphan, which looks like a finding and is actually a broken parse.
T="${WORK}/linkless"; make_tree "$T"; printf '# Docs\n\nNo links here.\n' >"$T/index.md"
run_case "an index.md with no links: REFUSES rather than orphaning the whole tree" "$T" "--strict" 2 \
    "yielded no links at all"

# --- the network half is informational, in BOTH directions ---
T="${WORK}/net"; make_tree "$T"
set +e
out="$(DOCS_COVERAGE_LLMS_URL='http://127.0.0.1:1/llms.txt' python3 "$TOOL" --docs "$T" --strict 2>&1)"
code=$?
set -e
if [ "$code" -eq 0 ] && printf '%s' "$out" | grep -qF "UNKNOWN"; then
    echo "  ok: an unreachable site reports UNKNOWN and never fails the run"
    PASS=$((PASS + 1))
else
    echo "  FAIL: an unreachable site did not degrade to a non-failing UNKNOWN (exit ${code})"
    FAIL=$((FAIL + 1))
fi

# And the navigation verdict must stay UNKNOWN unconditionally — it is not observable from
# llms.txt at all, so no run, however clean, may report it as fine.
if printf '%s' "$out" | grep -qF "A green run of this script does NOT mean the site navigates correctly"; then
    echo "  ok: the navigation state is reported UNKNOWN, never inferred clean"
    PASS=$((PASS + 1))
else
    echo "  FAIL: the report no longer states that navigation is unobservable"
    FAIL=$((FAIL + 1))
fi

# --- WIRING ---
if grep -F "bash .github/scripts/test-docs-coverage.sh" "${REPO_ROOT}/.github/workflows/code-quality.yml" \
    | grep -qvE '^[[:space:]]*#'; then
    echo "  ok: this harness runs in code-quality.yml, on a live line"
    PASS=$((PASS + 1))
else
    echo "  FAIL: test-docs-coverage.sh is not invoked in code-quality.yml"
    FAIL=$((FAIL + 1))
fi

if grep -F "python3 scripts/docs-coverage.py --strict --no-network" \
    "${REPO_ROOT}/.github/workflows/code-quality.yml" | grep -qvE '^[[:space:]]*#'; then
    echo "  ok: the check itself runs in CI, with --strict and without the network"
    PASS=$((PASS + 1))
else
    echo "  FAIL: docs-coverage.py --strict --no-network does not run in code-quality.yml"
    echo "        (--no-network is deliberate: the publish half compares against PRODUCTION,"
    echo "         which is meaningless on a PR branch and never affects the exit code)"
    FAIL=$((FAIL + 1))
fi

# The real tree must actually pass, or the CI step above is red on arrival.
set +e
python3 "$TOOL" --docs "${REPO_ROOT}/docs" --strict --no-network >/dev/null 2>&1
real_code=$?
set -e
if [ "$real_code" -eq 0 ]; then
    echo "  ok: the real docs/ tree passes --strict"
    PASS=$((PASS + 1))
else
    echo "  FAIL: the real docs/ tree does not pass --strict (exit ${real_code})"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
