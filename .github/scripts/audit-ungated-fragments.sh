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
#
# REVIEWED 2026-08-16 (`__temper_ungated_survey`, migration 20260816000020, the survey act, task
# 01a00c0b-9a02). The function-NAME set grows by one; the SQL-file set grows by one; both Rust
# counts grow by one.
#
#   VERDICT: `query_survey` computes `resources_visible_to(p_principal)` once and hands the array
#     down — same shape as the other gated wrappers. BUT survey has TWO visibility gates, which is
#     new to this file: the RESOURCE gate (the member join, filtered by `p_visible_ids`) and the
#     REGION gate (inside `wayfind_region_scores`, by `p_principal`). The ungated core takes BOTH
#     `p_visible_ids` and `p_principal` — a different shape from the other ungated cores, which
#     take only `p_visible_ids`. The `p_principal` is the compiler's `$1` (always bound first),
#     not a second id set. The audit invariant — "every ungated fragment is handed the RBAC verdict
#     as `p_visible_ids`" — holds for the resource gate. The region gate is inside
#     `wayfind_region_scores`, which is NOT an ungated function and is not in this baseline.
#   EMITTER: the call goes through `emit_ungated_core_call`'s `CoreCall::Survey` arm, which writes
#     `VISIBLE_IDS` and `PRINCIPAL_BIND` itself. Same one-emitter rule as the other arms.
#   RESIDUE: unchanged and accepted.
#
#   NOTE THE SECOND ARGUMENT, which is the thing to get backwards: `p_principal` is in the second
#     position (after `p_visible_ids`), not the first. The other ungated cores take only
#     `p_visible_ids`; survey takes `p_visible_ids` THEN `p_principal`. A future widening that
#     moves `p_principal` or adds a second id set would be the thing to review here.
read -r -d '' SQL_BASELINE <<'EOF' || true
__temper_ungated_find_exact
__temper_ungated_find_resources_with
__temper_ungated_find_wide
__temper_ungated_follow_from
__temper_ungated_survey
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
#
# REVIEWED 2026-08-16 (survey becomes an act, task 01a00c0b-9a02) — both counts move by one.
#   validate/mod.rs 4 -> 5: `CALLABLE_FRAGMENTS` gains
#     `query_survey -> __temper_ungated_survey`. Naming a core here still does NOT call one.
#   query_plan.rs 4 -> 5: a fifth `EMIT_*` constant (`EMIT_SURVEY`), and nothing else. Same
#     reasoning as the selection entry — the count is constants, not call sites.
#   VERDICT/EMITTER: the survey call goes through `emit_ungated_core_call`'s `CoreCall::Survey`
#     arm, which writes `VISIBLE_IDS` and `PRINCIPAL_BIND` itself. Survey is the only core that
#     takes BOTH — the others take only `VISIBLE_IDS` — because `wayfind_region_scores` applies
#     its own region visibility by principal. The `PRINCIPAL_BIND` is the compiler's `$1`, not a
#     second id set, so the one-emitter security property holds.
read -r -d '' RUST_BASELINE <<'EOF' || true
5 crates/temper-core/src/types/query/validate/mod.rs
5 crates/temper-substrate/src/readback/query_plan.rs
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

