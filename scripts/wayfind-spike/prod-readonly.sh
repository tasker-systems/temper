#!/bin/zsh
# Read-only prod query runner. Never prints the connection string.
# Usage: ./pq.sh <file.sql>
set -e
CONN="$(neonctl connection-string main \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 \
  --role-name neondb_owner --database-name neondb 2>/dev/null | tail -1)"
if [ -z "$CONN" ]; then
  echo "EMPTY CONNECTION STRING — neonctl failed (check --role-name/--org-id)" >&2
  exit 1
fi
exec psql "$CONN" -X -A -F ' | ' -v ON_ERROR_STOP=1 \
  -c "SET default_transaction_read_only = on" -f "$1"
