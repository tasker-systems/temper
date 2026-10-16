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
# THE FIVE SETS
# ----------------
#   1. Rust writes  — raw `INSERT INTO` / `UPDATE` / `DELETE FROM` / `MERGE INTO kb_team_members`
#                     in `crates/*/src/**.rs`, per file — the table bare, schema-qualified or
#                     quoted, and the verb and the table on one line or split across two. Test
#                     modules are skipped BY STRUCTURE: `#[cfg(test)]` or `#[cfg(all(test, …))]`,
#                     followed (after any other attributes) by a `mod NAME {` line, opens a region
#                     that runs to its matching close brace, counted by a small lexer that ignores
#                     braces in comments, char literals and (raw) strings across lines. A skip still
#                     open at end of file fails CLOSED. `any(test, …)` and `not(test)` compile into
#                     some production build, so they skip nothing; neither does a `#[cfg(test)]` on
#                     a non-mod item, a `#[cfg(feature = "test-db")]`, or a doc comment mentioning
#                     `#[cfg(test)]`. A module gated only at its `mod x;` declaration elsewhere
#                     (`subscription_test_support.rs`) is invisible to a per-file pass, so it is
#                     named in the baseline instead.
#   2. Rust callers — calls of `_admin_team_membership_write` in `crates/*/src/**.rs`, outside
#                     comments and test modules. The baseline is EXACTLY ONE: `membership_write`.
#                     That is the seal's pin (plan P16): a second caller could hand the writer
#                     anything it liked.
#   3. Token mints  — struct literals of the three writer tokens (`GenesisToken`,
#                     `ClaimedInvitation`, `IdpReconcileToken`). Private fields keep other modules
#                     out; this pins each defining module to its one mint.
#   4. SQL writes   — function-keyed, exactly as `audit-grant-sinks.sh`: the NAME of the function
#                     whose body writes the table, or `<top-level>:<file>` for DML outside any
#                     function. A redefinition of an allowlisted function in a later migration does
#                     not move the set; a new writer does. Case-insensitive. A body closes once its
#                     two dollar tags (any tag) are seen and a statement end follows, on that line
#                     or a later one; scope also resets at every file boundary.
#   5. SQL callers  — functions (or top-level statements) that call `_admin_team_membership_write`.
#                     The baseline is EMPTY: the writer authorizes nothing, so a SQL caller could
#                     write any act with the last-owner guard off.
#
# NAMED BLIND SPOT
# ----------------
# `temper-substrate`'s `replay::restore_table` restores `kb_team_members` verbatim through dynamic
# SQL built from `INPUT_TABLES`, so no grep sees it. It is not a membership act (spec "Not routed":
# a replay re-materializes rows already decided), and this script's green says NOTHING about it.
# The same holds for any write whose table name is assembled at runtime (`format!`, a `const`), and
# for production Rust outside `crates/*/src` (the repo-root `api/` shims write nothing today).
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

# ── Writer-token mints: <count> <path> ──────────────────────────────────────────────────────────
#
# REVIEWED 2026-10-16 (team-role rank, PR C, task C6, RG-2 L3) — one struct literal per token,
# each in the one function plan P16 names: `GenesisToken` in `team_service::create_team` (after its
# team insert), `ClaimedInvitation` in `invitation_service::accept_invitation` (from the claim
# `UPDATE`'s row), `IdpReconcileToken` in `saml_provisioning_service::reconcile_idp_memberships`.
# Private fields stop other modules; this pins the defining module to its one mint. `#impl` keys
# pin the token's `impl` blocks, since one could mint through `Self { … }`: the one today is
# `ClaimedInvitation`'s three getters (fix-pass review, 2026-10-16), which build nothing.
read -r -d '' TOKEN_BASELINE <<'EOF' || true
1 crates/temper-services/src/services/invitation_service.rs
1 crates/temper-services/src/services/invitation_service.rs#impl
1 crates/temper-services/src/services/saml_provisioning_service.rs
1 crates/temper-services/src/services/team_service.rs
EOF

