#!/usr/bin/env bash
# sqlx-schema-crosscheck.sh — does what a migration CLAIMS about itself survive contact with what
# the compiler recorded?
#
# WHY THIS EXISTS
# ---------------
# `20260731000010` made every migration state its class. A statement nobody can contradict is
# decoration, and the clause this closes says so:
#
#   "a-classification-is-checkable-against-the-migration-it-describes — The claim a migration makes
#    about its own safety can be tested against what the migration actually does. A statement that
#    no one can contradict is decoration, and this is precisely where a human and an agent both
#    erred from memory on 2026-07-30."
#
# The `.sqlx` caches are the only artifact in the repo that records the BINARY's side of the
# contract, so they are what the claim is tested against. `sqlx-wire-diff.sh` computes the movement;
# this script decides what the movement means next to the declaration.
#
# THE ASYMMETRY IS THE DESIGN
# ---------------------------
# Spec §2's table, verbatim:
#
#   | wire-diff non-empty, no migration declares shape-breaking | fail, naming the query files and
#                                                                 the migration |
#   | migration declares shape-breaking, wire-diff empty        | pass, noted — a break can be
#                                                                 invisible to the cache; failing
#                                                                 here trains under-declaration |
#   | migration added with no declaration                       | fail |
#
# Row 2 is not a softness. The caches only see what a compile-checked query touches, so a genuine
# break can be invisible to them — a function only the MCP surface calls, a column read through a
# runtime query, an operator-run cutover. Failing an honest over-declaration would teach people to
# declare `additive` and hope, which is the behaviour that produced the outage.
#
# Row 3 is DELEGATED, not re-implemented. `audit-migration-declarations.sh` already owns "silence
# fails", and a second copy of that rule here would be two spellings free to drift. This script runs
# that one and adopts its verdict.
#
# THE ROW SPEC §2 DID NOT WRITE DOWN
# ----------------------------------
# The wire moved and this change adds NO migration at all. Not in the table, and a silent pass would
# be a hole: the caches say the schema returns something different while nothing here altered the
# schema. That is either a cache regenerated against a database the repo does not describe, or a
# migration applied out of band — both of them the drift this goal exists to catch. It fails, and
# says which two things to look at. Flagged as an extension of §2 rather than folded in quietly.
#
# USAGE
#   .github/scripts/sqlx-schema-crosscheck.sh
#   .github/scripts/sqlx-schema-crosscheck.sh --wire-verdict moved|unchanged|undeterminable \
#                                             --added-migrations <file>     # fixture seam
#
# Exit 0 = the claim and the evidence agree (or there is nothing to check). Exit 1 = they do not.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

WIRE_VERDICT=""
ADDED_FILE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --wire-verdict)     WIRE_VERDICT="${2:-}"; shift 2 ;;
        --added-migrations) ADDED_FILE="${2:-}"; shift 2 ;;
        *) echo "sqlx-schema-crosscheck: unknown argument: $1" >&2; exit 1 ;;
    esac
done

MIGRATIONS_DIR="${MIGRATIONS_DIR:-migrations}"

# ── Row 3, delegated ────────────────────────────────────────────────────────────────────────────
# "Silence is a failure" has exactly one implementation. Run it and adopt its verdict rather than
# asking the same question a second way.
echo "── declaration completeness (audit-migration-declarations.sh) ──"
if ! MIGRATIONS_DIR="$MIGRATIONS_DIR" bash "${SCRIPT_DIR}/audit-migration-declarations.sh"; then
    echo
    echo "sqlx-schema-crosscheck: FAILED — a migration declares nothing, so there is no claim to"
    echo "cross-check. Fix the declaration first; the wire diff is not the problem here."
    exit 1
fi
echo

# ── The set of migrations this change ADDS ──────────────────────────────────────────────────────
if [ -z "$ADDED_FILE" ]; then
    cd "$(git rev-parse --show-toplevel)" || exit 1
    BASE=""
    for candidate in \
        "${WIRE_DIFF_BASE:-}" \
        "${GITHUB_BASE_SHA:-}" \
        "$( [ -n "${GITHUB_BASE_REF:-}" ] && git merge-base "origin/${GITHUB_BASE_REF}" HEAD 2>/dev/null )" \
        "$(git merge-base origin/main HEAD 2>/dev/null)" \
        "$(git rev-parse HEAD~1 2>/dev/null)"
    do
        [ -z "$candidate" ] && continue
        git cat-file -e "${candidate}^{commit}" 2>/dev/null || git fetch --depth=1 origin "$candidate" >/dev/null 2>&1 || true
        if git cat-file -e "${candidate}^{commit}" 2>/dev/null; then BASE="$candidate"; break; fi
    done
    ADDED_FILE="$(mktemp)"
    trap 'rm -f "$ADDED_FILE"' EXIT
    if [ -n "$BASE" ]; then
        git diff --name-only --diff-filter=A "$BASE" HEAD -- "${MIGRATIONS_DIR}/" > "$ADDED_FILE" 2>/dev/null || : > "$ADDED_FILE"
    else
        : > "$ADDED_FILE"
    fi
fi

ADDED_COUNT=0
[ -s "$ADDED_FILE" ] && ADDED_COUNT="$(grep -c . "$ADDED_FILE" || echo 0)"

