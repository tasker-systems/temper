#!/usr/bin/env bash
# audit-test-seeders.sh — no new hand-rolled profile/team/context seeder in temper-services tests/.
#
# WHY THIS EXISTS
# ---------------
# Ruling R24 (codebase recon rulings, 2026-11-08): fixtures come from `test_support`. On 2026-11-18
# the 118 integration-test files under crates/temper-services/tests carried 91 helper fns that
# inserted a kb_profiles row by hand, under 25 names (`seed_profile_with_context` alone had 21
# copies in 8 distinct bodies); counting teams and contexts, 172 helpers seeded one of the three
# tables by hand. Each was a private copy of the same fixture, and each drifted on its own. They
# were collapsed onto `temper_services::test_support::{profile, entity, caller, team, context}` in
# one PR, leaving the 66 below; this script is the ratchet that keeps them collapsed.
#
# WHAT IT CHECKS
# --------------
# Every helper fn in a .rs file under crates/temper-services/tests (recursively, so shared modules
# such as `common/` count). A helper is any `fn` that is not itself a test: one whose attributes
# carry no `#[sqlx::test`, `#[tokio::test` or `#[test]`. A helper OFFENDS when its body holds the
# text `INSERT INTO kb_profiles`, `INSERT INTO kb_teams` or `INSERT INTO kb_contexts`.
#
# A helper in the BASELINE below is a genuinely different seeder: one whose row differs from what a
# builder makes (an email or preferences on the profile, a team minted with a v4 id, an ON CONFLICT
# upsert, a `sqlx::Result` return, a parent link written in the same breath). It is exempt. The
# baseline only shrinks. A baseline entry that no longer offends (its file or fn was renamed,
# deleted, or collapsed onto a builder) fails the run until its line is deleted, so no helper stays
# exempt by accident. A new seeder that truly differs is added here in the PR that writes it, where
# the diff makes a reviewer look at it; prefer adding a builder to `test_support` instead.
#
# WHAT IT DOES NOT CHECK
# ----------------------
# - Inserts written inline in a TEST body (a `#[sqlx::test]` fn) are not helpers and are not read.
#   They are the stated residue of the collapse: 15 inline profile inserts and many inline team,
#   context and membership inserts remained on 2026-11-18. A closure inside a test is part of the
#   test.
# - Only those three tables, and only the literal spelling `INSERT INTO <table>` on one line. A
#   seeder that writes kb_team_members alone, splits `INSERT INTO` from the table name across lines,
#   reads its SQL from a file (`include_str!`), builds the statement with `format!`, or seeds through
#   a service call or a SQL function is not seen.
# - It reads text, not the module tree, and it finds a fn's end by the first `}` at the fn's own
#   indentation, which rustfmt guarantees. A nested fn inside a helper is part of that helper.
# - Only crates/temper-services/tests is scanned: not src/ (`test_support` itself is the builder
#   home), and not other crates' tests.
#
# USAGE
#   .github/scripts/audit-test-seeders.sh                    # verify (CI and `cargo make tripwires`)
#   UPDATE_BASELINE=1 .github/scripts/audit-test-seeders.sh  # print the current offender set
#
# Exit 0 = no offender outside the baseline, and every baseline entry still offends.
# Exit 1 = otherwise.
#
# TEST SEAMS (used by test-audit-test-seeders.sh, never in CI):
#   TEST_SEEDER_SCAN_ROOT     — scan this directory instead of crates/temper-services/tests
#   TEST_SEEDER_BASELINE_FILE — read the baseline from a file instead of the block below

set -euo pipefail

REPO="$(git rev-parse --show-toplevel)"
SCAN_ROOT="${TEST_SEEDER_SCAN_ROOT:-$REPO/crates/temper-services/tests}"
if [[ ! -d "$SCAN_ROOT" ]]; then
  echo "audit-test-seeders: no directory $SCAN_ROOT" >&2
  exit 1
fi

