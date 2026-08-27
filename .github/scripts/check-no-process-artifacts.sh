#!/usr/bin/env bash
# Fail if this repository carries process artifacts — specs, plans, reviews,
# spikes, or handoffs.
#
# WHY: these moved to the private `temper-artifacts` repository. The move is only
# half the change; the other half is this gate, because four in-repo files
# (CLAUDE.md, AGENTS.md, CONTRIBUTING.md, internal/README.md) used to instruct
# every agent session to write specs into `internal/superpowers/specs/`. Those
# instructions were corrected in the same commit as the move — but a fifth copy
# lives in `~/.claude/skills/temper/guidance/fundamentals.md`, which is
# MACHINE-LOCAL, is not in this repository, and no gate can reach. A session on a
# machine whose copy was never updated will helpfully re-create the directory and
# start filling it, one document at a time, and nothing else here would notice.
#
# So the failure this catches is not a person deciding to publish process
# material. It is the tree silently growing back from a stale instruction.
#
# The invariant is structural rather than classified, for the same reason
# check-docs-public-only.sh gives: "everything in docs/ is public, nothing else
# lives there" is checkable, whereas "every document in internal/ is classified
# correctly" is configuration that has to be got right every time. Categorical
# absence needs no per-document judgement, which is exactly why it was chosen over
# keeping the 27 specs the code used to cite.
#
# WHAT IT ASSERTS
#   (a) The scan saw a populated repository. An empty `git ls-files` would satisfy
#       every check below vacuously and report clean.
#   (b) `internal/superpowers/` does not exist. The tree that moved, by name — its
#       return is the specific regression this gate exists for.
#   (c) No tracked path has a forbidden directory component at ANY depth.
#
# Scoped to TRACKED files (`git ls-files`), not `find`. The invariant is about what
# this repository carries, not what sits in a working copy — and `.superpowers/` is
# a gitignored local scratch directory that a find-based scan would trip over.
set -euo pipefail
cd "$(dirname "$0")/../.."

FORBIDDEN='specs plans reviews spikes handoffs'
MOVED_TREE='internal/superpowers'

# (a) The scan must find something.
tracked="$(git ls-files | wc -l | tr -d ' ')"
if [ "$tracked" -lt 100 ]; then
    echo "FAIL: git ls-files returned ${tracked} paths — refusing to report clean on a scan that saw nothing." >&2
    exit 1
fi

failed=0

# (b) The moved tree, by name.
if git ls-files --error-unmatch "${MOVED_TREE}" >/dev/null 2>&1 \
   || [ -n "$(git ls-files "${MOVED_TREE}" | head -1)" ]; then
    echo "FAIL: ${MOVED_TREE}/ is tracked again — process artifacts moved to temper-artifacts." >&2
    echo "      If a tool or instruction put it back, fix the instruction, not just the tree." >&2
    failed=1
fi

# (c) No forbidden directory component anywhere in a tracked path.
for d in $FORBIDDEN; do
    while IFS= read -r hit; do
        [ -z "$hit" ] && continue
        echo "FAIL: ${hit} — a '${d}/' directory holds process artifacts; they belong in temper-artifacts." >&2
        failed=1
    done < <(git ls-files | grep -E "(^|/)${d}/" || true)
done

# Say what was CHECKED, not what is hoped. This is a denylist of directory names:
# it establishes the absence of those names, not that no process artifact is
# present under some other name.
[ "$failed" -eq 0 ] && echo "OK: ${tracked} tracked paths; no ${MOVED_TREE}/ and no directory named any of the $(echo $FORBIDDEN | wc -w | tr -d ' ') process-artifact trees at any depth. (Denylist: this detects those trees returning, not that every document is fit to publish.)"
exit "$failed"
