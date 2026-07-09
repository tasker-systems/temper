#!/usr/bin/env bash
# .github/scripts/test-check-openapi-routes.sh
#
# Test harness for check-openapi-routes.sh. Runs the checker against the real
# routes.rs and against synthetic fixtures, asserting the exit code.
#   bash .github/scripts/test-check-openapi-routes.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK_SCRIPT="${SCRIPT_DIR}/check-openapi-routes.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_ROUTES="${REPO_ROOT}/crates/temper-api/src/routes.rs"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# run_test NAME ROUTES_FILE EXPECTED_EXIT
run_test() {
    local test_name="$1"
    local routes_file="$2"
    local expected_exit="$3"

    local output actual_exit
    set +e
    output="$(bash "$CHECK_SCRIPT" "$routes_file" 2>&1)"
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

echo "Running check-openapi-routes.sh tests..."
echo ""

# --- (a) the real routes.rs passes (only allowlisted plain .route() mounts) ---
run_test "real routes.rs: passes" "$REAL_ROUTES" 0

# --- (b) an off-allowlist plain .route() fails ---
OFF_ALLOWLIST="${FIXTURE_DIR}/off_allowlist.rs"
cat > "$OFF_ALLOWLIST" <<'EOF'
fn gated_routes() -> OpenApiRouter<AppState> {
    use axum::routing::{get, post};

    OpenApiRouter::new()
        .routes(routes!(handlers::resources::list))
        // Allowlisted operator surface — fine.
        .route("/api/access/admin/promote", post(handlers::access::promote_admin))
        // Undocumented public route — MUST fail the gate.
        .route("/api/secret", get(handlers::secret::leak))
}
EOF
run_test "off-allowlist plain .route(): fails" "$OFF_ALLOWLIST" 1

# --- (c) a router using only .routes(routes!(…)) passes (no plain .route()) ---
ROUTES_ONLY="${FIXTURE_DIR}/routes_only.rs"
cat > "$ROUTES_ONLY" <<'EOF'
fn gated_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::resources::list, handlers::resources::create))
        .routes(routes!(handlers::teams::list))
        .routes(routes!(handlers::ingest::create))
}
EOF
run_test "only .routes(routes!()): passes" "$ROUTES_ONLY" 0

# --- multiline plain .route() with the path literal on the next line ---
MULTILINE="${FIXTURE_DIR}/multiline.rs"
cat > "$MULTILINE" <<'EOF'
fn gated_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .route(
            "/api/secret",
            get(handlers::secret::leak),
        )
}
EOF
run_test "off-allowlist multiline plain .route(): fails" "$MULTILINE" 1

# --- every allowlisted path, each on its own plain .route(), passes ---
ALL_ALLOWED="${FIXTURE_DIR}/all_allowed.rs"
cat > "$ALL_ALLOWED" <<'EOF'
fn gated_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .route("/api/access/admin/requests", get(a))
        .route("/api/access/admin/requests/{id}", patch(b))
        .route("/api/access/admin/settings", get(c).patch(d))
        .route("/api/access/admin/promote", post(e))
        .route("/internal/saml/reconcile", post(f))
        .route("/api/embed/dispatch", get(g).post(g))
}
EOF
run_test "all allowlisted plain .route()s: passes" "$ALL_ALLOWED" 0

# --- a fixture with no .route( at all passes (nothing to check) ---
EMPTY="${FIXTURE_DIR}/empty.rs"
cat > "$EMPTY" <<'EOF'
fn create_app(state: AppState) -> Router {
    Router::new().with_state(state)
}
EOF
run_test "no plain .route() at all: passes" "$EMPTY" 0

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
