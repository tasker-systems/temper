#!/usr/bin/env bash
#
# Fail if any committed ts-rs generated type drifts from the Rust that produces it.
#
# Regenerates every TypeScript tree `cargo make generate-ts-types` writes and fails if the result
# differs from what is committed.
#
# ## Why this exists, and why it is GENERAL rather than Slack-shaped
#
# Every other gate in this repo lives inside ONE language: clippy, tsc, biome, svelte-check. A type
# that crosses the Rust/TypeScript boundary is therefore checked by nothing — each side is
# self-consistent, so each side's gate is green while the two disagree.
#
# That is not hypothetical. PR #498 merged cleanly with `tsc` passing and 79/79 tests green, while
# the mention agent spoke a mint contract the server had stopped emitting: the TS types were
# internally consistent and every test mock asserted the retired shape. Shipped, it would have
# answered "please try again in a moment" to every refusal — the generic retry line, for states
# where retrying never works.
#
# A gate named after that incident would have caught that instance and left the identical hole
# everywhere else generate-ts-types emits. So this one covers EVERY tree, derived from the
# generator itself rather than from a list kept here — add a third consumer and it is covered with
# no edit to this file.
#
# ## What it does NOT cover
#
# ts-rs only reaches types that carry its derives, and the crate that owns most wire `status`
# discriminants — temper-api — has no ts-rs at all. `SlackMintResponse`'s `status` tag and
# `SlackLinkStateResponse`'s whole shape are still hand-mirrored in the mention agent, ungated.
# Those two are also allow-listed OUT of openapi.json, so the temper-ts SDK gate misses them for
# the same reason. Tracked as temper task 019f910b-579b-74c2-bf05-702aaed0a011, with the options
# weighed. Do not read a green run here as "the wire is covered" — this gate covers the types
# ts-rs emits, which is not the same set.
#
# Usage: bash .github/scripts/check-ts-rs-drift.sh
#
# TS_RS_DRIFT_REPO_ROOT / TS_RS_DRIFT_GENERATE_CMD are harness seams for
# test-check-ts-rs-drift.sh (which must run without cargo, in the pure-bash guard-tests job).
# No CI job sets them; rust-quality runs this unstubbed.

set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO_ROOT="${TS_RS_DRIFT_REPO_ROOT:-$DEFAULT_ROOT}"
GENERATE_CMD="${TS_RS_DRIFT_GENERATE_CMD:-cargo make generate-ts-types}"
CARGO_MAKE_FILE="$REPO_ROOT/tools/cargo-make/main.toml"

# Derive the output trees from the generator's own TS_RS_EXPORT_DIR lines. DELIBERATELY not a list
# maintained here: two copies of "where the types go" is the same drift this gate exists to stop,
# one level up. A tree added to main.toml is covered the moment it is added.
TREES=()
if [ -f "$CARGO_MAKE_FILE" ]; then
    while IFS= read -r tree; do
        [ -n "$tree" ] && TREES+=("$tree")
    done < <(grep -o 'TS_RS_EXPORT_DIR=\${CARGO_MAKE_WORKING_DIRECTORY}/[^ ]*' "$CARGO_MAKE_FILE" |
        sed 's|^TS_RS_EXPORT_DIR=\${CARGO_MAKE_WORKING_DIRECTORY}/||' | sort -u)
fi

# Zero trees means the derivation stopped matching main.toml — a renamed task, reformatted script
# lines, a move to another file. The loop below would then run zero times and this gate would exit
# 0 having checked nothing. Refuse instead: a gate that cannot fail is worse than no gate, because
# it reads as coverage.
if [ ${#TREES[@]} -eq 0 ]; then
    echo "ERROR: no ts-rs output trees found in $CARGO_MAKE_FILE." >&2
    echo "       This gate derives them from TS_RS_EXPORT_DIR=\${CARGO_MAKE_WORKING_DIRECTORY}/…" >&2
    echo "       lines; finding none means that pattern has drifted and this gate is checking" >&2
    echo "       nothing. Fix the derivation rather than deleting this check." >&2
    exit 1
fi

# Assert each tree has something TRACKED before regenerating. `git status` over a path git does not
# know about reports nothing, so without this a gitignored or never-committed tree would pass
# forever while checking nothing. Same reasoning as check-temper-ts-drift.sh's ls-files assertion,
# applied per tree because there is more than one.
for tree in "${TREES[@]}"; do
    if [ -z "$(git -C "$REPO_ROOT" ls-files -- "$tree")" ]; then
        echo "ERROR: $tree has no files tracked by git, so there is nothing to diff against." >&2
        echo "       Either the tree is gitignored, or generate-ts-types writes somewhere this" >&2
        echo "       path no longer names. Until that is fixed this gate checks nothing." >&2
        exit 1
    fi
done

echo "Regenerating ts-rs types into: ${TREES[*]}"
# shellcheck disable=SC2086 # the stub form in tests is a compound command, so word-splitting is wanted
(cd "$REPO_ROOT" && eval $GENERATE_CMD) >/dev/null

# `git status --porcelain`, NOT `git diff --exit-code`. The diff form reports only tracked-file
# changes, so a NEWLY derived type — a brand-new .ts nobody has committed — is invisible to it and
# the gate passes while the consumer has no generated counterpart at all. That is not theoretical:
# packages/temper-ui/.../slack_link.ts sat exactly like that after its derives were added.
# `status` covers modified, deleted, AND untracked in one predicate.
DIRTY=""
for tree in "${TREES[@]}"; do
    tree_status="$(git -C "$REPO_ROOT" status --porcelain -- "$tree")"
    [ -n "$tree_status" ] && DIRTY+="$tree_status"$'\n'
done

if [ -n "$DIRTY" ]; then
    echo >&2
    echo "ERROR: generated TypeScript types are out of date with the Rust that produces them." >&2
    echo >&2
    printf '%s' "$DIRTY" >&2
    echo >&2
    echo "       Run: cargo make generate-ts-types" >&2
    echo "       then STAGE the result (git add). This gate compares against git, not against a" >&2
    echo "       fresh build, so types you have just correctly regenerated still fail while they" >&2
    echo "       sit unstaged." >&2
    exit 1
fi

echo "ts-rs generated types are up to date with the Rust (${#TREES[@]} tree(s) checked)"
