#!/usr/bin/env bash
# audit-ungated-fragments.sh — enumerate every `__temper_ungated_` fragment and every production
# site that names one, and fail if either set has grown without review.
#
# WHY THIS EXISTS
# ---------------
# An ungated fragment applies NO visibility gate. It is handed the RBAC verdict as
# `p_visible_ids uuid[]` and trusts its caller absolutely. That exists because a CTE cannot be
# passed to a function, and hoisting the gate is the only way a composition computes
# `resources_visible_to` once rather than once per stage (migrations/20260808000030, spec §5).
#
# The invariant "every mechanic acts only on resources visible to the principal" is therefore no
# longer a property of these bodies. It is held by structure and by this script — and by nothing
# else. So a new call site, or a new ungated function, must be seen by a human.
#
# WHAT THIS CANNOT DO, AND IT IS THE LIKELIER FAILURE
# ---------------------------------------------------
# This pins *where* a core is called. It can never pin *what is passed*. The realistic bug is not a
# rogue call site — it is an APPROVED site handing over an upstream stage's ids where the visible
# set belongs: CI green, RBAC bypassed, every returned row still plausible.
#
# That failure is closed somewhere else, structurally: `query_plan.rs::emit_ungated_core_call` is
# the only emitter of these calls, and the id source is NOT a parameter of it. There is no wrong set
# to pass because there is no argument for it. See
# `every_ungated_core_call_takes_its_ids_from_the_hoisted_relation_and_nothing_else`.
#
# WHAT THIS IS NOT: A DATABASE PERMISSION
# ---------------------------------------
# The application connects as the owning role, so anyone holding a psql connection, the Neon
# console, or the app credentials can call `__temper_ungated_find_exact` with an arbitrary `uuid[]`
# and receive ungated rows. `REVOKE` buys nothing. This is source discipline. It is a real, accepted
# residue of the split (spec §6), stated here so a reader does not mistake a green tick for a
# capability boundary.
#
# DERIVED, NOT PINNED
# -------------------
# Both halves are DERIVED from the `__temper_ungated_` prefix rather than listed. The repo's own
# lesson from `assert_every_compiled_in_doc_is_vetoed` is that a hand-maintained enumeration rots
# while a derived set does not — that test greps for `include_str!` rather than trusting a list. The
# baselines below are what the derivation currently yields, not the definition of what to look for.
#
# DECLARED SCOPE, so a narrowing is not mistaken for coverage
# ----------------------------------------------------------
# The Rust half scans `crates/**/src/**.rs` only. Test trees are DELIBERATELY out of scope: a test
# calling a core directly is expected (that is how the cores' own witnesses prove they are ungated),
# it ships in no binary, and it runs against an ephemeral database. Including them would make the
# baseline churn on every new witness until nobody read it. Named as an exclusion rather than left
# to be inferred — `audit-grant-sinks.sh`'s header records what happens when a guard's field of view
# narrows and the number moves the reassuring way.
#
# USAGE
#   .github/scripts/audit-ungated-fragments.sh            # verify against the baseline (CI mode)
#   .github/scripts/audit-ungated-fragments.sh --list     # just print the current sets
#   UPDATE_BASELINE=1 .github/scripts/audit-ungated-fragments.sh   # print blocks to paste, after review
#
# Exit 0 = both sets unchanged. Exit 1 = something was added/removed/moved — review required.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Overridable so the test harness can point either scan at a fixture directory.
MIGRATIONS_DIR="${MIGRATIONS_DIR:-migrations}"
CRATES_DIR="${CRATES_DIR:-crates}"

PREFIX='__temper_ungated_'