# ── The wire verdict ────────────────────────────────────────────────────────────────────────────
if [ -z "$WIRE_VERDICT" ]; then
    echo "── wire diff (sqlx-wire-diff.sh) ──"
    WIRE_OUT="$(bash "${SCRIPT_DIR}/sqlx-wire-diff.sh" 2>&1)"; wrc=$?
    printf '%s\n' "$WIRE_OUT"
    echo
    case "$wrc" in
        0)  WIRE_VERDICT="unchanged" ;;
        10) WIRE_VERDICT="moved" ;;
        *)  WIRE_VERDICT="undeterminable" ;;
    esac
else
    WIRE_OUT="(wire verdict supplied: ${WIRE_VERDICT})"
fi

# ── Which added migrations declare shape-breaking ───────────────────────────────────────────────
# Parsed by audit-migration-declarations.sh --list, so the declaration is read by exactly one
# parser. Filter its output to the versions this change adds.
DECLS="$(MIGRATIONS_DIR="$MIGRATIONS_DIR" bash "${SCRIPT_DIR}/audit-migration-declarations.sh" --list 2>/dev/null)"

ADDED_VERSIONS=""
while read -r path; do
    [ -z "$path" ] && continue
    base="$(basename "$path")"
    ADDED_VERSIONS="${ADDED_VERSIONS}${base%%_*}"$'\n'
done < "$ADDED_FILE"

BREAKING=""
ADDED_DECLARED=""
while read -r v; do
    [ -z "$v" ] && continue
    cls="$(printf '%s\n' "$DECLS" | awk -F'\t' -v ver="$v" '$1 == ver { print $2; exit }')"
    ADDED_DECLARED="${ADDED_DECLARED}  ${v}: ${cls:-<none>}"$'\n'
    [ "$cls" = "shape-breaking" ] && BREAKING="${BREAKING}${v} "
done <<< "$ADDED_VERSIONS"

echo "── cross-check ──"
echo "migrations added by this change: ${ADDED_COUNT}"
[ -n "$ADDED_DECLARED" ] && printf '%s' "$ADDED_DECLARED"
echo "wire verdict: ${WIRE_VERDICT}"
echo

# ── The table ───────────────────────────────────────────────────────────────────────────────────
if [ "$WIRE_VERDICT" = "undeterminable" ]; then
    if [ "$ADDED_COUNT" -gt 0 ]; then
        echo "sqlx-schema-crosscheck: FAILED — the wire diff is UNDETERMINABLE and this change adds"
        echo "${ADDED_COUNT} migration(s), which is exactly the situation this check exists for."
        echo "'I could not look' is not 'nothing moved'. Resolve the base revision and re-run."
        exit 1
    fi
    echo "sqlx-schema-crosscheck: PASS (undeterminable wire diff, but this change adds no migration,"
    echo "so there is no claim to cross-check). Said out loud rather than passed silently."
    exit 0
fi

if [ "$WIRE_VERDICT" = "moved" ] && [ "$ADDED_COUNT" -eq 0 ]; then
    echo "sqlx-schema-crosscheck: FAILED — the wire contract moved, and there is"
    echo "no migration in this change to explain it."
    echo
    echo "The caches say the schema returns something different for a query whose text did not"
    echo "change, while nothing here altered the schema. Two things to look at:"
    echo "  1. a .sqlx cache regenerated against a database holding migrations this repo does not,"
    echo "  2. a migration applied out of band."
    echo "(This row is not in spec §2's table. It is an extension, because a silent pass here would"
    echo " be a hole in exactly the drift this goal exists to catch.)"
    exit 1
fi

if [ "$WIRE_VERDICT" = "moved" ] && [ -z "$BREAKING" ]; then
    echo "sqlx-schema-crosscheck: FAILED — the wire contract moved and no migration in this change"
    echo "declares itself shape-breaking."
    echo
    echo "Migrations added, with what each claims:"
    printf '%s' "$ADDED_DECLARED"
    echo
    echo "The moved entries are listed above. Either the classification is wrong, or the cache was"
    echo "regenerated against a schema this change does not carry. A function's return type or"
    echo "argument list changing is shape-breaking even when every in-repo caller was updated in the"
    echo "same commit — the running binary decodes by type, and it is a caller you did not update."
    echo "See DEPLOYING.md § 'A function's return type is a wire contract with the binary'."
    exit 1
fi

if [ "$WIRE_VERDICT" = "unchanged" ] && [ -n "$BREAKING" ]; then
    echo "sqlx-schema-crosscheck: PASS, NOTED — ${BREAKING% } declares shape-breaking while the wire"
    echo "diff is empty."
    echo
    echo "This is deliberately not a failure. The caches only see what a compile-checked query"
    echo "touches, so a genuine break can be invisible to them — a function only the MCP surface"
    echo "calls, a column read through a runtime query, an operator-run cutover. Failing an honest"
    echo "over-declaration would train under-declaration, which is the behaviour that produced the"
    echo "2026-07-30 outage."
    exit 0
fi

echo "sqlx-schema-crosscheck: PASS — the declaration and the compiler's record agree."
exit 0