# `<path relative to crates/temper-services/tests>::<fn name>`. Delete a line when its helper goes.
read -r -d '' BASELINE <<'EOF' || true
admin_ledger_test.rs::context_owned_by
admin_recheck_doors_test.rs::gating_team
admin_recheck_erasure_doors_test.rs::person_with_a_resource
agent_exercise_test.rs::seed_steward_reach
artifact_ask_read_test.rs::world
auto_join_reconcile_test.rs::flagged_team
blob_delete_door_test.rs::seed_team_context
blob_estate_disclosure_test.rs::seed_personal_team_context
blob_finalize_cap_test.rs::seed_commit_fixture
blob_upload_owner_bounds_test.rs::seed_begin_fixture
body_precondition_test.rs::modify_but_not_read
changed_by_filter_test.rs::seed_profile
changed_by_filter_test.rs::seed_team_context
context_change_doors_test.rs::reader
context_display_name_not_slug_test.rs::seed_profile_with_named_context
context_read_predicate_test.rs::personal_context
context_read_predicate_test.rs::profile
context_read_predicate_test.rs::team
context_read_predicate_test.rs::team_context
db_backend_principal_routing_test.rs::seed_auditor_job
delegated_grant_recheck_test.rs::fixture
entitlements_test.rs::a_gating_team
erasure_replay_test.rs::insert_profile
erasure_survey_test.rs::insert_profile
erasure_survey_test.rs::seed_bare_personal_context
erasure_survey_test.rs::seed_content
erasure_survey_test.rs::seed_team_context
field_scrub_sweep_test.rs::world
grant_door_recipient_bounds_test.rs::fixture
grant_door_recipient_bounds_test.rs::team
grant_to_callers_machine_test.rs::fixture
grant_to_callers_machine_test.rs::team
machine_reach_routes_test.rs::fixture
machine_reach_routes_test.rs::team
meta_precondition_test.rs::modify_but_not_read
person_erasure_contexts_test.rs::other_team_context
person_erasure_contexts_test.rs::person
person_erasure_estate_test.rs::other_team_context
person_erasure_estate_test.rs::person
person_erasure_identity_test.rs::person
person_erasure_identity_test.rs::team
personal_space_admin_test.rs::personal_resource
public_within_fence_test.rs::world
query_artifact_ask_test.rs::world
reachable_teams_one_definition_test.rs::profile
reachable_teams_one_definition_test.rs::team
reachable_teams_one_definition_test.rs::team_context
reassign_move_race_test.rs::context
saml_mapping_door_test.rs::team
saml_mapping_pool_elevation_test.rs::team
saml_provisioning_test.rs::seed
saml_provisioning_test.rs::three_teams_in_lock_order
sensitivity_erasure_bridge_test.rs::world
shape_loader_test.rs::seed_world
shape_schema_no_fetch_test.rs::seed_world
slack_vault_seal_format_test.rs::pinned_grant
soft_deleted_team_confers_nothing_test.rs::fixture
stale_join_decision_test.rs::a_gating_team
standing_totality_test.rs::a_profile_for_log
system_initialization_template_test.rs::bootstraps_a_first_admin
team_grant_mutation_is_role_gated_test.rs::profile
team_grant_mutation_is_role_gated_test.rs::team
team_grant_mutation_is_role_gated_test.rs::team_context
team_reach_bench.rs::corpus
team_reach_one_walk_test.rs::team
visibility_invariant_harness.rs::insert_world
EOF
if [[ -n "${TEST_SEEDER_BASELINE_FILE:-}" ]]; then
  BASELINE="$(cat "$TEST_SEEDER_BASELINE_FILE")"
fi

# Print "<fn name>" once for each offending helper fn in one file.
offending_helpers() {
  awk '
    function reset_attrs() { test_attr = 0 }
    BEGIN { in_fn = 0; reset_attrs() }
    in_fn {
      if ($0 ~ /INSERT INTO kb_(profiles|teams|contexts)([^A-Za-z0-9_]|$)/) hit = 1
      if ($0 == indent "}") {
        if (!is_test && hit) print name
        in_fn = 0; reset_attrs()
      }
      next
    }
    /^[[:space:]]*#\[(sqlx::test|tokio::test|test\])/ { test_attr = 1; next }
    /^[[:space:]]*(#\[|\/\/)/ { next }
    /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/ {
      line = $0
      match(line, /^[[:space:]]*/); indent = substr(line, 1, RLENGTH)
      sub(/^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+/, "", line)
      match(line, /^[A-Za-z_][A-Za-z0-9_]*/); name = substr(line, 1, RLENGTH)
      is_test = test_attr; hit = ($0 ~ /INSERT INTO kb_(profiles|teams|contexts)([^A-Za-z0-9_]|$)/)
      # A fn whose body opens and closes on its own line, or a bodiless declaration.
      if ($0 ~ /\}[[:space:]]*$/ || $0 ~ /;[[:space:]]*$/) {
        if (!is_test && hit) print name
        reset_attrs(); next
      }
      in_fn = 1; next
    }
    { reset_attrs() }
  ' "$1"
}

offenders() {
  (cd "$SCAN_ROOT" && find . -name '*.rs' -type f | sed 's|^\./||' | LC_ALL=C sort) | while read -r rel; do
    offending_helpers "$SCAN_ROOT/$rel" | while read -r fn; do echo "$rel::$fn"; done
  done | LC_ALL=C sort -u
}

current="$(offenders)"

if [[ -n "${UPDATE_BASELINE:-}" ]]; then
  echo "$current"
  exit 0
fi

fail=0
while read -r entry; do
  [[ -z "$entry" ]] && continue
  if ! grep -qxF -- "$entry" <<<"$BASELINE"; then
    echo "audit-test-seeders: $entry inserts kb_profiles/kb_teams/kb_contexts by hand." >&2
    echo "  Use temper_services::test_support (profile, entity, caller, team, context), or add a builder there (R24)." >&2
    fail=1
  fi
done <<<"$current"

while read -r entry; do
  [[ -z "$entry" ]] && continue
  if ! grep -qxF -- "$entry" <<<"$current"; then
    echo "audit-test-seeders: $entry no longer seeds by hand (or no longer exists); delete its baseline line." >&2
    fail=1
  fi
done <<<"$BASELINE"

if [[ $fail -eq 0 ]]; then
  n=$(grep -c . <<<"$BASELINE" || true)
  echo "audit-test-seeders: no new hand-rolled seeder; $n genuinely-different seeder(s) in the baseline"
fi
exit $fail
