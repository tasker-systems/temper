#!/usr/bin/env bash
# audit-unattributed-decisions.sh — stop a decision nobody took from entering the tree as prose.
#
# WHY THIS EXISTS
# ---------------
# `[2026-08-12]` Three planning sessions on the `/api/query` arc were spent undoing "firm decisions"
# that no human ever made. The anatomy is always the same, and it is not lying — it is genre:
#
#   1. An agent writes a design paragraph in the first person, in a commit message or a doc comment.
#   2. Nothing marks it as unratified, because our prose voice for a *decision* and our prose voice
#      for a *proposal* are identical.
#   3. It hardens — most damagingly into a TEST NAME, which every later agent greps and reads as a
#      settled invariant.
#   4. Later plans are built on top of it. By the time anyone asks who decided it, the cost of
#      finding out exceeds the cost of just believing it.
#
# The specimen this guard was built from: `Composition.intention` sat on the envelope rather than on
# each stage, which meant a composition could only ever ask ONE question — every find stage in a DAG
# interrogating the same string, and "find A, find B, intersect them" inexpressible. That placement
# entered in commit 3d73a70b (2026-08-03) as a first-person paragraph, hardened into the test name
# `the_intention_is_a_composition_level_field_not_a_per_stage_one`, and steered planning until Pete
# read it and said "I have no idea where that came from, who made that ruling." Nobody had. It was
# then ruled for the first time, the other way, as spec ⟨7⟩.
#
# The tell was already in the file and nothing was reading it. EVERY other decision in
# `composition.rs` carries an attribution — `[decided — 2026-08-08, Pete]` on `Intention`'s
# absent-embedding rule and on the `on_stage_refusal` tombstone, `ADJ-4 [2026-08-10, Pete]` on the
# `meta_detail` and `bounds` removals. The intention's placement carried none. **Attribution is the
# discriminator we already had; its absence just never cost anything.** This guard makes it cost.
#
# WHAT IT ASSERTS
#   (a) The scan finds something. An empty scan set fails: with a scope that matches no files, every
#       count assertion below passes vacuously and the guard reports green while checking nothing.
#   (b) For each file in SCOPE, the number of comment lines that speak in the decision voice while
#       carrying NO attribution within ±8 lines equals the recorded baseline. Growth is the failure.
#
# WHAT IT DOES NOT ASSERT — and these are real limits, named rather than papered over.
#   * **It does not read commit messages.** Step 1 of the anatomy above is invisible to any tree
#     scan. The counterpart control for that half is the plan's `## Decisions this PR takes` block,
#     which the closing summary must reproduce verbatim — process, not a gate. This guard covers the
#     hardening (steps 2–4), not the origin.
#   * **It cannot tell a decision from a description.** `deliberately` in "the field is deliberately
#     private" is a decision; in "this deliberately mirrors the sibling" it is a description. The
#     guard flags both and lets a human sort them. A classifier that tried to tell them apart would
#     be wrong quietly, which is the failure mode already under repair.
#   * **The baseline is RECORDED, NOT ENDORSED.** The 35 incumbents below were counted, not
#     adjudicated. They are a named backlog, not a clean bill. Do not cite a file's presence here as
#     evidence its prose was reviewed.
#   * **Scope is one module.** See SCOPE below for why, and for what it costs.
#
# HOW TO RESPOND WHEN IT GOES RED
#   You added decision-voiced prose with no attribution. Three legitimate answers, in order of
#   preference:
#     1. It IS a decision someone took — attribute it: `[decided — YYYY-MM-DD, <who>]`. Free, and it
#        is what every neighbouring decision in these files already does.
#     2. It is NOT a decision — it is a description or a proposal. Reword so it does not speak in the
#        decision voice, or mark it `[proposed — …]` / `[PROVISIONAL — …]`.
#     3. It is a decision and nobody has taken it yet. **Then stop and ask.** That is the whole point
#        of this file. Do not raise the baseline to get to green.
#   Raising a baseline number is legitimate ONLY alongside answer 1 or 2 above having been considered
#   and rejected for a stated reason. It is a deliberate act; that is why it is a diff and not a
#   threshold.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Overridable so the test harness can point the scan at a fixture tree.
SCOPE="${SCOPE:-crates/temper-core/src/types/query}"

