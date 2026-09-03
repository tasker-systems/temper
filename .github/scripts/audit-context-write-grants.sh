#!/usr/bin/env bash
# audit-context-write-grants.sh — no production surface mints a kb_contexts access grant with a
# write-class capability bit. Fail on the first one that appears.
#
# WHY THIS EXISTS
# ---------------
# `context_authorable_by_profile` admits an explicit write-grant arm (the
# `profile_explicit_grant` delegation, floored on context liveness by 20260826000110). No
# production surface mints such a row: each of the four grant doors bakes a non-context subject
# kind into `GrantCapabilityRequest`, and the other three `GrantWarrant` arms pin their subject
# kinds in the enum, where a fifth arm is a compile error. The arm is therefore a guardrail for
# a delegation act no design has been written for — authoring into a context without owning it,
# belonging to its team, or holding a minting design.
#
# This script keeps that state observable: the first surface that can light the arm fails CI
# until a human records the design decision. There is deliberately no UPDATE_BASELINE and no
# runtime allowlist — the expected set is empty, so the revisit IS editing this script (the
# failure message names what the edit must carry).
#
# FIELD OF VIEW (what this guard does not watch)
# ----------------------------------------------
# - kb_contexts grant rows carrying only can_read: the read-grant arm of `contexts_readable_by`
#   is a designed, floored axis. This guard watches write-class bits only (can_write).
# - write-class grants on every other subject kind: those are audit-grant-sinks' territory
#   (AUTHORITY + ATTENUATION at each sink, per site).
# - revocation surfaces: revocation does not mint, and de-escalation is deliberately
#   weaker-gated (see RevokeWarrant's header).
# - tests/ directories: a test fixture minting the row is how the arm's SQL gets its per-arm
#   coverage (edge_endpoint_authz_test does exactly that). A src-resident #[cfg(test)] fixture
#   is NOT excluded — it ships in the crate, so a hit there needs its review like any other.
#
# Exit 0 = no context write-grant mint surface in view. Exit 1 = one appeared, or the scan broke.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Overridable so the test harness can point the scans at fixture directories.
CRATES_DIR="${CRATES_DIR:-crates}"
MIGRATIONS_DIR="${MIGRATIONS_DIR:-migrations}"

fail=0

# ---------------------------------------------------------------------------
# Probe 1 — the grant doors. Every path into the generic chokepoint
# (`insert_grant` ← `GrantWarrant::Administered`) is fed by a
# `GrantCapabilityRequest`, and each door bakes its subject kind as a string
# literal. Flag any door whose literal is kb_contexts.
#
# Canary: the scan must see the four known doors. Zero subject literals inside
# request blocks means the scan stopped matching (renamed struct, changed
# formatting) — an empty result must fail rather than pass.
# ---------------------------------------------------------------------------
request_subjects() {
  grep -rn --include='*.rs' -A12 'GrantCapabilityRequest {' "$CRATES_DIR" 2>/dev/null \
    | grep -E '^[^:]*/src/[^:]*\.rs-' \
    | grep -E 'subject_table:[[:space:]]*"' \
    | sed -E 's/^([^-]+)-[0-9]+-.*subject_table:[[:space:]]*"([^"]+)".*/\1 \2/' \
    | sort -u \
    || true
}

CURRENT_REQUESTS="$(request_subjects)"
CONTEXT_REQUESTS="$(printf '%s\n' "$CURRENT_REQUESTS" | grep ' kb_contexts$' || true)"
DOOR_COUNT="$(printf '%s\n' "$CURRENT_REQUESTS" | grep -c . || true)"

if [[ "$DOOR_COUNT" -lt 4 ]]; then
  echo "audit-context-write-grants: FAIL — the door scan found $DOOR_COUNT GrantCapabilityRequest" >&2
  echo "  subject literals (expected ≥ 4). The scan broke; it did not find the doors gone." >&2
  echo "  Check CRATES_DIR=$CRATES_DIR." >&2
  fail=1
fi

if [[ -n "$CONTEXT_REQUESTS" ]]; then
  echo "audit-context-write-grants: FAIL — a grant door accepts kb_contexts subjects:" >&2
  printf '%s\n' "$CONTEXT_REQUESTS" | sed 's/^/  /' >&2
  fail=1
