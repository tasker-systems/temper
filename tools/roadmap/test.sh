#!/usr/bin/env bash
# The roadmap snapshot's witnesses (Temper task 01a1fdd1), over fixed records with no network:
#   1. the fixture draws the expected lanes and counts (fixture/expected.md, checked by hand); an
#      `advances` edge into a task, from a session or a goal, is no task's goal link; and no goal
#      is named but by its public title, no withheld goal at all, and no task ever (ROAD-6);
#   2. two draws over the same records differ only in the stated date and read times, however the
#      rows are ordered and the traverse is batched;
#   3. a failed, unparseable, truncated or partial read exits non-zero and writes nothing, and a
#      traverse batch with no node for a task it was seeded with, or holding other than its node's
#      `degree` of edges at it, is refused, naming the task;
#   4. the only write is one create, every task's edges come from `graph traverse` batches of at
#      most 250 seeds that seed each task once, never a `resource show`, and a dry run writes
#      nothing;
#   5. the areas are internal/agents/conventions.md's vocabulary, in its order.
# snapshot.sh runs against fixture/temper, a fake that answers from the fixture, logs each call,
# refuses more than 250 seeds, and refuses `resource show`.
# CI runs this in .github/workflows/quality-gate.yml#guard-tests.
# shellcheck disable=SC2016 # the jq programs and the backtick pattern are single-quoted on purpose
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
records="$here/fixture/records.json"
expected="$here/fixture/expected.md"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/roadmap-test.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

failures=0
ok() { echo "ok   $1"; }
bad() {
    echo "FAIL $1" >&2
    failures=$((failures + 1))
}

