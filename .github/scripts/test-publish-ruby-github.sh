#!/usr/bin/env bash
# .github/scripts/test-publish-ruby-github.sh
#
# Harness for publish-ruby.sh's GitHub Packages push path.
#
# GitHub Packages RubyGems has no versions-list API, so the duplicate detector
# is the push refusal itself — a text classifier over gem push's failure. That
# classifier would otherwise be witnessed only by an actual registry push at a
# tagged release. A stub `gem` makes push behavior selectable per case and the
# call ORDER observable: the version-agreement guard must refuse before any
# gem command runs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/../../tools/scripts/release/publish-ruby.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Stub `gem`, logging every invocation. `push` exits per $STUB_PUSH_EXIT and
# emits $STUB_PUSH_LOG to stderr; everything else succeeds.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/gem" <<'STUB'
#!/bin/sh
echo "gem $*" >> "$GEM_CALLS"
if [ "$1" = "push" ]; then
    [ -n "${STUB_PUSH_LOG:-}" ] && printf '%s\n' "$STUB_PUSH_LOG" >&2
    exit "${STUB_PUSH_EXIT:-0}"
fi
exit 0
STUB
chmod +x "$STUB_DIR/gem"

REPO="$TMP/repo"
git init -q "$REPO"
GEM_DIR="$REPO/clients/temper-rb"
mkdir -p "$GEM_DIR/lib/temper"
printf "VERSION = '%s'\n" "${STUB_DECLARED:-9.9.9}" > "$GEM_DIR/lib/temper/version.rb"

run_target() {
    CALLS="$1"; shift
    : > "$CALLS"
    (cd "$REPO" && PATH="$STUB_DIR:$PATH" GEM_CALLS="$CALLS" HOME="$REPO" \
        GITHUB_TOKEN=stub-token bash "$TARGET" "$@")
}

# --- 1. Fresh version: build precedes push, host and key pinned --------------
# The positive control. The push MUST carry --key github --host <GitHub
# Packages>: a push without them goes to rubygems.org, which is the wrong
# place by construction.
CALLS_GOOD="$TMP/calls-good"
run_target "$CALLS_GOOD" "9.9.9" > "$TMP/good.log" 2>&1 \
    || fail "a fresh publish was refused: $(cat "$TMP/good.log")"
grep -q "gem build" "$CALLS_GOOD" || fail "the gem was never built: $(cat "$CALLS_GOOD")"
grep -q "push --key github --host https://rubygems.pkg.github.com/tasker-systems temper-rb-9.9.9.gem" \
    "$CALLS_GOOD" || fail "the push was not pinned to GitHub Packages: $(cat "$CALLS_GOOD")"
BUILD_LINE=$(grep -n " build " "$CALLS_GOOD" | head -1 | cut -d: -f1)
PUSH_LINE=$(grep -n " push " "$CALLS_GOOD" | head -1 | cut -d: -f1)
[ "$BUILD_LINE" -lt "$PUSH_LINE" ] || fail "push ran before build: $(cat "$CALLS_GOOD")"

echo "PASS: a fresh version builds then pushes to GitHub Packages with the key pinned"

# --- 2. Duplicate refusal: loud idempotent skip, exit 0 ----------------------
# The bite. GitHub refuses a duplicate version push; the classifier must read
# that as "already published — nothing to do" and exit 0, never as an error
# and never as a second push attempt.
for TEXT in "temper-rb-9.9.9 has already been published" \
            "The gem version 9.9.9 already exists"; do
    CALLS_DUP="$TMP/calls-dup"
    : > "$CALLS_DUP"
    (cd "$REPO" && PATH="$STUB_DIR:$PATH" GEM_CALLS="$CALLS_DUP" HOME="$REPO" \
        GITHUB_TOKEN=stub STUB_PUSH_EXIT=1 STUB_PUSH_LOG="$TEXT" \
        bash "$TARGET" 9.9.9) > "$TMP/dup.log" 2>&1 \
        || fail "a duplicate refusal exited non-zero (must be an idempotent skip) [$TEXT]"
    grep -q "already published — nothing to do" "$TMP/dup.log" \
        || fail "the duplicate refusal was not reported as a loud skip [$TEXT]: $(cat "$TMP/dup.log")"
    [ "$(grep -c " push " "$CALLS_DUP")" -eq 1 ] \
        || fail "the duplicate path pushed more than once [$TEXT]: $(cat "$CALLS_DUP")"
done

echo "PASS: a duplicate-version refusal skips loudly and exactly once"

# --- 3. Any other push failure: exit non-zero, error surfaced ----------------
CALLS_REAL="$TMP/calls-real"
(cd "$REPO" && PATH="$STUB_DIR:$PATH" GEM_CALLS="$CALLS_REAL" HOME="$REPO" \
    GITHUB_TOKEN=stub STUB_PUSH_EXIT=1 STUB_PUSH_LOG="bad response Unauthorized" \
    bash "$TARGET" 9.9.9) > "$TMP/real.log" 2>&1 \
    && fail "an unrelated push failure exited 0"
grep -q "ERROR: gem push failed" "$TMP/real.log" \
    || fail "an unrelated push failure did not name itself: $(cat "$TMP/real.log")"
grep -q "bad response Unauthorized" "$TMP/real.log" \
    || fail "the push failure's own text was swallowed: $(cat "$TMP/real.log")"

echo "PASS: an unrelated push failure exits non-zero with its text surfaced"

# --- 4. Version disagreement: refuse before any gem command ------------------
printf "VERSION = '%s'\n" "0.1.0" > "$GEM_DIR/lib/temper/version.rb"
run_target "$TMP/calls-mismatch" "9.9.9" > "$TMP/mismatch.log" 2>&1 \
    && fail "a version mismatch published anyway"
grep -q "Temper::VERSION is 0.1.0, but 9.9.9 was requested" "$TMP/mismatch.log" \
    || fail "the refusal did not name the disagreeing version: $(cat "$TMP/mismatch.log")"
[ ! -s "$TMP/calls-mismatch" ] \
    || fail "a mismatched version still invoked gem: $(cat "$TMP/calls-mismatch")"

echo "PASS: a version disagreement is refused before any gem command"

# --- 5. Dry-run: build and inspect, never push, never need credentials -------
printf "VERSION = '%s'\n" "9.9.9" > "$GEM_DIR/lib/temper/version.rb"
rm -f "$REPO/.gem/credentials"
CALLS_DRY="$TMP/calls-dry"
: > "$CALLS_DRY"
(cd "$REPO" && env -u GITHUB_TOKEN PATH="$STUB_DIR:$PATH" GEM_CALLS="$CALLS_DRY" HOME="$REPO" \
    bash "$TARGET" 9.9.9 --dry-run) > "$TMP/dry.log" 2>&1 \
    || fail "a dry-run failed: $(cat "$TMP/dry.log")"
grep -q " push " "$CALLS_DRY" && fail "the dry-run pushed: $(cat "$CALLS_DRY")"
grep -q "would push" "$TMP/dry.log" \
    || fail "the dry-run did not say what it would do: $(cat "$TMP/dry.log")"
[ ! -f "$REPO/.gem/credentials" ] || fail "the dry-run wrote gem credentials"

echo "PASS: a dry-run builds and inspects but never pushes or writes credentials"