fi

# ---------------------------------------------------------------------------
# Probe 2 — raw mint statements. A statement that writes kb_access_grants and
# binds kb_contexts together with a can_write bit mints the arm directly,
# bypassing the doors. Scanned across Rust src trees and migrations/ alike;
# the window is the statement itself (write keyword to its terminating `;`).
#
# Canary: the migrations scan must see the known kb_access_grants write
# statements (the two immutable backfills and the SQL chokepoint's INSERT).
# Zero means the scan broke.
# ---------------------------------------------------------------------------
mint_statement_files() {
  local files
  # No file list ⇒ no awk: an empty list would make awk read stdin and hang.
  files="$(find "$CRATES_DIR" "$MIGRATIONS_DIR" \( -name '*.rs' -path '*/src/*' \) -o -name '*.sql' 2>/dev/null | sort || true)"
  if [[ -z "$files" ]]; then
    return 0
  fi
  awk '
    /(INSERT INTO|UPDATE)[[:space:]]+kb_access_grants([^a-z_]|$)/ {
      in_stmt = 1; subj_ctx = 0; wr = 0
    }
    in_stmt {
      if ($0 ~ /kb_contexts/) subj_ctx = 1
      if ($0 ~ /can_write/) wr = 1
      if (/;/) {
        if (subj_ctx && wr) print FILENAME
        in_stmt = 0
      }
    }
  ' $files 2>/dev/null \
    | sort -u \
    || true
}

CURRENT_MINTS="$(mint_statement_files)"

# A broken scan (nonexistent dir, unexpanded glob) must land in the canary below, not crash
# `set -e` silently — the canary's whole job is to say WHY nothing was seen.
SQL_WRITE_SIGHT="$(awk '
  /(INSERT INTO|UPDATE)[[:space:]]+kb_access_grants([^a-z_]|$)/ && !/^[[:space:]]*--/ { n++ }
  END { print n + 0 }
' "$MIGRATIONS_DIR"/*.sql 2>/dev/null || true)"
SQL_WRITE_SIGHT="${SQL_WRITE_SIGHT:-0}"

if [[ "$SQL_WRITE_SIGHT" -lt 2 ]]; then
  echo "audit-context-write-grants: FAIL — the migrations scan saw $SQL_WRITE_SIGHT kb_access_grants" >&2
  echo "  write statements (expected ≥ 2). The scan broke; check MIGRATIONS_DIR=$MIGRATIONS_DIR." >&2
  fail=1
fi

if [[ -n "$CURRENT_MINTS" ]]; then
  echo "audit-context-write-grants: FAIL — a mint statement writes kb_contexts with can_write:" >&2
  printf '%s\n' "$CURRENT_MINTS" | sed 's/^/  /' >&2
  fail=1
fi

if [[ "${1:-}" == "--list" ]]; then
  echo "GrantCapabilityRequest subject literals (src trees):"
  printf '%s\n' "$CURRENT_REQUESTS" | sed 's/^/  /'
  echo "Mint statements naming kb_contexts with can_write:"
  if [[ -n "$CURRENT_MINTS" ]]; then printf '%s\n' "$CURRENT_MINTS" | sed 's/^/  /'; else echo "  (none)"; fi
  exit 0
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-context-write-grants: OK — no kb_contexts write-grant mint surface ($DOOR_COUNT doors scanned, $SQL_WRITE_SIGHT SQL write statements in view)."
  exit 0
fi

cat >&2 <<'MSG'
audit-context-write-grants: FAIL — a kb_contexts write-grant mint surface appeared, or the scan broke.

`context_authorable_by_profile`'s explicit write-grant arm is a guardrail: no production design
has been written for delegating context authorship by grant row. If the surface in the diff is
deliberate, the design decision lands IN THIS SCRIPT before it can ship — who may mint, against
which authority, with what attenuation — as a scoped exclusion beside a comment naming the
decision. If the arm should stay unminted, change the surface to a subject kind with a minting
design.

See also: audit-grant-sinks.sh (AUTHORITY + ATTENUATION at every kb_access_grants write-site).
MSG
exit 1