# Draw the body from a records file: draw <started> <finished> <records> > body
draw() {
    jq -j -L "$here" --arg started "$1" --arg finished "$2" --rawfile recipe "$here/recipe.md" \
        -f "$here/snapshot.jq" "$3"
}
# The body without its dated opening line: the only line two draws may differ in.
without_date() {
    grep -v -e '^\*\*As of ' "$1"
}
# The fixture's traverse, as one graph the fake serves.
graph() {
    jq '.traverse[0].reply' "$1"
}
# The fixture's single traverse batch, split into batches of <n> seeds, in reverse order.
rebatch() {
    jq --argjson n "$1" '.traverse[0] as $b
        | .traverse = ([range(0; $b.seeds | length; $n) as $i | {seeds: $b.seeds[$i:$i + $n], reply: $b.reply}
            | .seeds as $s | .reply |= (($s | map({(.): true}) | add) as $seed
                | (.edges | map(select($seed[.source] or $seed[.target]))) as $edges
                | ($seed + ([$edges[] | .source, .target] | map({(.): true}) | add // {})) as $keep
                | {nodes: (.nodes | map(select($keep[.id]))), edges: $edges})] | reverse)' "$2"
}
# A directory the fake temper serves, from the fixture after a jq edit: fake <name> <jq filter>
fake() {
    local dir="$scratch/$1"
    mkdir -p "$dir"
    jq "$2" "$records" > "$dir/records.json"
    jq '.goals' "$dir/records.json" > "$dir/goals.json"
    jq '.tasks' "$dir/records.json" > "$dir/tasks.json"
    graph "$dir/records.json" > "$dir/graph.json"
    echo "$dir"
}
# Run snapshot.sh against a fake: snapshot <dir> [--dry-run]; stdout, stderr and status land in
# <dir>. BATCH, when set, is passed as ROADMAP_TRAVERSE_BATCH.
snapshot() {
    local dir="$1"
    shift
    local status=0
    env TEMPER="$here/fixture/temper" FAKE_TEMPER_DIR="$dir" ${BATCH:+ROADMAP_TRAVERSE_BATCH="$BATCH"} \
        bash "$here/snapshot.sh" "$@" > "$dir/stdout" 2> "$dir/stderr" || status=$?
    echo "$status" > "$dir/status"
}
# How many creates the fake was asked for; 0 when it was never run.
creates() {
    if [ -e "$1/calls.log" ]; then
        grep -c '^resource create' "$1/calls.log" || true
    else
        echo 0
    fi
}
# Why a run's call log is not <n> depth-1 traverses of at most <max> seeds each, seeding every
# task in <records> exactly once, with no per-task read; empty when it is:
# traversed <dir> <n> <max> <records>
traversed() {
    local log="$1/calls.log" want_n="$2" max="$3" traverses want got biggest
    traverses="$(grep -c '^graph traverse ' "$log" || true)"
    want="$(jq -r '.tasks.rows[].id' "$4" | sort)"
    got="$(grep '^graph traverse ' "$log" | grep -o -e '--from [^ ]*' | cut -d ' ' -f 2 | sort)"
    biggest="$(grep '^graph traverse ' "$log" | while IFS= read -r l; do echo "$l" | grep -o -e '--from ' | wc -l; done | sort -n | tail -n 1 | tr -d ' ')"
    if grep -q '^resource show ' "$log"; then
        echo "it made a per-task \`resource show\`: $(grep -c '^resource show ' "$log")"
    elif [ "$traverses" != "$want_n" ]; then
        echo "expected $want_n \`graph traverse\` calls, saw $traverses"
    elif grep '^graph traverse ' "$log" | grep -v -q -e '--depth 1\( \|$\)'; then
        echo "a traverse is not at --depth 1"
    elif [ "${biggest:-0}" -gt "$max" ]; then
        echo "a traverse was seeded with $biggest tasks, more than $max"
    elif [ "$want" != "$got" ]; then
        echo "the traverses did not seed exactly the $(echo "$want" | wc -l | tr -d ' ') tasks once each"
    fi
}

# 1. The fixture draws the expected lanes and counts.
if draw 2026-11-21T03:00:00Z 2026-11-21T03:00:04Z "$records" > "$scratch/body.md" &&
    diff -u "$expected" "$scratch/body.md"; then
    ok "fixed records draw fixture/expected.md"
else
    bad "fixed records draw fixture/expected.md"
fi

# 1b. Only an edge from a task is its goal link. The fixture holds `advances` edges into tasks,
# from a session and from a goal; give every task more of each, and the draw does not move.
into_tasks="$(jq '[.traverse[0].reply as $g | ($g.nodes | map({(.id): .doc_type}) | add) as $type
    | $g.edges[] | select(.label == "advances" and $type[.target] == "task" and $type[.source] != "task")] | length' "$records")"
jq '[.tasks.rows[].id] as $tasks
    | .traverse[0].reply.nodes |= map(if .doc_type == "task" then .degree += 2 else . end)
    | .traverse[0].reply.nodes += [{id: "60000000-0000-7000-8000-000000000099", title: "a busy session", doc_type: "session", degree: ($tasks | length)}]
    | .traverse[0].reply.edges += [$tasks | to_entries[] | {id: "71000000-0000-7000-8000-\(1000000000000 + .key | tostring | .[1:])",
        source: "60000000-0000-7000-8000-000000000099", target: .value, edge_kind: "leads_to", polarity: "forward",
        label: "advances", weight: 1.0}]
    | .traverse[0].reply.edges += [$tasks | to_entries[] | {id: "72000000-0000-7000-8000-\(1000000000000 + .key | tostring | .[1:])",
        source: "10000000-0000-7000-8000-000000000002", target: .value, edge_kind: "leads_to", polarity: "forward",
        label: "advances", weight: 1.0}]' "$records" > "$scratch/into.json"
if [ "$into_tasks" -lt 2 ]; then
    bad "an advances edge into a task is no task's goal link: the fixture holds only $into_tasks"
elif draw 2026-11-21T03:00:00Z 2026-11-21T03:00:04Z "$scratch/into.json" > "$scratch/into.md" &&
    diff -u "$expected" "$scratch/into.md"; then
    ok "an advances edge into a task, from a session or a goal, is no task's goal link"
else
    bad "an advances edge into a task, from a session or a goal, is no task's goal link"
fi

# 1c. ROAD-6: nothing the owner did not write for this page is printed. No goal's record title,
# no task's title, and no withheld goal's id or public words appear; every shown goal's public
# title does.
leaks="$(jq -r '.goals.rows[].title, .tasks.rows[].title' "$records" | while IFS= read -r t; do
    if grep -q -F -- "$t" "$scratch/body.md"; then echo "$t"; fi
done)"
withheld="$(jq -r '.goals.rows[] | select((.managed_meta["temper-status"] == "active" or .managed_meta["temper-status"] == "paused")
    and ((.open_meta.public_title | type) != "string" or (.open_meta.public_outcome | type) != "string"))
    | .id, (.open_meta.public_title // empty)' "$records" | while IFS= read -r t; do
    if grep -q -F -- "$t" "$scratch/body.md"; then echo "$t"; fi
done)"
shown_n="$(jq '[.goals.rows[] | select((.managed_meta["temper-status"] == "active" or .managed_meta["temper-status"] == "paused")
    and (.open_meta.public_title | type) == "string" and (.open_meta.public_outcome | type) == "string")] | length' "$records")"
