#!/usr/bin/env bash
# .github/scripts/test-audit-mcp-route-auth.sh
#
# Test harness for audit-mcp-route-auth.sh. Runs the auditor against the real router.rs and
# against fixtures DERIVED from it by one edit each, asserting the auditor fails and says WHY:
#
#   - a new public route in a NEW sub-router group fails as an unknown posture
#   - a new public route in a REVIEWED public group fails the baseline diff
#   - require_mcp_auth deleted, renamed-with-suffix, or surviving only in a comment fails wiring
#   - nest_service("/mcp" retargeted fails the wiring assertion
#   - the mcp_routes declaration renamed fails loudly rather than silently skipping
#   - the handler behind a reviewed public path swapped fails the baseline diff
#   - a method APPENDED to a reviewed route fails the baseline diff (joined handler column)
#   - a crate-qualified handler ident fails the baseline diff (full chain frozen)
#   - a .nest( inside build_router fails outright
#   - a .merge( of a helper router outside the baseline fails
#   - a .nest_service( with an unreviewed argument fails
#   - a const-path .route( fails as UNPARSEABLE rather than swallowing routes
#   - a Router::default() second assembly site fails check (a2)
#   - a router with no extractable routes fails loudly rather than passing vacuously
#   - UPDATE_BASELINE=1 refuses to run on a failing fixture, and runs clean on a passing one
#   - GREEN direction: a comment containing route-shaped text freezes and trips nothing
#
# WHY A HARNESS RATHER THAN A COMMENT
# -----------------------------------
# A guard that cannot fail is worse than no guard: it emits a green tick that means nothing. The
# tests below are the evidence that this one CAN fail, re-run on every CI run. Fixtures are
# derived from the live router.rs rather than hand-written, so they cannot rot into testing a
# shape the file no longer has.
#
# Every fixture probe also overrides MCP_SRC_DIR to a directory containing ONLY that fixture
# copy — otherwise check (a2) would co-trip on the REAL router.rs (which is then "outside" the
# fixture ROUTER_FILE) and the exit-code half of the assertion would be vacuous.
#
#   bash .github/scripts/test-audit-mcp-route-auth.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-mcp-route-auth.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_ROUTER="${REPO_ROOT}/crates/temper-mcp/src/router.rs"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# run_test NAME ROUTER_FILE EXPECTED_EXIT [EXPECTED_SUBSTRING] [EXTRA_ENV]
#
# A fixture never matches the reviewed route BASELINE, so exit code alone cannot distinguish "the
# wiring assertion bit" from "the baseline diff tripped". EXPECTED_SUBSTRING pins the actual reason.
# EXTRA_ENV is extra KEY=VALUE assignments (word-split intentionally) for probes needing more
# overrides than ROUTER_FILE.
run_test() {
    local test_name="$1"
    local router_file="$2"
    local expected_exit="$3"
    local expected_substr="${4:-}"
    local extra_env="${5:-}"

    local output actual_exit
    set +e
    # shellcheck disable=SC2086
    output="$(env $extra_env ROUTER_FILE="$router_file" bash "$AUDIT_SCRIPT" 2>&1)"
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "  FAIL: ${test_name}"
        echo "    expected exit=${expected_exit} actual exit=${actual_exit}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    if [ -n "$expected_substr" ] && ! printf '%s' "$output" | grep -qF -- "$expected_substr"; then
        echo "  FAIL: ${test_name}"
        echo "    exit code matched but expected message not found: ${expected_substr}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    echo "  PASS: ${test_name}"
    PASS=$((PASS + 1))
}

# stage_fixture_src FIXTURE — copy the fixture into a private src dir and print the EXTRA_ENV
# assignment that points MCP_SRC_DIR at it (so (a2) scans the fixture, not the real crate).
stage_fixture_src() {
    mkdir -p "${FIXTURE_DIR}/src"
    cp "$1" "${FIXTURE_DIR}/src/router.rs"
    printf '%s' "MCP_SRC_DIR=${FIXTURE_DIR}/src"
}

echo "Running audit-mcp-route-auth.sh tests..."
echo ""

