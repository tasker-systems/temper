#!/usr/bin/env bash
set -euo pipefail

VAULT="${TEMPER_VAULT:-$(temper status 2>/dev/null | grep 'Root:' | awk '{print $2}')}"
MS_DIR="$VAULT/milestones"

if [ ! -d "$MS_DIR" ]; then
    echo "No milestones directory found at $MS_DIR"
    exit 0
fi

moved=0
skipped=0

for f in "$MS_DIR"/*.md; do
    [ -f "$f" ] || continue

    # Extract project from frontmatter (strip quotes if present)
    project=$(awk '/^---$/{n++; next} n==1 && /^project:/{gsub(/["\047]/, "", $2); print $2; exit}' "$f")

    if [ -z "$project" ]; then
        echo "SKIP (no project): $(basename "$f")"
        skipped=$((skipped + 1))
        continue
    fi

    dest="$MS_DIR/$project"
    mkdir -p "$dest"
    mv "$f" "$dest/"
    echo "MOVED: $(basename "$f") → $project/"
    moved=$((moved + 1))
done

echo ""
echo "Done: $moved moved, $skipped skipped"
