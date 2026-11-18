#!/usr/bin/env bash
# .github/scripts/check-no-line-citations.sh
#
# Guard the "no code comment cites a source location by line number" rule
# (internal/agents/code-quality.md; ruling R21 of the 2026-11-08 codebase recon).
#
# Why: a `foo.rs:123` citation is true at the commit that writes it and silently false once the
# file grows. The 2026-08-26 context-retirement review found fourteen wrong ones; two had been
# checked by their author and went stale three commits later, one landing on the single function
# that contradicted the sentence citing it. No diligence keeps such a citation true, so the format
# is refused. Name the symbol instead (fn, type, const, module path). A migration cited by file
# name is fine: a migration never changes, so that anchor cannot rot.
#
# What counts as a citation: `.rs:` followed by a digit, after `//` on the line — so `//`, `///`
# and `//!` comments and trailing comments. The `:NNN` continuation form (`` `a.rs:1` and `:9` ``)
# is not matched alone; it never appears without a leading `.rs:NNN`.
#
# Which files are held to which rule — mechanical, by path, so the sweep and the ratchet agree:
#   - STRICT: every .rs file under crates/<crate>/src/ or tests/<crate>/src/ (the e2e crate),
#     INCLUDING inline #[cfg(test)] modules and sibling tests.rs files. Zero citations. A test module inside src/ is src: it is read beside
#     the code it cites, and telling it apart would need a parser this check does not have.
#   - RATCHET: every other .rs file under crates/ or tests/ (<crate>/tests/, examples/, benches/,
#     build.rs, and tests/e2e/tests/). Each file's count is pinned in line-citation-ratchet.tsv: a count may not grow, a
#     file not listed may not start, and a count that falls must be lowered in the baseline in the
#     same change (so the recorded number stays the true one). Starting count, 2026-11-17: 114
#     citations in 35 files (56 in 25 under crates/, 58 in 10 under tests/e2e/).
#
# NOT CHECKED:
#   - Whether an existing ratchet citation leaves when its file is touched. That half of the ruling
#     is convention, judged in review; this check only stops the count growing.
#   - Block comments (`/* … */`), `#[doc = "…"]` attributes, and string literals.
#   - Line citations in any language other than Rust (`.ts:NN`, `.sql:NN`, `.py:NN`), and Rust
#     citations outside crates/ and tests/ (packages/, scripts/, .github/scripts/, internal/).
#   - Generated text derived from these comments (ts-rs .ts files, JSON-Schema snapshots,
#     tools_list.json): held through their source comment and their own drift gates.
#   - Whether a symbol a comment names exists, or whether the sentence is true.
#
# Usage:
#   .github/scripts/check-no-line-citations.sh            # check
#   .github/scripts/check-no-line-citations.sh --list     # print every citation found, then check
#   UPDATE_BASELINE=1 .github/scripts/check-no-line-citations.sh   # rewrite the ratchet baseline
#
# SCAN_ROOT (default: the repository root) points the check at a fixture tree; it scans
# "$SCAN_ROOT/crates" and "$SCAN_ROOT/tests" (if present) and reads "$SCAN_ROOT/.github/scripts/line-citation-ratchet.tsv".
#
# Bash 3.2 compatible (macOS default): no assoc arrays, no mapfile.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SCAN_ROOT="${SCAN_ROOT:-$REPO_ROOT}"
CRATES_DIR="${SCAN_ROOT}/crates"
BASELINE="${SCAN_ROOT}/.github/scripts/line-citation-ratchet.tsv"

LIST=0
if [ "${1:-}" = "--list" ]; then
    LIST=1
fi

if [ ! -d "$CRATES_DIR" ]; then
    echo "ERROR: crates directory not found: $CRATES_DIR" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Every .rs file under crates/ and tests/, relative to SCAN_ROOT, skipping build output.
ROOTS=(crates)
if [ -d "${SCAN_ROOT}/tests" ]; then
    ROOTS+=(tests)
fi
(cd "$SCAN_ROOT" && find "${ROOTS[@]}" -name '*.rs' -not -path '*/target/*' | LC_ALL=C sort) > "$TMP/files"

# TW-3: a scan that finds no Rust to read has lost its target; it must not pass.
if [ ! -s "$TMP/files" ]; then
    echo "ERROR: no .rs files found under $CRATES_DIR — the scan matched nothing" >&2
    exit 1
