#!/usr/bin/env bash
#
# One-off: migrate a temper vault to the owner-segmented layout.
#
#   Before: <vault>/<context>/<doc_type>/<slug>.md
#   After:  <vault>/@me/<context>/<doc_type>/<slug>.md
#
# Also:
#   - rewrites <vault>/.temper/manifest.json entries[*].path to prepend "@me/"
#   - backfills "temper-owner: \"@me\"" into every file's managed frontmatter
#     that does not already have one
#
# Idempotent: re-running on an already-migrated vault is a no-op.
#
# Usage:
#   ./migrate-vault-to-owner-segmented.sh <vault_path>          # dry-run
#   ./migrate-vault-to-owner-segmented.sh <vault_path> --apply  # actually do it

set -euo pipefail

VAULT="${1:-}"
MODE="${2:-dry-run}"

if [[ -z "$VAULT" ]]; then
    echo "usage: $0 <vault_path> [--apply]" >&2
    exit 1
fi

if [[ ! -d "$VAULT" ]]; then
    echo "error: vault not found: $VAULT" >&2
    exit 1
fi

if [[ "$MODE" != "dry-run" && "$MODE" != "--apply" ]]; then
    echo "error: second arg must be omitted (dry-run) or --apply" >&2
    exit 1
fi

DRY=1
if [[ "$MODE" == "--apply" ]]; then
    DRY=0
fi

say() { echo "[migrate-vault] $*"; }
do_cmd() {
    if [[ "$DRY" -eq 1 ]]; then
        echo "  DRY-RUN: $*"
    else
        echo "  $*"
        eval "$@"
    fi
}

say "vault: $VAULT"
say "mode:  $( [[ $DRY -eq 1 ]] && echo dry-run || echo APPLY )"
say ""

# 1. Move top-level context directories into @me/
say "==> Step 1: relocate context directories under @me/"
mkdir -p "$VAULT/@me" 2>/dev/null || true
moved=0
for dir in "$VAULT"/*; do
    [[ -d "$dir" ]] || continue
    name=$(basename "$dir")
    case "$name" in
        "@me"|"@"*|"+"*|".temper"|".git")
            continue  # already migrated or special
            ;;
    esac
    do_cmd "mv \"$dir\" \"$VAULT/@me/$name\""
    moved=$((moved+1))
done
say "  moved $moved context directories"
say ""

# 2. Rewrite manifest.json paths (prepend @me/)
MANIFEST="$VAULT/.temper/manifest.json"
if [[ -f "$MANIFEST" ]]; then
    say "==> Step 2: rewrite manifest paths"
    BAK="$MANIFEST.pre-migration-bak"
    if [[ "$DRY" -eq 0 ]]; then
        cp "$MANIFEST" "$BAK"
        say "  backup: $BAK"
    fi

    # Count paths that need rewriting (do not already start with @ or +)
    need=$(jq '[.entries[]? | .path | select(startswith("@") | not) | select(startswith("+") | not)] | length' "$MANIFEST")
    say "  manifest entries to rewrite: $need"

    if [[ "$need" -gt 0 ]]; then
        if [[ "$DRY" -eq 0 ]]; then
            tmp=$(mktemp)
            jq '(.entries[]? | .path) |= (if (startswith("@") or startswith("+")) then . else "@me/" + . end)' "$MANIFEST" > "$tmp"
            mv "$tmp" "$MANIFEST"
            say "  manifest rewritten"
        else
            say "  DRY-RUN: would rewrite $need entries"
        fi
    fi
else
    say "==> Step 2: no manifest.json — skipping"
fi
say ""

# 3. Backfill temper-owner: "@me" into frontmatter of every file under @me/
say "==> Step 3: backfill temper-owner frontmatter"
backfilled=0
if [[ -d "$VAULT/@me" ]]; then
    while IFS= read -r -d '' file; do
        # Skip files that already have a temper-owner field
        if head -30 "$file" | grep -q '^temper-owner:'; then
            continue
        fi
        # Check the file has frontmatter at all (starts with ---)
        if ! head -1 "$file" | grep -q '^---$'; then
            continue
        fi
        if [[ "$DRY" -eq 1 ]]; then
            echo "  DRY-RUN: would backfill temper-owner in $file"
        else
            # Insert "temper-owner: \"@me\"" after the first --- line
            awk '
                BEGIN { inserted=0 }
                /^---$/ && !inserted && NR > 1 { print "temper-owner: \"@me\""; inserted=1 }
                { print }
                END { if (!inserted) exit 1 }
            ' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
        fi
        backfilled=$((backfilled+1))
    done < <(find "$VAULT/@me" -type f -name '*.md' -print0)
fi
say "  backfilled temper-owner in $backfilled files"
say ""

say "==> Done."
if [[ "$DRY" -eq 1 ]]; then
    say "This was a dry run. Re-run with --apply to execute."
fi