# The RELATION watch: derives the relation names ungated functions READ (FROM/JOIN targets in
# their bodies), then finds migration files that CREATE/REPLACE/ALTER those relations. Catches a
# migration that redefines a view an ungated predicate reads without naming the prefix — the gap
# `20260815000050` demonstrated. See the RELATION WATCH block above for the full reasoning.
#
# The derivation: extract every `FROM <name>` / `JOIN <name>` from every migration file that
# defines an ungated function, keep only the `kb_*` / `wayfind_*` relation names (filtering out
# CTE aliases, `unnest`, `jsonb_build_*`, single-letter aliases), then find migration files that
# `CREATE` (including `OR REPLACE`) or `ALTER` those names.
sql_relations_current() {
  # Step 1: find migration files that define ungated functions.
  local definers
  definers="$(grep -rlE "CREATE([[:space:]]+OR[[:space:]]+REPLACE)?[[:space:]]+FUNCTION[[:space:]]+${PREFIX}" "$MIGRATIONS_DIR"/*.sql 2>/dev/null || true)"

  # Step 2: extract relation names from FROM/JOIN clauses in those files.
  local relations
  relations="$(echo "$definers" | xargs grep -hoE '(FROM|JOIN)[[:space:]]+[a-z_]+' 2>/dev/null \
    | sed -E 's/^(FROM|JOIN)[[:space:]]+//' \
    | sort -u \
    | grep -E '^(kb_|wayfind_)' \
    | grep -vE '^(kb_resources|kb_chunks|kb_edges|kb_events|kb_profiles)$' \
    || true)"

  # Step 3: find migration files that CREATE/REPLACE/ALTER those relations.
  # Build a grep pattern from the relation names and scan all migrations.
  local pattern
  pattern="$(echo "$relations" | tr '\n' '|' | sed 's/|$//')"
  if [[ -z "$pattern" ]]; then
    return 0
  fi

  grep -rlE "CREATE([[:space:]]+OR[[:space:]]+REPLACE)?[[:space:]]+(VIEW|TABLE|FUNCTION)[[:space:]]+($pattern)\b|ALTER[[:space:]]+(VIEW|TABLE)[[:space:]]+.*($pattern)\b" "$MIGRATIONS_DIR"/*.sql 2>/dev/null \
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
#
# REVIEWED 2026-08-15 (`20260815000030`, the element relation and the §7 tags ruling, task
# 01a00502) — the SQL-file set grows by one; the function-NAME set does not, and neither does the
# Rust half.
#
#   VERDICT: unchanged. `query_find_resources_with` still computes resources_visible_to(p_principal)
#     once and hands the array down; this migration does not touch the wrapper.
#   EMITTER: unchanged. `emit_ungated_core_call`'s `CoreCall::Selection` arm still writes
#     `VISIBLE_IDS` itself.
#   RESIDUE: unchanged and accepted.
#
#   BODY-ONLY, and specifically NOT a visibility change. Two narrowing predicates — `p_tags` and
#   `p_facets` — stop reading `kb_properties` directly and read `kb_property_elements`, the new
#   owner-agnostic element relation, instead.
#
#   THE ONE THING WORTH CHECKING HERE, because a view interposed under an ungated core is exactly
#   the shape that could smuggle rows in: `kb_property_elements` carries NO visibility predicate and
#   is NOT asked to. It is joined to `r.id` inside a correlated subquery whose candidate rows have
#   already passed `unnest(p_visible_ids)`, so it can only ever be consulted ABOUT a resource the
#   caller was handed. It cannot widen the candidate set, because it appears in no FROM clause that
#   produces one. A future edit that lifted it into the outer FROM would change that, and is the
#   thing this entry exists to make a reviewer look for.
# `20260815000040` widens `__temper_ungated_find_resources_with` with `p_properties`, reviewed
# 2026-08-15 against this file's own three questions:
#   1. VERDICT — unchanged. `p_visible_ids` is still the first argument and still the only
#      authorization input; the added parameter is a NARROWING, whose NULL narrows nothing (the
#      opposite polarity from the verdict, whose NULL admits nothing). Its two callers are the
#      gated wrapper, which computes `resources_visible_to(p_principal)`, and the compiler, which
#      passes the hoisted `__temper_vis` CTE.
#   2. EMITTER — yes. `CoreCall::Selection` routes through `emit_ungated_core_call`, and the new
#      `resource_properties_for` can only push a `QueryBind::Json` of the caller's predicate list;
#      it has no way to write `VISIBLE_IDS` or `PRINCIPAL_BIND`, which that emitter still fixes.
#   3. RESIDUE — unchanged; still source discipline rather than a database permission.
# The predicate correlates on `rp.resource_id = r.id` where `r` is already joined to
# `unnest(p_visible_ids)`, so it reads properties only of already-admitted rows. Witnessed rather
# than reasoned about, since a leak would be an existence oracle over a caller-chosen key:
# `find_resources_with.rs::a_second_principal_sees_none_of_another_principals_resources` now probes
# both open-key spellings.
# `20260815000050` (owner-agnostic property view, task 01a00675) is REVIEWED BELOW AND IS NOT IN THE
# SET — and the gap between those two facts is the entry's actual point.
#
# It defines no function; it redefines `kb_edge_properties` and `kb_resource_properties`, the
# relations both ungated predicates READ. A first draft of it named both cores in prose, so it
# appeared in this scan and was reviewed on that basis. Trimming that prose (the migration-prose
# standing request — a migration is the worst home for an explanation) dropped it back out, and the
# count fell 9 -> 8.
#
# **THE DERIVATION IS TEXTUAL, SO THIS FILE'S FIELD OF VIEW IS NARROWER THAN ITS SUBJECT.** A
# migration can redefine a relation an ungated predicate reads and never mention `__temper_ungated_`
# — and this guard will not fire. That is not hypothetical; it is what the trim just demonstrated,
# and it is the failure `audit-grant-sinks.sh`'s header names: a guard's view narrowing while the
# number moves the reassuring way. The review below is kept precisely because the entry left the
# set: deleting it with the baseline line would erase the one record that the question was asked.
#
# NOT WIDENED HERE. Watching the view names too is the obvious fix and is a change to a security
# guard's scope, which belongs in its own reviewed change rather than as a side effect of a
# predicate-parity PR. Recorded as a recommendation, not done.
#
# `[done — 2026-08-16, task 01a006f4]` The relation watch below IS that widening. It derives the
# relation names the ungated functions read and watches for migrations that CREATE/REPLACE/ALTER
# them, so `20260815000050` is now in scope through the `SQL_RELATIONS_BASELINE` rather than the
# prefix scan. The review below is kept because the entry left the prefix-scan set and the review
# is the one record that the question was asked.
#
# Reviewed 2026-08-15, task 01a00675:
#   1. VERDICT — unchanged, and the views were never part of it. `p_visible_ids` remains the sole
#      authorization input to both cores; these relations are consulted only inside correlated
#      subqueries whose candidate rows have ALREADY passed `unnest(p_visible_ids)` (the resource
#      side correlates `rp.resource_id = r.id`, the edge side `ep.edge_id = e.id` inside `adj`).
#      Neither appears in a FROM clause that PRODUCES a candidate, so neither can widen a set.
#      **The new `kb_owner_properties` inherits exactly this and adds nothing**: it is
#      `kb_properties` minus folded rows, with no principal, profile or visibility term anywhere in
#      it — it cannot express an authorization decision, correctly or incorrectly.
#   2. EMITTER — untouched. No Rust changed shape here; `emit_ungated_core_call` still fixes
#      `VISIBLE_IDS` and `PRINCIPAL_BIND` itself. (`query_plan.rs` did change in this task — two
#      property emitters became one `properties_slot` — but that function can only push a
#      `QueryBind::Json` of the caller's predicate list, exactly as both halves could before.)
#   3. RESIDUE — unchanged; still source discipline rather than a database permission.
#   THE SCOPING PREDICATE IS THE THING TO GUARD. Each wrapper's `owner_table = '...'` is what keeps
#      an edge predicate from reading resource properties and vice versa. It moved from two
#      hand-written view bodies into two wrappers over one base — same predicate, one fewer place to
#      lose `NOT is_folded`, which now lives in the base alone. A future edit that dropped a
#      wrapper's `owner_table` filter would cross the two owners' properties silently, and
#      `kb_properties.owner_id` is NOT unique across owner tables — that is the specific edit this
#      entry exists to make a reviewer look for. Both filters are witnessed, and both still pass
#      through the derived views:
#        `find_resources_with.rs::an_open_key_predicate_does_not_reach_another_owner_kinds_property`
#          — writes a block-owned row at the SAME owner_id and key, requires it not to match, then
#            flips `owner_table` to prove the empty answer was the filter and not a dead fixture.
#        `find_resources_with.rs::a_folded_property_is_not_narrowable`
#          — the `NOT is_folded` half, which `20260815000050` moved into the base. It is now
#          asserted in one place and inherited by both wrappers rather than restated in each.
#
# Reviewed 2026-08-16, task 01a001af (the ordering operator):
#   1. VERDICT — unchanged. The new `compare` arm is a predicate inside the same `CASE q.op`
#      that `contains` and `has_key` already occupy; it correlates `rp.resource_id = r.id` /
#      `ep.edge_id = e.id` exactly as they do, so it can no more widen the candidate set than
#      they can. The `jsonb_typeof` type guard is a NARROWING — it drops cross-type rows to
#      `ELSE false` — never a widening.
#   2. EMITTER — untouched. `properties_slot` serializes the caller's `Vec<PropertyPredicate>`
#      to jsonb; the new variant rides through verbatim, same as `contains` does. No Rust call
#      shape changed.
#   3. RESIDUE — unchanged; still source discipline rather than a database permission.
#   No new ungated FUNCTION: both bodies are `CREATE OR REPLACE` at byte-identical signatures.
#   The new SQL file is listed because it edits two ungated bodies, which is the same reason
#   `20260815000010` and `20260815000040` are listed.
# Reviewed 2026-08-17, task 01a0057e (decompose the walk):
#   1. VERDICT — unchanged. The refactor touches the walk's internal shape (undirected CTE,
#      edge-ID carry) but not the gate contract: `p_visible_ids` is still the only visibility
#      input, still applied in `admitted` upstream of `adj`, and NULL still admits nothing.
#   2. EMITTER — untouched. No Rust call shape changed; the 8-arity overload and
#      `query_follow_from` both delegate to the 9-arity body unchanged.
#   3. RESIDUE — unchanged; still source discipline rather than a database permission.
#   No new ungated FUNCTION: one `CREATE OR REPLACE` at a byte-identical signature. The new SQL
#   file is listed because it redefines the ungated body, which is the same reason
#   `20260815000010` is listed.
# Reviewed 2026-08-17, task 01a0112c (the walk gains an offset):
#   **A NEW UNGATED ARITY, which none of the entries above added** — read this one on its own
#   terms rather than by analogy. `20260817000020` issues `CREATE FUNCTION` for a ten-parameter
#   `__temper_ungated_follow_from`, so the closing sentence the two entries above share ("no new
#   ungated FUNCTION") is FALSE here and is deliberately not repeated. `SQL_BASELINE` deduplicates
#   by function NAME, so a new arity of an existing name does not move it — this block is the only
#   place the addition is recorded, which is precisely why it is written out.
#   1. VERDICT — unchanged, and the new parameter cannot reach it. `p_visible_ids` is still the
#      sole visibility input and is still applied in `admitted`, upstream of `adj`. `p_offset` is
#      applied in `ranked`, STRICTLY DOWNSTREAM of both, so it can only ever skip within a set the
#      gate has already produced. Paging cannot surface a row the caller could not already see;
#      the worst a wrong offset does is return fewer of the caller's own rows.
#      The gated 10-arity `query_follow_from` computes `resources_visible_to(p_principal)` once
#      and passes it down, identically to the arities it joins.
#   2. EMITTER — untouched in the way that matters. `emit_ungated_core_call`'s `Walk` arm still
#      writes the visible set as the FIRST argument from the `VISIBLE_IDS` constant, which no act
#      arm can supply; `offset` was appended as the LAST argument. The emitter remains the single
#      place the id source is fixed.
#   3. RESIDUE — unchanged; still source discipline rather than a database permission.
read -r -d '' SQL_FILES_BASELINE <<'EOF' || true
20260808000030_composable_find_family.sql
20260810000010_anchor_readability_both_kinds.sql
20260814000010_find_resources_with.sql
20260814000030_follow_from_provenance_sibling.sql
20260815000010_edge_property_predicates.sql
20260815000020_facets_fail_closed.sql
20260815000030_property_elements_and_tag_normalization.sql
20260815000040_resource_property_predicates.sql
20260816000010_range_operator.sql
20260816000020_survey_act.sql
20260817000010_decompose_walk.sql
20260817000020_follow_from_offset.sql
EOF

# ── THE RELATION WATCH — derived from what the cores READ, not what names them ──
#
# `[added — 2026-08-16, task 01a006f4]` The SQL-file scan above watches migrations that MENTION the
# `__temper_ungated_` prefix. A migration that redefines a RELATION an ungated predicate reads can
# change what the core sees without naming it — and `20260815000050` did exactly that, redefining
# `kb_edge_properties` and `kb_resource_properties` after trimming the prose that mentioned the
# cores. The count fell 9 → 8 and CI went green, which is the guard's own stated failure mode:
# "a guard's view narrowing while the number moves the reassuring way."
#
# This scan closes that gap. It DERIVES the relation names the ungated functions read (FROM/JOIN
# targets in their bodies), then watches for migrations that CREATE, REPLACE, or ALTER those
# relations. The watch-set follows the cores rather than being restated beside them — the same
# "DERIVED, NOT PINNED" discipline the other scans follow.
#
# WHAT IT WATCHES: every relation name appearing in a FROM or JOIN clause inside an ungated
# function body, filtered to those that are VIEWs or functions (the ones a CREATE OR REPLACE can
# silently change). Base tables (`kb_resources`, `kb_chunks`, `kb_edges`) are excluded — their
# schema changes are DDL, which is visible and shape-breaking, not the silent redefinition a view
# swap is.
#
# WHAT IT DOES NOT WATCH: base tables, and relations read by non-ungated functions. The first would
# put most migrations in scope (the task's own warning); the second is not this guard's subject.
# `resources_visible_to` is the gate itself, not a relation the cores read for data — it is the one
# thing the cores are handed rather than consulting.
read -r -d '' SQL_RELATIONS_BASELINE <<'EOF' || true
20260624000001_canonical_schema.sql
20260626000001_fts_search_index.sql
20260709000002_kb_resource_workflow_props_view.sql
20260712000030_region_anchor_expand.sql
20260728000010_workflow_props_status.sql
20260731000050_wayfind_per_map_fairness.sql
20260808000020_search_arm_shared_interiority.sql
20260815000010_edge_property_predicates.sql
20260815000030_property_elements_and_tag_normalization.sql
20260815000040_resource_property_predicates.sql
20260815000050_owner_agnostic_property_view.sql
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
SQL_RELATIONS_CURRENT="$(sql_relations_current)"
RUST_CURRENT="$(rust_current)"

if [[ "${1:-}" == "--list" ]]; then
  echo "--- SQL functions:"; echo "$SQL_CURRENT"
  echo "--- SQL files:";     echo "$SQL_FILES_CURRENT"
  echo "--- SQL relations:"; echo "$SQL_RELATIONS_CURRENT"
  echo "--- Rust sites:";    echo "$RUST_CURRENT"
  exit 0
fi

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "--- SQL functions:"; echo "$SQL_CURRENT"
  echo "--- SQL files:";     echo "$SQL_FILES_CURRENT"
  echo "--- SQL relations:"; echo "$SQL_RELATIONS_CURRENT"
  echo "--- Rust sites:";    echo "$RUST_CURRENT"
  echo "^^^ copy the blocks above into SQL_BASELINE / SQL_FILES_BASELINE /" >&2
  echo "    SQL_RELATIONS_BASELINE / RUST_BASELINE, only after reviewing each" >&2
  echo "    new site for who supplies its RBAC verdict." >&2
  exit 0
fi

fail=0

# A scan that finds NOTHING must fail, not pass. An empty set diffs clean against an empty baseline
# while asserting nothing at all, and the failure mode is mundane — a renamed directory, a changed
# CREATE spelling. This is the guard `audit-grant-sinks.sh` learned to add after its SQL half
# silently stopped covering the authoritative write.
for pair in "SQL functions:$SQL_CURRENT" "SQL files:$SQL_FILES_CURRENT" "SQL relations:$SQL_RELATIONS_CURRENT" "Rust sites:$RUST_CURRENT"; do
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

check "sqlfns"     "$SQL_BASELINE"           "$SQL_CURRENT"           "-u"
check "sqlfiles"   "$SQL_FILES_BASELINE"     "$SQL_FILES_CURRENT"     "-u"
check "sqlrel"     "$SQL_RELATIONS_BASELINE" "$SQL_RELATIONS_CURRENT" "-u"
check "rust"       "$RUST_BASELINE"          "$RUST_CURRENT"          "-k2"

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
