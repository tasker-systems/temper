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

# `cognitive-maps` is the odd one out and is here deliberately: it did not MOVE, it was
# RETIRED — temperkb.io is the source now. So its reappearance under docs/ is not a
# relocation regressing, it is a decision being undone, and it is the one name on this
# list with no `internal/` counterpart to point a reader at.
FORBIDDEN='superpowers development agents code-reviews security decisions research specs experiments registers api cognitive-maps'

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

# (c) No loose .md at the docs/ root. Every moved-out tree was a DIRECTORY, so the
# forbidden-name check above cannot see a stray design doc or audit dropped at the top
# level — and eleven of them sat there when this gate was first written, which is exactly
# the shape it missed. Public documentation is reached through a door or a section;
# `index.md` is the one legitimate root page in the target structure.
while IFS= read -r f; do
    [ -z "$f" ] && continue
    case "$(basename "$f")" in
        index.md) continue ;;
    esac
    echo "FAIL: $f is a loose page at the docs/ root — file it under a section, or move it to internal/." >&2
    failed=1
done < <(find docs -maxdepth 1 -type f -name '*.md' 2>/dev/null)

# Say what was CHECKED, not what is hoped. "none in an internal tree" reads as the
# invariant, and a denylist of names cannot establish the invariant — it establishes
# the absence of the names on it. One un-added name and this line would still print.
[ "$failed" -eq 0 ] && echo "OK: docs/ holds $count files; no top-level directory named any of the $(echo $FORBIDDEN | wc -w | tr -d ' ') known-internal trees, and no loose .md at the root except index.md. (Denylist: this detects the known trees returning, not that every page is fit to publish.)"
exit "$failed"