lines_n="$(grep -c '^- \*\*\(Now\|Next\|Later\)' "$scratch/body.md" || true)"
if [ -n "$leaks" ]; then
    bad "ROAD-6: the snapshot prints a record title: $leaks"
elif [ -n "$withheld" ]; then
    bad "ROAD-6: the snapshot names a withheld goal: $withheld"
elif [ "$shown_n" != "$lines_n" ] || [ "$lines_n" -lt 2 ]; then
    bad "ROAD-6: expected one line per goal with a public title and outcome ($shown_n), saw $lines_n"
elif ! grep -q '2 open goals are withheld' "$scratch/body.md"; then
    bad "ROAD-5: the withheld goals are not counted"
else
    ok "a goal is named only by its public title, a withheld goal and a task never, and the withheld are counted"
fi

# 2. The same records, served in the opposite order, batched differently, and read on another day.
jq '.goals.rows |= reverse | .tasks.rows |= reverse | .traverse[0].reply.nodes |= reverse | .traverse[0].reply.edges |= reverse' \
    "$records" > "$scratch/reversed-rows.json"
rebatch 4 "$scratch/reversed-rows.json" > "$scratch/reversed.json"
if [ "$(jq '.traverse | length' "$scratch/reversed.json")" -lt 3 ]; then
    bad "two draws differ only in the stated date and read times: the rebatched fixture has under 3 batches"
elif draw 2026-12-25T18:30:00Z 2026-12-25T18:32:10Z "$scratch/reversed.json" > "$scratch/other.md" &&
    [ -s "$scratch/body.md" ] && ! cmp -s "$scratch/body.md" "$scratch/other.md" &&
    diff -u <(without_date "$scratch/body.md") <(without_date "$scratch/other.md"); then
    ok "two draws differ only in the stated date and read times, however rows and batches are ordered"
else
    bad "two draws differ only in the stated date and read times, however rows and batches are ordered"
fi

# 3a. The rule refuses a partial or out-of-shape read, and says which.
refuses() {
    local name="$1" edit="$2" names="$3"
    jq "$edit" "$4" > "$scratch/edited.json"
    if draw 2026-11-21T03:00:00Z 2026-11-21T03:00:04Z "$scratch/edited.json" > "$scratch/out" 2> "$scratch/err"; then
        bad "$name: drew a snapshot"
    elif [ -s "$scratch/out" ]; then
        bad "$name: printed a partial snapshot"
    elif ! grep -q -- "$names" "$scratch/err"; then
        bad "$name: the error does not name '$names': $(cat "$scratch/err")"
    else
        ok "$name: refused, naming '$names'"
    fi
}
rebatch 5 "$records" > "$scratch/batched.json"
tasks_n="$(jq '.tasks.rows | length' "$records")"
refuses "a truncated goal listing" '.goals.truncated = true' "goal listing was truncated" "$records"
refuses "a short task listing" ".tasks.rows |= .[1:] | .tasks.returned = $((tasks_n - 1))" \
    "returned $((tasks_n - 1)) of $tasks_n rows" "$records"