# --- (1) the real router.rs passes: the reviewed route set, wiring present, assembly clean ---
run_test "real router.rs: passes" "$REAL_ROUTER" 0

# --- (2) a new public route in a NEW group: unknown posture, fails naming the group ---
NEW_GROUP="${FIXTURE_DIR}/new_group.rs"
awk '{ print } /let health = Router::new\(\)/ { print "    let admin_routes = Router::new().route(\"/admin/ping\", get(|| async { \"ok\" }));" }' "$REAL_ROUTER" > "$NEW_GROUP"
run_test "new route in new group: fails as unknown posture" "$NEW_GROUP" 1 \
    "UNKNOWN auth posture" "$(stage_fixture_src "$NEW_GROUP")"

# --- (3) a new public route in a REVIEWED public group: the baseline diff is the trip ---
RENAMED_PATH="${FIXTURE_DIR}/renamed_public_path.rs"
sed 's#"/mcp/health"#"/mcp/health-v2"#' "$REAL_ROUTER" > "$RENAMED_PATH"
run_test "public path changed: fails baseline diff" "$RENAMED_PATH" 1 \
    "route set changed" "$(stage_fixture_src "$RENAMED_PATH")"

# --- (4) require_mcp_auth deleted from the mcp_routes block: the wiring assertion ---
# The `,` keeps the use-line (which ends in `;`) out of the deletion.
NO_AUTH="${FIXTURE_DIR}/no_require_mcp_auth.rs"
sed '/require_mcp_auth,/d' "$REAL_ROUTER" > "$NO_AUTH"
run_test "require_mcp_auth deleted: fails wiring" "$NO_AUTH" 1 \
    "'require_mcp_auth' not mounted in the mcp_routes" "$(stage_fixture_src "$NO_AUTH")"

# --- (4b) require_mcp_auth renamed with a suffix: the whole-ident match refuses it ---
SUFFIX_AUTH="${FIXTURE_DIR}/suffix_auth.rs"
sed 's/require_mcp_auth,/require_mcp_auth_v2,/' "$REAL_ROUTER" > "$SUFFIX_AUTH"
run_test "require_mcp_auth_v2: whole-ident match refuses it" "$SUFFIX_AUTH" 1 \
    "'require_mcp_auth' not mounted in the mcp_routes" "$(stage_fixture_src "$SUFFIX_AUTH")"

# --- (4c) the real layer deleted but its name left in a comment: comment stripping refuses it ---
COMMENT_AUTH="${FIXTURE_DIR}/comment_auth.rs"
sed 's/            require_mcp_auth,/            \/\/ gating now lives in lax_auth; unchanged from require_mcp_auth/' "$REAL_ROUTER" > "$COMMENT_AUTH"
run_test "require_mcp_auth only in a comment: fails wiring" "$COMMENT_AUTH" 1 \
    "'require_mcp_auth' not mounted in the mcp_routes" "$(stage_fixture_src "$COMMENT_AUTH")"

# --- (5) the nest retargeted: the auth layer no longer rides /mcp ---
NO_NEST="${FIXTURE_DIR}/no_nest.rs"
sed 's#"/mcp", mcp_service#"/mcp-v2", mcp_service#' "$REAL_ROUTER" > "$NO_NEST"
run_test "nest_service(\"/mcp\" retargeted: fails wiring" "$NO_NEST" 1 \
    "'nest_service(\"/mcp\"' not present in the mcp_routes" "$(stage_fixture_src "$NO_NEST")"

# --- (5b) .nest_service with a different argument: refused outright ---
BAD_NEST_SVC="${FIXTURE_DIR}/bad_nest_svc.rs"
sed 's#\.nest_service("/mcp", mcp_service)#.nest_service("/admin", mcp_service)#' "$REAL_ROUTER" > "$BAD_NEST_SVC"
run_test "nest_service with unreviewed argument: fails" "$BAD_NEST_SVC" 1 \
    "unreviewed argument" "$(stage_fixture_src "$BAD_NEST_SVC")"