# How far from a decision-voiced line an attribution may sit and still count as covering it. A doc
# block routinely carries its `[decided — …]` several lines above the sentence it governs, so a
# same-line rule would flag most of the correctly-attributed corpus and the guard would be rebaselined
# into meaninglessness within a week. Eight lines is the observed span of the longest attributed block
# in `registry.rs`; widen it only with a measurement, never to clear a red.
WINDOW="${WINDOW:-8}"

# The decision voice: the vocabulary this codebase uses when it means "this was chosen, and the
# alternative was considered." Kept SMALL on purpose. `rather than` is the repo's single most common
# decision idiom and is deliberately NOT here — it appears in ordinary descriptive prose constantly,
# and including it would drown the signal in a baseline nobody could review. A guard whose output is
# too large to read is a guard that gets rebaselined reflexively.
DECISION_VOICE='deliberately|on purpose|by decision|[^a-zA-Z]Rejected[^a-zA-Z]|is a ruling|was a ruling'

# What counts as attribution: a named decider, or an adjudication number that resolves to one.
# `[found — …]`, `[measured — …]`, `[observed — …]` and `[verified — …]` are deliberately NOT
# attribution — they mark EVIDENCE, and evidence is not a decision. Conflating them would let an
# agent clear this guard by citing its own measurement as though a person had ruled on it.
#
# The literal `[` is written `[[]`, NOT `\[`, and that is load-bearing rather than stylistic. These
# patterns reach awk through `-v`, which processes escape sequences in the VALUE before the string is
# ever compiled as a regex — so `\[` arrives as a bare `[` and turns `\[20[0-9]...` into the bracket
# expression `[20[0-9]`, which matches a completely different set. The guard's first run caught this
# in itself: `registry.rs` scanned 6 where the same pattern written as an awk literal scanned 7,
# because three lines were being counted as attributed that are not. `[[]` survives `-v` intact.
ATTRIBUTION='decided —|ADJ-[0-9]|[[]20[0-9][0-9]-[0-9][0-9]-[0-9][0-9], [A-Z]'

# The incumbent set as of 2026-08-12, recorded so growth is visible. RECORDED, NOT ENDORSED — see
# WHAT IT DOES NOT ASSERT. Sorted by path; `<count> <path>`.
#
# SCOPE is `crates/temper-core/src/types/query` alone, and that is a deliberate first cut rather than
# the finished shape. This module is where the failure was observed, where the contract under active
# design lives, and where the corpus is small enough that 35 lines is a backlog a person can actually
# work through. A whole-repo baseline would run to hundreds and would be rebaselined without reading,
# which is the outcome this guard exists to prevent. **Widening the scope is the intended direction of
# travel** — do it one module at a time, with the new incumbents read rather than merely counted.
# `[lowered — 2026-08-15]` filter.rs 3 -> 1. Two unattributed decision-voiced comments went with the
# TYPE that carried them: `PropertySubject`'s *"OPEN, deliberately — kb_properties.owner_table is a
# varchar mirroring no DDL enum"* and its `Other` arm's *"addressable but deliberately not a queryable
# subject"*. Both halves of the predicate-container split now have containers, so the subject tag was
# deleted (PR #682) and its doc with it. A removal, not a rebaseline-to-green: the two comments no
# longer exist rather than no longer being flagged.
read -r -d '' BASELINE <<'EOF' || true
1 crates/temper-core/src/types/query/act.rs
5 crates/temper-core/src/types/query/composition.rs
6 crates/temper-core/src/types/query/disposition.rs
1 crates/temper-core/src/types/query/envelope.rs
1 crates/temper-core/src/types/query/filter.rs
2 crates/temper-core/src/types/query/id_set.rs
1 crates/temper-core/src/types/query/mod.rs
7 crates/temper-core/src/types/query/registry.rs
1 crates/temper-core/src/types/query/scalars.rs
1 crates/temper-core/src/types/query/stage.rs
1 crates/temper-core/src/types/query/validate/capability.rs
3 crates/temper-core/src/types/query/validate/mod.rs
1 crates/temper-core/src/types/query/validate/shape.rs
EOF

