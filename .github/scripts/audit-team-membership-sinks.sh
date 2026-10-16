#!/usr/bin/env bash
# audit-team-membership-sinks.sh — enumerate every production write-site to kb_team_members and
# every caller of its recorded writer, and fail if either set has grown without review.
#
# WHY THIS EXISTS
# ---------------
# A team membership row is authority: an `owner` or `maintainer` row manages a team, and a row on
# the gating team once minted a system admin. The team-role rank work (temper-artifacts
# `specs/2026-10-08-team-role-rank-design.md`, C8 and Q5; plan `plans/2026-10-14-team-role-rank.md`
# task C6) routed every recorded membership act through ONE SQL writer,
# `_admin_team_membership_write` (migration `20261101200000_team_membership_changed.sql`), which
# writes the row and appends its `team_membership_changed` admin event in one statement. Rust
# reaches that writer through ONE function, `authz::team_membership::membership_write`, which takes
# a sealed `AuthorizedMembershipChange` — a value only the rank rule or a writer-specific token can
# build.
#
# A seal nobody checks erodes one convenient raw INSERT at a time. This script is the check. It
# pins three sets against a reviewed baseline, so none can grow without a human answering, for the
# new site, the one question every membership write must answer:
#
#   RECORDED or DERIVED? Either the write goes through `_admin_team_membership_write` (and so is
#   rank-bounded and on the ledger), or it is DERIVED state under spec R13 — a row the system
#   computes from something already recorded elsewhere (auto-join from standing, a personal team
#   from a profile), which no person chose and which therefore records no act.
#
# THE THREE HALVES
# ----------------
#   1. Rust writes  — raw `INSERT INTO` / `UPDATE` / `DELETE FROM kb_team_members` in
#                     `crates/*/src/**.rs`, per file. Test modules are skipped BY STRUCTURE: a
#                     `#[cfg(...)]` attribute whose predicate names the bare `test` cfg, followed
#                     (after any other attributes) by a `mod NAME {` line, opens a region that runs
#                     to its matching close brace. A `#[cfg(test)]` on any other item (a fn, an
#                     impl) skips nothing, a `#[cfg(feature = "test-db")]` is not a test cfg, and a
#                     doc comment mentioning `#[cfg(test)]` cannot start a skip (the attribute must
#                     open its line). A module gated only at its `mod x;` declaration elsewhere
#                     (`subscription_test_support.rs`) is invisible to a per-file pass, so it is
#                     named in the baseline instead.
#   2. Rust callers — calls of `_admin_team_membership_write` in `crates/*/src/**.rs`, outside
#                     comments and test modules. The baseline is EXACTLY ONE: `membership_write`.
#                     That is the seal's pin (plan P16): a second caller could hand the writer
#                     anything it liked.
#   3. SQL writes   — function-keyed, exactly as `audit-grant-sinks.sh`: the NAME of the function
#                     whose body writes the table, or `<top-level>:<file>` for DML outside any
#                     function. A redefinition of an allowlisted function in a later migration does
#                     not move the set; a new writer does. Case-insensitive. A function body ends at
#                     ANY closing dollar tag followed by `;` (`$$;`, `$fn$;`, `$function$;`, and
#                     `$$ LANGUAGE plpgsql;`) — the grant script's `$$;`-only reset predates the
#                     other tags.
#
# NAMED BLIND SPOT
# ----------------
# `temper-substrate`'s `replay::restore_table` restores `kb_team_members` verbatim through dynamic
# SQL built from `INPUT_TABLES`, so no grep sees it. It is not a membership act (spec "Not routed":
# a replay re-materializes rows already decided), and this script's green says NOTHING about it.
# Multi-line raw strings with unbalanced braces inside a test module could also end the structural
# skip early (which counts more, the safe direction) — string and char literals on one line are
# stripped before braces are counted.
#
# USAGE
#   .github/scripts/audit-team-membership-sinks.sh            # verify (CI mode)
#   .github/scripts/audit-team-membership-sinks.sh --list     # print the current sets
#   UPDATE_BASELINE=1 .github/scripts/audit-team-membership-sinks.sh   # print a proposed baseline
#
# Exit 0 = every set unchanged. Exit 1 = a set moved — review required.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Overridable so the harness can point each half at a fixture tree.
CRATES_DIR="${CRATES_DIR:-crates}"
MIGRATIONS_DIR="${MIGRATIONS_DIR:-migrations}"

