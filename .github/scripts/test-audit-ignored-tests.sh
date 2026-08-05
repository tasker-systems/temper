#!/usr/bin/env bash
# test-audit-ignored-tests.sh — prove audit-ignored-tests.sh fails where it must.
#
# A tripwire nobody has watched fail is a tripwire nobody knows is connected. Every arm below is a
# MUTATION: break exactly one thing the script claims to catch, and require it to go red for THAT
# reason — not merely to go red.
#
# Run: bash .github/scripts/test-audit-ignored-tests.sh

set -euo pipefail

SCRIPT="$(git rev-parse --show-toplevel)/.github/scripts/audit-ignored-tests.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail=0

# Build a fixture tree: crates/ + tests/ with a known ignore population.
setup() {
  rm -rf "$TMP/root"
  mkdir -p "$TMP/root/crates/x/src" "$TMP/root/tests/e2e/tests"
  cat > "$TMP/root/crates/x/src/lib.rs" <<'RS'
#[ignore = "benchmark: run explicitly with --ignored"]
fn a() {}
RS
  cat > "$TMP/root/tests/e2e/tests/t.rs" <<'RS'
#[ignore = "deferred: blocked on the readback identity gap"]
fn b() {}
RS
  printf '1 crates/x/src/lib.rs\n1 tests/e2e/tests/t.rs\n' > "$TMP/baseline"
}

run() { IGNORE_SCAN_ROOT="$TMP/root" IGNORE_BASELINE_FILE="$TMP/baseline" bash "$SCRIPT" "$@"; }

check() { # <name> <expected-exit> <must-contain>
  local name="$1" want="$2" needle="$3"
  local out rc
  set +e
  out="$(run 2>&1)"; rc=$?
  set -e
  if [[ "$rc" == "$want" ]] && grep -qF -- "$needle" <<<"$out"; then
    echo "  ok  — $name"; pass=$((pass+1))
  else
    echo "  FAIL — $name (exit $rc, wanted $want)"; echo "$out" | sed 's/^/        /'
    fail=$((fail+1))
  fi
}

echo "audit-ignored-tests.sh:"

# 1. Baseline matches → clean.
setup
check "an unchanged set passes" 0 "set unchanged"

# 2. A bare #[ignore] → hard fail, and it must be the NO-REASON arm, not the set-changed arm.
#    This matters: a bare ignore also changes the counts, so a weaker script would fail with the
#    wrong message and the author would UPDATE_BASELINE it instead of writing a reason.
setup
printf '\n#[ignore]\nfn c() {}\n' >> "$TMP/root/crates/x/src/lib.rs"
check "a bare #[ignore] fails for having no reason" 1 "#[ignore] with no reason"

# 3. A reasoned ignore that is not pinned → set-changed.
setup
printf '\n#[ignore = "deferred: plausible"]\nfn c() {}\n' >> "$TMP/root/crates/x/src/lib.rs"
check "an unpinned reasoned ignore fails as a set change" 1 "set of #[ignore]d tests changed"

# 4. REMOVING one also fails. The direction that is not the risk still has to be deliberate —
#    otherwise an un-ignore silently shrinks the pinned set and nobody records that it now runs.
setup
: > "$TMP/root/tests/e2e/tests/t.rs"
check "removing an ignore fails too" 1 "set of #[ignore]d tests changed"

# 5. A doc comment MENTIONING #[ignore] must not count. There is a real one in the repo
#    (cloud_warmup_e2e_test.rs) explaining a past incident; counting it would make the baseline
#    churn on prose edits, which is how a tripwire becomes something people UPDATE_BASELINE on reflex.
setup
printf '\n/// `#[ignore]` — this is prose about a past incident, not an attribute.\nfn d() {}\n' \
  >> "$TMP/root/tests/e2e/tests/t.rs"
check "a doc comment mentioning #[ignore] does not count" 0 "set unchanged"

# 6. --list prints the reasons, which is the triage affordance.
setup
set +e
out="$(IGNORE_SCAN_ROOT="$TMP/root" IGNORE_BASELINE_FILE="$TMP/baseline" bash "$SCRIPT" --list 2>&1)"
rc=$?
set -e
if [[ $rc == 0 ]] && grep -qF "readback identity gap" <<<"$out"; then
  echo "  ok  — --list prints each ignore's reason"; pass=$((pass+1))
else
  echo "  FAIL — --list prints each ignore's reason (exit $rc)"; fail=$((fail+1))
fi

echo
echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
