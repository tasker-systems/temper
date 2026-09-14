#!/usr/bin/env bash
# .github/scripts/test-publish-ruby.sh
#
# Harness for publish-ruby.sh's rubygems.org push path.
#
# The gem publishes to rubygems.org under OIDC trusted publishing, and the
# duplicate detector is the host's versions API — probed BEFORE the build, so a
# re-cut release skips loudly without touching a registry push. Both properties
# would otherwise be witnessed only by an actual push at a tagged release. A
# stub pair (`gem` + `curl`) makes push behavior selectable per case and the
# call ORDER observable: the versions probe must decide before anything builds
# or pushes, the push must be UNPINNED (the default host is rubygems.org — a
# `--key`/`--host` push now targets the retired GitHub Packages lane), and the
# version-agreement guard must refuse before any gem command runs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/../../tools/scripts/release/publish-ruby.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Stub `gem`, logging every invocation. `push` exits per $STUB_PUSH_EXIT;
# everything else (build, specification) succeeds.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/gem" <<'STUB'
#!/bin/sh
echo "gem $*" >> "$GEM_CALLS"
if [ "$1" = "push" ]; then
    exit "${STUB_PUSH_EXIT:-0}"
fi
exit 0
STUB

# Stub `curl`, logging every invocation. The versions-API probe reads the body
# from $STUB_CURL_BODY and exits per $STUB_CURL_EXIT (0 = API answered).
cat > "$STUB_DIR/curl" <<'STUB'
#!/bin/sh
echo "curl $*" >> "$CURL_CALLS"
if [ "${STUB_CURL_EXIT:-0}" = "0" ]; then
    printf '%s' "${STUB_CURL_BODY:-[]}"
fi
exit "${STUB_CURL_EXIT:-0}"
STUB
chmod +x "$STUB_DIR/gem" "$STUB_DIR/curl"

REPO="$TMP/repo"
git init -q "$REPO"
GEM_DIR="$REPO/clients/temper-rb"
mkdir -p "$GEM_DIR/lib/temper"
printf "VERSION = '%s'\n" "${STUB_DECLARED:-9.9.9}" > "$GEM_DIR/lib/temper/version.rb"

run_target() {
    CALLS="$1"; shift
    : > "$CALLS"
    CURL_LOG="$TMP/curl-calls"
    : > "$CURL_LOG"
    (cd "$REPO" && PATH="$STUB_DIR:$PATH" GEM_CALLS="$CALLS" \
        CURL_CALLS="$CURL_LOG" HOME="$REPO" \
        STUB_CURL_BODY="${STUB_CURL_BODY:-[]}" STUB_CURL_EXIT="${STUB_CURL_EXIT:-0}" \
        bash "$TARGET" "$@")
}

# --- 1. Fresh version: probe precedes build precedes an UNPINNED push --------
# The positive control. The push MUST carry no --key and no --host: the default
# host is rubygems.org, and a pinned push targets the retired GitHub Packages
# lane. The probe must run first — it is what decides.
CALLS_GOOD="$TMP/calls-good"
run_target "$CALLS_GOOD" "9.9.9" > "$TMP/good.log" 2>&1 \
    || fail "a fresh publish was refused: $(cat "$TMP/good.log")"
grep -q "gem build" "$CALLS_GOOD" || fail "the gem was never built: $(cat "$CALLS_GOOD")"
grep -q "gem push temper-rb-9.9.9.gem" "$CALLS_GOOD" \
    || fail "the push never ran: $(cat "$CALLS_GOOD")"
grep -E "push .*(--key|--host)" "$CALLS_GOOD" \
    && fail "the push was pinned (--key/--host) — rubygems.org is the default host: $(cat "$CALLS_GOOD")"
grep -q "versions/temper-rb.json" "$TMP/curl-calls" \
    || fail "the versions-API probe never ran: $(cat "$TMP/curl-calls")"
BUILD_LINE=$(grep -n " build " "$CALLS_GOOD" | head -1 | cut -d: -f1)
PUSH_LINE=$(grep -n " push " "$CALLS_GOOD" | head -1 | cut -d: -f1)
[ "$BUILD_LINE" -lt "$PUSH_LINE" ] || fail "push ran before build: $(cat "$CALLS_GOOD")"

echo "PASS: a fresh version probes the API, builds, then pushes unpinned to rubygems.org"

# --- 2. Already-published version: loud idempotent skip, nothing runs ---------
# The bite. The versions API lists the requested version; the script must skip
# loudly, exit 0, and never build or push — a re-cut release is idempotent.
STUB_CURL_BODY='[{"number":"0.1.0"},{"number":"9.9.9"}]' run_target "$TMP/calls-skip" "9.9.9" \
    > "$TMP/skip.log" 2>&1 \
    || fail "an all-published re-run exited non-zero (must be an idempotent skip): $(cat "$TMP/skip.log")"
grep -q "already published — nothing to do" "$TMP/skip.log" \
    || fail "the published version was not reported as a loud skip: $(cat "$TMP/skip.log")"
[ ! -s "$TMP/calls-skip" ] \
    || fail "an already-published version still invoked gem: $(cat "$TMP/calls-skip")"

echo "PASS: an already-published version skips loudly and invokes no gem commands"

# --- 3. Version disagreement: refuse before any gem command ------------------
printf "VERSION = '%s'\n" "0.1.0" > "$GEM_DIR/lib/temper/version.rb"
run_target "$TMP/calls-mismatch" "9.9.9" > "$TMP/mismatch.log" 2>&1 \
    && fail "a version mismatch published anyway"
grep -q "Temper::VERSION is 0.1.0, but 9.9.9 was requested" "$TMP/mismatch.log" \
    || fail "the refusal did not name the disagreeing version: $(cat "$TMP/mismatch.log")"
[ ! -s "$TMP/calls-mismatch" ] \
    || fail "a mismatched version still invoked gem: $(cat "$TMP/calls-mismatch")"

echo "PASS: a version disagreement is refused before any gem command"
printf "VERSION = '%s'\n" "9.9.9" > "$GEM_DIR/lib/temper/version.rb"

# --- 4. Any other push failure: exit non-zero, error surfaced -----------------
# A clean probe (version absent) followed by a failing push is a real failure:
# loud, non-zero, with the push's own text surfaced.
STUB_PUSH_EXIT=1 run_target "$TMP/calls-real" "9.9.9" > "$TMP/real.log" 2>&1 \
    && fail "an unrelated push failure exited 0"
grep -q "ERROR: gem push failed" "$TMP/real.log" \
    || fail "an unrelated push failure did not name itself: $(cat "$TMP/real.log")"

echo "PASS: a push failure after a clean probe exits non-zero and names itself"

# --- 5. Dry-run: build and inspect, never push -------------------------------
CALLS_DRY="$TMP/calls-dry"
: > "$CALLS_DRY"
(cd "$REPO" && PATH="$STUB_DIR:$PATH" GEM_CALLS="$CALLS_DRY" HOME="$REPO" \
    bash "$TARGET" 9.9.9 --dry-run) > "$TMP/dry.log" 2>&1 \
    || fail "a dry-run failed: $(cat "$TMP/dry.log")"
grep -q " push " "$CALLS_DRY" && fail "the dry-run pushed: $(cat "$CALLS_DRY")"
grep -q "would push" "$TMP/dry.log" \
    || fail "the dry-run did not say what it would do: $(cat "$TMP/dry.log")"
[ ! -f "$REPO/.gem/credentials" ] || fail "the dry-run wrote gem credentials"

echo "PASS: a dry-run builds and inspects but never pushes or writes credentials"