# The reviewed SQL baseline: the NAMES of ungated functions defined in migrations/.
#
# Keyed on function name, never on file or line — migrations are append-only and immutable, so a
# shipped function is changed by a NEW migration redefining it. A per-file baseline would churn on
# every routine redefinition and get rebaselined reflexively until it meant nothing. The set of
# names changes only when a genuinely new ungated body appears, which is the event worth attention.
#
# REVIEWED 2026-08-08 (composable find fragments, plan Task 9) — the two founding members.
#   __temper_ungated_find_exact / __temper_ungated_find_wide (migrations/20260808000030)
#   Each is the deployed arm's body with its `resources_visible_to(p_principal)` join replaced by
#   `JOIN unnest(p_visible_ids)`. They keep the anchor readability check for BOTH anchor kinds —
#   cogmap and context, via `anchor_readable_by_profile` since migration 20260810000010 — which the
#   id set cannot express: "may this principal use this map or context as a scope" is one boolean per
#   call and a property of no row. They take `p_anchor_reader` for it rather than a caller-asserted
#   boolean, so the core cannot be lied to about that authorization.
#
# REVIEWED 2026-08-14 (selection becomes an act, task 01a0003c beat 2) — the third member.
#   __temper_ungated_find_resources_with (migrations/20260814000010)
#   VERDICT: the gated wrapper `query_find_resources_with` computes
#     `resources_visible_to(p_principal)` and hands it down — the caller's own gate, same shape as
#     `query_find_exact`. It keeps the anchor readability check for both kinds via
#     `anchor_readable_by_profile` and takes `p_anchor_reader` for it, so it cannot be lied to about
#     that authorization either.
#   EMITTER: `query_plan.rs::emit_ungated_core_call`, the same sole emitter the other two go
#     through, which is where the visible set and the principal are fixed rather than passed.
#     `[amended — 2026-08-14]` This entry first read *"not yet — beat 3 wires it, and the Rust
#     baseline below is unchanged for exactly that reason."* True when written and false one commit
#     later; corrected here rather than left, because a stale sentence inside a security guard is
#     read as its current reasoning. The Rust review it promised is recorded below.
#   RESIDUE: unchanged and accepted. The prefix is source discipline, not a database permission.
#
# REVIEWED 2026-08-14 (`__temper_ungated_follow_from`, migration 20260814000030, the follow-from
# provenance sibling). **Reviewed TWICE, and the first note is kept below because being wrong that
# way is the thing this header is for.**
#
#   VERDICT: `query_follow_from` computes `resources_visible_to(p_principal)` once and hands the
#     array down. Same shape as `query_find_resources_with`.
#   EMITTER: the call goes through `emit_ungated_core_call`'s `CoreCall::Walk` arm, which — like the
#     other three — writes `VISIBLE_IDS` itself rather than taking it as a field. There is no wrong
#     set to pass because there is no argument for it.
#   RESIDUE: unchanged and accepted.
#
#   **The first pass said "EMITTER: not yet emitted from Rust", and told the next reviewer that the
#   Rust baseline was "deliberately UNCHANGED by this entry — a reviewer seeing it move later is
#   seeing the wiring, which is the thing worth looking at."** That was true of the migration commit
#   and FALSE two commits later in the same branch, when the compiler arm landed and both Rust
#   counts went 3 -> 4. A stale note in a security guard is worse than no note: this one pointed at
#   the wiring and simultaneously explained away the very baseline movement that would have shown
#   it. Recorded rather than overwritten, because "the reviewed note aged inside one PR" is a
#   failure mode the next person should expect rather than rediscover.
#
#   NOTE A SECOND ID SET, which is new to this file and is the likeliest thing to get backwards:
#     this core takes `p_bound_ids` beside `p_visible_ids`, and their NULL polarities are OPPOSITE —
#     a NULL visible set admits NOTHING (fail-closed), a NULL bound is UNBOUNDED. They are not
#     interchangeable and neither is a gate for the other. `CoreCall::Walk` is the only variant
#     carrying both, so it is the only place they could be swapped.
#
# REVIEWED 2026-08-15 (`20260815000010`, EdgeFilter's third axis, task 01a000c2) — the SQL-file set
# grows by one; the function-NAME set does not, and that is the fact to check rather than assume.
#
#   VERDICT: unchanged. `query_follow_from` still computes `resources_visible_to(p_principal)` once
#     and hands the array down — at BOTH arities, because the incumbent 8-arity wrapper now
#     delegates to the widened 9-arity one rather than recomputing the set. One place names
#     `resources_visible_to`, not two.
#   EMITTER: unchanged. Still `emit_ungated_core_call`'s `CoreCall::Walk` arm, which writes
#     `VISIBLE_IDS` itself. The new `edge_properties` field is a narrowing expression, never an id
#     source, so it cannot be a wrong set to pass.
#   RESIDUE: unchanged and accepted.
#
#   WHY NO NEW FUNCTION NAME. `20260815000010` widens `__temper_ungated_follow_from` by ADDING a
#   ninth parameter, so Postgres holds two functions under one name. That is an overload, not a new
#   fragment: the 8-arity form is `CREATE OR REPLACE`d into a two-line delegation to the 9-arity
#   body, so there is exactly ONE walk in the schema. A reviewer checking that this entry moved only
#   the file set is checking the right thing.
#
#   AND THE REASON THE WIDENED SIGNATURE CARRIES NO `DEFAULT`, which is a security-adjacent fact
#   rather than a style one: with a default on the added parameter, EVERY 8-argument call becomes
#   `function ... is not unique` (measured, Postgres 18). The failure would land at run time on the
#   gated wrapper and on `search_graph_expand` — i.e. a migration declaring itself additive would
#   break the gated path while the ungated body kept working. Do not add a default here later.
#
# REVIEWED 2026-08-15 (`20260815000020`, p_facets fails closed, task 01a00510) — the SQL-file set
# grows by one; the function-NAME set does not, and neither does the Rust half.
#
#   VERDICT: unchanged. `query_find_resources_with` still computes resources_visible_to(p_principal)
#     once and hands the array down; this migration does not touch the wrapper.
#   EMITTER: unchanged. `emit_ungated_core_call`'s `CoreCall::Selection` arm still writes
#     `VISIBLE_IDS` itself.
#   RESIDUE: unchanged and accepted.
#
#   BODY-ONLY, and specifically NOT a visibility change: it makes a malformed `p_facets` narrow to
#   nothing instead of to everything, and stops a key-less element raising out of
#   `jsonb_build_object`. Note the direction — the pre-fix behaviour returned MORE rows than asked
#   for, never more than the caller could SEE. `p_visible_ids` was doing its job throughout; the
#   defect was in a narrowing predicate, not in the gate.
read -r -d '' SQL_BASELINE <<'EOF' || true
__temper_ungated_find_exact
__temper_ungated_find_resources_with
__temper_ungated_find_wide
__temper_ungated_follow_from
EOF

