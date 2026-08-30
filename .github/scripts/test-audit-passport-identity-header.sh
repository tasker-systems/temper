#!/usr/bin/env bash
# .github/scripts/test-audit-passport-identity-header.sh
#
# Guard test for audit-passport-identity-header.sh. Five probes, two of which assert the guard
# stays GREEN — the docs lane (markdown may name the header; that is where the rule lives) and
# the ambient-token distinction (x-vercel-oidc-token is a different, shorter string whose readers
# in the webhook gate are legitimate). A tripwire that reds on prose or on the ambient header
# gets re-baselined without being read, which is the same outcome as no tripwire by a slower
# route.
#
# The red probes assert the OUTPUT names the flagged file, not merely a non-zero exit — a guard
# that fails for the wrong reason (crash, empty file list) still "passes" a bare exit-code check.
#
#   bash .github/scripts/test-audit-passport-identity-header.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-passport-identity-header.sh"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# run NAME SCAN_ROOT EXPECTED_EXIT [EXPECTED_SUBSTRING] — SCAN_ROOT "REAL" means the tracked tree.
run() {
    local name="$1" scan_root="$2" expected_exit="$3" expected_substr="${4:-}"
    local output actual_exit
    set +e
    if [ "$scan_root" = "REAL" ]; then
        output="$(bash "$AUDIT_SCRIPT" 2>&1)"
    else
        output="$(SCAN_ROOT="$scan_root" bash "$AUDIT_SCRIPT" 2>&1)"
    fi
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        bad "$name" "expected exit=${expected_exit} actual exit=${actual_exit}" "output: ${output}"
        return
    fi
    if [ -n "$expected_substr" ] && ! printf '%s' "$output" | grep -qF -- "$expected_substr"; then
        bad "$name" "exit matched but expected message missing: ${expected_substr}" "output: ${output}"
        return
    fi
    ok "$name"
}

echo "Running audit-passport-identity-header.sh tests..."
echo ""

# --- (a) the real tree is clean: the header reaches no code today ---
run "real tracked tree: clean" "REAL" 0 "OK"

# --- (b) Rust code READING the header is caught, and the failure names the file ---
T1="$(mktemp -d)"
cat > "${T1}/middleware.rs" <<'EOF'
// Correlate the edge leg with our own authentication.
let passport = headers.get("x-vercel-oidc-passport-token").cloned();
if let Some(claims) = passport.and_then(decode_passport) {
    return authenticated(claims.sub);
}
EOF
run "rust code reading the header: caught, file named" "$T1" 1 "middleware.rs"

# --- (c) case does not matter: HTTP headers are case-insensitive, so neither is the needle ---
T2="$(mktemp -d)"
cat > "${T2}/hooks.ts" <<'EOF'
export function identify(request: Request): string | null {
  // Passport fronting this origin.
  const jwt = request.headers.get('X-Vercel-OIDC-Passport-Token');
  return jwt ? parseSub(jwt) : null;
}
EOF
run "camel-case spelling in TS: caught" "$T2" 1 "hooks.ts"

# --- (d) markdown may NAME the header — the rule itself does ---
T3="$(mktemp -d)"
cat > "${T3}/rule.md" <<'EOF'
`x-vercel-oidc-passport-token` is never an input to authentication or authorization.
EOF
cat > "${T3}/clean.rs" <<'EOF'
pub fn nothing() {}
EOF
run "markdown naming the header: not flagged" "$T3" 0 "OK"

# --- (e) the AMBIENT x-vercel-oidc-token does not match — its readers are legitimate ---
# Substring direction that matters: the ambient name is shorter; a needle that matched it would
# red on the webhook gate's own attestation read.
T4="$(mktemp -d)"
cat > "${T4}/webhook.rs" <<'EOF'
// The broker attestation rides on every inbound request as the ambient edge token.
let ambient = headers.get("x-vercel-oidc-token").cloned();
assert_eq!(claims.client_id, expected_client_id);
EOF
run "ambient x-vercel-oidc-token: not flagged" "$T4" 0 "OK"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
