#!/usr/bin/env bash
# .github/scripts/test-publish-ruby.sh
#
# Harness for publish-ruby.sh's two stages: `--out DIR` builds the gem, `--from DIR` pushes it to
# rubygems.org.
#
# The gem publishes under OIDC trusted publishing, and the duplicate detector is the host's
# versions API — probed in both stages, so a re-cut release skips loudly without touching a
# registry push. Both properties would otherwise be witnessed only by an actual push at a tagged
# release. Stubs for `gem`, `curl` and `ruby` make push behavior selectable per case and the call
# ORDER observable: the versions probe must decide before anything builds or pushes, the push must
# be UNPINNED (the default host is rubygems.org — a `--key`/`--host` push targets the retired
# GitHub Packages lane), the version-agreement guard must refuse before any probe or gem command
# runs, and the publish stage must judge the name and version the gem ITSELF declares (`ruby`'s
# stub answers what a real `Gem::Package#spec` read would).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/../../tools/scripts/release/publish-ruby.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Stub `gem`, logging every invocation. `build … --output F` writes F; `push` exits per
# $STUB_PUSH_EXIT.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/gem" <<'STUB'
#!/bin/sh
echo "gem $*" >> "$GEM_CALLS"
if [ "$1" = "push" ]; then
    exit "${STUB_PUSH_EXIT:-0}"
fi
if [ "$1" = "build" ]; then
    out=""
    prev=""
    for a in "$@"; do
        [ "$prev" = "--output" ] && out="$a"
        prev="$a"
    done
    [ -n "$out" ] && printf 'gem bytes\n' > "$out"
fi
exit 0
STUB

# Stub `curl`, logging every invocation. The versions-API probe reads the body from
# $STUB_CURL_BODY and exits per $STUB_CURL_EXIT (0 = API answered).
cat > "$STUB_DIR/curl" <<'STUB'
#!/bin/sh
echo "curl $*" >> "$CURL_CALLS"
if [ "${STUB_CURL_EXIT:-0}" = "0" ]; then
    printf '%s' "${STUB_CURL_BODY:-[]}"
fi
exit "${STUB_CURL_EXIT:-0}"
STUB

# Stub `ruby`: the publish stage reads the gem's own spec with it. Answers $STUB_GEM_SPEC.
cat > "$STUB_DIR/ruby" <<'STUB'
#!/bin/sh
echo "ruby $*" >> "$GEM_CALLS"
printf '%s' "${STUB_GEM_SPEC:-temper-rb 9.9.9}"
STUB
chmod +x "$STUB_DIR/gem" "$STUB_DIR/curl" "$STUB_DIR/ruby"

REPO="$TMP/repo"
git init -q "$REPO"
GEM_DIR="$REPO/clients/temper-rb"
mkdir -p "$GEM_DIR/lib/temper"
printf "VERSION = '%s'\n" "9.9.9" > "$GEM_DIR/lib/temper/version.rb"

run_target() {
    CALLS="$1"; shift
    : > "$CALLS"
    CURL_LOG="$TMP/curl-calls"
    : > "$CURL_LOG"
    (cd "$REPO" && PATH="$STUB_DIR:$PATH" GEM_CALLS="$CALLS" \
        CURL_CALLS="$CURL_LOG" HOME="$REPO" \
        STUB_CURL_BODY="${STUB_CURL_BODY:-[]}" STUB_CURL_EXIT="${STUB_CURL_EXIT:-0}" \
        STUB_GEM_SPEC="${STUB_GEM_SPEC:-temper-rb 9.9.9}" \
        bash "$TARGET" "$@")
}

OUT="$TMP/out"

# --- 1. Build stage, fresh version: probe, then build the gem into the output directory ---
CALLS_BUILD="$TMP/calls-build"
run_target "$CALLS_BUILD" "9.9.9" --out "$OUT" > "$TMP/build.log" 2>&1 \
    || fail "a fresh build was refused: $(cat "$TMP/build.log")"
grep -q "gem build" "$CALLS_BUILD" || fail "the gem was never built: $(cat "$CALLS_BUILD")"
[ -s "$OUT/temper-rb-9.9.9.gem" ] || fail "the gem was not written to the output directory: $(ls -la "$OUT")"
grep -q " push " "$CALLS_BUILD" && fail "the build stage pushed: $(cat "$CALLS_BUILD")"
grep -q "versions/temper-rb.json" "$TMP/curl-calls" \
    || fail "the versions-API probe never ran in the build stage: $(cat "$TMP/curl-calls")"

echo "PASS: the build stage probes the API, builds the gem, and never pushes"

# --- 2. Publish stage, fresh version: an UNPINNED push of the built gem, no build ---------
# The push MUST carry no --key and no --host: the default host is rubygems.org, and a pinned push
# targets the retired GitHub Packages lane.
CALLS_PUB="$TMP/calls-pub"
run_target "$CALLS_PUB" "9.9.9" --from "$OUT" > "$TMP/pub.log" 2>&1 \
    || fail "a fresh publish was refused: $(cat "$TMP/pub.log")"
grep -q "gem push $OUT/temper-rb-9.9.9.gem" "$CALLS_PUB" \
    || fail "the push never ran on the built gem: $(cat "$CALLS_PUB")"
grep -E "push .*(--key|--host)" "$CALLS_PUB" \
    && fail "the push was pinned (--key/--host) — rubygems.org is the default host: $(cat "$CALLS_PUB")"
