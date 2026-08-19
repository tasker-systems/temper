#!/usr/bin/env bash
# audit-migration-declarations.sh — every migration states whether it is safe to apply ahead of the
# binary that pairs with it, and CI fails the ones that say nothing.
#
# WHY THIS EXISTS
# ---------------
# On 2026-07-30, migration `20260730000010` changed `facet_set` and `property_set` from
# `RETURNS uuid` to `RETURNS uuid[]`. `CREATE OR REPLACE FUNCTION` drops nothing and updates every
# in-repo caller in the same commit, so it read as additive — to a human and to an agent, both
# answering from memory. It was not: the running binary decodes the result by type, so it is a
# caller nobody updated. Every write through those wrappers failed for ~40 minutes while reads
# stayed healthy, so the first notification was a user hitting a write.
#
# Spec §1 (internal/superpowers/specs/2026-07-30-schema-binary-pairing-design.md), verbatim:
#   "A migration that says nothing is not thereby safe — the absent statement must be as loud as a
#    wrong one, which means CI fails on a migration with no declaration at all."
#
# This script is that half. It does NOT check whether a declaration is TRUE — that is the wire-diff
# cross-check, a separate job that reads the compiler's record. This one only enforces that an
# answer exists, names a real migration, and uses the vocabulary.
#
# THE RULE IS SET-SHAPED, NOT PER-FILE
# ------------------------------------
# Not "every file declares itself exactly once". The backfill `20260731000020` declares 148
# versions in one file, because the 148 shipped before the mechanism existed and a shipped
# migration cannot be edited (sqlx checksum-verifies at `sqlx-core/src/migrate/migrator.rs:175`).
# So:
#
#   A. every migration version is declared by SOME migration file;
#   B. every declared version corresponds to a migration file that exists;
#   C. every class token is one of the two, and every reason is non-empty.
#
# A and B together already catch the copy-paste that a filename-match rule was meant to catch:
# paste another migration's version into a new one and your own version is declared nowhere, so A
# fails. They also mean there is no legacy exemption carved out anywhere — the 148 are covered
# because something declares them, not because they are excused.
#
# TWO PARSING DECISIONS, BOTH LOAD-BEARING
# ----------------------------------------
#   * Declarations are read ACROSS LINES. Every declaration in this repo spans three or four lines,
#     because the reason is a sentence. A line-oriented grep would report every real migration as
#     undeclared while passing its own unit tests.
#   * FULL-LINE `--` COMMENTS ARE STRIPPED FIRST. Migration headers here are long and quote SQL
#     freely, and `DEPLOYING.md`'s worked example is a `declare_migration` call. A checker that
#     counts commented text lets a migration satisfy the rule by talking about it.
#     Trailing inline comments after code are NOT stripped, because `--` also appears inside string
#     literals and a naive strip would truncate a reason. Declarations are written on their own
#     lines here, so this costs nothing.
#
# USAGE
#   .github/scripts/audit-migration-declarations.sh          # verify (CI mode)
#   .github/scripts/audit-migration-declarations.sh --list   # print every parsed declaration
#   MIGRATIONS_DIR=<dir> .github/scripts/audit-migration-declarations.sh   # test-fixture seam
#
# Exit 0 = every migration declares. Exit 1 = something is undeclared, misdeclared, or malformed.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Overridable so the test harness can point the scan at a fixture directory.
MIGRATIONS_DIR="${MIGRATIONS_DIR:-migrations}"

# The vocabulary. There are now FOUR spellings of these two tokens: the `migration_class` enum in
# `20260731000010` (the SQL one), `DEPLOYING.md` § "Every migration declares which of the two it is"
# (the prose one), this copy, and `ADDITIVE`/`SHAPE_BREAKING` in
# `crates/temper-substrate/src/migrate_ledger.rs` (the router's, which decides whether a deploy may
# apply a migration). Keep all four in step.
#
# THREE OF THE FOUR ARE THE PRICE OF RUNNING WITHOUT SOMETHING: this checker must run with no
# database and no toolchain, and the router must decide about a PENDING migration, whose class is
# not in the database yet. Neither can read the enum.
#
# Worth distinguishing from the parser duplication next door, which is dangerous enough to need a
# shared corpus (`scripts/migration-declaration-corpus.txt`): a token renamed in SQL and not here
# fails EVERY migration loudly, and one renamed in SQL and not in the router halts EVERY deploy
# loudly. This drift announces itself. A parser that silently read the wrong class does not.
VALID_CLASSES="additive shape-breaking"

