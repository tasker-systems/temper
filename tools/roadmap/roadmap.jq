# temper's dev-roadmap snapshot, drawn from the Temper records: the whole rule, as jq definitions.
# recipe.md says the same in words, and ends every snapshot. snapshot.jq calls `snapshot`;
# test.sh calls the parts. The rules are ROAD-1 to ROAD-6 of the tasker-systems conventions
# baseline (doctrine/roadmap.md).
#
# `snapshot($started; $finished; $recipe)` takes one object, as snapshot.sh assembles it from
# `temper` (and as fixture/records.json holds):
#   {"goals": <resource list page>, "tasks": <resource list page>,
#    "traverse": [{"seeds": [<task id>...], "reply": <graph traverse --depth 1 from those seeds>}...]}
# The traverse is read in batches because the service takes at most 250 seeds a call; every
# listed task is the seed of exactly one batch. $started and $finished are YYYY-MM-DDTHH:MM:SSZ,
# when the reads began and ended; $recipe is recipe.md. It answers the snapshot's markdown body.
# Nothing in it depends on the order of the input rows or of the batches, and only the stated
# date and read times depend on the arguments.
#
# The snapshot is published to +temper-community, whose readers are wider than the goal records'
# (ROAD-6). So a goal is named only by the public title its owner wrote on it, with the owner's
# outcome line under it; a goal without both is withheld and only counted (ROAD-5). No task is
# ever named: a task title is not written for that audience either.
#
# Any read that is partial or out of shape is an error: jq exits non-zero, prints nothing on
# stdout, and snapshot.sh writes nothing.

# The values of open_meta.area, in the order internal/agents/conventions.md writes them (ROAD-3),
# with the heading each is drawn under. test.sh checks this list against that file.
def areas:
  [["identity-and-access", "Identity and access"], ["surfaces", "Surfaces"],
   ["reliability", "Reliability"], ["release-and-deploy", "Release and deploy"],
   ["reads-and-search", "Reads and search"], ["interop", "Interop"],
   ["knowledge-model", "Knowledge model"], ["records-and-ledger", "Records and ledger"],
   ["privacy", "Privacy"], ["ingest-and-writes", "Ingest and writes"], ["process", "Process"],
   ["ui", "UI"]];
def area_keys: areas | map(.[0]);

# The goal keys the owner writes for the wider audience (ROAD-6).
def public_title_key: "public_title";
def public_outcome_key: "public_outcome";

def goal_statuses: ["active", "paused", "completed", "cancelled"];
def task_stages: ["backlog", "in-progress", "done", "cancelled"];

# A listing's rows, only if it is whole: not marked truncated, and holding as many rows as it says
# it returned and as its total.
def whole($what):
  if (.rows | type) != "array" then error("the \($what) listing answered in a shape this tool does not read: it has no `rows` list")
  elif (.total | type) != "number" or (.returned | type) != "number" or (.truncated | type) != "boolean"
    then error("the \($what) listing answered in a shape this tool does not read: it lacks `total`, `returned` or `truncated`")
  elif .truncated then error("the \($what) listing was truncated: it says so (`truncated: true`), holding \(.rows | length) of \(.total) rows; refusing to draw from a partial page")
  elif .returned != (.rows | length) then error("the \($what) listing was truncated: it says it returned \(.returned) rows but holds \(.rows | length); refusing to draw from a partial page")
  elif (.rows | length) != .total then error("the \($what) listing was truncated: it returned \(.rows | length) of \(.total) rows; refusing to draw from a partial page")
  else .rows end;

def need($row; $key; $what):
  if ($row[$key] | type) == "string" then $row[$key]
  else error("a \($what) row has no `\($key)`") end;

# A string value of an open_meta key, trimmed; null when absent, not a string, or blank.
def text_of($meta; $key):
  if ($meta | type) == "object" and ($meta[$key] | type) == "string"
  then ($meta[$key] | gsub("^\\s+|\\s+$"; "")) | (if length > 0 then . else null end)
  else null end;

