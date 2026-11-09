#!/usr/bin/env bash
# .github/scripts/test-audit-handler-authz-drift.sh
#
# Test harness for audit-handler-authz-drift.sh — the guard that pins authorization predicates
# called from a surface (temper-api handlers, the temper-mcp tool layer) instead of a service.
#
# Its baseline is EMPTY, so "green" is only evidence if the guard can see anything at all. Each red
# case below violates exactly one of the guard's conditions on a fixture that satisfies the others:
#
#   (a) a handler calling a listed predicate directly reds, naming the file
#   (b) an MCP tool calling a listed predicate directly reds, naming the file
#   (c) a missing handlers scan root reds (it does not read as "no calls")
#   (d) a missing MCP scan root reds
#   (e) a listed predicate whose definition is gone reds, naming it
#   (f) a listed predicate narrowed to pub(crate) reds — no surface can call it any more
#   (g) GREEN: a `//` comment naming a predicate is not a call
#
# Fixtures are COPIES of the trees the auditor reads, with exactly one edit.
#
#   bash .github/scripts/test-audit-handler-authz-drift.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT="${SCRIPT_DIR}/audit-handler-authz-drift.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ACCESS_SERVICE='crates/temper-services/src/services/access_service.rs'

make_fixture() {
    local dest="$1"
    mkdir -p "$dest"/crates/temper-api/src "$dest"/crates/temper-services/src/services
    cp -R "$REPO_ROOT"/crates/temper-api/src/handlers "$dest"/crates/temper-api/src/
    cp -R "$REPO_ROOT"/crates/temper-mcp "$dest"/crates/
    cp "$REPO_ROOT/$ACCESS_SERVICE" "$dest/$ACCESS_SERVICE"
}

# run_case NAME ROOT EXPECTED_EXIT [EXPECTED_SUBSTRING…]
#
# Every red case exits 1, so each pins its reason with a substring: the case asserts the guard
# failed FOR THE REASON THE FIXTURE CONSTRUCTED, not merely that it failed.
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

# A fixture edit that silently does not apply turns a red case into a green control. Assert it.
applied() {
    local label="$1"; shift
    if ! "$@"; then
        echo "  FAIL  $label — fixture edit did not apply"
        FAIL=$((FAIL + 1))
    fi
}

echo "test-audit-handler-authz-drift"

# ── the control: the real tree is green ──────────────────────────────────────────────────────
run_case "real tree passes" "$REPO_ROOT" 0 "audit-handler-authz-drift: OK"

# ── the control on a fixture: the copy alone is green, so every red below is its one edit ──────
F0="$WORK/f0"; make_fixture "$F0"
run_case "unedited fixture passes" "$F0" 0 "audit-handler-authz-drift: OK"

# ── (a) the F-3 shape: a handler gates on a predicate the service should own ──────────────────
FA="$WORK/fa"; make_fixture "$FA"
cat > "$FA/crates/temper-api/src/handlers/widget.rs" <<'RS'
pub async fn promote_widget(state: AppState, auth: AuthContext) -> ApiResult<()> {
    if !access_service::is_system_admin(&state.pool, auth.profile_id).await? {
        return Err(ApiError::Forbidden);
    }
    widget_service::promote(&state.pool).await
}
RS
run_case "(a) a handler calling is_system_admin reds" \
    "$FA" 1 "FAIL — handler-side authorization calls changed" "handlers/widget.rs"

# ── (b) the same shape in the MCP tool layer, with the other predicate ────────────────────────
FB="$WORK/fb"; make_fixture "$FB"
cat > "$FB/crates/temper-mcp/src/tools/widget.rs" <<'RS'
async fn widget(pool: &PgPool, profile_id: ProfileId) -> Result<bool> {
    Ok(access_service::has_system_access(pool, profile_id).await?)
}
RS
run_case "(b) an MCP tool calling has_system_access reds" \
    "$FB" 1 "FAIL — handler-side authorization calls changed" "temper-mcp/src/tools/widget.rs"

# ── (c)+(d) a vanished scan root is a broken scan, never an empty one ─────────────────────────
FC="$WORK/fc"; make_fixture "$FC"
rm -rf "$FC/crates/temper-api/src/handlers"
applied "(c)" test ! -e "$FC/crates/temper-api/src/handlers"
run_case "(c) a missing handlers root reds" \
    "$FC" 1 "scan root 'crates/temper-api/src/handlers' does not exist"

FD="$WORK/fd"; make_fixture "$FD"
rm -rf "$FD/crates/temper-mcp/src"
applied "(d)" test ! -e "$FD/crates/temper-mcp/src"
run_case "(d) a missing MCP root reds" \
    "$FD" 1 "scan root 'crates/temper-mcp/src' does not exist"

# ── (e) a listed predicate that no longer exists is a needle that can never match ─────────────
FE="$WORK/fe"; make_fixture "$FE"
perl -pi -e 's/\bpub async fn has_system_access\b/pub async fn has_platform_access/' \
    "$FE/$ACCESS_SERVICE"
applied "(e)" grep -q 'fn has_platform_access' "$FE/$ACCESS_SERVICE"
run_case "(e) a renamed predicate reds, naming it" \
    "$FE" 1 "predicate 'has_system_access' has no pub fn definition"

# ── (f) narrowing a predicate to pub(crate) makes it unreachable from every surface ───────────
FF="$WORK/ff"; make_fixture "$FF"
perl -pi -e 's/^pub async fn is_system_admin\b/pub(crate) async fn is_system_admin/' \
    "$FF/$ACCESS_SERVICE"
applied "(f)" grep -q '^pub(crate) async fn is_system_admin' "$FF/$ACCESS_SERVICE"
run_case "(f) a predicate narrowed to pub(crate) reds, naming it" \
    "$FF" 1 "predicate 'is_system_admin' has no pub fn definition"

# ── (g) prose is not a call: the guard must not red on a comment ──────────────────────────────
FG="$WORK/fg"; make_fixture "$FG"
printf '\n// The service checks `is_system_admin` itself; this handler does not.\n' \
    >> "$FG/crates/temper-api/src/handlers/access.rs"
run_case "(g) a // comment naming a predicate stays green" \
    "$FG" 0 "audit-handler-authz-drift: OK"

echo
echo "  $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
