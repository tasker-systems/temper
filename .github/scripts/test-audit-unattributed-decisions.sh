#!/usr/bin/env bash
# .github/scripts/test-audit-unattributed-decisions.sh
#
# Test harness for audit-unattributed-decisions.sh. A tripwire nobody has watched fail is not a
# tripwire.
#
# Five claims are under test, and each probe breaks EXACTLY the invariant its claim names — a probe
# that goes red for an adjacent reason proves nothing about the property it was written for:
#
#   1. A NEW decision-voiced comment with no attribution is CAUGHT. The headline case.
#   2. The SAME comment, attributed, is NOT caught. Without this, claim 1 is satisfied by a guard
#      that flags every comment — it would go red on the right input for the wrong reason, and would
#      be rebaselined into uselessness on its first real week.
#   3. An attribution just OUTSIDE the window does not cover the line. The window is a real bound,
#      not decoration; a guard that accepted an attribution anywhere in the file would be cleared by
#      one marker at the top of a 900-line module.
#   4. An EVIDENCE marker — `[found — …]`, `[measured — …]` — does NOT count as attribution. This is
#      the property most likely to be "helpfully" relaxed by a later edit, and relaxing it lets an
#      agent clear the guard by citing its own measurement as though a person had ruled.
#   5. A scan that finds NOTHING fails rather than passing vacuously.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-unattributed-decisions.sh"
PASS=0
FAIL=0

FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# run_audit DIR BASELINE_CONTENT — run the guard over a fixture tree, echo "<exit> <output>".
run_audit() {
    local dir="$1" baseline="$2" bfile out rc
    bfile="${FIXTURE_ROOT}/baseline.$$"
    printf '%s' "$baseline" > "$bfile"
    set +e
    out="$(SCOPE="$dir" BASELINE_FILE="$bfile" SCAN_UNTRACKED=1 bash "$AUDIT_SCRIPT" 2>&1)"
    rc=$?
    set -e
    printf '%s\n%s' "$rc" "$out"
}

expect_rc() {
    local label="$1" want="$2" got_all="$3"
    local got; got="$(printf '%s' "$got_all" | head -1)"
    if [ "$got" = "$want" ]; then
        ok "$label"
    else
        bad "$label" "expected exit $want, got $got" "$(printf '%s' "$got_all" | tail -n +2)"
    fi
}

# ── Claim 1: unattributed decision voice is caught ────────────────────────────
d="${FIXTURE_ROOT}/c1"; mkdir -p "$d"
cat > "${d}/lib.rs" <<'EOF'
/// A perfectly ordinary type.
pub struct Thing {
    // The field is deliberately private, so a caller cannot construct one by hand.
    inner: u8,
}
EOF
# Asserted on the OUTPUT, not on the exit code alone. An exit-1 probe is satisfied by any failure
# whatsoever — including the script dying before it compares anything, which is exactly what this
# probe did on its first run (a `grep -v` on an empty baseline exiting 1 under `pipefail`). Requiring
# the flagged path to be NAMED is what makes the probe about the property rather than about redness.
c1="$(run_audit "$d" "")"
if [ "$(printf '%s' "$c1" | head -1)" = "1" ] && printf '%s' "$c1" | grep -q 'c1/lib.rs'; then
    ok "claim 1 — unattributed decision voice goes RED, naming the file"
else
    bad "claim 1 — unattributed decision voice goes RED, naming the file" \
        "expected exit 1 AND the output to name c1/lib.rs" "$c1"
fi

# ── Claim 2: the same line, attributed, is clean ──────────────────────────────
d="${FIXTURE_ROOT}/c2"; mkdir -p "$d"
cat > "${d}/lib.rs" <<'EOF'
/// A perfectly ordinary type.
pub struct Thing {
    // The field is deliberately private, so a caller cannot construct one by hand.
    // `[decided — 2026-08-12, Pete]`
    inner: u8,
}
EOF
expect_rc "claim 2 — the SAME line, attributed, stays GREEN" 0 "$(run_audit "$d" "")"

# ── Claim 3: an attribution outside the window does not cover ─────────────────
# Nine lines of separation against a window of eight: one past the bound, so the probe fails for the
# window and not for anything else. Claim 2 has already shown the attribution itself is recognised.
d="${FIXTURE_ROOT}/c3"; mkdir -p "$d"
{
  echo '// `[decided — 2026-08-12, Pete]` — this covers something else entirely.'
  for i in $(seq 1 9); do echo "// filler line $i"; done
  echo '// The field is deliberately private, so a caller cannot construct one by hand.'
} > "${d}/lib.rs"
expect_rc "claim 3 — an attribution 9 lines away (window 8) does NOT cover" 1 "$(run_audit "$d" "")"

# ── Claim 4: evidence is not attribution ──────────────────────────────────────
d="${FIXTURE_ROOT}/c4"; mkdir -p "$d"
cat > "${d}/lib.rs" <<'EOF'
// The field is deliberately private, so a caller cannot construct one by hand.
// `[measured — 2026-08-12]` constructing one by hand was possible in 3 of 4 call sites.
// `[found — 2026-08-12]` and two of those were in production.
EOF
expect_rc "claim 4 — [measured —] / [found —] do NOT count as attribution" 1 "$(run_audit "$d" "")"

# ── Claim 5: an empty scan fails rather than passing vacuously ────────────────
d="${FIXTURE_ROOT}/c5"; mkdir -p "$d"
cat > "${d}/lib.rs" <<'EOF'
// Nothing here speaks in the decision voice at all.
pub struct Thing;
EOF
expect_rc "claim 5 — an empty scan against a non-empty baseline is FATAL" 1 \
    "$(run_audit "$d" '1 some/file.rs')"

echo
echo "PASS: $PASS  FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