def goal:
  . as $r
  | need($r; "id"; "goal") as $id | need($r; "title"; "goal") as $title
  | ($r.managed_meta["temper-status"] // "") as $status
  | if (goal_statuses | any(. == $status) | not) then
      error("goal \($id) has status \($status | tojson), which is none of \(goal_statuses | tojson); the lanes cannot tell whether it is open")
    else
      {id: $id, status: $status, updated: need($r; "updated"; "goal"),
       area: (if ($r.open_meta.area | type) == "string" then $r.open_meta.area else null end),
       public_title: text_of($r.open_meta; public_title_key),
       public_outcome: text_of($r.open_meta; public_outcome_key),
       open: ($status == "active" or $status == "paused")}
      | . + {public: (.public_title != null and .public_outcome != null)}
    end;

# One traverse batch, checked before it is trusted. Every seed must have a node in the reply, and
# the reply must hold exactly as many edges at it as that node's `degree` says it has: a task the
# traverse left out, or whose edges it did not all return, is a partial read, never a task with
# no goal. It answers the batch's `advances` edges whose source is one of its seeds; an edge into
# a task, from a session or anything else, is no task's link.
def batch:
  if (.seeds | type) != "array" or (.reply | type) != "object" then
    error("a traverse batch answered in a shape this tool does not read: it lacks `seeds` or `reply`")
  else . end
  | .seeds as $seeds | .reply
  | if (.nodes | type) != "array" or (.edges | type) != "array" then
      error("the traverse answered in a shape this tool does not read: it lacks a `nodes` or `edges` list")
    elif any(.nodes[]; (.id | type) != "string") then
      error("the traverse answered in a shape this tool does not read: a node has no string `id`")
    elif any(.edges[]; (.source | type) != "string" or (.target | type) != "string") then
      error("the traverse answered in a shape this tool does not read: an edge has no string `source` or `target`")
    else
      (.nodes | map({(.id): .degree}) | add // {}) as $degree
      | ([.edges[] | if .source == .target then .source else .source, .target end]
         | group_by(.) | map({(.[0]): length}) | add // {}) as $at
      | ($seeds | map({(.): true}) | add // {}) as $seed
      | (first($seeds[] as $s | select($degree | has($s) | not) | $s) // null) as $missing
      | (first($seeds[] as $s | select(($degree[$s] | type) != "number") | $s) // null) as $nodegree
      | (first($seeds[] as $s | select($degree[$s] != ($at[$s] // 0)) | $s) // null) as $short
      | if $missing != null then
          error("the traverse has no node for task \($missing), which it was seeded with: its edges were not read, so it cannot be counted as having no goal")
        elif $nodegree != null then
          error("the traverse gives task \($nodegree) no numeric `degree`, so it cannot show that every one of its edges was read")
        elif $short != null then
          error("the traverse holds \($at[$short] // 0) of the \($degree[$short]) edges its node gives task \($short), which it was seeded with: a partial read, so its goal links cannot be trusted")
        else
          [.edges[] | select(.label == "advances" and $seed[.source]) | {source, target}]
        end
    end;

# Every batch's links, after checking that the batches seeded every listed task exactly once.
def links_of($task_ids):
  if type != "array" then error("the traverse answered in a shape this tool does not read: it is not a list of batches")
  else . end
  | ([.[].seeds[]?] | sort) as $seeded
  | ($task_ids | sort) as $want
  | if $seeded != ($seeded | unique) then
      error("a task was seeded in more than one traverse batch")
    elif $seeded != $want then
      error("the traverse batches were not seeded with exactly the \($want | length) listed tasks (they hold \($seeded | length) seeds); its goal links cannot be trusted")
    else
      [.[] | batch[]] | group_by(.source) | map({(.[0].source): (map(.target) | unique)}) | add // {}
    end;

def task($links):
  . as $r
  | need($r; "id"; "task") as $id
  | ($r.managed_meta["temper-stage"] // "") as $stage
  | if (task_stages | any(. == $stage) | not) then
      error("task \($id) has stage \($stage | tojson), which is none of \(task_stages | tojson); the counts cannot tell whether it is filed")
    elif ($r.open_meta | type) != "object" then
      error("task \($id) has no `open_meta`, so its claim and ready markers cannot be read")
    else
      {id: $id, stage: $stage, updated: need($r; "updated"; "task"),
       filed: ($stage == "backlog" or $stage == "in-progress"),
       claimed: ($r.open_meta | has("claim_id") and .claim_id != null),
       ready: ($r.open_meta.state == "ready"),
       links: ($links[$id] // [])}
    end;

def lane_of: if .claimed + .ready_unclaimed > 0 then "Now" elif .filed > 0 then "Next" else "Later" end;
def lane_rank: {"Now": 0, "Next": 1, "Later": 2}[.];
def order: sort_by([(.lane | lane_rank), (.status != "active"), .public_title, .id]);

# Text on one line, for a link's text or a line under it: no `[`, `]` or `\` to break the link.
def oneline: [splits("\\s+") | select(length > 0)] | join(" ");
def linktext: oneline | gsub("\\\\"; "\\\\") | gsub("\\["; "\\[") | gsub("\\]"; "\\]");

def plural($n; $one; $many): "\($n) \(if $n == 1 then $one else $many end)";
def blank_zero: if . == 0 then "" else tostring end;

def line:
  "- **\(.lane)\(if .status == "paused" then " · paused" else "" end)** · "
  + "[\(.public_title | linktext)](./\(.id)) · \(.claimed) claimed, \(.ready_unclaimed) ready, \(.filed) filed  \n"
  + "  \(.public_outcome | oneline)\n";

def snapshot($started; $finished; $recipe):
  . as $in
  | ($in.goals | whole("goal") | map(goal) | sort_by(.id)) as $goals
  | ($in.tasks | whole("task")) as $task_rows
  | ($in.traverse | links_of([$task_rows[] | need(.; "id"; "task")])) as $links
  | ($task_rows | map(task($links)) | sort_by(.id)) as $tasks
  | ($goals | map({key: .id, value: .}) | from_entries) as $by_id
  # Each task counts toward the one open goal it advances, or toward nothing.
  | ($tasks | map(. + {goal: (if (.links | length) == 1 and ($by_id[.links[0]].open // false) then .links[0] else null end)})) as $placed
  | ($goals | map(select(.open)) | map(. as $g
      | ($placed | map(select(.goal == $g.id))) as $work
      | $g + {claimed: ($work | map(select(.filed and .claimed)) | length),
              ready_unclaimed: ($work | map(select(.filed and (.claimed | not) and .ready)) | length),
              filed: ($work | map(select(.filed)) | length)}
      | . + {lane: lane_of})) as $open
  | ($open | map(select(.public))) as $shown
  | ($open | map(select(.public | not))) as $withheld
  | ($shown | map(select(.area as $a | area_keys | any(. == $a) | not)) | order) as $ungrouped
  | ($placed | map(select(.filed))) as $filed
  | ($shown | map({(.id): true}) | add // {}) as $is_shown
  | ($filed | map(select(.goal != null and $is_shown[.goal]))) as $on_map
  | ($filed | map(select(.goal != null and ($is_shown[.goal] | not)))) as $behind_withheld
  | ($filed | map(select(.goal == null))) as $off_map
  | ($off_map | map(select((.links | length) > 1))) as $off_multiple
  | ($off_map | map(select((.links | length) == 1 and ($by_id[.links[0]] != null)))) as $off_closed
  | ($off_map | map(select((.links | length) == 0 or ((.links | length) == 1 and $by_id[.links[0]] == null)))) as $off_none
  | ($started | split("T")[0]) as $date
  | ([$goals[].updated, $tasks[].updated] | max) as $newest
  | ($tasks | map(select(.stage == "in-progress" and (.claimed | not))) | length) as $unclaimed_in_progress
  | "**As of \($date). Every lane is read from the records.** This is a snapshot of direction, not a promise. "
    + "A newer snapshot supersedes it, and this one stays as it was. "
    + "It is read from the goal records in `+temper-dev/core` and the work items in `+temper-dev/core-workflows`, "
    + "from \($started) to \($finished), by `tools/roadmap/snapshot.sh` in the temper repository"
    + (if $newest == null then "; no records were listed.\n\n"
       else "; the newest record it read last changed at \($newest).\n\n" end)
    + "## How to read it\n\n"
    + "- **Grouped by product area.** Each goal is grouped under its one area, from a closed vocabulary of \(areas | length). "
    + "Under each area are its goals. The line under each goal says what it brings or closes, in words its owner wrote for this page.\n"
    + "- **Each goal's counts** are its work items: how many are **claimed**, how many are **ready** and unclaimed, and how many are **filed** in all. "
    + "A work item is filed while it is open (in backlog or in progress), and it counts toward the one goal it advances. "
    + "A small claimed item and a large body of filed work therefore read differently.\n"
    + "- **The lanes, all read from the records:**\n"
    + "  - **Now:** some of the goal's work is claimed, or ready and unclaimed.\n"
    + "  - **Next:** the goal has filed work, none of it claimed or ready.\n"
    + "  - **Later:** the goal has no filed work.\n"
    + "- **Paused goals follow the same rule.** A paused goal is still open, so it sits in the lane its work puts it in, and its line is marked *paused*.\n"
    + "- **A work item's stage does not set its lane.** \"Ready\" and \"claimed\" are marks on the work item itself, and \"in progress\" is neither. "
    + (if $unclaimed_in_progress == 0 then "No work item is in progress without a claim.\n"
       elif $unclaimed_in_progress == 1 then "1 work item is in progress without a claim; it counts as filed, and as ready if it is marked ready.\n"
       else "\($unclaimed_in_progress) work items are in progress without a claim; each counts as filed, and as ready where it is marked ready.\n" end)
    + "- **No scores.** Value, risk and size are not scored per item, and goals are ordered by area and lane, not by any score. Each goal states what it protects; read that instead.\n\n"
    + "## At a glance\n\n"
    + "| Area | Now | Next | Later |\n|---|---|---|---|\n"
    + (areas | map(.[0] as $a | .[1] as $name | ($shown | map(select(.area == $a))) as $g
        | "| \($name) | \($g | map(select(.lane == "Now")) | length | blank_zero) | \($g | map(select(.lane == "Next")) | length | blank_zero) | \($g | map(select(.lane == "Later")) | length | blank_zero) |\n")
       | join(""))
    + (if ($ungrouped | length) > 0 then
         "| No area | \($ungrouped | map(select(.lane == "Now")) | length | blank_zero) | \($ungrouped | map(select(.lane == "Next")) | length | blank_zero) | \($ungrouped | map(select(.lane == "Later")) | length | blank_zero) |\n"
       else "" end)
    + "| **All areas** | **\($shown | map(select(.lane == "Now")) | length)** | **\($shown | map(select(.lane == "Next")) | length)** | **\($shown | map(select(.lane == "Later")) | length)** |\n\n"
    + "\(plural($open | length; "open goal"; "open goals")): \($open | map(select(.status == "active")) | length) active, \($open | map(select(.status == "paused")) | length) paused"
    + (if ($withheld | length) == 0 then "" else "; \($shown | length) are shown and \($withheld | length) withheld (see the end of this page)" end)
    + ". Of the \(plural($filed | length; "filed work item"; "filed work items")), \($on_map | length) advance an open goal shown here: "
    + "\($on_map | map(select(.claimed)) | length) claimed, \($on_map | map(select(.ready and (.claimed | not))) | length) ready and unclaimed, "
    + "and \($on_map | map(select((.claimed | not) and (.ready | not))) | length) filed and waiting.\n\n"
    + (areas | map(.[0] as $a | .[1] as $name | ($shown | map(select(.area == $a)) | order) as $g
        | "## \($name)\n\n" + (if ($g | length) == 0 then "No open goal is shown here.\n\n" else ($g | map(line) | join("")) + "\n" end))
       | join(""))
    + (if ($ungrouped | length) == 0 then ""
       else "## No area\n\nEach of these carries no `open_meta.area`, or one outside the vocabulary, so it sits in no area above.\n\n"
         + ($ungrouped | map(line) | join("")) + "\n" end)
    + "## Not on this map\n\n"
    + "- **\($off_map | length) of the \(plural($filed | length; "filed work item"; "filed work items")) \(if ($off_map | length) == 1 then "advances" else "advance" end) no open goal.** Of them, "
    + "\(plural($off_closed | length; "advances"; "advance")) a goal that is completed or cancelled, \(plural($off_none | length; "advances"; "advance")) no goal, "
    + "and \(plural($off_multiple | length; "advances"; "advance")) more than one goal and \(if ($off_multiple | length) == 1 then "counts" else "count" end) for none. They are not counted above. "
    + (($off_map | map(select(.claimed or .stage == "in-progress")) | length) as $busy
       | if $busy == 0 then "None of them is claimed or in progress.\n"
         else "\($busy) of them \(if $busy == 1 then "is" else "are" end) claimed or in progress.\n" end)
    + "- **Completed and cancelled goals are not listed** (\($goals | map(select(.status == "completed")) | length) completed, "
    + "\($goals | map(select(.status == "cancelled")) | length) cancelled). A snapshot shows what is outstanding.\n"
    + (if ($ungrouped | length) == 0 then "- **Every goal shown carries an area,** so no goal is listed without one.\n"
       else "- **\(plural($ungrouped | length; "goal shown carries"; "goals shown carry")) no area from the vocabulary,** and \(if ($ungrouped | length) == 1 then "is" else "are" end) listed by name under *No area*.\n" end)
    + (if ($withheld | length) == 0 then "- **No goal is withheld.** Every open goal carries a public title and outcome line its owner wrote for this page.\n"
       else "- **\(plural($withheld | length; "open goal is"; "open goals are")) withheld.** "
         + "A goal is shown only under a public title and outcome line its owner wrote for this page, so that no line describes a weakness that is not yet fixed. "
         + "\(if ($withheld | length) == 1 then "This one carries" else "These carry" end) none yet, so \(if ($withheld | length) == 1 then "it is" else "they are" end) counted here and not named, "
         + "and the \(plural($behind_withheld | length; "filed work item"; "filed work items")) that advance \(if ($withheld | length) == 1 then "it" else "them" end) are not counted above.\n" end)
    + "\n" + $recipe;
