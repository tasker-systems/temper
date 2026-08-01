#!/bin/zsh
# Run ONE materialize arm against a Neon BRANCH. Never prints the connection string.
#
# `DATABASE_URL` is assigned per-command, never exported, so no later process in this shell can
# inherit a writable connection. The real guard is in the harness itself: it requires
# `--expect-host` to match the host it actually connected to (allowlist — a wrong or stale string
# fails closed), and refuses prod `main`'s endpoint unconditionally on top of that. This wrapper is
# convenience, not safety; the safety does not depend on it.
#
# Usage: ./branch-write.sh <branch-id> <arm>
#        arm ∈ none | resolution-0.2 | w-cos-1.0 | workflow-default
set -e
BRANCH="$1"; ARM="$2"
if [ -z "$BRANCH" ] || [ -z "$ARM" ]; then
  echo "usage: ./branch-write.sh <branch-id> <arm>" >&2
  exit 2
fi
if [ "$BRANCH" = "main" ] || [ "$BRANCH" = "br-square-rice-amcinqwz" ]; then
  echo "REFUSING: $BRANCH is production main." >&2
  exit 1
fi
ROOT="${0:a:h}/../.."
CONN="$(neonctl connection-string "$BRANCH" \
  --project-id crimson-fog-23541670 --org-id org-wild-snow-32921543 \
  --role-name neondb_owner --database-name neondb 2>/dev/null | tail -1)"
if [ -z "$CONN" ]; then
  echo "EMPTY CONNECTION STRING — neonctl failed" >&2
  exit 1
fi
HOST="$(print -r -- "$CONN" | sed -E 's|^.*@([^/]+)/.*$|\1|')"
DATABASE_URL="$CONN" "$ROOT/target/release/examples/region_materialize_arms" \
  --expect-host "$HOST" --arm "$ARM"