# ── Rust writes: <count> <path>, sorted by path ─────────────────────────────────────────────────
#
# REVIEWED 2026-10-16 (team-role rank, PR C, task C6) — the first baseline. Every file below writes
# kb_team_members OUTSIDE a test module, and none of them is a recorded member act:
#   access_service.rs — `promote_admin`'s owner upsert. UNRECORDED, by decision (plan P9): it is
#     admin-only, on the governance path, and its gating-team row is inert under R11. PR D (the
#     0.7.0 train, spec R4/C10) deletes the write and this line with it.
#   scenario/access/loader.rs — the substrate scenario fixture loader. Not an act: it seeds a
#     declared world for a test or a demo, it is not reachable from any request.
#   subscription_test_support.rs — test-only. Gated `#[cfg(feature = "test-db")]` at its
#     `mod subscription_test_support;` declaration in `services/mod.rs`, so the per-file
#     structural skip cannot see the gate. Named here instead of excluded by path, so a production
#     write added to it would still move its count.
read -r -d '' BASELINE <<'EOF' || true
1 crates/temper-services/src/services/access_service.rs
2 crates/temper-services/src/services/subscription_test_support.rs
1 crates/temper-substrate/src/scenario/access/loader.rs
EOF

# ── Rust callers of the SQL writer: <count> <path> ──────────────────────────────────────────────
#
# REVIEWED 2026-10-16 (team-role rank, PR C, task C6) — exactly one: the `sqlx::query_scalar!` in
# `authz::team_membership::membership_write`, whose only argument is the sealed
# `AuthorizedMembershipChange`. Any second line here is a write path that skips the seal.
read -r -d '' CALLER_BASELINE <<'EOF' || true
1 crates/temper-services/src/authz/team_membership.rs
EOF

# ── SQL writes: function names, and <top-level>:<file> ──────────────────────────────────────────
#
# REVIEWED 2026-10-16 (team-role rank, PR C, task C6) — the spec's writer table plus plan P6:
#   _admin_team_membership_write — THE recorded writer. Row and event in one statement.
#   ensure_auto_join_memberships, backfill_auto_join_team, auto_join_reconcile — DERIVED (R13):
#     auto-join rows follow standing and a team's `auto_join_role`, which is capped at `member`
#     (PR A); `ON CONFLICT DO NOTHING`, machines skipped. No person chose them.
#   sync_personal_team — DERIVED (R13): a profile's personal team and its `owner` row, a function
#     of the profile's existence.
#   machine_client_caps_profile — DERIVED (plan P6): lowers a machine profile's owner/maintainer
#     rows to `member`, the machine ceiling; on registration it is machine personal-team genesis.
#   sync_system_membership — HISTORICAL. Defined in the canonical schema and dropped in
#     `20260722000100`; an immutable migration, so its name stays in the scan. A function created
#     again under this name would be read against this note.
#   <top-level>:20261019100000_machines_never_govern.sql — a one-off data step in a shipped,
#     immutable migration: it lowered machine rows above `member` once, and cannot run again.
read -r -d '' SQL_BASELINE <<'EOF' || true
<top-level>:20261019100000_machines_never_govern.sql
_admin_team_membership_write
auto_join_reconcile
backfill_auto_join_team
ensure_auto_join_memberships
machine_client_caps_profile
sync_personal_team
sync_system_membership
EOF

