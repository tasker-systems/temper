#!/usr/bin/env bash
# .github/scripts/test-check-wire-types-in-core.sh
#
# Test harness for check-wire-types-in-core.sh. Runs the checker against the real crates/ tree and
# against synthetic fixture trees, asserting the exit code.
#   bash .github/scripts/test-check-wire-types-in-core.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK_SCRIPT="${SCRIPT_DIR}/check-wire-types-in-core.sh"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# fixture NAME CRATE — a fresh crates tree with one file in CRATE; the file body comes on stdin.
fixture() {
    local dir="${FIXTURE_DIR}/$1"
    mkdir -p "${dir}/$2/src"
    cat > "${dir}/$2/src/lib.rs"
    printf '%s' "$dir"
}

# run_test NAME CRATES_DIR EXPECTED_EXIT
run_test() {
    local test_name="$1"
    local crates_dir="$2"
    local expected_exit="$3"

    local output actual_exit
    set +e
    if [ -n "$crates_dir" ]; then
        output="$(bash "$CHECK_SCRIPT" "$crates_dir" 2>&1)"
    else
        output="$(bash "$CHECK_SCRIPT" 2>&1)"
    fi
    actual_exit=$?
    set -e

    if [ "$actual_exit" -eq "$expected_exit" ]; then
        echo "  PASS: ${test_name}"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: ${test_name}"
        echo "    expected exit=${expected_exit} actual exit=${actual_exit}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
    fi
}

echo "Running check-wire-types-in-core.sh tests..."
echo ""

# --- the real tree passes. Runs with NO argument: the default is what CI scans. ---
run_test "real crates/ tree: passes" "" 0

# --- a plain derive in a server crate fails ---
API_DERIVE="$(fixture api_derive temper-api <<'EOF'
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct Body { pub x: u8 }
EOF
)"
run_test "ToSchema derive in temper-api: fails" "$API_DERIVE" 1

# --- a feature-gated derive counts too ---
GATED="$(fixture gated temper-workflow <<'EOF'
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct Body { pub x: u8 }
EOF
)"
run_test "cfg_attr-gated ToSchema in temper-workflow: fails" "$GATED" 1

# --- query parameters are the contract too ---
PARAMS="$(fixture params temper-api <<'EOF'
#[derive(Debug, Deserialize, IntoParams)]
pub struct Q { pub limit: Option<i64> }
EOF
)"
run_test "IntoParams derive in temper-api: fails" "$PARAMS" 1

# --- a hand-written impl is caught, not only a derive ---
MANUAL="$(fixture manual temper-services <<'EOF'
impl utoipa::ToSchema for Body {}
EOF
)"
run_test "hand-written ToSchema impl: fails" "$MANUAL" 1

# --- temper-core is where they belong ---
CORE="$(fixture core temper-core <<'EOF'
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct Body { pub x: u8 }
EOF
)"
run_test "ToSchema in temper-core: passes" "$CORE" 0

# --- temper-principal is the ruled exception (core depends on it) ---
PRINCIPAL="$(fixture principal temper-principal <<'EOF'
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub enum Standing { Approved }
EOF
)"
run_test "ToSchema in temper-principal: passes" "$PRINCIPAL" 0

# --- a type with no utoipa derive (an internal door's body) is not a published wire type ---
INTERNAL="$(fixture internal temper-api <<'EOF'
#[derive(Debug, Serialize)]
pub struct WarmSummary { pub warmed: u32 }
EOF
)"
run_test "internal-door type without utoipa: passes" "$INTERNAL" 0

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
