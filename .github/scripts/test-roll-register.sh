#!/usr/bin/env bash
# .github/scripts/test-roll-register.sh
#
# Test harness for tools/scripts/release/roll-register.sh — W7 of the register-entries spec
# (temper-artifacts:specs/2026-10-11-release-register-entries-design.md, D6): the roll moves
# exactly the entries the tag's own tree carries, leaves an entry merged after the tag in the
# open window, and a second run is a no-op.
#
# The release tree and the reader are copied into a sandbox git repo, so the script's REPO_ROOT
# resolves there and its `git ls-tree <tag>` reads a fixture history.
#
#   bash .github/scripts/test-roll-register.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

mkdir -p "${WORK}/tools/scripts" "${WORK}/release-register/unreleased"
cp -R "${REPO}/tools/scripts/release" "${WORK}/tools/scripts/release"
cp -R "${REPO}/tools/manifest" "${WORK}/tools/manifest"
cp "${REPO}/release-register/vocabulary.toml" "${WORK}/release-register/vocabulary.toml"
touch "${WORK}/release-register/unreleased/.gitkeep"
ROLL="${WORK}/tools/scripts/release/roll-register.sh"
sg() { git -C "$WORK" -c user.name=t -c user.email=t@t -c commit.gpgsign=false -c tag.gpgsign=false "$@"; }

entry() { # $1 = id
  cat > "${WORK}/release-register/unreleased/$1.toml" <<TOML
title    = "Entry $1"
pr       = "self"
classes  = ["additive"]
surfaces = ["http"]
status   = "signal-only"
body = """
Prose.
"""
TOML
}

echo "test-roll-register"
echo

entry shipped-a
entry shipped-b
sg init -q -b main
sg add -A
sg commit -q -m "release"
sg tag v1.2.3
entry after-tag
sg add -A
sg commit -q -m "merged after the tag"

# ── 1. The roll moves exactly what the tag shipped ──────────────────────────────────────────────
out="$(bash "$ROLL" v1.2.3 2>&1)"; rc=$?
R="${WORK}/release-register"
if [ "$rc" -eq 0 ] \
    && [ -f "$R/v1.2.3/shipped-a.toml" ] && [ -f "$R/v1.2.3/shipped-b.toml" ] \
    && [ ! -e "$R/unreleased/shipped-a.toml" ] && [ ! -e "$R/unreleased/shipped-b.toml" ] \
    && [ -f "$R/unreleased/after-tag.toml" ] && [ ! -e "$R/v1.2.3/after-tag.toml" ] \
    && printf '%s' "$out" | grep -q 'Rolled 2 entries'; then
  ok "the tag's entries move to v1.2.3/; the entry merged after the tag stays open"
else bad "the roll moves exactly the tag's entries" "exit=$rc" "$out" "$(cd "$R" && find . -name '*.toml' | sort)"; fi

# ── 2. The moves are staged renames — the roll PR is a commit away ──────────────────────────────
if sg diff --cached --name-status -M | grep -q '^R.*unreleased/shipped-a.toml.*v1.2.3/shipped-a.toml'; then
  ok "the moves are staged as renames"
else bad "the moves are staged as renames" "$(sg diff --cached --name-status -M)"; fi

# ── 3. A second run is a no-op ──────────────────────────────────────────────────────────────────
out="$(bash "$ROLL" v1.2.3 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'Nothing to roll' && [ -f "$R/unreleased/after-tag.toml" ]; then
  ok "a second run moves nothing"
else bad "a second run moves nothing" "exit=$rc" "$out"; fi

# ── 4. A tag that is not present refuses, rather than rolling nothing silently ──────────────────
out="$(bash "$ROLL" v9.9.9 2>&1)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'not present locally'; then
  ok "an absent tag refuses"
else bad "an absent tag refuses" "exit=$rc" "$out"; fi

# ── 5. A malformed tag argument refuses ─────────────────────────────────────────────────────────
out="$(bash "$ROLL" 1.2.3 2>&1)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'Usage'; then
  ok "a tag argument without the v<x.y.z> shape refuses"
else bad "a malformed tag argument refuses" "exit=$rc" "$out"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
