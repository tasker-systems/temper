#!/usr/bin/env bash
#
# Type-check every Vercel TypeScript function entrypoint (api/**/*.ts) the way Vercel's
# @vercel/node builder does: one entrypoint at a time, from the repo root, with NO tsconfig.
#
# ## Why this exists
#
# `packages/temper-cloud`'s own `bun run typecheck` (tsconfig.api.json) already covers
# api/**/*.ts, and it was green while every production build logged TS2580 / TS2307 for
# packages/temper-cloud/src/oauth/* `[observed — 2026-11-01, dpl_A8nSdp4uDbRtnGuUmTSFbgJ7DNhf]`.
# The two checks resolve types from different places:
#
#   * tsconfig.api.json lives in packages/temper-cloud, so TypeScript auto-includes
#     packages/temper-cloud/node_modules/@types — where bun's isolated linker puts that
#     package's @types/node.
#   * Vercel finds no tsconfig walking up from api/ (the repo root has none), so it uses
#     defaults and auto-includes only <repo root>/node_modules/@types. Under the isolated
#     linker that holds the ROOT package.json's direct devDependencies and nothing else.
#
# An entrypoint whose import graph happened to reach a dependency whose .d.ts references node
# (pino, for one) got Buffer/process that way and passed; the oauth entrypoints did not.
# Vercel logs those errors and deploys anyway, so nothing failed — this script is the failure.
#
# The fix it guards is `@types/node` in the root package.json. Remove it and this fails.
#
# Per entrypoint, not one combined program: a combined program pulls node types in through
# ANY entry and masks the entries that lack them, which is exactly the false green above.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

tsc="$root/node_modules/.bin/tsc"
if [ ! -x "$tsc" ]; then
  echo "::error::$tsc not found — run 'bun install' (root workspace) first" >&2
  exit 1
fi

if [ -e tsconfig.json ] || [ -e api/tsconfig.json ]; then
  # Either would change what Vercel checks against, and this script would stop mirroring it.
  echo "::error::a tsconfig.json at the repo root or in api/ changes Vercel's type check; update $0 to match" >&2
  exit 1
fi

mapfile -t entries < <(find api -name '*.ts' -not -name '*.d.ts' | sort)
if [ "${#entries[@]}" -eq 0 ]; then
  echo "no api/**/*.ts entrypoints — nothing to check"
  exit 0
fi

failed=0
for entry in "${entries[@]}"; do
  if out="$("$tsc" --noEmit --skipLibCheck --esModuleInterop --target es2022 --module commonjs "$entry" 2>&1)"; then
    echo "ok    $entry"
  else
    echo "FAIL  $entry"
    printf '%s\n' "$out" | sed 's/^/      /'
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "::error::Vercel's per-entrypoint type check of api/**/*.ts fails (see above). Vercel deploys anyway, so CI is the only place this fails." >&2
  exit 1
fi