# The structural test-module skip, shared by both Rust halves. It walks each file itself, because
# the skip needs every line in order, not just the hits. Prints `<path>` once per matching,
# non-comment, non-test line.
rust_hits() {
  local pattern="$1"
  local f
  # `find` rather than `grep -r --include`, so the walk sees every file once, in order.
  # Paths print as `crates/<rest>` whatever CRATES_DIR is, so a fixture tree is read against the
  # same baseline keys as the real one.
  find "$CRATES_DIR" -type f -name '*.rs' -path '*/src/*' 2>/dev/null | sort | while read -r f; do
    awk -v pat="$pattern" -v path="crates/${f#"$CRATES_DIR"/}" '
      # Strip single-line string and char literals, so their braces do not count.
      function code_only(s) {
        gsub(/"([^"\\]|\\.)*"/, "\"\"", s)
        gsub(/'\''([^'\''\\]|\\.)'\''/, "''", s)
        return s
      }
      BEGIN { skip = 0; depth = 0; pending = 0 }
      {
        line = $0
        if (skip) {
          c = code_only(line)
          depth += gsub(/\{/, "{", c) - gsub(/\}/, "}", c)
          if (depth <= 0) { skip = 0; depth = 0 }
          next
        }
        if (pending) {
          if (line ~ /^[[:space:]]*#\[/ || line ~ /^[[:space:]]*$/ || line ~ /^[[:space:]]*\/\//) {
            # Another attribute, a blank, or a comment between the cfg and its item.
          } else if (line ~ /^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\{/) {
            c = code_only(line)
            depth = gsub(/\{/, "{", c) - gsub(/\}/, "}", c)
            pending = 0
            if (depth > 0) { skip = 1 }
            next
          } else {
            pending = 0
          }
        }
        # A cfg attribute OPENING its line whose predicate names the bare `test` cfg (not
        # `feature = "test-db"`, not `not(test)`).
        if (line ~ /^[[:space:]]*#\[cfg\(/ && line ~ /[(,[:space:]]test[),[:space:]]/ && line !~ /not\([[:space:]]*test/) {
          pending = 1
          next
        }
        if (line ~ /^[[:space:]]*\/\//) next
        if (tolower(line) ~ pat) print path
      }
    ' "$f"
  done
}

count_by_path() {
  sort | uniq -c | awk '{printf "%s %s\n", $1, $2}' | sort -k2 || true
}

rust_writes() {
  rust_hits '(insert[[:space:]]+into|update|delete[[:space:]]+from)[[:space:]]+(public\.)?kb_team_members([^a-z0-9_]|$)' | count_by_path
}

rust_callers() {
  rust_hits '_admin_team_membership_write' | count_by_path
}

sql_writes() {
  awk '
    {
      low = tolower($0)
    }
    low ~ /^[[:space:]]*create([[:space:]]+or[[:space:]]+replace)?[[:space:]]+function/ {
      line = $0
      sub(/.*[Ff][Uu][Nn][Cc][Tt][Ii][Oo][Nn][[:space:]]+/, "", line)
      sub(/[[:space:]]*\(.*/, "", line)
      sub(/^public\./, "", line)
      fn = line
    }
    low ~ /(insert[[:space:]]+into|update|delete[[:space:]]+from)[[:space:]]+(public\.)?kb_team_members([^a-z0-9_]|$)/ {
      if (low ~ /^[[:space:]]*--/) { }
      else if (fn != "") { print fn }
      else { base = FILENAME; sub(/.*\//, "", base); print "<top-level>:" base }
    }
    # A closing dollar tag followed by a statement end closes the function body: one tag on a body
    # line (`$$;`, `$fn$;`, `$$ LANGUAGE plpgsql;`), or an open AND a close on a one-line CREATE.
    $0 ~ /;[[:space:]]*$/ {
      tags = $0
      n = gsub(/\$[A-Za-z_]*\$/, "", tags)
      if (low ~ /^[[:space:]]*create/ ? n >= 2 : n >= 1) { fn = "" }
    }
  ' "$MIGRATIONS_DIR"/*.sql 2>/dev/null | sort -u || true
}

CURRENT="$(rust_writes)"
CALLERS="$(rust_callers)"
SQL_CURRENT="$(sql_writes)"

if [[ "${1:-}" == "--list" || "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo "--- callers:"
  echo "$CALLERS"
  echo "--- SQL:"
  echo "$SQL_CURRENT"
  if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
    echo "^^^ copy the three blocks above into BASELINE / CALLER_BASELINE / SQL_BASELINE in this" >&2
    echo "    script — only after classifying each changed site as RECORDED or DERIVED (R13)," >&2
    echo "    with a dated REVIEWED note saying which and why." >&2
  fi
  exit 0
fi

DIFF_DIR="$(mktemp -d)"
trap 'rm -rf "$DIFF_DIR"' EXIT
fail=0

compare() {
  local name="$1" baseline="$2" current="$3" out="$DIFF_DIR/$1.diff"
  if ! diff <(printf '%s\n' "$baseline" | sort -k2 | sed '/^$/d') \
            <(printf '%s\n' "$current" | sed '/^$/d') >"$out" 2>&1; then
    echo "audit-team-membership-sinks: FAIL — the set of $name changed." >&2
    echo "diff (baseline -> current):" >&2
    cat "$out" >&2
    echo >&2
    fail=1
  fi
}

compare "Rust kb_team_members write-sites" "$BASELINE" "$CURRENT"
compare "Rust callers of _admin_team_membership_write" "$CALLER_BASELINE" "$CALLERS"

# An empty scan must fail, not pass: the recorded writer lives in SQL, so finding nothing means the
# scan stopped matching (a renamed directory, a changed quoting), not that the writers are gone.
if [[ -z "$SQL_CURRENT" ]]; then
  echo "audit-team-membership-sinks: FAIL — the migrations scan found NO kb_team_members writes." >&2
  echo "  _admin_team_membership_write writes the table; finding zero means the scan broke." >&2
  echo "  Check MIGRATIONS_DIR=$MIGRATIONS_DIR." >&2
  fail=1
else
  if ! diff <(printf '%s\n' "$SQL_BASELINE" | sort -u | sed '/^$/d') \
            <(printf '%s\n' "$SQL_CURRENT") >"$DIFF_DIR/sql.diff" 2>&1; then
    echo "audit-team-membership-sinks: FAIL — the set of SQL kb_team_members writers changed." >&2
    echo "diff (baseline -> current):" >&2
    cat "$DIFF_DIR/sql.diff" >&2
    echo >&2
    fail=1
  fi
fi
if [[ -z "$CALLERS" ]]; then
  echo "audit-team-membership-sinks: FAIL — no Rust caller of _admin_team_membership_write found." >&2
  echo "  membership_write is its one caller; finding none means the scan broke. Check CRATES_DIR=$CRATES_DIR." >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-team-membership-sinks: OK — $(printf '%s\n' "$CURRENT" | grep -c .) Rust write files, $(printf '%s\n' "$CALLERS" | grep -c .) writer caller file, $(printf '%s\n' "$SQL_CURRENT" | grep -c .) SQL writers."
  exit 0
fi

cat >&2 <<'MSG'
A kb_team_members write path changed. Before accepting it, classify each new or moved site:
  RECORDED — it goes through authz::team_membership::membership_write (the only caller of
             _admin_team_membership_write), so the rank rule or a writer token authorized it and
             the admin ledger records it; or
  DERIVED  — it computes rows from state recorded elsewhere (spec R13: auto-join, personal team,
             the machine ceiling), so no person chose it and no act is owed.
A raw write that is neither is a membership act nobody authorized and nobody can see. Route it.
MSG
echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-team-membership-sinks.sh" >&2
exit 1
