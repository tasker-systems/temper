#!/usr/bin/env bash
# Harness for check-production-features.sh. Pure bash: `cargo tree` is stubbed through
# CARGO_TREE_CMD with fixture graphs, so this runs with no toolchain.
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
SCRIPT="${REPO_ROOT}/.github/scripts/check-production-features.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
PASS=0
FAIL=0

# stub NAME BODY — a `cargo tree` stand-in that prints BODY (with @ROOT@ as the repo path) for any
# package. BODY lines are `<crate> v<ver> (@ROOT@/<path>) [<features>]`.
stub() {
  local name="$1" body="$2"
  printf '%s\n' "$body" | sed "s|@ROOT@|${REPO_ROOT}|g" > "${WORK}/${name}.out"
  cat > "${WORK}/${name}" <<EOF
#!/usr/bin/env bash
cat "${WORK}/${name}.out"
EOF
  chmod +x "${WORK}/${name}"
}

# run_test NAME STUB PACKAGES EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_test() {
  local test_name="$1" stub_name="$2" pkgs="$3" expected_exit="$4" expected_substr="${5:-}"
  local output actual_exit
  set +e
  output="$(CARGO_TREE_CMD="${WORK}/${stub_name}" DEPLOYED_PACKAGES="$pkgs" bash "$SCRIPT" 2>&1)"
  actual_exit=$?
  set -e
  if [[ "$actual_exit" -ne "$expected_exit" ]]; then
    echo "  FAIL: ${test_name} (expected exit ${expected_exit}, got ${actual_exit})"
    echo "    output: ${output}"
    FAIL=$((FAIL + 1)); return
  fi
  if [[ -n "$expected_substr" ]] && ! printf '%s' "$output" | grep -qF -- "$expected_substr"; then
    echo "  FAIL: ${test_name} (message not found: ${expected_substr})"
    echo "    output: ${output}"
    FAIL=$((FAIL + 1)); return
  fi
  echo "  PASS: ${test_name}"
  PASS=$((PASS + 1))
}

CLEAN='temper-cloud v0.6.0 (@ROOT@) []
temper-api v0.6.0 (@ROOT@/crates/temper-api) []
temper-services v0.6.0 (@ROOT@/crates/temper-services) []
temper-services v0.6.0 (@ROOT@/crates/temper-services) [] (*)
temper-ingest v0.6.0 (@ROOT@/crates/temper-ingest) [embed]
tokio v1.0.0 [full,test-util]'

echo "Running check-production-features.sh tests..."

stub clean "$CLEAN"
run_test "a clean graph passes (a third-party test-ish feature is not this check's question)" \
  clean "temper-cloud" 0 "OK"

stub harness "${CLEAN/temper-services v0.6.0 (@ROOT@\/crates\/temper-services) []/temper-services v0.6.0 (@ROOT@/crates/temper-services) [test-harness]}"
run_test "temper-services built with test-harness: fails" harness "temper-cloud" 1 \
  "compiles a test feature into a shipped build"

stub testdb "${CLEAN/temper-api v0.6.0 (@ROOT@\/crates\/temper-api) []/temper-api v0.6.0 (@ROOT@/crates/temper-api) [default,test-db]}"
run_test "any workspace crate with a test feature among others: fails" testdb "temper-cloud" 1 \
  "crates/temper-api"

stub no_services 'temper-cloud v0.6.0 (@ROOT@) []
temper-api v0.6.0 (@ROOT@/crates/temper-api) []'
run_test "temper-cloud with no temper-services line: fails (nothing was read)" no_services \
  "temper-cloud" 1 "has no temper-services line"

stub empty ''
run_test "an empty tree: fails (the package's own line is missing)" empty "temper-cli" 1 \
  "its own line is not in the cargo tree output"

cat > "${WORK}/broken" <<'EOF'
#!/usr/bin/env bash
exit 101
EOF
chmod +x "${WORK}/broken"
run_test "cargo tree failing: fails" broken "temper-cloud" 1 "cargo tree failed"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[[ "$FAIL" -eq 0 ]]