# The reviewed Rust baseline: <count> <path>, sorted by path. Every production file that NAMES an
# ungated fragment, comment lines excluded.
#
# REVIEWED 2026-08-08 (composable find fragments, plan Task 9).
#   temper-core/src/types/query/validate/mod.rs — `CALLABLE_FRAGMENTS`, the map from a declared
#     mechanic to the fragment `/api/query` emits. Naming a core here does NOT call one: this crate
#     has no database access and the map only decides which acts are reachable from this surface.
#     `[path moved — 2026-08-12]` `validate.rs` became `validate/mod.rs` when validation split into
#     a shape pass and a capability pass. The map did not move and the count is unchanged; the
#     shape pass may not read it, which is the point of that split.
#   temper-substrate/src/readback/query_plan.rs — the compiler, at 2: the two `EMIT_FIND_*`
#     constants and nothing else. The match arms that emit these calls go through the constants, so
#     they do not name the prefix — which is why a count of 2 rather than 4 is the correct reading
#     and not a scan that is missing half the file. `emit_ungated_core_call` is the sole emitter and
#     the place the visible set and the principal are fixed rather than passed.
#
# REVIEWED 2026-08-14 (selection becomes an act, task 01a0003c beat 3) — both counts move by one,
# and this is the review the SQL baseline above said would come due when the emitter landed.
#   validate/mod.rs 2 -> 3: `CALLABLE_FRAGMENTS` gains
#     `query_find_resources_with -> __temper_ungated_find_resources_with`. Naming a core here still
#     does NOT call one — this crate has no database access and the map only decides which acts are
#     reachable from this surface.
#   query_plan.rs 2 -> 3: a third `EMIT_*` constant, and nothing else. The count is constants, not
#     call sites — the match arms emit through the constants and so never name the prefix — which is
#     why 3 rather than 6 is the correct reading.
#   VERDICT/EMITTER: the selection call goes through `emit_ungated_core_call`, which is still the
#     ONE place `VISIBLE_IDS` and `PRINCIPAL_BIND` are written INTO A CORE CALL'S ARGUMENTS — which
#     is the position a caller could influence, and the precise form of the claim. `[corrected —
#     2026-08-14]` This said "the ONE place they are written", full stop, which is false: the
#     `__temper_vis` CTE defines the verdict and `unusable_tally` reads it. Neither is an argument
#     position, so the property holds — but a guard whose stated evidence fails on a grep is a guard
#     people stop believing. That function became an enum to take
#     a second call shape; a second EMITTER was rejected precisely because the security property is
#     that there is one place, and the second one is the one nobody audits.
read -r -d '' RUST_BASELINE <<'EOF' || true
4 crates/temper-core/src/types/query/validate/mod.rs
4 crates/temper-substrate/src/readback/query_plan.rs
EOF

