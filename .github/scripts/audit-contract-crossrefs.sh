#!/usr/bin/env bash
# audit-contract-crossrefs.sh — every `$ref` from a hand-written contract into `openapi.json`
# resolves to a schema that is actually published.
#
# WHY THIS EXISTS
# ---------------
# `[2026-08-13, PR D]` `internal/api/query.openapi.yaml` used to hand-write 37 schemas, 34 of which
# `openapi.json` had come to publish independently. Two statements of one wire type is a drift site
# by construction, and it drifted: in the week before D the file accumulated five new lag entries,
# one of them SILENT — it named a refusal code, `COMPOSITION_INVALID`, that no line of `crates/`
# has ever contained. A client written from the document would key on it, never match a real
# refusal, and report a caller fault as something else.
#
# D's repair was to stop restating: every schema whose shape has shipped became a `$ref` into the
# generated document, and only the six that are DESIGNED-BUT-UNBUILT stayed hand-written. The
# argument for that repair is one sentence, and it is in the file's own header — **"a restatement
# can drift; a `$ref` cannot."**
#
# That sentence is true about drift and silent about the failure a `$ref` DOES have. A `$ref` to a
# schema that stops existing does not drift, it DANGLES: the name is simply absent from the target,
# no tool in this repo reads the yaml, and nothing says a word. Rename a Rust type — an ordinary
# refactor, and `cargo make openapi` will happily regenerate around it — and the contract quietly
# starts pointing at nothing. Without this script D would have traded 34 loud-eventually drift sites
# for 32 permanently silent ones, which is not obviously the better trade.
#
# So this is the guard that makes the header's sentence honest.
#
# WHAT IT ASSERTS
#   (a) The scan finds something. Zero refs fails: a moved or renamed contract file would make every
#       per-ref check below pass vacuously and the guard would report green having checked nothing.
#       This is the same non-vacuity rule the other audits in this directory carry.
#   (b) Every `../../openapi.json#/components/schemas/<Name>` resolves to a key that exists in
#       `openapi.json`'s `components.schemas`.
#
# WHAT IT DOES NOT ASSERT — named rather than papered over.
#   * **It does not compare shapes.** It cannot: the whole point of a `$ref` is that there is only
#     one shape left to compare against. A schema that resolves but has changed MEANING is invisible
#     here, and always will be — that is what the generated document's own drift gates are for.
#   * **It does not police the six hand-written schemas.** They are TARGETS and are SUPPOSED to
#     differ from anything published; a guard asserting they match would be asserting the opposite
#     of what they are for. When one lands, it becomes a `$ref` and this script starts covering it.
#   * **It reads the contract files as text, not as YAML.** A `$ref` written across a line break, or
#     built by an anchor, would be missed. Neither appears in the corpus today and both would be
#     unusual in a hand-written contract; if one ever does, this comment is the place that admits
#     the scan was never total.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

SPEC="openapi.json"
# Scope is a glob, not a list of files, so a second hand-written contract is covered the day it is
# added rather than the day someone remembers to extend this line.
#
# Collected with a read loop rather than `mapfile`, which is bash 4+: macOS ships bash 3.2, and a
# tripwire that only runs in CI is one nobody runs before pushing. Every audit here is meant to be
# runnable locally in seconds.
CONTRACTS=()
while IFS= read -r found; do
  [[ -n "$found" ]] && CONTRACTS+=("$found")
done < <(find internal/api -name '*.openapi.yaml' -type f 2>/dev/null | sort)

if [[ ! -f "$SPEC" ]]; then
  echo "FATAL: $SPEC not found — cannot resolve any cross-reference against it." >&2
  exit 1
fi
if [[ ${#CONTRACTS[@]} -eq 0 ]]; then
  echo "FATAL: no hand-written contract found under internal/api/. If they moved, move this scan with" >&2
  echo "       them — an empty scope makes every check below pass having verified nothing." >&2
  exit 1
fi

# Every schema name the generated document publishes.
published="$(jq -r '.components.schemas | keys[]' "$SPEC")"

total=0
dangling=0
for file in "${CONTRACTS[@]}"; do
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    total=$((total + 1))
    if ! grep -qxF "$name" <<<"$published"; then
      echo "DANGLING  $file -> $SPEC#/components/schemas/$name" >&2
      dangling=$((dangling + 1))
    fi
  done < <(grep -oE '\.\./\.\./openapi\.json#/components/schemas/[A-Za-z0-9_]+' "$file" \
             | sed 's#.*/##' | sort -u)
done

if [[ $total -eq 0 ]]; then
  echo "FATAL: found ${#CONTRACTS[@]} contract file(s) but zero cross-references into $SPEC." >&2
  echo "       Either the reference style changed or the split was undone; this guard checks" >&2
  echo "       nothing until it is pointed at whatever replaced it." >&2
  exit 1
fi

if [[ $dangling -gt 0 ]]; then
  cat >&2 <<EOF

$dangling of $total cross-reference(s) point at a schema $SPEC does not publish.

A hand-written contract \$ref'd it and the generated document no longer has it — usually a Rust
type renamed or dropped, or a route removed so utoipa stopped collecting the schema. The contract
now documents something that does not exist, and nothing else in this repo reads that file, so
this script is the only thing that will ever tell you.

Fix by pointing the \$ref at the schema's new name, or — if the shape is genuinely gone from the
product — by deleting the block, not by re-inlining the old shape. Re-inlining recreates exactly
the duplicate-definition drift the split removed.
EOF
  exit 1
fi

echo "OK: $total cross-reference(s) from ${#CONTRACTS[@]} contract file(s) all resolve in $SPEC."
