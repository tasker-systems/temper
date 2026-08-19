#!/usr/bin/env bash
#
# Fail if the committed `docs/reference/config` tree drifts from the `TemperConfig` that produces it.
#
# Sibling of check-cli-reference-drift.sh, same two questions and the same reasons. Read that
# script's header first; only the differences are recorded here.
#
# ## Where the prose comes from
#
# The valuable half of a config reference is the descriptions, and those are the doc comments on
# the Rust struct. The tempting shortcut is to scrape `///` lines out of config.rs. schemars is what
# makes that unnecessary: the `JsonSchema` derive lifts each doc comment into the schema, so the
# reference is rendered from a COMPILED artifact of the types rather than from a parse of the file
# that declares them.
#
# That is why the emit runs a cargo example rather than a `temper` subcommand. `temper config
# schema` would put a docs-generation detail into the user-facing CLI surface — and from there into
# the generated CLI reference, which would then be documenting the machinery that documents it.
#
# ## Two questions, not one
#
# 1. **Is it current?** Re-render and diff. Proves the page is REPRODUCIBLE.
# 2. **Is it complete?** Cross-check against a flat walk of the same schema, via
#    scripts/check-config-reference-complete.py.
#
# The second is not theoretical here in the way it was for the CLI. The renderer's tree walk had
# THREE traversal bugs while being written — `Option<T>` spelled as `anyOf`, `Vec<T>` hiding its
# element type behind `items.$ref`, and a `$ref`'d enum mislabelled — and together they silently
# omitted 10 of 27 fields. Every one would have shipped behind a green diff.
#
# ## Why the emit writes a FILE rather than piping
#
# `cargo run ... | python3 render.py` hides the producer's exit status, and a failed build then
# reaches the renderer as an empty stdin — valid-looking absence. The example's output goes to a
# temp file and the renderer is handed the path, so "the build failed" and "the schema is empty"
# stay distinguishable.
#
# Usage: bash .github/scripts/check-config-reference-drift.sh
#
# CONFIG_REF_REPO_ROOT / CONFIG_REF_TREE / CONFIG_REF_EMIT_CMD / CONFIG_REF_RENDER_CMD /
# CONFIG_REF_COMPLETE_CMD are harness seams for test-check-config-reference-drift.sh, which must run
# without cargo in the pure-bash guard-tests job. No CI job sets them.

set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO_ROOT="${CONFIG_REF_REPO_ROOT:-$DEFAULT_ROOT}"
TREE="${CONFIG_REF_TREE:-docs/reference/config}"

SCHEMA_JSON="$(mktemp)"
LOG="$(mktemp)"
trap 'rm -f "$SCHEMA_JSON" "$LOG"' EXIT

EMIT_CMD="${CONFIG_REF_EMIT_CMD:-cargo run -q -p temper-core --features config-schema --example config-reference}"
RENDER_CMD="${CONFIG_REF_RENDER_CMD:-python3 scripts/emit-config-reference.py --input $SCHEMA_JSON --out $TREE}"
COMPLETE_CMD="${CONFIG_REF_COMPLETE_CMD:-python3 scripts/check-config-reference-complete.py --input $SCHEMA_JSON --tree $TREE}"

if [ -z "$(git -C "$REPO_ROOT" ls-files -- "$TREE")" ]; then
    echo "ERROR: $TREE has no files tracked by git, so there is nothing to diff against." >&2
    echo "       Either the tree is gitignored, or the render writes somewhere this path no" >&2
    echo "       longer names. Until that is fixed this gate checks nothing." >&2
    exit 1
fi

echo "Emitting the TemperConfig schema from temper-core"
# Status captured OUTSIDE any `if !`: after `if ! cmd; then`, `$?` is the status of the NEGATION,
# so the obvious spelling exits 0 on a failed emit and the gate passes the case it exists to fail.
set +e
# shellcheck disable=SC2086 # the stub form in tests is a compound command, so word-splitting is wanted
(cd "$REPO_ROOT" && eval $EMIT_CMD) >"$SCHEMA_JSON" 2>"$LOG"
emit_status=$?
set -e
if [ "$emit_status" -ne 0 ]; then
    echo >&2
    echo "ERROR: the schema emit failed, so nothing was rendered and this gate cannot tell you" >&2
    echo "       whether the committed reference is current. Its output:" >&2
    echo >&2
    cat "$LOG" >&2
    exit "$emit_status"
fi

# An emit that "succeeds" while producing nothing would leave the tree clean and this gate green.
# The renderer refuses an empty schema too, but failing HERE names the right culprit.
if [ ! -s "$SCHEMA_JSON" ]; then
    echo >&2
    echo "ERROR: the schema emit produced an empty document. Its stderr:" >&2
    cat "$LOG" >&2
    exit 1
fi

echo "Rendering the config reference into: $TREE"
set +e
# shellcheck disable=SC2086
(cd "$REPO_ROOT" && eval $RENDER_CMD) >"$LOG" 2>&1
render_status=$?
set -e
if [ "$render_status" -ne 0 ]; then
    echo >&2
    echo "ERROR: rendering the config reference failed. Its output:" >&2
    echo >&2
    cat "$LOG" >&2
    exit "$render_status"
fi
cat "$LOG"

# The count is read from the renderer's own report, so a reworded report refuses loudly rather
# than passing silently. `|| true` is load-bearing: under `set -o pipefail` a non-matching grep
# fails the pipeline and `set -e` kills the script HERE, with none of the diagnosis printed.
described="$(grep -oE 'describing [0-9]+ fields' "$LOG" | grep -oE '[0-9]+' | head -1 || true)"
if [ -z "$described" ] || [ "$described" -lt 1 ]; then
    echo >&2
    echo "ERROR: the render reported no fields, so this gate compared nothing. Expected a line" >&2
    echo "       like 'Emitted 1 config reference files describing N fields' with N >= 1." >&2
    echo >&2
    cat "$LOG" >&2
    exit 1
fi

# `git status --porcelain`, NOT `git diff --exit-code`: the diff form cannot see a newly rendered
# page that nobody has committed yet.
DIRTY="$(git -C "$REPO_ROOT" status --porcelain -- "$TREE")"
if [ -n "$DIRTY" ]; then
    echo >&2
    echo "ERROR: the committed config reference is out of date with TemperConfig." >&2
    echo >&2
    printf '%s\n' "$DIRTY" >&2
    echo >&2
    echo "       Run: cargo run -q -p temper-core --features config-schema \\" >&2
    echo "              --example config-reference > /tmp/config-schema.json && \\" >&2
    echo "            python3 scripts/emit-config-reference.py --input /tmp/config-schema.json" >&2
    echo "       then COMMIT the result. Staging is NOT enough: this gate uses" >&2
    echo "       \`git status --porcelain\`, which reports staged-vs-HEAD changes too." >&2
    exit 1
fi

echo "Cross-checking the rendered page against a flat walk of the schema"
set +e
# shellcheck disable=SC2086
(cd "$REPO_ROOT" && eval $COMPLETE_CMD)
complete_status=$?
set -e
if [ "$complete_status" -ne 0 ]; then
    echo >&2
    echo "       ^ The page is REPRODUCIBLE (the diff above was clean) but not COMPLETE." >&2
    echo "       That combination is exactly what the drift check alone cannot detect, and it" >&2
    echo "       is not hypothetical: three traversal bugs omitted 10 of 27 fields while this" >&2
    echo "       renderer was being written." >&2
    exit "$complete_status"
fi

echo "docs/reference/config is up to date with TemperConfig (${described} fields checked)"
