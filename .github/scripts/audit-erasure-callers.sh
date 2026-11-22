#!/usr/bin/env bash
# audit-erasure-callers.sh — pin the production code that calls an erasure act, an erasure
# redaction function, or sets the replay setting or marker.
#
# WHY THIS EXISTS
# ---------------
# The erasure acts (`*_erasure_execute`) and their redaction halves (`*_erasure_apply_redaction`,
# `_project_*_redactions`) are SQL functions that do what they are handed: the authority to call
# them is decided in Rust, at the door (`&SystemAdmin` on the admin routes). A new service path
# that reaches one of them without that check, mounted on an ordinary route, passes every other
# audit. Likewise setting `temper.replaying` and writing the replay-target marker (a write to
# `kb_replay_target`, a call of `_replay_target_mark`) are what let the guards stand down for
# replay (`_replay_mode()`, 20261203000010); only the replay walk should do either.
#
# It pins LITERAL SQL sites only: the line where the SQL names the function, the setting or the
# table. A Rust wrapper that reaches one of them (`replay::replay`, which marks its target;
# `test_schema::mark_replay_target`) is out of its view, and so is every caller of that wrapper.
#
# This does NOT forbid a caller. It PINS the reviewed set, so a new caller, or one that moves,
# fails CI until a reviewer answers: what admits this caller, and is it the act's own door?
#
# WHAT IT SCANS
# -------------
#   * Rust under crates/*/src and api/*.rs. Each file is read only up to its first test-gated
#     MODULE (`#[cfg(test)]` / `#[cfg(all(test …))]` directly above a `mod`); comment lines are
#     dropped. A file that is wholly test code, declared by a test-gated `mod NAME;`, is skipped
#     whole (`out-of-line-test-modules.sh`).
#   * SQL under migrations/templates/ (operator-run against a live database), `--` lines dropped.
#
# Each match is counted per (class, file): a second call in a baselined file reds as well.
#
# NOT CHECKED
# -----------
#   * Migrations. The functions are defined and call each other there; a shipped migration is
#     history, and the ledger exception and write guards are what bound them.
#   * Integration tests (crates/*/tests) and in-file test modules: test code, not a door.
#   * Whether a baselined caller is itself gated. The baseline records that a reviewer read it;
#     the route-auth audits and the act's own tests hold the gate.
#   * A call spelled through a runtime-built string (`format!("SELECT {name}(…)")`), or a
#     setting named through a bind (`set_config($1, …)`). The patterns match literal names.
#   * A statement split across lines between its verb and its table (`INSERT INTO` on one line,
#     `kb_replay_target` on the next). Each line is matched alone.
#   * Production code below an in-file test module: the scan of a file stops at its first
#     test-gated `mod`, as audit-raw-standing-writers.sh's does.
#
# USAGE
#   .github/scripts/audit-erasure-callers.sh          # verify (CI mode)
#   .github/scripts/audit-erasure-callers.sh --list   # print current callers
#   UPDATE_BASELINE=1 .github/scripts/audit-erasure-callers.sh   # print a baseline after review

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${REPO_ROOT_OVERRIDE:-$(git rev-parse --show-toplevel)}"

# Reviewed baseline: <count> <class> <path>. Every entry is a reasoned caller:
#   erasure_service.rs           the person act's door; the admin route holds &SystemAdmin.
#   resource_erasure_service.rs  the resource act's door; the admin route holds &SystemAdmin.
#   replay.rs                    the replay walk: re-projects recorded erasures at their ledger
#                                positions, sets the setting transaction-local, marks its empty
#                                target first and clears the marker once the walk succeeds.
#   test_schema.rs               test-support only (feature-gated): marks a seeded test database,
#                                bypassing the empty-ledger trigger by DDL (disable, insert, enable).
#   temper-migrate main.rs       prose: the exit-5 message names the setting.
read -r -d '' BASELINE <<'EOF' || true
1 replay_setting crates/temper-migrate/src/main.rs
1 execute crates/temper-services/src/services/erasure_service.rs
1 execute crates/temper-services/src/services/resource_erasure_service.rs
2 apply_redaction crates/temper-substrate/src/replay.rs
3 projector crates/temper-substrate/src/replay.rs
2 replay_marker crates/temper-substrate/src/replay.rs
2 replay_setting crates/temper-substrate/src/replay.rs
3 replay_marker crates/temper-substrate/src/test_schema.rs
EOF

