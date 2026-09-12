#!/usr/bin/env bash
# Fail if the re-block write op — `reblock_resource` / `reblock_resource_with` /
# `reblock_resource_in_tx` — gains a caller outside the allowlist.
#
# WHY: the op's doc comment used to claim its reachability was "enforced by the
# `reblock_scope_fence` tripwire". No such fence exists — the string occurred once, in that
# comment. The real enforcement is structural, and this gate is part of it: every cloud write
# surface reaches the op only through temper-substrate's gated write-path hook
# (`apply_blocking_policy_in_tx`, called at the create/update/finalize tails), and the one
# direct production caller — the corpus-adoption Backend command in temper-services — gates
# per resource with the acting principal's own write predicates. Everything else is tests.
# A direct call is a NEW authority path into the ledger: this gate makes that a deliberate,
# reviewable allowlist edit instead of something a grep would have to catch later.
#
# WHAT IT ASSERTS
#   (a) The scan saw a populated repository. An empty `git ls-files` would satisfy the sweep
#       vacuously and report clean.
#   (b) Every tracked .rs file that mentions the op family is on the explicit allowlist below.
#   (c) Every allowlisted file still mentions the op family — a stale entry is a lie about the
#       fence's reach, the mirror image of (b).
#
# The read-only survey (`survey_reblock_resource`) is deliberately NOT fenced: it writes
# nothing. The word-boundary pattern excludes it, so a future caller of the survey does not
# owe this allowlist an edit.
#
# Scoped to TRACKED files (`git ls-files` via `git grep`), not `find` — the invariant is about
# what this repository carries, not what sits in a working copy.
set -euo pipefail
cd "$(dirname "$0")/../.."

# The op's reachable-from set. Edit this list ONLY in the same commit that adds (or retires)
# the caller, and say why in that commit: each entry is a production authority path or the
# witness that pins one.
ALLOWLIST='
crates/temper-substrate/src/writes.rs
crates/temper-substrate/tests/reblock.rs
crates/temper-substrate/tests/reblock_survey.rs
crates/temper-services/src/backend/db_backend.rs
'

PATTERN='(^|[^_[:alnum:]])reblock_resource(_with|_in_tx)?([^_[:alnum:]]|$)'

# (a) The scan must find something.
tracked="$(git ls-files '*.rs' | wc -l | tr -d ' ')"
if [ "$tracked" -lt 50 ]; then
    echo "FAIL: git ls-files returned ${tracked} .rs paths — refusing to report clean on a scan that saw nothing." >&2
    exit 1
fi

failed=0

# (b) No caller outside the allowlist.
while IFS= read -r hit; do
    [ -z "$hit" ] && continue
    if ! grep -qxF "$hit" <<<"$ALLOWLIST"; then
        echo "FAIL: re-block op reached from a non-allowlisted file: ${hit}" >&2
        failed=1
    fi
done < <(git grep -lE "$PATTERN" -- '*.rs' || true)

# (c) No stale allowlist entry.
while IFS= read -r allowed; do
    [ -z "$allowed" ] && continue
    if ! git grep -qE "$PATTERN" -- "$allowed"; then
        echo "FAIL: allowlist entry no longer mentions the re-block op (stale entry): ${allowed}" >&2
        failed=1
    fi
done <<<"$ALLOWLIST"

if [ "$failed" -ne 0 ]; then
    echo "" >&2
    echo "The re-block op is the ledger's one re-partition write; its callers are its authority" >&2
    echo "surface. Add the new caller to ALLOWLIST in this script — in the same commit, with the" >&2
    echo "reason — or route the call through the gated paths this fence protects." >&2
    exit 1
fi

echo "OK: re-block op callers match the allowlist (${tracked} .rs files scanned)."