# The SQL half: ungated functions DEFINED in migrations. A definition is what creates the hazard; a
# call from inside another SQL function is caught by the same scan, since it would have to name the
# prefix somewhere the Rust half does not look — see the CALLERS check below.
sql_current() {
  grep -rhoE \
    "CREATE([[:space:]]+OR[[:space:]]+REPLACE)?[[:space:]]+FUNCTION[[:space:]]+${PREFIX}[a-z0-9_]+" \
    "$MIGRATIONS_DIR"/*.sql 2>/dev/null \
  | sed -E 's/.*[Ff][Uu][Nn][Cc][Tt][Ii][Oo][Nn][[:space:]]+//' \
  | sort -u \
  || true
}

# Every migration file that MENTIONS the prefix, so a new SQL-side caller is visible even when it
# defines nothing. Keyed on basename: migrations are immutable, so this set only ever grows by a
# genuinely new file.
sql_files_current() {
  grep -rlE "$PREFIX" "$MIGRATIONS_DIR"/*.sql 2>/dev/null \
  | sed -E 's|.*/||' \
  | sort -u \
  || true
}

# REVIEWED 2026-08-08 — the defining migration, plus the gated wrappers it re-points, are all in one
# file. `scripts/measure/gate-shape-comparison.sql` also names the prefix but is not a migration and
# is not scanned; it is a read-only measurement harness that defines and calls nothing.
#
# REVIEWED 2026-08-10 (ADJ-1/ADJ-6) — `20260810000010` is CREATE OR REPLACE at byte-identical
# signatures on the SAME two cores (the anchor guard now covers both anchor kinds via
# `anchor_readable_by_profile`) plus `query_find_exact` (the wide wrapper's guaranteed-empty CASE,
# applied symmetrically). No new ungated function, no new caller — the function-name set above is
# unchanged; only the file set grows.
#
# REVIEWED 2026-08-14 (task 01a0003c beat 2) — `20260814000010` defines the third ungated core and
# its gated wrapper, both new. Two CREATE FUNCTION, no DROP and no replace: the whole point of the
# act framing is that a narrowing expressed as a SET needs a new function rather than new parameters
# on the shipped find fragments, which would have been shape-breaking and halted the deploy.
read -r -d '' SQL_FILES_BASELINE <<'EOF' || true
20260808000030_composable_find_family.sql
20260810000010_anchor_readability_both_kinds.sql
20260814000010_find_resources_with.sql
20260814000030_follow_from_provenance_sibling.sql
20260815000010_edge_property_predicates.sql
20260815000020_facets_fail_closed.sql
EOF

# The Rust half: production files naming an ungated fragment, per file. Comment lines are excluded
# so prose about the hazard does not move the count — the same treatment audit-grant-sinks.sh gives
# its own definition lines.
rust_current() {
  grep -rnE --include='*.rs' "$PREFIX" "$CRATES_DIR" 2>/dev/null \
  | grep -E '^[^:]*/src/[^:]*\.rs:' \
  | grep -vE '^[^:]*:[0-9]+:[[:space:]]*//' \
  | awk -F: '{print $1}' \
  | sort | uniq -c \
  | awk '{printf "%s %s\n", $1, $2}' \
  | sort -k2 \
  || true
}