refuses "a task listing whose returned disagrees" ".tasks.returned = $((tasks_n + 1))" \
    "says it returned $((tasks_n + 1)) rows but holds $tasks_n" "$records"
refuses "a goal listing with no rows" 'del(.goals.rows)' "has no \`rows\` list" "$records"
refuses "a traverse with no node for a seeded task" \
    '.traverse[0].reply.nodes |= map(select(.id != "20000000-0000-7000-8000-000000000009"))' \
    "no node for task 20000000-0000-7000-8000-000000000009" "$records"
refuses "a traverse missing one edge at a seeded task" \
    '(.traverse[0].reply.edges | map(.source == "20000000-0000-7000-8000-000000000005" or .target == "20000000-0000-7000-8000-000000000005") | index(true)) as $i | del(.traverse[0].reply.edges[$i])' \
    "the traverse holds [0-9]* of the [0-9]* edges its node gives task 20000000-0000-7000-8000-000000000005" "$records"
refuses "a traverse repeating an edge at a seeded task" \
    '.traverse[0].reply.edges += [.traverse[0].reply.edges[] | select(.source == "20000000-0000-7000-8000-000000000003")][:1]' \
    "edges its node gives task 20000000-0000-7000-8000-000000000003" "$records"
refuses "a traverse with no degree for a seeded task" \
    '.traverse[0].reply.nodes |= map(if .id == "20000000-0000-7000-8000-000000000011" then del(.degree) else . end)' \
    "no numeric \`degree\`" "$records"
refuses "a traverse with no edge list" 'del(.traverse[0].reply.edges)' "lacks a \`nodes\` or \`edges\` list" "$records"
refuses "a traverse edge with no source" '.traverse[0].reply.edges[0] |= del(.source)' "no string \`source\`" "$records"
refuses "a traverse that is not a list of batches" '.traverse = .traverse[0].reply' "not a list of batches" "$records"
refuses "a later batch with no node for its seed" \
    '(.traverse[-1].seeds[0]) as $s | .traverse[-1].reply.nodes |= map(select(.id != $s))' \
    "no node for task" "$scratch/batched.json"
refuses "a batch that is missing" 'del(.traverse[1])' "were not seeded with exactly the $tasks_n listed tasks" "$scratch/batched.json"
refuses "a task seeded in two batches" '.traverse[1].seeds += [.traverse[0].seeds[0]]' "more than one traverse batch" "$scratch/batched.json"
refuses "an unknown goal status" '.goals.rows[0].managed_meta["temper-status"] = "dormant"' "none of" "$records"
refuses "an unknown task stage" '.tasks.rows[0].managed_meta["temper-stage"] = "parked"' "none of" "$records"

# 3b. snapshot.sh, against the fake, writes nothing when a read fails.
writes_nothing() {
    local name="$1" dir="$2" names="$3"
    if [ "$(cat "$dir/status")" = 0 ]; then
        bad "$name: exited 0"
    elif [ "$(creates "$dir")" != 0 ] || [ -e "$dir/created.md" ]; then
        bad "$name: wrote a snapshot"
    elif [ -s "$dir/stdout" ]; then
        bad "$name: printed a snapshot"
    elif ! grep -q -- "$names" "$dir/stderr" || ! grep -q "nothing was written" "$dir/stderr"; then
        bad "$name: stderr does not name '$names' and say nothing was written: $(cat "$dir/stderr")"
    else
        ok "$name: exits non-zero and writes nothing"
    fi
}
dir="$(fake truncated '.tasks.truncated = true')"
snapshot "$dir"
writes_nothing "snapshot.sh, a truncated task listing" "$dir" "task listing was truncated"

