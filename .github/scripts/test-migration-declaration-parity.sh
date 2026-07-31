#!/usr/bin/env bash
# .github/scripts/test-migration-declaration-parity.sh
#
# The AWK half of the migration-declaration parity check. Runs every case in
# `scripts/migration-declaration-corpus.txt` through the REAL `parse_declarations`
# from audit-migration-declarations.sh — extracted from the shipped script rather
# than restated here, because a restatement would only ever test itself.
#
# The Rust half is `migrate_ledger.rs::declaration_corpus_matches_the_shell`,
# which reads the same corpus through `include_str!`. Neither harness checks the
# other directly; both check the corpus, which is what makes them agree.
#
# WHY THE TWO MUST AGREE
#   The awk half decides whether CI fails a migration that declares nothing. The
#   Rust half decides whether the DEPLOY may apply a migration. A Rust parser that
#   read `additive` where the call says `shape-breaking` would auto-apply an
#   operator-gated migration — the 2026-07-30 outage class, re-entered through the
#   mechanism built to prevent it.
#
# Prior art: .github/scripts/test-containment-parity.sh, same shape, same reason.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
AUDIT_SH="${REPO_ROOT}/.github/scripts/audit-migration-declarations.sh"
CORPUS="${REPO_ROOT}/scripts/migration-declaration-corpus.txt"

fail() { echo "FAIL: $*" >&2; exit 1; }

[ -f "$AUDIT_SH" ] || fail "audit-migration-declarations.sh not found at $AUDIT_SH"
[ -f "$CORPUS" ] || fail "corpus not found at $CORPUS"

# Pull the parser out of the shipped checker. The script runs a full audit on
# source, so it cannot be sourced; the function is extracted textually, from its
# opening line to the first line that is exactly `}`.
#
# Extraction failing SILENTLY is the hazard: an empty extraction evals to nothing,
# every case then errors on an undefined function, and a careless harness would
# report that as a pass. So the extraction is asserted non-empty AND the function
# is asserted live before any verdict is checked.
FN_SRC="$(awk '
    /^parse_declarations\(\) \{$/ { capture = 1 }
    capture { print }
    capture && /^\}$/ { exit }
' "$AUDIT_SH")"

[ -n "$FN_SRC" ] \
    || fail "could not extract parse_declarations from audit-migration-declarations.sh — renamed or reformatted?"
grep -q '^}$' <<<"$FN_SRC" \
    || fail "extracted function is not terminated — extraction captured a partial body"

eval "$FN_SRC"
command -v parse_declarations >/dev/null || fail "parse_declarations is not defined after eval"

TMPDIR_PARITY="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_PARITY"' EXIT
PROBE="${TMPDIR_PARITY}/probe.sql"

# Render awk's `ver<TAB>cls<TAB>reason<TAB>kind` as the corpus writes a parse.
#
# `awk -F'\t'` and NOT `while IFS=$'\t' read`. Tab is an IFS *whitespace*
# character, so bash collapses a run of them into one delimiter — an empty class
# token then shifts every field left and the harness reports a parser
# disagreement that is entirely its own. Measured here, not reasoned about: the
# corpus case `an empty class token` failed exactly that way on the first run.
render() {
    awk -F'\t' '
        NF == 0 { next }
        $1 == "" && $2 == "" && $3 == "" { next }
        $1 == "!MALFORMED" { print "!MALFORMED"; next }
        {
            line = $4 " " $1 " " $2
            sub(/[[:space:]]+$/, "", line)
            print line
        }
    '
}

# Prove the parser is live before trusting any verdict: one that always emitted
# nothing would satisfy every negative case in the corpus silently. A corpus is
# not a control.
printf "SELECT declare_migration(1, 'additive', 'why');\n" > "$PROBE"
[ "$(parse_declarations "$PROBE" | render)" = "declare 1 additive" ] \
    || fail "sanity: the parser did not read a plainly well-formed declaration"
printf "CREATE TABLE t (id int);\n" > "$PROBE"
[ -z "$(parse_declarations "$PROBE" | render)" ] \
    || fail "sanity: the parser read a declaration out of SQL that contains none"

CASES=0
FAILURES=0
CASE_NAME=""
EXPECTED=""

run_case() {
    [ -z "$CASE_NAME" ] && return 0
    CASES=$((CASES + 1))
    local got
    got="$(parse_declarations "$PROBE" | render)"
    if [ "$got" != "$EXPECTED" ]; then
        FAILURES=$((FAILURES + 1))
        echo "MISMATCH in case: ${CASE_NAME}" >&2
        echo "  expected: $(echo "$EXPECTED" | tr '\n' '|')" >&2
        echo "  awk gave: $(echo "$got" | tr '\n' '|')" >&2
    fi
}

while IFS= read -r line || [ -n "$line" ]; do
    trimmed="${line#"${line%%[![:space:]]*}"}"
    case "$trimmed" in
        '' | '#'*)
            continue
            ;;
        '=== '*)
            run_case
            CASE_NAME="${trimmed#=== }"
            EXPECTED=""
            : > "$PROBE"
            ;;
        'sql:'*)
            [ -n "$CASE_NAME" ] || fail "\`sql:\` before any \`===\` case header"
            content="${trimmed#sql:}"
            printf '%s\n' "${content# }" >> "$PROBE"
            ;;
        'expect:'*)
            [ -n "$CASE_NAME" ] || fail "\`expect:\` before any \`===\` case header"
            content="${trimmed#expect:}"
            content="${content# }"
            EXPECTED="${EXPECTED:+${EXPECTED}$'\n'}${content}"
            ;;
        *)
            fail "corpus line is neither a case header, \`sql:\`, nor \`expect:\`: ${line}"
            ;;
    esac
done < "$CORPUS"
run_case

# A corpus that parses to nothing would let this harness pass while checking
# nothing — the exact failure the containment corpus was written to refuse.
[ "$CASES" -ge 20 ] || fail "corpus parsed to only ${CASES} cases — did the format change?"

if [ "$FAILURES" -gt 0 ]; then
    echo "" >&2
    echo "${FAILURES} of ${CASES} corpus cases disagree with the awk parser." >&2
    echo "The Rust twin is migrate_ledger.rs::parse_declarations; one of the two has drifted." >&2
    exit 1
fi

echo "migration-declaration parity: ${CASES} corpus cases agree with the awk parser"