fi

# One line per citation: path<TAB>line<TAB>text.
: > "$TMP/hits"
while IFS= read -r rel; do
    awk -v file="$rel" '
        {
            i = index($0, "//")
            if (i > 0 && substr($0, i) ~ /\.rs:[0-9]/) {
                printf "%s\t%d\t%s\n", file, FNR, $0
            }
        }
    ' "${SCAN_ROOT}/${rel}" >> "$TMP/hits"
done < "$TMP/files"

if [ "$LIST" -eq 1 ]; then
    awk -F'\t' '{ printf "%s:%s: %s\n", $1, $2, $3 }' "$TMP/hits"
fi

# STRICT: crates/<crate>/src/… and tests/<crate>/src/…
awk -F'\t' '$1 ~ /^(crates|tests)\/[^\/]+\/src\// { printf "  %s:%s: %s\n", $1, $2, $3 }' "$TMP/hits" > "$TMP/strict"

# RATCHET: everything else, as per-file counts.
awk -F'\t' '$1 !~ /^(crates|tests)\/[^\/]+\/src\// { n[$1]++ } END { for (f in n) printf "%s\t%d\n", f, n[f] }' \
    "$TMP/hits" | LC_ALL=C sort > "$TMP/ratchet"

if [ "${UPDATE_BASELINE:-0}" = "1" ]; then
    {
        echo "# Line-number citations in comments outside crates/*/src and tests/*/src, pinned per file."
        echo "# Written by: UPDATE_BASELINE=1 .github/scripts/check-no-line-citations.sh"
        echo "# A count may fall, never rise. Replace a citation with the symbol it names."
        cat "$TMP/ratchet"
    } > "$BASELINE"
    echo "check-no-line-citations: wrote $(wc -l < "$TMP/ratchet" | tr -d ' ') file(s) to ${BASELINE#"${SCAN_ROOT}/"}"
fi

FAILED=0

if [ -s "$TMP/strict" ]; then
    FAILED=1
    {
        echo "ERROR: a comment under crates/*/src or tests/*/src cites a source location by line number:"
        cat "$TMP/strict"
        echo ""
        echo "A line number goes stale as soon as the file grows. Name the symbol the sentence is"
        echo "about instead (a fn, type, const, field or module path), or delete the citation if"
        echo "the sentence stands without it. Never swap one line number for another."
    } >&2
fi

if [ ! -f "$BASELINE" ]; then
    echo "ERROR: ratchet baseline not found: ${BASELINE}" >&2
    exit 1
fi
grep -v '^#' "$BASELINE" | grep -v '^[[:space:]]*$' | LC_ALL=C sort > "$TMP/baseline" || true

# Join current counts against the baseline; a file missing on one side counts 0 there.
LC_ALL=C join -t "$(printf '\t')" -a 1 -a 2 -e 0 -o '0,1.2,2.2' "$TMP/ratchet" "$TMP/baseline" \
    > "$TMP/joined"

awk -F'\t' '$2 > $3 { printf "  %s: %d citation(s), baseline %d\n", $1, $2, $3 }' "$TMP/joined" > "$TMP/grew"
awk -F'\t' '$2 < $3 { printf "  %s: %d citation(s), baseline %d\n", $1, $2, $3 }' "$TMP/joined" > "$TMP/fell"

if [ -s "$TMP/grew" ]; then
    FAILED=1
    {
        echo "ERROR: line-number citations in test or example code grew past the ratchet:"
        cat "$TMP/grew"
        echo ""
        echo "Outside the src trees the existing citations are tolerated until their file is"
        echo "touched, but no new one may land. Name the symbol instead. Run with --list to see"
        echo "each citation."
    } >&2
fi

if [ -s "$TMP/fell" ]; then
    FAILED=1
    {
        echo "ERROR: line-number citations fell below the ratchet baseline — good; record it:"
        cat "$TMP/fell"
        echo ""
        echo "Lower the baseline in the same change so the pinned count stays the true one:"
        echo "  UPDATE_BASELINE=1 .github/scripts/check-no-line-citations.sh"
    } >&2
fi

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

TOTAL="$(awk -F'\t' '{ s += $2 } END { print s + 0 }' "$TMP/ratchet")"
echo "check-no-line-citations: no line citation in crates/*/src or tests/*/src; ${TOTAL} held by the ratchet outside it"
