#!/usr/bin/env bash
# .github/scripts/test-calculate-versions.sh
#
# Test harness for tools/scripts/release/calculate-versions.sh — the release
# calculator's Gate 1, the retirement-train trigger (the compat-deprecation
# regime's D-C1/D-C2, landed by PR #944 as ec3506bf). M is the reserved era
# level: a shape-breaking row ROUTED to the retirement train refuses a patch
# next (naming its rows) and takes the era bump under --minor; a bare
# shape-breaking row and a converted row's re-declaration (additive +
# behavioral) compute a patch without refusal — the declared-class gate owns
# the unrouted refusal at PR time (D-C2), the calculator only forces the era
# question.
#
# WHAT THE FIXTURES EXERCISE, AND WHAT THEY CANNOT
# ------------------------------------------------
# Registers flow through the REAL parser — register_window_rows and
# field_has_token in lib/common.sh, reached via --register (the flag exists
# for harness fixtures). The tools/scripts/release tree is copied fresh into
# a sandbox per run, so the code under test is the current tree and the
# copied REPO_ROOT resolves into the sandbox: a fixture VERSION, no leaf
# version files. The one seam replaced is detect-changes.sh — the git-diff
# input, not the parser, whose class booleans would otherwise depend on
# ambient history (a depth-2 CI checkout and a full local clone diff
# differently). The stub emits fixed KEY=VALUEs with CORE_CHANGED=true.
# Known hole: the capture/eval contract between the calculator and
# detect-changes is invisible here — a capture that swallowed a failing
# diff would pass this harness with the stub supplying the answer.
#
# Bite direction: reverting the trigger to any-shape-breaking-fires reds the
# no-refusal arms; neutering the gate reds the refusal arm.
#
#   bash .github/scripts/test-calculate-versions.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# Sandbox the calculator: the current release tree under a fixture REPO_ROOT,
# with a fixture VERSION and a fixed-input detect-changes.sh standing in for
# the git diff.
mkdir -p "${WORK}/tools/scripts"
cp -R "${REPO}/tools/scripts/release" "${WORK}/tools/scripts/release"
chmod +x "${WORK}/tools/scripts/release/calculate-versions.sh"
echo "0.5.0" > "${WORK}/VERSION"
cat > "${WORK}/tools/scripts/release/detect-changes.sh" <<'STUB'
#!/usr/bin/env bash
echo "CHANGES_BASE_REF=v0.5.0"
echo "CORE_CHANGED=true"
echo "WIRE_CHANGED=false"
echo "CLIENTS_CHANGED=false"
echo "PACKAGES_CHANGED=false"
echo "SCHEMA_CHANGED=false"
echo "INFRA_CHANGED=false"
STUB
chmod +x "${WORK}/tools/scripts/release/detect-changes.sh"
CALC="${WORK}/tools/scripts/release/calculate-versions.sh"

REG="${WORK}/register.md"
make_register() { # $1 = pr value, $2 = classes value
  cat > "$REG" <<MD
# Release-verdict register

## Since v0.5.0 — unreleased

- **Citation 1 — the row under probe**
  Prose.
pr: $1
classes: $2
surfaces: http
status: signal-only
MD
}

# Every probe runs the copied calculator against a fixture register, so the
# row grammar is read by the real window parser.
run_calc() { # $1 = register path, remaining args passed through (e.g. --minor)
  local reg="$1"; shift
  bash "$CALC" --register "$reg" "$@" 2>&1
}

echo "test-calculate-versions"
echo

# ── 1. TRIGGER — a routed row refuses a patch next, naming its row ──────────────
make_register 943 "shape-breaking, retirement-train"
out="$(run_calc "$REG")"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'require --minor' \
    && printf '%s' "$out" | grep -q 'pr:943' \
    && printf '%s' "$out" | grep -q 'the row under probe'; then
  ok "routed row refuses a patch next, naming the row"
else bad "routed row refuses a patch next, naming the row" "exit=$rc" "$out"; fi

# ── 2. TRIGGER — the routed row under --minor computes the era bump ─────────────
make_register 943 "shape-breaking, retirement-train"
out="$(run_calc "$REG" --minor)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'NEXT_CORE_VERSION=0.6.0'; then
  ok "routed row with --minor computes the era bump (0.5.0 -> 0.6.0)"
else bad "routed row with --minor computes the era bump (0.5.0 -> 0.6.0)" "exit=$rc" "$out"; fi

# ── 3. TRIGGER — a bare shape-breaking row computes a patch, no refusal ─────────
make_register 901 shape-breaking
out="$(run_calc "$REG")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'NEXT_CORE_VERSION=0.5.1'; then
  ok "bare shape-breaking row computes a patch without refusal"
else bad "bare shape-breaking row computes a patch without refusal" "exit=$rc" "$out"; fi

# ── 4. TRIGGER — a converted row (additive + behavioral) computes a patch ───────
make_register 906 "additive, behavioral"
out="$(run_calc "$REG")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'NEXT_CORE_VERSION=0.5.1'; then
  ok "converted row (additive + behavioral) computes a patch without refusal"
else bad "converted row (additive + behavioral) computes a patch without refusal" "exit=$rc" "$out"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