# ── Two seams the test harness needs, and neither is reachable in production ──
#
# `BASELINE_FILE` replaces the recorded set above, so a fixture tree can be checked against fixture
# paths. `SCAN_UNTRACKED` swaps `git ls-files` for `find`, because a fixture lives in a mktemp dir
# that git has never heard of. Production sets neither: CI runs the script bare, so the baseline is
# the one in this file and the file list is the tracked one.
#
# The `git ls-files` default is not incidental. An untracked scratch file must not be able to turn
# this red (that would train people to ignore it), and a file deleted but not yet committed must not
# keep it green. Both properties are lost under `SCAN_UNTRACKED`, which is why it is opt-in.
if [ -n "${BASELINE_FILE:-}" ]; then
  BASELINE="$(cat "$BASELINE_FILE")"
fi

scan_one() {
  awk -v window="$WINDOW" -v voice="$DECISION_VOICE" -v attribution="$ATTRIBUTION" '
    { line[NR] = $0
      if ($0 ~ attribution) attr[NR] = 1 }
    END {
      n = 0
      for (i = 1; i <= NR; i++) {
        # Comments only. A decision voiced in CODE is not prose masquerading as a ruling.
        if (line[i] !~ /^[[:space:]]*(\/\/|#)/) continue
        if (line[i] !~ voice) continue
        covered = 0
        for (j = i - window; j <= i + window; j++) if (j in attr) { covered = 1; break }
        if (!covered) n++
      }
      if (n > 0) printf "%d %s\n", n, FILENAME
    }' "$1"
}

list_files() {
  if [ -n "${SCAN_UNTRACKED:-}" ]; then
    find "$SCOPE" -type f -name '*.rs' 2>/dev/null | sort || true
  else
    git ls-files "$SCOPE" | grep -E '\.rs$' || true
  fi
}

current() {
  local f
  while IFS= read -r f; do
    [ -f "$f" ] || continue
    scan_one "$f"
  done < <(list_files)
}

CURRENT="$(current | sort -k2)"
# `sed` rather than `grep -v` to drop blank lines: under `set -o pipefail` a `grep` that matches
# nothing exits 1 and kills the script before any comparison runs, with no output. That is not
# theoretical — the bite probe for "an attributed line stays green" hit it on its first run, and the
# probe for "an unattributed line goes red" was PASSING FOR THAT REASON rather than for the
# property it names. `sed` has no such exit-status opinion.
EXPECTED="$(printf '%s\n' "$BASELINE" | sed '/^[[:space:]]*$/d' | sort -k2)"

# (a) Fail closed. A scope that matches nothing satisfies every count assertion vacuously.
if [ -z "$CURRENT" ] && [ -n "$EXPECTED" ]; then
  echo "FATAL: scanned '$SCOPE' and found no files with decision-voiced prose, but the baseline" >&2
  echo "       expects several. Either SCOPE is wrong or the scan is broken — this guard must not" >&2
  echo "       report green by checking nothing." >&2
  exit 1
fi

# (b) The counts.
if [ "$CURRENT" = "$EXPECTED" ]; then
  echo "OK: unattributed decision-voiced prose in '$SCOPE' matches the recorded baseline."
  echo "    (Recorded, not endorsed — $(printf '%s\n' "$EXPECTED" | awk '{s+=$1} END{print s+0}') incumbents remain a named backlog.)"
  exit 0
fi

echo "audit-unattributed-decisions: the set of unattributed decision-voiced comments has MOVED." >&2
echo >&2
diff <(printf '%s\n' "$EXPECTED") <(printf '%s\n' "$CURRENT") >&2 || true
echo >&2
echo "A line is flagged when a comment speaks in the decision voice" >&2
echo "  ($DECISION_VOICE)" >&2
echo "with no attribution ($ATTRIBUTION) within $WINDOW lines." >&2
echo >&2
echo "If you ADDED one: attribute it '[decided — YYYY-MM-DD, <who>]', or reword it so it does not" >&2
echo "claim a ruling, or — if it IS a ruling nobody has taken — STOP AND ASK. Do not raise the" >&2
echo "baseline to reach green; that is the exact move this guard exists to make expensive." >&2
echo >&2
echo "If you REMOVED one (a count went down): lower the baseline in this file, in the same commit." >&2
exit 1