# --- (6) the mcp_routes declaration renamed: fails loudly, never silently skips ---
RENAMED_BLOCK="${FIXTURE_DIR}/renamed_block.rs"
sed 's/let mcp_routes/let mcp_service_routes/' "$REAL_ROUTER" > "$RENAMED_BLOCK"
run_test "mcp_routes renamed: fails loudly" "$RENAMED_BLOCK" 1 \
    "sub-router group 'mcp_routes' not found" "$(stage_fixture_src "$RENAMED_BLOCK")"

# --- (7) the handler behind a reviewed public path swapped: baseline diff, path alone insufficient ---
SWAPPED_HANDLER="${FIXTURE_DIR}/swapped_handler.rs"
sed 's/discovery::oauth_protected_resource/discovery::some_other_metadata/' "$REAL_ROUTER" > "$SWAPPED_HANDLER"
run_test "handler swapped behind reviewed path: fails baseline diff" "$SWAPPED_HANDLER" 1 \
    "route set changed" "$(stage_fixture_src "$SWAPPED_HANDLER")"

# --- (7b) a crate-qualified handler ident: the full ::-chain is frozen, not its first two segments ---
CRATE_QUALIFIED="${FIXTURE_DIR}/crate_qualified.rs"
sed 's/discovery::oauth_protected_resource/crate::discovery::totally_different/' "$REAL_ROUTER" > "$CRATE_QUALIFIED"
run_test "crate-qualified handler swap: fails baseline diff" "$CRATE_QUALIFIED" 1 \
    "route set changed" "$(stage_fixture_src "$CRATE_QUALIFIED")"

# --- (7c) a method APPENDED to a reviewed route: joined handler column trips the baseline ---
APPENDED_METHOD="${FIXTURE_DIR}/appended_method.rs"
sed 's/post(discovery::register_client)/post(discovery::register_client).post(discovery::brand_new_handler)/' "$REAL_ROUTER" > "$APPENDED_METHOD"
run_test "method appended to reviewed route: fails baseline diff" "$APPENDED_METHOD" 1 \
    "route set changed" "$(stage_fixture_src "$APPENDED_METHOD")"

# --- (8) no extractable routes: vacuous green is a failure mode ---
EMPTY_ROUTER="${FIXTURE_DIR}/empty.rs"
: > "$EMPTY_ROUTER"
run_test "router with no build_router: fails loudly" "$EMPTY_ROUTER" 1 \
    "no routes extracted"

# --- (9) a second router-assembly site under the crate: fails even with a clean build_router ---
MCP_SRC_FIX="${FIXTURE_DIR}/mcp_src"
mkdir -p "$MCP_SRC_FIX"
cp "$REAL_ROUTER" "$MCP_SRC_FIX/router.rs"
printf 'use axum::Router;\npub fn stray() -> Router {\n    Router::new()\n}\n' > "$MCP_SRC_FIX/stray.rs"
run_test "second assembly site: fails" "${MCP_SRC_FIX}/router.rs" 1 \
    "outside the frozen router file" "MCP_SRC_DIR=${MCP_SRC_FIX}"

# --- (9b) Router::default() is assembly too ---
MCP_SRC_DEFAULT="${FIXTURE_DIR}/mcp_src_default"
mkdir -p "$MCP_SRC_DEFAULT"
cp "$REAL_ROUTER" "$MCP_SRC_DEFAULT/router.rs"
printf 'use axum::Router;\npub fn stray() -> Router {\n    Router::default()\n}\n' > "$MCP_SRC_DEFAULT/stray.rs"
run_test "Router::default second site: fails" "${MCP_SRC_DEFAULT}/router.rs" 1 \
    "outside the frozen router file" "MCP_SRC_DIR=${MCP_SRC_DEFAULT}"

# --- (10) a helper router merged in: its routes live outside the baseline ---
HELPER_MERGE="${FIXTURE_DIR}/helper_merge.rs"
awk '{ print } /\.merge\(health\)/ { print "            .merge(admin_routes())," }' "$REAL_ROUTER" > "$HELPER_MERGE"
run_test ".merge of a helper router: fails" "$HELPER_MERGE" 1 \
    "outside the reviewed groups" "$(stage_fixture_src "$HELPER_MERGE")"

