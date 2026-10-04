#!/usr/bin/env bash
# .github/scripts/test-check-client-op-registry.sh
#
# Test harness for check-client-op-registry.sh. Runs the checker against the real temper-client
# src and against synthetic fixture directories, asserting the exit code.
#   bash .github/scripts/test-check-client-op-registry.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK_SCRIPT="${SCRIPT_DIR}/check-client-op-registry.sh"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# fixture NAME — a fresh source directory; the file body comes on stdin.
fixture() {
    local dir="${FIXTURE_DIR}/$1"
    mkdir -p "$dir"
    cat > "${dir}/client.rs"
    printf '%s' "$dir"
}

# run_test NAME SRC_DIR EXPECTED_EXIT
run_test() {
    local test_name="$1"
    local src_dir="$2"
    local expected_exit="$3"

    local output actual_exit
    set +e
    if [ -n "$src_dir" ]; then
        output="$(bash "$CHECK_SCRIPT" "$src_dir" 2>&1)"
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

echo "Running check-client-op-registry.sh tests..."
echo ""

# --- the real temper-client src passes. Runs with NO argument: the default is what CI scans. ---
run_test "real temper-client src: passes" "" 0

# --- a method spelling its own path fails ---
RAW="$(fixture raw <<'EOF'
impl Client {
    pub async fn get(&self) -> Result<()> {
        let req = self.http.get("/api/secret");
        Ok(())
    }
}
EOF
)"
run_test "raw \"/api/ literal in a method: fails" "$RAW" 1

# --- a format! path fails too: the literal prefix is what is matched ---
FORMATTED="$(fixture formatted <<'EOF'
fn path(id: Uuid) -> String {
    format!("/api/resources/{id}/secret")
}
EOF
)"
run_test "raw \"/api/ inside format!: fails" "$FORMATTED" 1

# --- the registry file itself is exempt ---
mkdir -p "${FIXTURE_DIR}/registry"
cat > "${FIXTURE_DIR}/registry/ops.rs" <<'EOF'
registry! { published { X = Get "/api/x" => "x"; } unpublished {} }
EOF
run_test "ops.rs carries literals: passes" "${FIXTURE_DIR}/registry" 0

# --- doc comments and line comments name routes without sending them ---
COMMENTS="$(fixture comments <<'EOF'
/// GET "/api/resources" — the list door.
// "/api/legacy" was removed.
fn f() {}
EOF
)"
run_test "\"/api/ in comments: passes" "$COMMENTS" 0

# --- a #[cfg(test)] module may assert rendered paths as literals ---
TESTS="$(fixture tests <<'EOF'
fn f() {}

#[cfg(test)]
mod tests {
    #[test]
    fn renders() {
        assert_eq!(render(), "/api/graph/entry");
        if true { assert!(true); }
    }
}
EOF
)"
run_test "\"/api/ inside #[cfg(test)] mod: passes" "$TESTS" 0

# --- the test-module skip ends at its closing brace: code AFTER a test module is still scanned ---
AFTER_TESTS="$(fixture after_tests <<'EOF'
#[cfg(test)]
mod tests {
    fn t() { let _ = "/api/fine-in-tests"; }
}

fn later() -> String {
    "/api/escaped".to_string()
}
EOF
)"
run_test "\"/api/ after a #[cfg(test)] mod closes: fails" "$AFTER_TESTS" 1

# --- #[cfg(test)] on a non-module item does not open a skip ---
CFG_ITEM="$(fixture cfg_item <<'EOF'
#[cfg(test)]
const FIXTURE: u8 = 1;
fn f() -> String {
    "/api/escaped".to_string()
}
EOF
)"
run_test "#[cfg(test)] on a const does not hide later code: fails" "$CFG_ITEM" 1

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