# ── SQL callers of the recorded writer ──────────────────────────────────────────────────────────
#
# REVIEWED 2026-10-16 (team-role rank, PR C, task C6, RG-1/RG-2) — none. The writer authorizes
# nothing (its migration header says so); a SQL function calling it could write any act with any
# `via`, the last-owner guard off. Rust's `membership_write` is its only caller, pinned above.
read -r -d '' SQL_CALLER_BASELINE <<'EOF' || true
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

# The structural test-module skip, shared by every Rust half. It walks each file itself, because
# the skip needs every line in order, not just the hits. Prints `<path>` once per matching,
# non-comment, non-test line.
#
#   $1  a pattern a single (lowercased) line must match;
#   $2  optional: a statement-verb pattern a line may END with, completed by $3 on the next code
#       line (`INSERT INTO` / `kb_team_members` split across two lines);
#   $4  optional: a pattern that excludes an otherwise matching line (a type definition).
#
# Only `#[cfg(test)]` and `#[cfg(all(test, …))]` open a skip. Any other predicate naming `test` —
# `any(test, feature = "x")`, `any(test, unix)`, `not(test)` — compiles into some production
# build, so a module under it is scanned. A skip that is still open at end of file fails CLOSED:
# the file is reported under a `#unclosed-test-module` key, which no baseline carries.
rust_hits() {
  local pattern="$1" verb="${2:-}" table="${3:-}" exclude="${4:-}"
  local f
  # `find` rather than `grep -r --include`, so the walk sees every file once, in order.
  # Paths print as `crates/<rest>` whatever CRATES_DIR is, so a fixture tree is read against the
  # same baseline keys as the real one.
  find "$CRATES_DIR" -type f -name '*.rs' -path '*/src/*' 2>/dev/null | sort | while read -r f; do
    # Patterns travel through the environment: `awk -v` would process their backslashes.
    PAT="$pattern" VERB="$verb" TABLE="$table" EXCL="$exclude" \
    awk -v path="crates/${f#"$CRATES_DIR"/}" '
      # The net brace count of one line, as Rust code: a small lexer whose state (inside a
      # string, a raw string with N hashes, or a block comment) carries ACROSS lines, so a brace
      # inside a multi-line string, a raw-string fixture or a comment never counts. Char literals
      # (an open brace in single quotes, an escaped quote) are skipped; a lifetime is not one.
      function braces(s,   i, n, ch, nx, k, d) {
        n = length(s); d = 0; i = 1
        while (i <= n) {
          ch = substr(s, i, 1); nx = substr(s, i + 1, 1)
          if (lx == "block") {
            if (ch == "*" && nx == "/") { lx = ""; i += 2; continue }
            i++; continue
          }
          if (lx == "str") {
            if (ch == "\\") { i += 2; continue }
            if (ch == "\"") lx = ""
            i++; continue
          }
          if (lx == "raw") {
            if (ch == "\"" && substr(s, i + 1, hashes) == rawclose) { lx = ""; i += 1 + hashes; continue }
            i++; continue
          }
          if (ch == "/" && nx == "/") break
          if (ch == "/" && nx == "*") { lx = "block"; i += 2; continue }
          if (ch == "\"") { lx = "str"; i++; continue }
          # `r"…"` / `r#"…"#`, and the byte and C forms `br…` / `cr…`.
          if (ch == "r" && (nx == "#" || nx == "\"") &&
              (i == 1 || substr(s, i - 1, 1) !~ /[A-Za-z0-9_]/ ||
               (substr(s, i - 1, 1) ~ /[bc]/ && (i == 2 || substr(s, i - 2, 1) !~ /[A-Za-z0-9_]/)))) {
            k = i + 1; hashes = 0
            while (substr(s, k, 1) == "#") { hashes++; k++ }
            if (substr(s, k, 1) == "\"") {
              rawclose = ""; for (j = 0; j < hashes; j++) rawclose = rawclose "#"
              lx = "raw"; i = k + 1; continue
            }
          }
          if (ch == "'\''") {
            if (nx == "\\") { k = index(substr(s, i + 2), "'\''"); i = (k ? i + 2 + k : n + 1); continue }
            if (substr(s, i + 2, 1) == "'\''") { i += 3; continue }
          }
          if (ch == "{") d++
          else if (ch == "}") d--
          i++
        }
        return d
      }
      BEGIN {
        skip = 0; depth = 0; pending = 0; prev = ""; lx = ""
        pat = ENVIRON["PAT"]; verb = ENVIRON["VERB"]; table = ENVIRON["TABLE"]; excl = ENVIRON["EXCL"]
      }
      {
        line = $0
        if (skip) {
          depth += braces(line)
          if (depth <= 0) { skip = 0; depth = 0 }
          next
        }
        if (pending) {
          if (line ~ /^[[:space:]]*#\[/ || line ~ /^[[:space:]]*$/ || line ~ /^[[:space:]]*\/\//) {
            # Another attribute, a blank, or a comment between the cfg and its item.
          } else if (line ~ /^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\{/) {
            depth = braces(line)
            pending = 0
            if (depth > 0) { skip = 1 }
            next
          } else {
            pending = 0
          }
        }
        # `#[cfg(test)]`, or `#[cfg(all(…))]` naming the bare `test` cfg with no `any(` or `not(`
        # inside it — a predicate that is false in every production build.
        if (line ~ /^[[:space:]]*#\[cfg\(test\)\]/ ||
            (line ~ /^[[:space:]]*#\[cfg\(all\(/ && line ~ /[(,[:space:]]test[),[:space:]]/ &&
             line !~ /any\(/ && line !~ /not\(/)) {
          pending = 1
          next
        }
        if (line ~ /^[[:space:]]*\/\//) next
        low = tolower(line)
        if (excl != "" && low ~ excl) { prev = low; next }
        if (low ~ pat) print path
        else if (verb != "" && prev ~ verb && low ~ table) print path
        if (low !~ /^[[:space:]]*$/) prev = low
      }
      END { if (skip) print path "#unclosed-test-module" }
    ' "$f"
  done
}

count_by_path() {
  sort | uniq -c | awk '{printf "%s %s\n", $1, $2}' | sort -k2 || true
}

# The table, bare or schema-qualified, quoted or not (`\"kb_team_members\"` inside a Rust string).
# No backslash inside a bracket expression: mawk (CI's awk on Ubuntu) rejects it.
TABLE_RE='(\\|")*(public(\\|")*[.](\\|")*)?kb_team_members([^a-z0-9_]|$)'
VERB_RE='(insert[[:space:]]+into|update([[:space:]]+only)?|delete[[:space:]]+from([[:space:]]+only)?|merge[[:space:]]+into|truncate([[:space:]]+table)?([[:space:]]+only)?|copy)'

rust_writes() {
  rust_hits "${VERB_RE}[[:space:]]+${TABLE_RE}" \
    "${VERB_RE}[[:space:]]*(\\\\)?\$" "^([[:space:]]|\")*${TABLE_RE}" | count_by_path
}

rust_callers() {
  rust_hits '_admin_team_membership_write' | count_by_path
}

# Struct-literal mints of the three writer tokens (plan P16): each type has a private field and is
# built in exactly one place. A second literal in the defining module is a second mint the
# compiler would allow, so it is pinned here. Type definitions, impls and return types are not
# mints.
rust_token_mints() {
  {
    rust_hits '(genesistoken|idpreconciletoken|claimedinvitation)[[:space:]]*\{' '' '' \
      '(struct|impl|->)[[:space:]]*[a-z_:]*(genesistoken|idpreconciletoken|claimedinvitation)'
    # An `impl` block on a token can mint through `Self { … }`, which the literal pattern cannot
    # see; pin the blocks themselves, keyed `<file>#impl`.
    rust_hits '^[[:space:]]*impl[^{]*[^a-z0-9_](genesistoken|idpreconciletoken|claimedinvitation)([^a-z0-9_]|$)' \
      | sed 's/$/#impl/'
  } | count_by_path
}

# The SQL half. Function-keyed: prints `W<TAB><key>` for a function (or `<top-level>:<file>`)
# whose body writes the table, and `C<TAB><key>` for one that CALLS the recorded writer.
#
# Scope: a `CREATE FUNCTION` opens a function; its body closes once two dollar tags (the open and
# the close, any tag name) have been seen and a statement end follows — on the same line
# (`$$;`, `$fn$ LANGUAGE plpgsql;`) or a later one (`$$` then `LANGUAGE plpgsql;`). Scope also
# resets at every file boundary, so a function a migration leaves open cannot adopt the next
# migration's DML. A nested dollar quote inside a body closes the scope early, which reports the
# rest of the body as top-level — the noisy direction, never the silent one.
sql_scan() {
  awk '
    # A function still open when its file ends fails CLOSED (a body this scan cannot see the end
    # of — a single-quoted `AS '…'` body, or a tag it does not parse), under a key no baseline
    # carries, rather than adopting the rest of the file.
    FNR == 1 {
      if (fn != "" && prevfile != "") { b = prevfile; sub(/.*\//, "", b); print "W\t<unclosed-function>:" b }
      fn = ""; tags = 0; closing = 0; prevfile = FILENAME
    }
    { low = tolower($0) }
    low ~ /^[[:space:]]*create([[:space:]]+or[[:space:]]+replace)?[[:space:]]+function/ {
      # The name follows the FIRST `function` on the line, so a trailing comment cannot rename it.
      line = substr($0, match(low, /function[[:space:]]+/) + RLENGTH)
      sub(/[[:space:]]*\(.*/, "", line)
      sub(/^public\./, "", line)
      fn = line; tags = 0; closing = 0
    }
    low !~ /^[[:space:]]*--/ {
      key = fn
      if (key == "") { base = FILENAME; sub(/.*\//, "", base); key = "<top-level>:" base }
      if (low ~ /(insert[[:space:]]+into|update([[:space:]]+only)?|delete[[:space:]]+from([[:space:]]+only)?|merge[[:space:]]+into|truncate([[:space:]]+table)?([[:space:]]+only)?|copy)[[:space:]]+"?(public"?\.)?"?kb_team_members([^a-z0-9_]|$)/) {
        print "W\t" key
      }
      if (low ~ /_admin_team_membership_write"?[[:space:]]*\(/ &&
          low !~ /^[[:space:]]*(create|comment|drop|grant|revoke|alter)[[:space:]]/ &&
          fn != "_admin_team_membership_write") {
        print "C\t" key
      }
    }
    fn != "" {
      t = $0
      tags += gsub(/\$([A-Za-z_][A-Za-z0-9_]*)?\$/, "", t)
      if (tags >= 2) closing = 1
      if (closing && $0 ~ /;[[:space:]]*(--.*)?$/) { fn = ""; tags = 0; closing = 0 }
    }
    END { if (fn != "") { b = FILENAME; sub(/.*\//, "", b); print "W\t<unclosed-function>:" b } }
  ' "$MIGRATIONS_DIR"/*.sql 2>/dev/null || true
}

sql_writes() {
  sql_scan | awk -F'\t' '$1 == "W" { print $2 }' | sort -u || true
}

sql_callers() {
  sql_scan | awk -F'\t' '$1 == "C" { print $2 }' | sort -u || true
}

CURRENT="$(rust_writes)"
CALLERS="$(rust_callers)"
TOKEN_MINTS="$(rust_token_mints)"
SQL_CURRENT="$(sql_writes)"
SQL_CALLERS="$(sql_callers)"

if [[ "${1:-}" == "--list" || "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo "--- callers:"
  echo "$CALLERS"
  echo "--- token mints:"
  echo "$TOKEN_MINTS"
  echo "--- SQL:"
  echo "$SQL_CURRENT"
  echo "--- SQL callers:"
  echo "$SQL_CALLERS"
  if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
    echo "^^^ copy the blocks above into BASELINE / CALLER_BASELINE / TOKEN_BASELINE /" >&2
    echo "    SQL_BASELINE / SQL_CALLER_BASELINE in this" >&2
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
compare "writer-token mints" "$TOKEN_BASELINE" "$TOKEN_MINTS"
compare "SQL callers of _admin_team_membership_write" "$SQL_CALLER_BASELINE" "$SQL_CALLERS"

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
  echo "audit-team-membership-sinks: OK — $(printf '%s\n' "$CURRENT" | grep -c .) Rust write files, $(printf '%s\n' "$CALLERS" | grep -c .) writer caller file, $(printf '%s\n' "$TOKEN_MINTS" | grep -c .) token-mint files, $(printf '%s\n' "$SQL_CURRENT" | grep -c .) SQL writers, $(printf '%s\n' "$SQL_CALLERS" | grep -c .) SQL callers."
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
