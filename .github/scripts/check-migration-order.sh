#!/usr/bin/env bash
# Fail if this change adds a migration numbered at or below the newest migration on its base.
#
# WHY: a migration's version is its file name, chosen when the branch is cut. Another PR can land a
# higher one first, and nothing used to notice: this branch's lower-numbered file then applies
# AFTER the higher one on every deployed database (temper-migrate applies whatever is unapplied, by
# set, not by "above the newest"), but BEFORE it on any database migrated from scratch, so the two
# databases have applied the same migrations in different orders. Renumbering is free while the
# migration lives only on a branch, and is never safe once a shared database has applied it
# (sqlx stores each applied version and checksum, and refuses a source missing an applied version).
# So the check belongs before the merge.
#
# WHAT IT COMPARES. The migrations this change ADDS are the files in the index (`git ls-files`:
# the staged tree in a pre-commit hook, HEAD in CI) that are not in the base's tree. Each must sort
# strictly above the base's highest version. Two files sharing one version fail too. Files the base
# has and this change lacks are ignored: that is a branch behind its base, which a merge fixes.
#
# THE BASE, in order: the first argument; else $GITHUB_BASE_SHA (CI sets it on a pull_request and a
# merge_group, the group's parent, so a queued PR is checked against the entries ahead of it);
# else origin/main.
#
# "I could not look" never reads as clean in CI: an unresolvable base exits 2 when $GITHUB_ACTIONS
# is set. Outside CI (a pre-commit hook, offline) it warns and exits 0, because a local check that
# blocks commits on a missing fetch gets bypassed with --no-verify. A CI push to main has no base
# and no PR to fail, so it exits 0 with a note.
#
#   bash .github/scripts/check-migration-order.sh [BASE_REF]
set -euo pipefail
cd "$(dirname "$0")/../.."

MIGRATION_RE='^migrations/([0-9]+)_[^/]*\.sql$'

BASE="${1:-${GITHUB_BASE_SHA:-}}"
if [ -z "$BASE" ]; then
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        echo "check-migration-order: no base revision (a push, not a PR or queue run): nothing to compare."
        exit 0
    fi
    BASE="origin/main"
fi

if ! git rev-parse --verify --quiet "${BASE}^{tree}" >/dev/null; then
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        echo "check-migration-order: FAIL: cannot resolve the base revision '${BASE}'." >&2
        echo "  The workflow must fetch it before this step (see 'Fetch the base revision')." >&2
        exit 2
    fi
    echo "check-migration-order: WARNING: cannot resolve '${BASE}'; migration order NOT checked." >&2
    echo "  Run 'git fetch origin main' to check it locally. CI checks it regardless." >&2
    exit 0
fi

# version<TAB>path for every migration file in a listing on stdin. awk, not `sed \t` or `grep -P`:
# the pre-commit hook runs this on macOS too, where neither exists.
versions_of() {
    grep -E "$MIGRATION_RE" | awk '{ f = $0; sub(/^migrations\//, "", f); split(f, a, "_"); print a[1] "\t" $0 }' || true
}

base_list="$(git ls-tree --name-only "$BASE" -- migrations/ | versions_of | sort -n)"
head_list="$(git ls-files -- migrations/ | versions_of | sort -n)"

if [ -z "$head_list" ]; then
    echo "check-migration-order: FAIL: found no migrations in this tree; refusing to report clean." >&2
    exit 2
fi

status=0

dupes="$(cut -f1 <<<"$head_list" | uniq -d)"
if [ -n "$dupes" ]; then
    status=1
    echo "check-migration-order: FAIL: more than one migration file shares a version:" >&2
    for v in $dupes; do
        awk -F'\t' -v v="$v" '$1 == v { print "  " $2 }' <<<"$head_list" >&2
    done
fi

if [ -n "$base_list" ]; then
    base_max_line="$(tail -n1 <<<"$base_list")"
    base_max="$(cut -f1 <<<"$base_max_line")"
    base_max_file="$(cut -f2 <<<"$base_max_line")"
    # Added is judged by PATH, not version: a branch behind its base can carry a file whose version
    # the base already holds under another name, and that collision is exactly what must fail.
    base_paths="$(cut -f2 <<<"$base_list")"

    added=0
    while IFS=$'\t' read -r version path; do
        [ -z "$version" ] && continue
        grep -qxF "$path" <<<"$base_paths" && continue
        added=$((added + 1))
        # Versions are 14-digit timestamps; compare as strings of equal length, then numerically,
        # so a short or long version cannot slip past a lexical comparison.
        if [ "${#version}" -lt "${#base_max}" ] || { [ "${#version}" -eq "${#base_max}" ] && [[ ! "$version" > "$base_max" ]]; }; then
            status=1
            echo "check-migration-order: FAIL: ${path} is numbered at or below the base's newest" >&2
            echo "  migration, ${base_max_file}. Renumber it above ${base_max} (git mv, then update" >&2
            echo "  the version in its declare_migration call and anything that cites it)." >&2
        fi
    done <<<"$head_list"

    if [ "$status" -eq 0 ]; then
        echo "check-migration-order: OK: ${added} added migration(s), all above ${base_max} (base ${BASE})."
    fi
fi

exit "$status"
