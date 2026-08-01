#!/bin/zsh
# Read-only query runner against a NEON BRANCH. Never prints the connection string.
#
# The sibling `prod-readonly.sh` targets `main`; this one targets a disposable branch, and is what
# validates that a branch reproduces prod before any arm measured on it is believed. It keeps the
# same `SET default_transaction_read_only = on` floor, so a read probe cannot write even here —
# the branch's writes go through `region_materialize_arms`, which carries its own allowlist guard,
# and never through this script.
#
# Usage: ./branch-readonly.sh <branch-id> <file.sql>
set -e
BRANCH="$1"; FILE="$2"
if [ -z "$BRANCH" ] || [ -z "$FILE" ]; then
  echo "usage: ./branch-readonly.sh <branch-id> <file.sql>" >&2
  exit 2
fi
if [ "$BRANCH" = "main" ] || [ "$BRANCH" = "br-square-rice-amcinqwz" ]; then
  echo "REFUSING: $BRANCH is production main. Use prod-readonly.sh for prod reads." >&2
  exit 1
fi
CONN="$(neonctl connection-string "$BRANCH" \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 \
  --role-name neondb_owner --database-name neondb 2>/dev/null | tail -1)"
if [ -z "$CONN" ]; then
  echo "EMPTY CONNECTION STRING — neonctl failed (check branch id/--org-id)" >&2
  exit 1
fi
exec psql "$CONN" -X -A -F ' | ' -v ON_ERROR_STOP=1 \
  -c "SET default_transaction_read_only = on" -f "$FILE"
