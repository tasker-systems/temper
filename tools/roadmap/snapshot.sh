#!/usr/bin/env bash
# Draw temper's dev-roadmap snapshot from the Temper records (the ROAD rules of the conventions
# baseline; Temper task 01a1fdd1).
#
#   tools/roadmap/snapshot.sh --dry-run   # print the snapshot, write nothing
#   tools/roadmap/snapshot.sh             # create it as a new research resource
#
# Reads every goal in +temper-dev/core and every task in +temper-dev/core-workflows with
# `temper resource list --all`, then every task's edges with `temper graph traverse --depth 1`,
# seeded with every listed task in batches of at most 250 (the most the service takes in one
# call; it refuses more rather than drop any). roadmap.jq holds the rule; this script only reads
# and writes.
#
# The one write is `temper resource create --type research` in +temper-community/dev-roadmap,
# tagged roadmap-snapshot: it never updates, moves or deletes anything, so no older snapshot is
# rewritten (ROAD-1). Any failed, unparseable, truncated or partial read stops it with a non-zero
# exit before that create, naming the read. A batch with no node for a task it was seeded with, or
# with other than that node's `degree` of edges at it, is partial (roadmap.jq checks both), and
# there is no fallback to per-task reads.
#
# TEMPER names the temper to run (default: temper on PATH). It needs a login that can read both
# source contexts and write the publish context.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temper="${TEMPER:-temper}"
goals_context="+temper-dev/core"
tasks_context="+temper-dev/core-workflows"
publish_context="+temper-community/dev-roadmap"
# The service refuses a traverse of more than 250 seeds. ROADMAP_TRAVERSE_BATCH lowers it, so the
# test can draw a small fixture across several batches.
batch_size="${ROADMAP_TRAVERSE_BATCH:-250}"

dry_run=false
case "${1:-}" in
    --dry-run) dry_run=true ;;
    "") ;;
    *)
        echo "usage: $0 [--dry-run]" >&2
        exit 2
        ;;
esac

fail() {
    echo "roadmap-snapshot: $*" >&2
    echo "roadmap-snapshot: no snapshot was drawn, and nothing was written" >&2
    exit 1
}

case "$batch_size" in
    '' | *[!0-9]*) fail "ROADMAP_TRAVERSE_BATCH must be a whole number from 1 to 250, not '$batch_size'" ;;
esac
[ "$batch_size" -ge 1 ] && [ "$batch_size" -le 250 ] ||
    fail "ROADMAP_TRAVERSE_BATCH must be a whole number from 1 to 250, not '$batch_size'"
command -v jq > /dev/null 2>&1 || fail "jq is not on PATH"
command -v "$temper" > /dev/null 2>&1 || fail "could not run \`$temper\`: not found"

work="$(mktemp -d "${TMPDIR:-/tmp}/roadmap-snapshot.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# Run one temper read and keep the first JSON value it prints: temper may print more after it.
#   read_json <out> <the read, as an error names it> <temper arguments...>
read_json() {
    local out="$1" what="$2"
    shift 2
    local cmd=("$temper" "$@" --format json)
    if ! "${cmd[@]}" > "$out.raw" 2> "$out.err"; then
        fail "could not run \`temper $what\`: $(tr '\n' ' ' < "$out.err") $(head -c 400 "$out.raw" | tr '\n' ' ')"
    fi
    jq -c -n 'input' < "$out.raw" > "$out" 2> /dev/null ||
        fail "\`temper $what\` answered in a shape this tool does not read: its output is not JSON"
}
list() {
    local out="$1"
    shift
    read_json "$out" "$*" "$@"
}

started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
list "$work/goals.json" resource list --type goal --context "$goals_context" --all --with open-meta
list "$work/tasks.json" resource list --type task --context "$tasks_context" --all --with open-meta

# Every task's edges, seeded with every listed task in batches, so each task's goal link is read
# the same way. roadmap.jq refuses a batch with no node for a seed or short of a seed's degree,
# and batches that do not seed every listed task exactly once.
jq -r '(.rows // [])[] | .id // empty' "$work/tasks.json" > "$work/task-ids"
: > "$work/batches.jsonl"
total="$(wc -l < "$work/task-ids" | tr -d ' ')"
n=0
offset=0
while [ "$offset" -lt "$total" ]; do
    n=$((n + 1))
    seeds=()
    while IFS= read -r id; do
        seeds+=(--from "$id")
    done < <(tail -n "+$((offset + 1))" "$work/task-ids" | head -n "$batch_size")
    read_json "$work/traverse-$n.json" "graph traverse --from <each of $((${#seeds[@]} / 2)) tasks, batch $n> --depth 1" \
        graph traverse "${seeds[@]}" --depth 1
    tail -n "+$((offset + 1))" "$work/task-ids" | head -n "$batch_size" |
        jq -R -s -c --slurpfile reply "$work/traverse-$n.json" \
            '{seeds: (split("\n") | map(select(length > 0))), reply: $reply[0]}' >> "$work/batches.jsonl"
    offset=$((offset + batch_size))
done
finished="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n --slurpfile goals "$work/goals.json" --slurpfile tasks "$work/tasks.json" \
    --slurpfile traverse "$work/batches.jsonl" '{goals: $goals[0], tasks: $tasks[0], traverse: $traverse}' \
    > "$work/records.json"
if ! jq -j -L "$here" --arg started "$started" --arg finished "$finished" \
    --rawfile recipe "$here/recipe.md" -f "$here/snapshot.jq" "$work/records.json" \
    > "$work/body.md" 2> "$work/body.err"; then
    fail "$(sed 's/^jq: error (at [^)]*): //' "$work/body.err" | tr '\n' ' ')"
fi

if [ "$dry_run" = true ]; then
    cat "$work/body.md"
    exit 0
fi

title="Dev roadmap snapshot — ${started%%T*} (read from the records)"
meta="$(jq -c -n --arg date "${started%%T*}" --arg from "$started" --arg to "$finished" \
    '{tags: ["roadmap-snapshot"], date: $date, records_read_from: $from, records_read_to: $to}')"
create=("$temper" resource create --type research --context "$publish_context" --title "$title"
    --open-meta "$meta" --body - --format json)
uncertain() {
    echo "roadmap-snapshot: the create did not report a new resource: $*" >&2
    echo "roadmap-snapshot: it may still have landed, so look for a new roadmap-snapshot in $publish_context before drawing again" >&2
    exit 1
}
if ! "${create[@]}" < "$work/body.md" > "$work/created.raw" 2> "$work/created.err"; then
    uncertain "$(tr '\n' ' ' < "$work/created.err")"
fi
id="$(jq -r -n 'input | .id // empty' < "$work/created.raw" 2> /dev/null || true)"
[ -n "$id" ] || uncertain "its answer names no id"
echo "created $id: $title"