# class<TAB>pattern (POSIX ERE, lowercase: each line is matched lowercased, as SQL is
# case-insensitive). `(\\?")?` admits a quoted identifier, escaped or not, inside a Rust string.
#   replay_setting  any line naming the setting: set_config, SET, a quoted or E'' spelling, a
#                   connection option. A read or a message naming it is pinned too: in
#                   production code, naming the setting at all is the reviewed act.
read -r -d '' PATTERNS <<'EOF' || true
execute	[a-z_]*_erasure_execute(\\?")?[[:space:]]*\(
apply_redaction	[a-z_]*_erasure_apply_redaction(\\?")?[[:space:]]*\(
projector	_project_[a-z_]*redactions(\\?")?[[:space:]]*\(
replay_setting	temper[[:space:]]*\.[[:space:]]*replaying
replay_marker	(insert[[:space:]]+into|merge[[:space:]]+into|update|delete[[:space:]]+from|truncate([[:space:]]+table)?|copy|alter[[:space:]]+table([[:space:]]+if[[:space:]]+exists)?)[[:space:]]+(only[[:space:]]+)?((\\?")?public(\\?")?\.)?(\\?")?kb_replay_target|_replay_target_mark(\\?")?[[:space:]]*\(
EOF

# shellcheck source=out-of-line-test-modules.sh
source "$SCRIPT_DIR/out-of-line-test-modules.sh"

current() {
  {
    local skipped
    skipped="$(out_of_line_test_files crates)"
    {
      find crates -path '*/src/*' -name '*.rs' -print 2>/dev/null
      if [ -d api ]; then find api -maxdepth 1 -name '*.rs' -print 2>/dev/null; fi
    } \
    | sort \
    | while IFS= read -r f; do
        if grep -qxF -- "$f" <<<"$skipped"; then continue; fi
        # The patterns ride ENVIRON, not `-v`: `-v` escape-processes its value.
        SCAN_PATS="$PATTERNS" awk -v f="$f" '
          BEGIN {
            n = split(ENVIRON["SCAN_PATS"], lines, "\n")
            for (i = 1; i <= n; i++) {
              if (lines[i] == "") continue
              t = index(lines[i], "\t")
              cls[++k] = substr(lines[i], 1, t - 1)
              pat[k] = substr(lines[i], t + 1)
            }
          }
          pending && /^[[:space:]]*$/ { next }
          pending && /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]/ { exit }
          { pending = 0 }
          /^[[:space:]]*#\[cfg\((all\()?test/ { pending = 1; next }
          /^[[:space:]]*\/\// { next }
          { l = tolower($0); for (i = 1; i <= k; i++) if (l ~ pat[i]) print cls[i], f }
        ' "$f" || echo "SCAN-ERROR $f"
      done
    if [ -d migrations/templates ]; then
      find migrations/templates -name '*.sql' -print 2>/dev/null | sort \
      | while IFS= read -r f; do
          SCAN_PATS="$PATTERNS" awk -v f="$f" '
            BEGIN {
              n = split(ENVIRON["SCAN_PATS"], lines, "\n")
              for (i = 1; i <= n; i++) {
                if (lines[i] == "") continue
                t = index(lines[i], "\t")
                cls[++k] = substr(lines[i], 1, t - 1)
                pat[k] = substr(lines[i], t + 1)
              }
            }
            /^[[:space:]]*--/ { next }
            { l = tolower($0); for (i = 1; i <= k; i++) if (l ~ pat[i]) print cls[i], f }
          ' "$f" || echo "SCAN-ERROR $f"
        done
    fi
  } \
  | sort | uniq -c \
  | awk '{c = $1; $1 = ""; sub(/^ /, ""); printf "%s %s\n", c, $0}' \
  | sort -k3,3 -k2,2 \
  || true
}

CURRENT="$(current)"

# An awk that dies on a file must FAIL the scan, never shrink it.
if grep -q 'SCAN-ERROR' <<< "$CURRENT"; then
  echo "audit-erasure-callers: FAIL — the scan errored on:" >&2
  grep 'SCAN-ERROR' <<< "$CURRENT" >&2
  exit 1
fi

if [[ "${1:-}" == "--list" ]]; then
  echo "$CURRENT"
  exit 0
fi
if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo "^^^ copy into BASELINE after confirming what admits each new caller." >&2
  exit 0
fi

# The erasure doors call the acts today, so an empty result means the scan broke.
if [[ -z "$CURRENT" ]]; then
  echo "audit-erasure-callers: FAIL — the scan found no callers at all; the scan is broken." >&2
  exit 1
fi

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -k3,3 -k2,2)"
DIFF_FILE="$(mktemp)"
trap 'rm -f "$DIFF_FILE"' EXIT
if diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >"$DIFF_FILE" 2>&1; then
  echo "audit-erasure-callers: OK — erasure, redaction and replay-setting callers unchanged."
  exit 0
fi

cat >&2 <<'MSG'
audit-erasure-callers: FAIL — the set of erasure, redaction or replay-setting callers changed.

The erasure acts and redaction functions do what they are handed; the door decides who may call
them. The replay setting and marker lower the guards. Before accepting a new or moved caller,
confirm what admits it (the act's admin door, or the replay walk) and record that reason in the
baseline.

diff (baseline -> current):
MSG
cat "$DIFF_FILE" >&2
echo >&2
echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-erasure-callers.sh" >&2
exit 1