grep -q "gem build" "$CALLS_PUB" && fail "the publish stage built: $(cat "$CALLS_PUB")"
SPEC_LINE=$(grep -n "^ruby " "$CALLS_PUB" | head -1 | cut -d: -f1)
PUSH_LINE=$(grep -n " push " "$CALLS_PUB" | head -1 | cut -d: -f1)
[ -n "$SPEC_LINE" ] && [ "$SPEC_LINE" -lt "$PUSH_LINE" ] \
    || fail "the gem's own spec was not read before the push: $(cat "$CALLS_PUB")"

echo "PASS: the publish stage reads the gem's own spec, then pushes it unpinned to rubygems.org"

# --- 3. Already-published version: both stages skip loudly, nothing runs ---------------
for stage in out from; do
    STUB_CURL_BODY='[{"number":"0.1.0"},{"number":"9.9.9"}]' run_target "$TMP/calls-skip" "9.9.9" "--$stage" "$TMP/skipdir" \
        > "$TMP/skip.log" 2>&1 \
        || fail "an all-published re-run of --$stage exited non-zero (must be an idempotent skip): $(cat "$TMP/skip.log")"
    grep -q "already published — nothing to do" "$TMP/skip.log" \
        || fail "the published version was not reported as a loud skip in --$stage: $(cat "$TMP/skip.log")"
    [ ! -s "$TMP/calls-skip" ] \
        || fail "an already-published version still invoked gem in --$stage: $(cat "$TMP/calls-skip")"
done

echo "PASS: an already-published version skips loudly in both stages and invokes no gem commands"

# --- 4. Version disagreement: the build stage refuses before any probe or gem command ---
printf "VERSION = '%s'\n" "0.1.0" > "$GEM_DIR/lib/temper/version.rb"
run_target "$TMP/calls-mismatch" "9.9.9" --out "$TMP/mismatch-out" > "$TMP/mismatch.log" 2>&1 \
    && fail "a version mismatch built anyway"
grep -q "Temper::VERSION is 0.1.0, but 9.9.9 was requested" "$TMP/mismatch.log" \
    || fail "the refusal did not name the disagreeing version: $(cat "$TMP/mismatch.log")"
[ ! -s "$TMP/calls-mismatch" ] \
    || fail "a mismatched version still invoked gem: $(cat "$TMP/calls-mismatch")"
[ ! -s "$TMP/curl-calls" ] \
    || fail "a mismatched version still probed the registry: $(cat "$TMP/curl-calls")"
printf "VERSION = '%s'\n" "9.9.9" > "$GEM_DIR/lib/temper/version.rb"

echo "PASS: a version disagreement is refused before any probe or gem command"

# --- 5. Publish stage: a gem declaring another name or version is refused ---------------
STUB_GEM_SPEC="temper-rb 9.9.8" run_target "$TMP/calls-tamper" "9.9.9" --from "$OUT" > "$TMP/tamper.log" 2>&1 \
    && fail "a gem declaring another version was pushed"
grep -q "declares 'temper-rb 9.9.8', expected 'temper-rb 9.9.9'" "$TMP/tamper.log" \
    || fail "the refusal did not name the gem's own declaration: $(cat "$TMP/tamper.log")"
grep -q " push " "$TMP/calls-tamper" && fail "a mismatched gem still reached push: $(cat "$TMP/calls-tamper")"

echo "PASS: the publish stage refuses a gem whose own spec disagrees with the tag"

# --- 6. Publish stage: an unpublished version with no gem is refused --------------------
mkdir -p "$TMP/empty"
run_target "$TMP/calls-missing" "9.9.9" --from "$TMP/empty" > "$TMP/missing.log" 2>&1 \
    && fail "a missing gem for an unpublished version exited 0"
grep -q "temper-rb-9.9.9.gem is not in $TMP/empty, and temper-rb 9.9.9 is not published" "$TMP/missing.log" \
    || fail "the refusal did not say what is missing: $(cat "$TMP/missing.log")"

echo "PASS: an unpublished version whose gem is absent fails loudly"

# --- 7. Any other push failure: exit non-zero, error surfaced ----------------------------
STUB_PUSH_EXIT=1 run_target "$TMP/calls-real" "9.9.9" --from "$OUT" > "$TMP/real.log" 2>&1 \
    && fail "an unrelated push failure exited 0"
grep -q "ERROR: gem push failed" "$TMP/real.log" \
    || fail "an unrelated push failure did not name itself: $(cat "$TMP/real.log")"

echo "PASS: a push failure after a clean probe exits non-zero and names itself"

# --- 8. Dry-run: the publish stage never pushes or writes credentials --------------------
CALLS_DRY="$TMP/calls-dry"
run_target "$CALLS_DRY" "9.9.9" --from "$OUT" --dry-run > "$TMP/dry.log" 2>&1 \
    || fail "a dry-run failed: $(cat "$TMP/dry.log")"
grep -q " push " "$CALLS_DRY" && fail "the dry-run pushed: $(cat "$CALLS_DRY")"
grep -q "would push" "$TMP/dry.log" \
    || fail "the dry-run did not say what it would do: $(cat "$TMP/dry.log")"
[ ! -f "$REPO/.gem/credentials" ] || fail "the dry-run wrote gem credentials"

echo "PASS: a dry-run never pushes or writes credentials"

# --- 9. Stage flags: exactly one of --out/--from, and --dry-run only with --from --------
for args in "9.9.9" "9.9.9 --out a --from b" "9.9.9 --out a --dry-run"; do
    # shellcheck disable=SC2086
    if run_target "$TMP/calls-flags" $args > "$TMP/flags.log" 2>&1; then
        fail "'$args' was accepted"
    fi
done

echo "PASS: a call naming no stage, both stages, or a build-stage dry-run is refused"
