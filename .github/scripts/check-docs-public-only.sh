#!/usr/bin/env bash
# Fail if docs/ contains anything that is not public documentation.
#
# WHY: docs/ is synced to the public documentation site. The safety property is
# structural, not configured — "everything in docs/ is public, nothing else
# lives there" — because the alternative, an allowlist, was got wrong once and
# published internal security audits.
#
# This asserts the ABSENCE of known-internal directory names. It cannot judge
# whether a given page is fit to publish; it only catches the failure mode that
# actually occurred, which was a whole internal tree sitting under docs/.
set -euo pipefail
cd "$(dirname "$0")/../.."

FORBIDDEN='superpowers development agents code-reviews security decisions research specs experiments registers api'

# (a) The scan must find something. A docs/ that does not exist, or is empty,
# would satisfy every assertion below while checking nothing.
count=$(find docs -type f 2>/dev/null | wc -l | tr -d ' ')
if [ "$count" -eq 0 ]; then
    echo "FAIL: docs/ has no files — refusing to report clean on an empty scan." >&2
    exit 1
fi

# (b) No forbidden directory may exist under docs/.
failed=0
for d in $FORBIDDEN; do
    if [ -e "docs/$d" ]; then
        echo "FAIL: docs/$d exists — internal material belongs in internal/." >&2
        failed=1
    fi
done

[ "$failed" -eq 0 ] && echo "OK: docs/ holds $count files, none in an internal tree."
exit "$failed"