dir="$(fake failing-traverse '.')"
touch "$dir/traverse.fail"
snapshot "$dir"
writes_nothing "snapshot.sh, a failing traverse" "$dir" "could not run \`temper graph traverse"

dir="$(fake failing-second-batch '.')"
touch "$dir/traverse.fail-2"
BATCH=7 snapshot "$dir"
writes_nothing "snapshot.sh, a traverse whose second batch fails" "$dir" "could not run \`temper graph traverse.*batch 2"

dir="$(fake garbage-traverse '.')"
touch "$dir/traverse.garbage"
snapshot "$dir"
writes_nothing "snapshot.sh, a traverse that is not JSON" "$dir" "graph traverse.*not JSON"

dir="$(fake partial-traverse '.traverse[0].reply.nodes |= map(select(.id != "20000000-0000-7000-8000-000000000007"))')"
snapshot "$dir"
writes_nothing "snapshot.sh, a traverse with no node for a seeded task" "$dir" \
    "no node for task 20000000-0000-7000-8000-000000000007"

dir="$(fake short-traverse '(.traverse[0].reply.edges | map(.source == "20000000-0000-7000-8000-000000000012" or .target == "20000000-0000-7000-8000-000000000012") | index(true)) as $i | del(.traverse[0].reply.edges[$i])')"
BATCH=6 snapshot "$dir"
writes_nothing "snapshot.sh, a batched traverse missing one edge at a seeded task" "$dir" \
    "edges its node gives task 20000000-0000-7000-8000-000000000012"

dir="$(fake garbage '.')"
echo "not json at all" > "$dir/goals.json"
snapshot "$dir"
writes_nothing "snapshot.sh, a listing that is not JSON" "$dir" "not JSON"

dir="$(fake bad-batch '.')"
BATCH=251 snapshot "$dir"
writes_nothing "snapshot.sh, a batch size above the service's 250" "$dir" "ROADMAP_TRAVERSE_BATCH must be"

dir="$scratch/no-temper"
mkdir -p "$dir"
status=0
TEMPER="$dir/temper" bash "$here/snapshot.sh" --dry-run > "$dir/stdout" 2> "$dir/stderr" || status=$?
echo "$status" > "$dir/status"
writes_nothing "snapshot.sh, no temper to run" "$dir" "could not run"

# 4. The only write is one create; a dry run writes nothing.
dir="$(fake write '.')"
BATCH=7 snapshot "$dir"
others="$(grep -v -e '^resource list ' -e '^graph traverse ' -e '^resource create ' "$dir/calls.log" || true)"
batches=$(((tasks_n + 6) / 7))
if [ "$(cat "$dir/status")" != 0 ]; then
    bad "a write: exited $(cat "$dir/status"): $(cat "$dir/stderr")"
elif [ "$(creates "$dir")" != 1 ] || [ -n "$others" ]; then
    bad "a write: expected exactly one create and only reads: $(cat "$dir/calls.log")"
elif ! grep -q '^resource create --type research --context +temper-community/dev-roadmap --title Dev roadmap snapshot — [0-9-]* (read from the records) --open-meta {"tags":\["roadmap-snapshot"\]' "$dir/calls.log"; then
    bad "a write: the create is not a tagged research resource in dev-roadmap: $(grep '^resource create' "$dir/calls.log")"
elif ! grep -q '^created 50000000-0000-7000-8000-000000000001: Dev roadmap snapshot — ' "$dir/stdout"; then
    bad "a write: stdout does not name the created resource: $(cat "$dir/stdout")"
