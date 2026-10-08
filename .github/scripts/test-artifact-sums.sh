#!/usr/bin/env bash
# .github/scripts/test-artifact-sums.sh
#
# Test harness for tools/scripts/release/artifact-sums.sh — the check that binds each registry
# lane's publish job to the bytes its own build job produced. Every case below is one way a
# substitute artifact could arrive, or one way a legitimate skip (a version already published)
# looks; the guard is worth nothing unless the first kind fails and the second passes.
#
# Bite direction: making `verify` compare file NAMES only reds "a byte changed" and "two files'
# contents swapped"; making it ignore unexpected files reds "an extra file".
#
#   bash .github/scripts/test-artifact-sums.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUMS_SH="${SCRIPT_DIR}/../../tools/scripts/release/artifact-sums.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# expect pass|fail NAME DIR — run verify against $EXPECTED.
expect() {
    local want="$1" name="$2" dir="$3" out rc
    out="$(SUMS="$EXPECTED" bash "$SUMS_SH" verify "$dir" 2>&1)"; rc=$?
    if { [ "$want" = pass ] && [ "$rc" -eq 0 ]; } || { [ "$want" = fail ] && [ "$rc" -ne 0 ]; }; then
        ok "$name"
    else
        bad "$name" "wanted ${want}, exit ${rc}" "$out"
    fi
}

# The build job's output: two packages and a hidden file upload-artifact would leave behind.
mkdir -p "$WORK/built"
printf 'one' > "$WORK/built/a-1.0.0.tgz"
printf 'two' > "$WORK/built/b-1.0.0.tgz"
printf 'dist' > "$WORK/built/.gitignore"
EXPECTED="$(bash "$SUMS_SH" emit "$WORK/built")"

if [ "$(printf '%s\n' "$EXPECTED" | grep -c .)" -eq 2 ] && ! printf '%s' "$EXPECTED" | grep -q gitignore; then
    ok "emit lists every non-hidden file, one line each"
else
    bad "emit lists every non-hidden file, one line each" "$EXPECTED"
fi

fresh() { rm -rf "$WORK/$1"; cp -R "$WORK/built" "$WORK/$1"; rm -f "$WORK/$1/.gitignore"; }

fresh same;    expect pass "the build job's own bytes pass" "$WORK/same"
fresh byte;    printf 'X' >> "$WORK/byte/a-1.0.0.tgz"; expect fail "a byte changed" "$WORK/byte"
fresh extra;   printf 'three' > "$WORK/extra/c-1.0.0.tgz"; expect fail "an extra file" "$WORK/extra"
fresh missing; rm "$WORK/missing/b-1.0.0.tgz"; expect fail "a missing file" "$WORK/missing"
fresh swap;    printf 'two' > "$WORK/swap/a-1.0.0.tgz"; printf 'one' > "$WORK/swap/b-1.0.0.tgz"
expect fail "two files' contents swapped" "$WORK/swap"
expect fail "no download while files were expected" "$WORK/absent"

EXPECTED=""
expect pass "no download and nothing expected (the version was already published)" "$WORK/absent"
mkdir -p "$WORK/empty"; expect pass "an empty download and nothing expected" "$WORK/empty"
expect fail "files arrived though the build job produced none" "$WORK/same"

echo ""
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