SQL_CURRENT="$(sql_current)"
SQL_FILES_CURRENT="$(sql_files_current)"
RUST_CURRENT="$(rust_current)"

if [[ "${1:-}" == "--list" ]]; then
  echo "--- SQL functions:"; echo "$SQL_CURRENT"
  echo "--- SQL files:";     echo "$SQL_FILES_CURRENT"
  echo "--- Rust sites:";    echo "$RUST_CURRENT"
  exit 0
fi

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "--- SQL functions:"; echo "$SQL_CURRENT"
  echo "--- SQL files:";     echo "$SQL_FILES_CURRENT"
  echo "--- Rust sites:";    echo "$RUST_CURRENT"
  echo "^^^ copy the blocks above into SQL_BASELINE / SQL_FILES_BASELINE / RUST_BASELINE, only" >&2
  echo "    after reviewing each new site for who supplies its RBAC verdict." >&2
  exit 0
fi

fail=0

# A scan that finds NOTHING must fail, not pass. An empty set diffs clean against an empty baseline
# while asserting nothing at all, and the failure mode is mundane — a renamed directory, a changed
# CREATE spelling. This is the guard `audit-grant-sinks.sh` learned to add after its SQL half
# silently stopped covering the authoritative write.
for pair in "SQL functions:$SQL_CURRENT" "SQL files:$SQL_FILES_CURRENT" "Rust sites:$RUST_CURRENT"; do
  label="${pair%%:*}"
  value="${pair#*:}"
  if [[ -z "$value" ]]; then
    echo "audit-ungated-fragments: FAIL — the $label scan found NOTHING." >&2
    echo "  The cores exist (migrations/20260808000030) and the compiler emits them, so zero means" >&2
    echo "  the scan broke rather than that the hazard is gone. MIGRATIONS_DIR=$MIGRATIONS_DIR" >&2
    echo "  CRATES_DIR=$CRATES_DIR" >&2
    fail=1
  fi
done

check() {
  local label="$1" baseline="$2" current="$3" sort_key="$4"
  local norm
  norm="$(printf '%s\n' "$baseline" | sort $sort_key)"
  if ! diff <(printf '%s\n' "$norm") <(printf '%s\n' "$current") >"/tmp/ungated-$label.diff" 2>&1; then
    echo "audit-ungated-fragments: FAIL — the set of $label changed." >&2
    echo "diff (baseline -> current):" >&2
    cat "/tmp/ungated-$label.diff" >&2
    echo >&2
    fail=1
  fi
}

check "sqlfns"   "$SQL_BASELINE"       "$SQL_CURRENT"       "-u"
check "sqlfiles" "$SQL_FILES_BASELINE" "$SQL_FILES_CURRENT" "-u"
check "rust"     "$RUST_BASELINE"      "$RUST_CURRENT"      "-k2"

if [[ "$fail" == "0" ]]; then
  echo "audit-ungated-fragments: OK — $(printf '%s\n' "$SQL_CURRENT" | grep -c .) ungated function(s), $(printf '%s\n' "$RUST_CURRENT" | grep -c .) production file(s) naming one."
  exit 0
fi

cat >&2 <<'MSG'
audit-ungated-fragments: FAIL — the ungated-fragment surface changed.

An ungated fragment applies NO visibility gate; it trusts its caller's `p_visible_ids` absolutely.
Before accepting this change, confirm for each new/changed site:
  1. VERDICT   — who computes the visible set it is handed, and is that the caller's own gate?
  2. EMITTER   — does the call go through the single emitter that fixes the id source, rather than
                 taking a set as an argument? (query_plan.rs::emit_ungated_core_call)
  3. RESIDUE   — nothing here is a database permission. The app connects as the owning role.
See migrations/20260808000030_composable_find_family.sql and spec §6.
MSG
echo "If the change is reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-ungated-fragments.sh" >&2
exit 1