# --- (10b) same-line laundering: a whitelisted merge must not hide a second merge beside it ---
LAUNDERED_MERGE="${FIXTURE_DIR}/laundered_merge.rs"
awk '{ print } /\.merge\(discovery_routes\)/ { print "            .merge(discovery_routes).merge(admin_routes())," ; next }' "$REAL_ROUTER" > "$LAUNDERED_MERGE"
run_test "whitelisted merge beside a smuggled one: fails per-occurrence" "$LAUNDERED_MERGE" 1 \
    "outside the reviewed groups" "$(stage_fixture_src "$LAUNDERED_MERGE")"

# --- (10c) a block comment naming require_mcp_auth must not satisfy the wiring assertion ---
BLOCK_COMMENT_AUTH="${FIXTURE_DIR}/block_comment_auth.rs"
awk '{ print } /            require_mcp_auth,/ { print "            /* auth wiring unchanged: require_mcp_auth still gates /mcp */" ; next }' "$REAL_ROUTER" \
  | sed 's/            require_mcp_auth,/            lax_auth,/' > "$BLOCK_COMMENT_AUTH"
run_test "require_mcp_auth only in a block comment: fails wiring" "$BLOCK_COMMENT_AUTH" 1 \
    "'require_mcp_auth' not mounted in the mcp_routes" "$(stage_fixture_src "$BLOCK_COMMENT_AUTH")"

# --- (11) .nest( inside build_router: refused outright ---
NESTED="${FIXTURE_DIR}/nested.rs"
awk '{ print } /\.merge\(health\)/ { print "            .nest(\"/admin\", admin_router())," }' "$REAL_ROUTER" > "$NESTED"
run_test ".nest( in build_router: fails outright" "$NESTED" 1 \
    ".nest() found in build_router" "$(stage_fixture_src "$NESTED")"

# --- (12) a const-path route: UNPARSEABLE, not silently swallowed ---
CONST_PATH="${FIXTURE_DIR}/const_path.rs"
awk '{ print } /let health = Router::new\(\)/ { print "    let admin_routes = Router::new().route(ADMIN_PATH, get(admin::ping));" }' "$REAL_ROUTER" > "$CONST_PATH"
run_test "const-path route: fails as UNPARSEABLE" "$CONST_PATH" 1 \
    "no literal path string" "$(stage_fixture_src "$CONST_PATH")"

# --- (13) a comment containing route-shaped text: GREEN direction — prose trips nothing ---
COMMENT_ROUTE="${FIXTURE_DIR}/comment_route.rs"
awk '{ print } /let health = Router::new\(\)/ { print "    // legacy: .route(\"/mcp/healthz\", get(health::probe)) — removed, do not re-add" }' "$REAL_ROUTER" > "$COMMENT_ROUTE"
# ROUTER_FILE IS the staged copy here: a green probe must not have a second Router::new()-bearing
# file lying around inside its own MCP_SRC_DIR, or (a2) would fire on the harness's plumbing.
mkdir -p "${FIXTURE_DIR}/green_src"
cp "$COMMENT_ROUTE" "${FIXTURE_DIR}/green_src/router.rs"
run_test "route-shaped comment: stays green" "${FIXTURE_DIR}/green_src/router.rs" 0 "" \
    "MCP_SRC_DIR=${FIXTURE_DIR}/green_src"

# --- (14) UPDATE_BASELINE=1 on a failing fixture: refused, cannot launder ---
run_test "UPDATE_BASELINE on failing fixture: refused" "$NO_AUTH" 1 \
    "UPDATE_BASELINE refused" "$(stage_fixture_src "$NO_AUTH") UPDATE_BASELINE=1"

# --- (15) UPDATE_BASELINE=1 on the clean tree: prints the baseline, exits 0 ---
run_test "UPDATE_BASELINE on clean tree: prints and exits 0" "$REAL_ROUTER" 0 \
    "copy into BASELINE" "UPDATE_BASELINE=1"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
