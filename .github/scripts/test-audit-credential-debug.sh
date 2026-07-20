#!/usr/bin/env bash
# .github/scripts/test-audit-credential-debug.sh
#
# Test harness for audit-credential-debug.sh. Runs the auditor against the real crates tree and
# against synthetic fixture trees, asserting exit code and failure reason.
#
# The fixtures cover both directions that matter: a NEW credential type with a derived Debug must
# be caught (the leak this guard exists for), and a type that hand-writes a redacting Debug must
# NOT be — otherwise the guard punishes the very convention it is defending and gets baselined
# into silence.
#
#   bash .github/scripts/test-audit-credential-debug.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-credential-debug.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# new_tree NAME — make a fixture crate tree and echo its root.
new_tree() {
    local root="${FIXTURE_DIR}/$1/crates/demo/src"
    mkdir -p "$root"
    echo "$root"
}

ok()   { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad()  { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# run_test NAME SCAN_ROOT EXPECTED_EXIT [EXPECTED_SUBSTRING] — end-to-end (baseline diff included).
run_test() {
    local test_name="$1" scan_root="$2" expected_exit="$3" expected_substr="${4:-}"
    local output actual_exit
    set +e
    output="$(SCAN_ROOT="$scan_root" bash "$AUDIT_SCRIPT" 2>&1)"
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        bad "$test_name" "expected exit=${expected_exit} actual exit=${actual_exit}" "output: ${output}"
        return
    fi
    if [ -n "$expected_substr" ] && ! printf '%s' "$output" | grep -qF -- "$expected_substr"; then
        bad "$test_name" "exit matched but expected message missing: ${expected_substr}" "output: ${output}"
        return
    fi
    ok "$test_name"
}

# expect_detects NAME SCAN_ROOT EXPECTED_TYPES — assert the DETECTOR's output exactly.
#
# Fixture trees are asserted through `--list`, never through the exit code. A fixture contains none
# of the repo's real types, so the baseline diff always fires and every fixture would "fail" for a
# reason that has nothing to do with what is being tested — an exit-code assertion here would pass
# whether or not the detector saw anything. EXPECTED_TYPES is the newline-separated set of type
# names expected, or empty for "detects nothing".
expect_detects() {
    local test_name="$1" scan_root="$2" expected="$3"
    local actual
    actual="$(SCAN_ROOT="$scan_root" bash "$AUDIT_SCRIPT" --list 2>&1 | awk -F'\t' 'NF>1{print $2}' | sort -u)"
    expected="$(printf '%s' "$expected" | sort -u)"
    if [ "$actual" = "$expected" ]; then
        ok "$test_name"
    else
        bad "$test_name" "expected types: [${expected}]" "actual types:   [${actual}]"
    fi
}

echo "Running audit-credential-debug.sh tests..."
echo ""

# --- (a) the real tree matches its reviewed baseline ---
run_test "real crates tree: matches baseline" "crates" 0

# --- (b) a NEW token-bearing type with a DERIVED Debug is caught ---
# This is the defect in the prompt: a new type carrying an act-as-the-human token, deriving Debug,
# one `?value` away from writing that token to the platform log.
T="$(new_tree derived)"
cat > "${T}/lib.rs" <<'EOF'
/// A freshly minted act-as-the-human token, on its way back to the agent.
#[derive(Debug, Clone, Serialize)]
pub struct ActAsHumanTicket {
    pub profile_id: Uuid,
    pub access_token: String,
    pub expires_in: u64,
}
EOF
expect_detects "new credential type with derived Debug: caught" "$T" "ActAsHumanTicket"
run_test "  ...and that trips the baseline diff end-to-end" "$T" 1 "credential-bearing types deriving"

# --- (c) the SAME type with a hand-written redacting Debug is NOT flagged ---
# The guard must reward the convention (MintOutcome / NewGrant / SlackMintResponse) rather than
# nag about it — a guard that fires on the correct fix teaches people to silence it.
T2="$(new_tree redacted)"
cat > "${T2}/lib.rs" <<'EOF'
/// Same type, redacting like MintOutcome does.
#[derive(Clone, Serialize)]
pub struct ActAsHumanTicket {
    pub profile_id: Uuid,
    pub access_token: String,
    pub expires_in: u64,
}

impl std::fmt::Debug for ActAsHumanTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActAsHumanTicket")
            .field("profile_id", &self.profile_id)
            .field("access_token", &"redacted")
            .finish()
    }
}
EOF
expect_detects "hand-written redacting Debug: not flagged" "$T2" ""

# --- (d) a multi-line #[derive(...)] still counts as deriving Debug ---
# Attribute lines are accumulated for exactly this; a formatter-wrapped derive must not slip past.
T3="$(new_tree multiline)"
cat > "${T3}/lib.rs" <<'EOF'
#[derive(
    Debug,
    Clone,
)]
pub struct WrappedSecretHolder {
    pub client_secret: String,
}
EOF
expect_detects "multi-line derive(Debug): caught" "$T3" "WrappedSecretHolder"

# --- (e) a credential type inside a #[cfg(test)] module is NOT flagged ---
# A test fixture holding a token is not a log-leak path; flagging them buries the real signal.
T4="$(new_tree cfgtest)"
cat > "${T4}/lib.rs" <<'EOF'
pub fn nothing() {}

#[cfg(test)]
mod tests {
    #[derive(Debug)]
    struct FakeCreds {
        access_token: String,
    }
}
EOF
expect_detects "credential type under #[cfg(test)]: not flagged" "$T4" ""

# --- (f) nested-module fields are not misattributed across type boundaries ---
# Regression test for this script's own first version: item extent was tracked by a column-0 `}`,
# so a struct declared inside a `mod` never closed and swallowed the NEXT struct's fields. Here
# Innocent has no credential and must not be reported on account of Guilty's field below it.
T5="$(new_tree nested)"
cat > "${T5}/lib.rs" <<'EOF'
mod inner {
    #[derive(Debug)]
    struct Innocent {
        payload: serde_json::Value,
        producing_anchor_id: Option<Uuid>,
    }

    #[derive(Clone)]
    struct Guilty {
        client_secret: String,
    }
}
EOF
expect_detects "nested module: no cross-type field misattribution" "$T5" ""

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
