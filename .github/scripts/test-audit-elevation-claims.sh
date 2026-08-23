#!/usr/bin/env bash
# .github/scripts/test-audit-elevation-claims.sh
#
# Test harness for audit-elevation-claims.sh — the guard that binds a surface's elevation claim to
# the gate it describes.
#
# WHY A HARNESS RATHER THAN A COMMENT
# -----------------------------------
# A guard that cannot fail is worse than no guard: it emits a green tick that means nothing, and
# this class has been "watched" by three photographs that could not fail by construction. So the
# tests below drive each trigger to RED on a fixture, and — just as important — drive the two
# things that must stay GREEN. A guard that fires on everything gets re-baselined reflexively and
# stops being read, which is the same outcome as no guard by a slower route.
#
#   (a) trigger 1 reds on a NEW elevation claim               — mechanism M2, the 2026-08-19 birth
#   (b) trigger 2 reds when a GATE's code moves               — mechanism M1, 6 of 8 measured
#   (c) trigger 2 NAMES the claims bound to the moved gate    — the message is the mechanism
#   (d) rewriting a gate's COMMENTS stays green               — fingerprints are code-only
#   (e) a plain `//` comment is not a claim                   — rustdoc and descriptions only
#
# Fixtures are COPIES of the live tree with exactly one edit, so they cannot rot into testing a
# shape the repo no longer has.
#
#   bash .github/scripts/test-audit-elevation-claims.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT="${SCRIPT_DIR}/audit-elevation-claims.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# A fixture is the subtree the auditor actually reads. Copying only that keeps each case cheap
# enough to run five of them in CI.
make_fixture() {
    local dest="$1"
    mkdir -p "$dest"/crates/{temper-cli,temper-api,temper-mcp,temper-services}/src
    cp -R "$REPO_ROOT"/crates/temper-cli/src/cli.rs "$dest"/crates/temper-cli/src/
    cp -R "$REPO_ROOT"/crates/temper-cli/src/commands "$dest"/crates/temper-cli/src/
    cp -R "$REPO_ROOT"/crates/temper-api/src/openapi.rs "$dest"/crates/temper-api/src/
    cp -R "$REPO_ROOT"/crates/temper-api/src/handlers "$dest"/crates/temper-api/src/
    cp -R "$REPO_ROOT"/crates/temper-mcp/src/service.rs "$dest"/crates/temper-mcp/src/
    cp -R "$REPO_ROOT"/crates/temper-mcp/src/tools "$dest"/crates/temper-mcp/src/
    cp -R "$REPO_ROOT"/crates/temper-services/src/services "$dest"/crates/temper-services/src/
    cp -R "$REPO_ROOT"/crates/temper-services/src/authz "$dest"/crates/temper-services/src/
}

# run_case NAME ROOT EXPECTED_EXIT [EXPECTED_SUBSTRING…]
#
# Exit code alone cannot distinguish "trigger 1 bit" from "trigger 2 bit" — both exit 1 — so every
# failing case pins the reason with a substring. That is the difference between asserting the guard
# failed and asserting it failed FOR THE REASON THE CASE CONSTRUCTED.
run_case() {
    local name="$1" root="$2" expected_exit="$3"; shift 3
    local output actual_exit=0
    output="$(REPO_ROOT_OVERRIDE="$root" bash "$AUDIT" 2>&1)" || actual_exit=$?

    if [[ "$actual_exit" != "$expected_exit" ]]; then
        echo "  FAIL  $name — expected exit $expected_exit, got $actual_exit"
        echo "$output" | sed 's/^/          /' | head -12
        FAIL=$((FAIL + 1)); return
    fi
    local substr
    for substr in "$@"; do
        if ! grep -qF -- "$substr" <<< "$output"; then
            echo "  FAIL  $name — exit $actual_exit was right, but output lacks: $substr"
            echo "$output" | sed 's/^/          /' | head -12
            FAIL=$((FAIL + 1)); return
        fi
    done
    echo "  ok    $name"
    PASS=$((PASS + 1))
}

echo "test-audit-elevation-claims"

# ── the control: the real tree is green, and says how much it does NOT cover ──────────────────
run_case "real tree passes, and names its unbound remainder" \
    "$REPO_ROOT" 0 "audit-elevation-claims: OK" "not yet bound."

# ── (a) trigger 1 — a claim BORN wrong ────────────────────────────────────────────────────────
#
# The 2026-08-19 shape exactly: a new subcommand module whose rustdoc calls itself operator-only.
T1="$WORK/t1"; make_fixture "$T1"
cat > "$T1/crates/temper-cli/src/commands/admin_widget.rs" <<'RS'
//! `temper admin widget` — operator-only widget provisioning.
RS
run_case "(a) a NEW elevation claim reds trigger 1" \
    "$T1" 1 "TRIGGER 1" "admin_widget.rs"

# ── (b)+(c) trigger 2 — the gate moved underneath the prose ───────────────────────────────────
#
# A real widening in miniature: MachineAuthority gains an arm. No string anywhere changes — which
# is the entire point, and exactly why a claim-only baseline stays green here.
T2="$WORK/t2"; make_fixture "$T2"
# The arms live in `services/machine_authz.rs`, NOT in `authz/machine.rs` — which is exactly the
# split that made the first version of this guard fingerprint the resolver and miss a new arm. The
# fixture edits where the arms actually are, and asserts it landed, so this case cannot silently
# become a no-op the way it did once already.
MACHINE_ARMS="$T2/crates/temper-services/src/services/machine_authz.rs"
perl -0pi -e 's/(enum MachineAuthority \{)/$1\n    TeamMaintainer,/' "$MACHINE_ARMS"
grep -q 'TeamMaintainer' "$MACHINE_ARMS" \
    || { echo "  FAIL  (b) fixture did not apply — the enum shape moved"; FAIL=$((FAIL + 1)); }
run_case "(b) a WIDENED gate reds trigger 2, with no string changed" \
    "$T2" 1 "TRIGGER 2" "gate: machine"
run_case "(c) trigger 2 names the claims bound to the moved gate" \
    "$T2" 1 "admin_machine.rs" "machine_registration_service.rs"

# ── (d) the anti-noise property — comment churn must NOT red ──────────────────────────────────
#
# If improving the prose above a gate reds every claim bound to it, the guard trains people to
# re-baseline without reading. Fingerprints strip comments so that this stays green.
T3="$WORK/t3"; make_fixture "$T3"
perl -0pi -e 's{//! Machine-registration authority}{//! REWRITTEN COMMENT — machine-registration authority}' \
    "$T3/crates/temper-services/src/authz/machine.rs"
printf '\n// A fresh implementation note that mentions system-admin and admin-gated behaviour.\n' \
    >> "$T3/crates/temper-services/src/authz/machine.rs"
grep -q 'REWRITTEN COMMENT' "$T3/crates/temper-services/src/authz/machine.rs" \
    || { echo "  FAIL  (d) fixture did not apply"; FAIL=$((FAIL + 1)); }
run_case "(d) rewriting a gate's COMMENTS stays green" \
    "$T3" 0 "audit-elevation-claims: OK"

# ── (e) a plain `//` comment is not a claim the system makes ──────────────────────────────────
T4="$WORK/t4"; make_fixture "$T4"
printf '\n// TODO: this path is operator-only until the gate lands.\n' \
    >> "$T4/crates/temper-cli/src/commands/admin_slack.rs"
run_case "(e) a plain // comment does not count as a claim" \
    "$T4" 0 "audit-elevation-claims: OK"

echo
echo "  $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
