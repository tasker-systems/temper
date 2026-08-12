#!/usr/bin/env bash
# .github/scripts/test-audit-ungated-fragments.sh
#
# Test harness for audit-ungated-fragments.sh. A tripwire nobody has watched fail is not a tripwire.
#
# Four claims are under test:
#   1. A NEW ungated FUNCTION is caught.
#   2. A NEW SQL-side CALLER that defines nothing is caught — the case the function scan alone
#      cannot see, and the reason the file half exists beside it.
#   3. A NEW production RUST site naming an ungated fragment is caught.
#   4. A scan that finds nothing FAILS rather than passing vacuously.
#
# ── ONE DELIBERATE DIVERGENCE FROM audit-grant-sinks.sh, stated because it looks like a mistake ──
#
# That harness asserts a REDEFINITION of a tracked function must NOT churn the baseline, on the
# grounds that migrations are immutable and a routine redefinition would train reviewers to
# UPDATE_BASELINE on reflex. The reasoning is right there and wrong here.
#
# There, the key is the write-SITE set: `_admin_grant_created` writing kb_access_grants is the fact,
# and rewriting its body does not change it. Here, the BODY IS THE HAZARD — an ungated core is
# exactly the place the visibility gate is absent, and a migration that redefines one is changing
# the code that trusts its caller absolutely. So a later migration touching a core SHOULD trip this
# guard, and claim 2 below asserts it does.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-ungated-fragments.sh"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# baseline_migrations DIR — a fixture migrations dir reproducing the two reviewed cores.
baseline_migrations() {
    local d="$1"
    mkdir -p "$d"
    cat > "${d}/20260808000030_composable_find_family.sql" <<'EOF'
CREATE FUNCTION __temper_ungated_find_exact(p_visible_ids uuid[], p_query text)
RETURNS TABLE (resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT v.resource_id FROM unnest(p_visible_ids) AS v(resource_id);
$$;

CREATE FUNCTION __temper_ungated_find_wide(p_visible_ids uuid[], p_emb vector)
RETURNS TABLE (resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT v.resource_id FROM unnest(p_visible_ids) AS v(resource_id);
$$;
EOF
}

# section NAME DIR — one labelled block of `--list`.
section() {
    local name="$1" dir="$2"
    MIGRATIONS_DIR="$dir" bash "$AUDIT_SCRIPT" --list 2>&1 \
      | awk -v want="--- ${name}:" '$0 == want {on=1; next} /^--- /{on=0} on'
}

# expect NAME DIR SECTION EXPECTED
expect() {
    local name="$1" dir="$2" sec="$3" expected actual
    expected="$(printf '%s' "$4")"
    actual="$(section "$sec" "$dir")"
    if [ "$actual" = "$expected" ]; then ok "$name"
    else bad "$name" "expected: [${expected}]" "actual:   [${actual}]"; fi
}

echo "Running audit-ungated-fragments.sh tests..."
echo ""

# --- (a) the real repo matches every reviewed baseline (end-to-end, exit 0) ---
set +e
OUT="$(bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 0 ]; then ok "real repo: SQL + Rust baselines all match"
else bad "real repo: SQL + Rust baselines all match" "exit=${RC}" "output: ${OUT}"; fi

# --- (b) the fixture reproducing the reviewed set yields exactly the baseline names ---
BASE="${FIXTURE_DIR}/base"
baseline_migrations "$BASE"
expect "fixture of the reviewed set: yields the 2 core names" "$BASE" "SQL functions" \
"__temper_ungated_find_exact
__temper_ungated_find_wide"

# --- (c) BITE: a NEW ungated function is caught ---
NEWFN="${FIXTURE_DIR}/newfn"
baseline_migrations "$NEWFN"
cat > "${NEWFN}/20260901000001_another_core.sql" <<'EOF'
CREATE FUNCTION __temper_ungated_find_everything(p_visible_ids uuid[])
RETURNS TABLE (resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT id FROM kb_resources;
$$;
EOF
expect "NEW ungated function: detected" "$NEWFN" "SQL functions" \
"__temper_ungated_find_everything
__temper_ungated_find_exact
__temper_ungated_find_wide"

set +e
OUT="$(MIGRATIONS_DIR="$NEWFN" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "__temper_ungated_find_everything"; then
    ok "  ...and fails the guard, naming the new core"
else
    bad "  ...and fails the guard, naming the new core" "exit=${RC}" "output: ${OUT}"
fi

# --- (d) THE SECOND HALF EARNS ITS PLACE: an SQL-side CALLER that defines nothing ---
#
# The function scan cannot see this — the new migration creates a perfectly ordinary gated-looking
# function that happens to call a core with a set it made up. The file scan is what catches it.
CALLER="${FIXTURE_DIR}/caller"
baseline_migrations "$CALLER"
cat > "${CALLER}/20260901000002_sneaky_caller.sql" <<'EOF'
CREATE FUNCTION search_everything(p_query text)
RETURNS TABLE (resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM __temper_ungated_find_exact(ARRAY(SELECT id FROM kb_resources), p_query) c;
$$;
EOF
expect "SQL caller that defines no core: function scan is UNCHANGED (it cannot see it)" \
"$CALLER" "SQL functions" \
"__temper_ungated_find_exact
__temper_ungated_find_wide"

expect "  ...and the FILE scan catches it" "$CALLER" "SQL files" \
"20260808000030_composable_find_family.sql
20260901000002_sneaky_caller.sql"

set +e
OUT="$(MIGRATIONS_DIR="$CALLER" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "20260901000002_sneaky_caller.sql"; then
    ok "  ...and fails the guard, naming the file"
else
    bad "  ...and fails the guard, naming the file" "exit=${RC}" "output: ${OUT}"
fi

# --- (e) BITE: a NEW production Rust site is caught ---
RUSTDIR="${FIXTURE_DIR}/crates/some-crate/src"
mkdir -p "$RUSTDIR"
cat > "${RUSTDIR}/rogue.rs" <<'EOF'
const ROGUE: &str = "__temper_ungated_find_exact";
EOF
ACTUAL="$(CRATES_DIR="${FIXTURE_DIR}/crates" bash "$AUDIT_SCRIPT" --list 2>&1 \
  | awk '$0 == "--- Rust sites:" {on=1; next} /^--- /{on=0} on' | awk '{print $1}')"
if [ "$ACTUAL" = "1" ]; then ok "NEW production Rust site: detected"
else bad "NEW production Rust site: detected" "expected one site, got: [${ACTUAL}]"; fi

set +e
OUT="$(CRATES_DIR="${FIXTURE_DIR}/crates" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "rogue.rs"; then
    ok "  ...and fails the guard, naming the file"
else
    bad "  ...and fails the guard, naming the file" "exit=${RC}" "output: ${OUT}"
fi

# --- (f) a comment mentioning a core does NOT count as a site ---
#
# The one place a cosmetic trip WOULD be corrosive: prose about the hazard is exactly what a good
# change adds, and if writing it moved the number, the number would stop being read.
COMMENTDIR="${FIXTURE_DIR}/crates-comment/some-crate/src"
mkdir -p "$COMMENTDIR"
cat > "${COMMENTDIR}/prose.rs" <<'EOF'
// Never call __temper_ungated_find_exact without a verdict.
/// See __temper_ungated_find_wide for the wide arm.
fn nothing() {}
EOF
ACTUAL="$(CRATES_DIR="${FIXTURE_DIR}/crates-comment" bash "$AUDIT_SCRIPT" --list 2>&1 \
  | awk '$0 == "--- Rust sites:" {on=1; next} /^--- /{on=0} on')"
if [ -z "$ACTUAL" ]; then ok "comment-only mention: not counted as a site"
else bad "comment-only mention: not counted as a site" "got: [${ACTUAL}]"; fi

# --- (g) an empty/missing migrations dir FAILS rather than passing vacuously ---
EMPTY="${FIXTURE_DIR}/empty"
mkdir -p "$EMPTY"
set +e
OUT="$(MIGRATIONS_DIR="$EMPTY" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "scan found NOTHING"; then
    ok "empty migrations dir: fails rather than passing vacuously"
else
    bad "empty migrations dir: fails rather than passing vacuously" "exit=${RC}" "output: ${OUT}"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