elif [ -n "$(traversed "$dir" "$batches" 7 "$records")" ]; then
    bad "a write: $(traversed "$dir" "$batches" 7 "$records")"
elif ! diff -u <(without_date "$expected") <(without_date "$dir/created.md"); then
    bad "a write, drawn in $batches batches, creates what roadmap.jq draws"
else
    ok "a write makes one create, of a tagged research resource, and otherwise only reads"
    ok "every task's goal link comes from $batches traverse batches of at most 7, each task seeded once, and no resource show"
fi

dry="$(fake dry-run '.')"
snapshot "$dry" --dry-run
if [ "$(cat "$dry/status")" != 0 ] || [ "$(creates "$dry")" != 0 ]; then
    bad "a dry run: exited $(cat "$dry/status") with $(creates "$dry") creates"
elif [ -n "$(traversed "$dry" 1 250 "$records")" ]; then
    bad "a dry run: $(traversed "$dry" 1 250 "$records")"
elif ! diff -u <(without_date "$dry/stdout") <(without_date "$dir/created.md"); then
    bad "a dry run prints what a write creates"
else
    ok "a dry run prints what a write creates, and writes nothing"
fi

# More tasks than one traverse takes: the default batches at 250, which the fake enforces.
many="$(fake many '.tasks.rows += [range(1000; 1300) as $n | {id: "21000000-0000-7000-8000-\(1000000000000 + $n | tostring | .[1:])",
        title: "an extra task", doc_type_name: "task", updated: "2026-11-01T00:00:00Z",
        managed_meta: {"temper-stage": "done"}, open_meta: {}}]
    | .tasks.total = (.tasks.rows | length) | .tasks.returned = (.tasks.rows | length)
    | .traverse[0].reply.nodes += [range(1000; 1300) as $n | {id: "21000000-0000-7000-8000-\(1000000000000 + $n | tostring | .[1:])",
        title: "an extra task", doc_type: "task", degree: 0}]')"
snapshot "$many" --dry-run
if [ "$(cat "$many/status")" != 0 ]; then
    bad "more than 250 tasks: exited $(cat "$many/status"): $(cat "$many/stderr")"
elif [ -n "$(traversed "$many" 2 250 "$many/records.json")" ]; then
    bad "more than 250 tasks: $(traversed "$many" 2 250 "$many/records.json")"
else
    ok "more than 250 tasks are read in traverses of at most 250 seeds"
fi

# No temper command in snapshot.sh writes anything but the create.
writes="$(grep -v '^[[:space:]]*#' "$here/snapshot.sh" |
    grep -n -E 'resource (update|delete|meta|annotate|facet|reblock|move)|edge (assert|fold|retype|reweight|facet)|data-artifact|blob ' || true)"
if [ ! -s "$here/snapshot.sh" ]; then
    bad "snapshot.sh does not exist"
elif [ -z "$writes" ]; then
    ok "snapshot.sh names no temper write but resource create"
else
    bad "snapshot.sh names another temper write: $writes"
fi

# 5. The areas are internal/agents/conventions.md's, in its order.
vocabulary="$(awk '/\*\*The grouping key \(ROAD-3\):\*\*/ { on = 1 } on && /^- \*\*The public line/ { exit } on' "$root/internal/agents/conventions.md" |
    grep -o '`[^`]*`' | tr -d '`' | grep -v '^open_meta\.area$' || true)"
ours="$(jq -n -r -L "$here" 'include "roadmap"; area_keys[]' || true)"
if [ -n "$vocabulary" ] && [ "$vocabulary" = "$ours" ]; then
    ok "the areas are internal/agents/conventions.md's vocabulary, in its order"
else
    bad "the areas differ from internal/agents/conventions.md's: '$vocabulary' vs '$ours'"
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures roadmap snapshot witness(es) failed" >&2
    exit 1
fi
echo "every roadmap snapshot witness passed"