LIST_ONLY=0
[ "${1:-}" = "--list" ] && LIST_ONLY=1

if [ ! -d "$MIGRATIONS_DIR" ]; then
    echo "audit-migration-declarations: no such directory: ${MIGRATIONS_DIR}" >&2
    exit 1
fi

# Parse one file into `version<TAB>class<TAB>reason` lines, one per declaration.
parse_declarations() {
    awk '
        # Drop full-line comments before anything else.
        /^[[:space:]]*--/ { next }
        { buf = buf " " $0 }
        END {
            rest = buf
            # A CALL, not any mention of the name. `20260731000010` both defines these functions
            # and COMMENTs ON them, and `CREATE FUNCTION declare_migration(p_version bigint, …)` /
            # `COMMENT ON FUNCTION declare_migration(bigint, …)` are not declarations of anything.
            # Anchoring on the invoking keyword is what separates a call from its definition — and
            # it still leaves a genuinely malformed call (`SELECT declare_migration(oops`) caught
            # rather than skipped, which a looser "ignore anything unparseable" rule would not.
            #
            # Both function names are parsed, and the distinction is carried through: a
            # `reclassify_migration` entry is VALIDATED like any other (real version, known class,
            # non-empty reason) but does NOT count as declaring anything. It names a migration that
            # already shipped, so letting it satisfy "this version is declared" would let a
            # migration declare itself by revising someone else.
            while (match(rest, /(SELECT|PERFORM)[[:space:]]+(declare|reclassify)_migration[[:space:]]*\(/)) {
                kind = (substr(rest, RSTART, RLENGTH) ~ /reclassify/) ? "reclassify" : "declare"
                rest = substr(rest, RSTART + RLENGTH)

                if (!match(rest, /^[[:space:]]*[0-9]+/)) { print "!MALFORMED\t\t\t"; continue }
                ver = substr(rest, RSTART, RLENGTH)
                gsub(/[[:space:]]/, "", ver)
                rest = substr(rest, RSTART + RLENGTH)

                if (!match(rest, /'"'"'[^'"'"']*'"'"'/)) { print "!MALFORMED\t\t\t"; continue }
                cls = substr(rest, RSTART + 1, RLENGTH - 2)
                rest = substr(rest, RSTART + RLENGTH)

                if (!match(rest, /'"'"'[^'"'"']*'"'"'/)) { print ver "\t" cls "\t\t" kind; continue }
                rsn = substr(rest, RSTART + 1, RLENGTH - 2)
                rest = substr(rest, RSTART + RLENGTH)

                print ver "\t" cls "\t" rsn "\t" kind
            }
        }
    ' "$1"
}

# ── Collect the two sets ────────────────────────────────────────────────────────────────────────
FILE_VERSIONS=""      # every migration file's own version
DECLARED_VERSIONS=""  # every version some file DECLARES
ALL_NAMED_VERSIONS="" # every version any call names, declaration or reclassification
PROBLEMS=""

shopt -s nullglob
FILES=( "$MIGRATIONS_DIR"/*.sql )
shopt -u nullglob

if [ ${#FILES[@]} -eq 0 ]; then
    echo "audit-migration-declarations: no .sql files in ${MIGRATIONS_DIR}" >&2
    exit 1
fi

for f in "${FILES[@]}"; do
    base="$(basename "$f")"
    fver="${base%%_*}"
    case "$fver" in
        ''|*[!0-9]*) PROBLEMS="${PROBLEMS}  ${base}: filename does not begin with a numeric version"$'\n'; continue ;;
    esac
    FILE_VERSIONS="${FILE_VERSIONS}${fver}"$'\n'

    # Split the four fields BY HAND rather than with `IFS=$'\t' read -r ver cls rsn kind`.
    #
    # Tab is an IFS *whitespace* character, so bash collapses a run of tabs into a single
    # delimiter. A declaration with an EMPTY class token — `declare_migration(V, '', 'why')` —
    # emits `V<TAB><TAB>why<TAB>declare`, and the collapsing read then shifts every field left:
    # the reason lands in `cls`, and the check below reports "declares class 'why', which is not
    # one of: …". The verdict stayed correct and the message named the wrong token, which is the
    # kind of wrong that survives review. `[observed — 2026-07-31]`, from the parity harness.
    while IFS= read -r rec; do
        ver="${rec%%$'\t'*}"; rest="${rec#*$'\t'}"
        cls="${rest%%$'\t'*}"; rest="${rest#*$'\t'}"
        rsn="${rest%%$'\t'*}"; kind="${rest#*$'\t'}"
        [ -z "${ver}${cls}${rsn}" ] && continue

        if [ "$ver" = '!MALFORMED' ]; then
            PROBLEMS="${PROBLEMS}  ${base}: a declare_migration call could not be parsed (expected a numeric version then two quoted arguments)"$'\n'
            continue
        fi

        [ "$LIST_ONLY" -eq 1 ] && printf '%s\t%s\t%s\n' "$ver" "$cls" "$rsn"

        # A reclassification revises a version that already shipped; it never declares one.
        [ "$kind" = "declare" ] && DECLARED_VERSIONS="${DECLARED_VERSIONS}${ver}"$'\n'
        # Either kind must still name a REAL migration, so a typo is caught in both.
        ALL_NAMED_VERSIONS="${ALL_NAMED_VERSIONS}${ver}"$'\n'

        # C — vocabulary and a reason that says something.
        if ! printf '%s\n' $VALID_CLASSES | grep -qx -- "$cls"; then
            PROBLEMS="${PROBLEMS}  ${base}: version ${ver} declares class '${cls}', which is not one of: ${VALID_CLASSES}"$'\n'
        fi
        if [ -z "$(printf '%s' "$rsn" | tr -d '[:space:]')" ]; then
            PROBLEMS="${PROBLEMS}  ${base}: version ${ver} declares a class with an empty reason — a verdict without its argument"$'\n'
        fi
    done < <(parse_declarations "$f")
done

[ "$LIST_ONLY" -eq 1 ] && exit 0

FILE_SET="$(printf '%s' "$FILE_VERSIONS" | sort -u)"
DECL_SET="$(printf '%s' "$DECLARED_VERSIONS" | sort -u)"
NAMED_SET="$(printf '%s' "$ALL_NAMED_VERSIONS" | sort -u)"

# A — a migration that says nothing.
UNDECLARED="$(comm -23 <(printf '%s\n' "$FILE_SET") <(printf '%s\n' "$DECL_SET") | sed '/^$/d')"
if [ -n "$UNDECLARED" ]; then
    while read -r v; do
        [ -z "$v" ] && continue
        PROBLEMS="${PROBLEMS}  ${v}: declares nothing. Silence is not a classification — add a declare_migration call naming this version."$'\n'
    done <<< "$UNDECLARED"
fi

# B — a declaration pointing at no migration.
GHOSTS="$(comm -13 <(printf '%s\n' "$FILE_SET") <(printf '%s\n' "$NAMED_SET") | sed '/^$/d')"
if [ -n "$GHOSTS" ]; then
    while read -r v; do
        [ -z "$v" ] && continue
        PROBLEMS="${PROBLEMS}  ${v}: named by a declare/reclassify call, but no migration file has that version. A copy-pasted or mistyped version argument."$'\n'
    done <<< "$GHOSTS"
fi

if [ -n "$PROBLEMS" ]; then
    echo "audit-migration-declarations: FAILED"
    echo
    printf '%s' "$PROBLEMS"
    echo
    echo "Every migration must state whether it is safe to apply ahead of the binary that pairs"
    echo "with it. The two classes are: ${VALID_CLASSES}."
    echo "See DEPLOYING.md § 'Every migration declares which of the two it is'."
    exit 1
fi

echo "audit-migration-declarations: OK — ${#FILES[@]} migrations, every version declared."
