#!/usr/bin/env bash
# .github/scripts/test-containment-parity.sh
#
# The SHELL half of the containment-parity check. Runs every line of
# `scripts/install/containment-corpus.txt` through the REAL
# `is_contained_relative_sh` from install.sh — extracted from the shipped script
# rather than restated here, because a restatement would only ever test itself.
#
# The Rust half is `manifest.rs::containment_corpus_matches_the_shell`, which
# reads the same corpus through `include_str!`. Neither harness checks the other
# directly; both check the corpus, which is what makes them agree.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
INSTALL_SH="${REPO_ROOT}/scripts/install/install.sh"
CORPUS="${REPO_ROOT}/scripts/install/containment-corpus.txt"

fail() { echo "FAIL: $*" >&2; exit 1; }

[ -f "$INSTALL_SH" ] || fail "install.sh not found at $INSTALL_SH"
[ -f "$CORPUS" ] || fail "corpus not found at $CORPUS"

# Pull the function out of the shipped installer. install.sh is a top-to-bottom
# script and cannot be sourced without running an install, and giving it a
# test-only flag would add surface to a `curl | sh` installer for no runtime
# reason — so the function is extracted textually, from its opening line to the
# first line that is exactly `}`.
#
# Extraction failing SILENTLY is the hazard: an empty extraction evals to
# nothing, every corpus line then errors on an undefined function, and a
# careless harness would report that as a pass. So the extraction is asserted
# non-empty AND the function is asserted callable before any verdict is checked.
FN_SRC="$(awk '
    /^is_contained_relative_sh\(\) \{$/ { capture = 1 }
    capture { print }
    capture && /^\}$/ { exit }
' "$INSTALL_SH")"

[ -n "$FN_SRC" ] \
    || fail "could not extract is_contained_relative_sh from install.sh — did it get renamed or reformatted?"
grep -q '^}$' <<<"$FN_SRC" \
    || fail "extracted function is not terminated — extraction captured a partial body"

eval "$FN_SRC"

command -v is_contained_relative_sh >/dev/null \
    || fail "is_contained_relative_sh is not defined after eval"

# Prove the predicate is live before trusting any verdict: a function that
# always returned 0 (or always 1) would satisfy half the corpus silently, and a
# corpus is not a control.
is_contained_relative_sh "temper" \
    || fail "sanity: the predicate refused a plainly contained path"
if is_contained_relative_sh "../escape"; then
    fail "sanity: the predicate accepted a plainly escaping path"
fi

CHECKED=0
FAILURES=0

while IFS= read -r LINE; do
    # Skip blanks and comments. Note the path itself may be empty, which is a
    # real corpus case — so emptiness is only skipped BEFORE the colon split.
    [ -n "$LINE" ] || continue
    case "$LINE" in \#*) continue ;; esac

    VERDICT="${LINE%%:*}"
    PATH_UNDER_TEST="${LINE#*:}"

    case "$VERDICT" in
        accept|reject) ;;
        *) fail "corpus line has an unknown verdict '$VERDICT': $LINE" ;;
    esac

    CHECKED=$((CHECKED + 1))

    if is_contained_relative_sh "$PATH_UNDER_TEST"; then
        ACTUAL=accept
    else
        ACTUAL=reject
    fi

    if [ "$ACTUAL" != "$VERDICT" ]; then
        echo "  MISMATCH: expected $VERDICT, got $ACTUAL for path '$PATH_UNDER_TEST'" >&2
        FAILURES=$((FAILURES + 1))
    fi
done < "$CORPUS"

# The vacuity floor. A corpus that parsed to nothing would run zero comparisons
# and exit 0 — the same fail-open the installer's own manifest floor exists to
# prevent, and the same one that let three tests in this branch's history pass
# while asserting nothing.
[ "$CHECKED" -gt 0 ] \
    || fail "corpus parsed to zero cases — refusing to report a pass on an empty check"

if [ "$FAILURES" -ne 0 ]; then
    fail "$FAILURES of $CHECKED corpus cases disagree with the shell predicate"
fi

echo "PASS: install.sh's containment predicate matches all $CHECKED shared corpus cases"
