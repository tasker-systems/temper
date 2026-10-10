#!/usr/bin/env bash
# .github/scripts/test-audit-route-auth.sh
#
# Test harness for audit-route-auth.sh's (b) TABLE/WIRING assertions. Runs the auditor against the
# real routes module and against fixture copies DERIVED from it by one targeted mutation each,
# asserting the auditor fails and names the reason.
#
# WHY A HARNESS RATHER THAN A COMMENT
# -----------------------------------
# The wiring assertion must be able to fail. It used to be a whole-file `grep -q` per layer name;
# every signature gate was mounted TWICE (create_app AND create_internal_app), so deleting one
# mount left the name present and the auditor GREEN while one deployed surface served the group
# ungated. The route table moved the surface again: both builders now consume ONE table through
# ONE apply_tier, so the possible regressions are a middleware lost from a TIER STACK, a row's
# TIER quietly changed, or a builder STOPPING CONSUMING the table. The fixtures below perform
# exactly those mutations on copies of the module and assert the auditor CAN fail, with the right
# message, on every CI run.
#
# Fixtures are derived from the live module rather than hand-written, so they cannot rot into
# testing a shape the code no longer has.
#
#   bash .github/scripts/test-audit-route-auth.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-route-auth.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_ROUTES="${REPO_ROOT}/crates/temper-api/src/routes"
REAL_API_SRC="${REPO_ROOT}/crates/temper-api/src"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# run_test NAME ROUTES_PATH EXPECTED_EXIT [EXPECTED_SUBSTRING]
#
# A fixture never matches the reviewed route BASELINE, so exit code alone cannot distinguish "the
# wiring assertion bit" from "the baseline diff tripped". EXPECTED_SUBSTRING pins the actual reason.
run_test() {
    local test_name="$1"
    local routes_path="$2"
    local expected_exit="$3"
    local expected_substr="${4:-}"

    local output actual_exit
    set +e
    # FIX_HANDLERS_DIR / FIX_API_SRC_DIR point the handler checks (e)/(f) and the source check (g)
    # at fixture copies; unset, the auditor reads the live tree.
    output="$(ROUTES_FILE="$routes_path" \
        HANDLERS_DIR="${FIX_HANDLERS_DIR:-${REPO_ROOT}/crates/temper-api/src/handlers}" \
        API_SRC_DIR="${FIX_API_SRC_DIR:-${REPO_ROOT}/crates/temper-api/src}" \
        bash "$AUDIT_SCRIPT" 2>&1)"
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

# copy_module OUTDIR — a fresh copy of the live routes module to mutate.
copy_module() {
    cp -R "$REAL_ROUTES" "$1"
}

echo "Running audit-route-auth.sh table/wiring tests..."
echo ""

# --- (a) the real routes module passes: every row present with its tier, every stack complete ---
run_test "real routes module: passes" "$REAL_ROUTES" 0

# --- (b) a middleware dropped from a tier stack must fail, naming the middleware ---
# The gated tier losing require_auth: every gated group on BOTH builders would authenticate nobody.
FIX="${FIXTURE_DIR}/no_require_auth"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
lines = [l for l in open(p).readlines() if "auth::require_auth" not in l]
open(p, "w").writelines(lines)
PYEOF
run_test "require_auth dropped from apply_tier: fails" "$FIX" 1 \
    "'auth::require_auth' not applied by apply_tier"

# The reconcile signature lost: internal_routes serves its HMAC group ungated.
FIX="${FIXTURE_DIR}/no_internal_signature"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
lines = [l for l in open(p).readlines() if "require_internal_signature" not in l]
open(p, "w").writelines(lines)
PYEOF
run_test "require_internal_signature dropped from apply_tier: fails" "$FIX" 1 \
    "'require_internal_signature' not applied by apply_tier"

# --- (c) a row's tier quietly changed must fail, naming the group ---
# internal_routes flipped to SelfGated is exactly the "serve it ungated" edit the old
# per-builder grep could not see either; the row pin is what makes it visible.
FIX="${FIXTURE_DIR}/tier_flip"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace(
    'key: "internal_routes", tier: Tier::InternalHmac(SignatureKind::Reconcile)',
    'key: "internal_routes", tier: Tier::SelfGated')
open(p, "w").write(s)
PYEOF
run_test "internal_routes tier flipped to SelfGated: fails" "$FIX" 1 \
    "table row changed"

# --- (d) a builder that stops consuming the table must fail ---
# The internal function assembling its own router is a parallel wiring path the row pins
# cannot see; the mount-from-table assertion is what catches it.
FIX="${FIXTURE_DIR}/internal_app_off_table"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
marker = "pub fn create_internal_app"
head, tail = s.split(marker, 1)
body, rest = tail.split("\n}", 1)
body = body.replace("app = app.merge(mount_group(&group, &state));", "// removed")
open(p, "w").write(head + marker + body + "\n}" + rest)
PYEOF
run_test "create_internal_app not mounting from the table: fails" "$FIX" 1 \
    "does not mount from the route table"

# --- (e) a renamed/removed app builder is caught rather than silently skipped ---
FIX="${FIXTURE_DIR}/renamed_builder"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace("pub fn create_internal_app(", "pub fn create_system_app(", 1)
open(p, "w").write(s)
PYEOF
run_test "create_internal_app renamed: fails loudly" "$FIX" 1 \
    "does not mount from the route table"

# --- (f) a row's build fn pointed at another group's fn must fail ---
# The one-token edit the table made easy: swap the gated row's build fn for public's and every
# (key, tier) pin stays textually true while the gated API mounts with no middleware at all.
FIX="${FIXTURE_DIR}/build_fn_swap"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace(
    'Group { key: "gated_routes", tier: Tier::Gated, build: Documented(gated_routes),',
    'Group { key: "gated_routes", tier: Tier::Gated, build: Documented(public_routes),')
open(p, "w").write(s)
PYEOF
run_test "gated row's build fn swapped to public_routes: fails" "$FIX" 1 \
    "table row changed"

# --- (g) a row's serves column flipped must fail ---
# webhook_intake (the broker-attestation door) gaining BothBuilders puts it on the second
# deployed Vercel function with zero signal; the reverse flip un-serves a signature group.
FIX="${FIXTURE_DIR}/serves_flip"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace(
    'key: "webhook_intake_routes", tier: Tier::SelfGated, build: Undocumented(webhook_intake_routes), body_limit: None, serves: Serves::AppOnly',
    'key: "webhook_intake_routes", tier: Tier::SelfGated, build: Undocumented(webhook_intake_routes), body_limit: None, serves: Serves::BothBuilders')
open(p, "w").write(s)
PYEOF
run_test "webhook row's serves flipped to BothBuilders: fails" "$FIX" 1 \
    "table row changed"

# --- (g2) a scoped handler mounted in the admin group must fail the membership check ---
# The admin ledger gates per act family and lets an actor read their own acts, so it is NOT
# admin-only. Mounted in admin.rs it would be published under the `Admin` tag as admin-only.
FIX="${FIXTURE_DIR}/scoped_in_admin"
copy_module "$FIX"
python3 - "$FIX/admin.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
marker = "        .routes(routes!(handlers::machine_clients::rebind))"
assert marker in s, "admin.rs marker missing"
s = s.replace(marker, marker + "\n        .routes(routes!(handlers::admin_ledger::list))", 1)
open(p, "w").write(s)
PYEOF
run_test "scoped handler mounted in admin_routes: fails" "$FIX" 1 \
    "does not mint the &SystemAdmin proof"

# --- (h) a middleware pair swapped inside a tier arm must fail the order pin ---
# Presence-greps stay green through a swap; the order pin is what catches it.
FIX="${FIXTURE_DIR}/order_swap"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
gated_arm = s.split("Tier::Gated => router", 1)[1].split("Tier::InternalHmac", 1)[0]
swapped = gated_arm.replace(
    "system_access::require_system_access,\n            ))\n            .layer(from_fn_with_state(state.clone(), auth::require_auth))",
    "auth::require_auth,\n            ))\n            .layer(from_fn_with_state(state.clone(), system_access::require_system_access))", 1)
assert swapped != gated_arm, "swap did not apply"
s = s.replace(gated_arm, swapped, 1)
open(p, "w").write(s)
PYEOF
run_test "Gated arm's auth/system-access pair swapped: fails" "$FIX" 1 \
    "order pin"

# --- (i) the order pins are bounded by arm: a name missing from its OWN arm fails even when a later
#     arm still has it ---
# The Gated arm losing require_system_access while HumanGated keeps it: an unbounded sequential
# search would find HumanGated's occurrence and pass over a gated tier that admits the unapproved.
FIX="${FIXTURE_DIR}/gated_arm_no_system_access"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
head, rest = s.split("Tier::Gated => router", 1)
arm, tail = rest.split("Tier::HumanGated =>", 1)
cut = """            .layer(from_fn_with_state(
                state.clone(),
                system_access::require_system_access,
            ))
"""
assert cut in arm, "Gated arm's system-access layer not found"
s = head + "Tier::Gated => router" + arm.replace(cut, "", 1) + "Tier::HumanGated =>" + tail
open(p, "w").write(s)
PYEOF
run_test "Gated arm missing require_system_access (HumanGated still has it): fails" "$FIX" 1 \
    "'system_access::require_system_access' not found in apply_tier's 'Tier::Gated =>' arm"

# The admin tier losing its machine refusal while HumanAuthOnly keeps one.
FIX="${FIXTURE_DIR}/human_gated_no_refusal"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
head, rest = s.split("Tier::HumanGated => router", 1)
arm, tail = rest.split("Tier::InternalHmac", 1)
cut = "            .layer(from_fn(auth::refuse_machine))\n"
assert cut in arm, "HumanGated arm's refusal not found"
s = head + "Tier::HumanGated => router" + arm.replace(cut, "", 1) + "Tier::InternalHmac" + tail
open(p, "w").write(s)
PYEOF
run_test "HumanGated arm missing refuse_machine (HumanAuthOnly still has it): fails" "$FIX" 1 \
    "'auth::refuse_machine' not found in apply_tier's 'Tier::HumanGated =>' arm"

# The refusal moved INNER to require_system_access — composed from the Gated arm instead of spelled
# out: an unapproved machine would be told it lacks system access, not that it is a machine.
FIX="${FIXTURE_DIR}/human_gated_refusal_innermost"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
head, rest = s.split("Tier::HumanGated => router", 1)
arm, tail = rest.split("Tier::InternalHmac", 1)
refusal = "            .layer(from_fn(auth::refuse_machine))\n"
assert refusal in arm
arm = arm.replace(refusal, "", 1)
arm = "\n" + refusal + arm.lstrip("\n")
s = head + "Tier::HumanGated => router" + arm + "Tier::InternalHmac" + tail
open(p, "w").write(s)
PYEOF
run_test "HumanGated refusal moved inner to require_system_access: fails" "$FIX" 1 \
    "'auth::refuse_machine' not found in apply_tier's 'Tier::HumanGated =>' arm (after offset"

# --- (j) a human-only row quietly moved to a machine-admitting tier fails ---
FIX="${FIXTURE_DIR}/admin_tier_admits_machines"
copy_module "$FIX"
python3 - "$FIX/mod.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
a = 'key: "admin_routes", tier: Tier::HumanGated,'
assert a in s
s = s.replace(a, 'key: "admin_routes", tier: Tier::Gated,', 1)
open(p, "w").write(s)
PYEOF
run_test "admin_routes tier relaxed to Gated: fails" "$FIX" 1 \
    "table row changed"

# --- (k) the machine-admitting status group gaining a handler fails ---
FIX="${FIXTURE_DIR}/status_group_grows"
copy_module "$FIX"
python3 - "$FIX/auth_only.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
a = "OpenApiRouter::new().routes(routes!(handlers::profiles::get))"
assert a in s
s = s.replace(a, a + "\n        .routes(routes!(handlers::profiles::update))", 1)
open(p, "w").write(s)
PYEOF
run_test "a handler added to auth_only_status_routes: fails" "$FIX" 1 \
    "the auth_only_status group's handler set changed"

# --- (l) a handler that names no identity extractor fails (e) ---
FIX_SRC="${FIXTURE_DIR}/src_no_extractor"
cp -R "$REAL_API_SRC" "$FIX_SRC"
python3 - "$FIX_SRC/handlers/access.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
a = "    State(state): State<AppState>,\n    _auth: AuthUser,\n) -> ApiResult<Json<PublicSystemSettings>> {"
assert a in s, "get_settings signature not found"
s = s.replace(a, "    State(state): State<AppState>,\n) -> ApiResult<Json<PublicSystemSettings>> {", 1)
open(p, "w").write(s)
PYEOF
FIX_HANDLERS_DIR="$FIX_SRC/handlers" FIX_API_SRC_DIR="$FIX_SRC"
run_test "an auth-covered handler with no extractor: fails" "$REAL_ROUTES" 1 \
    "names no identity extractor"
unset FIX_HANDLERS_DIR FIX_API_SRC_DIR

# --- (m) a new machine-admitting (AnyPrincipal) handler fails (f) ---
FIX_SRC="${FIXTURE_DIR}/src_new_any_principal"
cp -R "$REAL_API_SRC" "$FIX_SRC"
python3 - "$FIX_SRC/handlers/access.rs" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
a = "    _auth: AuthUser,\n) -> ApiResult<Json<PublicSystemSettings>> {"
assert a in s, "get_settings signature not found"
s = s.replace(a, "    _auth: AnyPrincipal,\n) -> ApiResult<Json<PublicSystemSettings>> {", 1)
open(p, "w").write(s)
PYEOF
FIX_HANDLERS_DIR="$FIX_SRC/handlers" FIX_API_SRC_DIR="$FIX_SRC"
run_test "a handler newly taking AnyPrincipal: fails" "$REAL_ROUTES" 1 \
    "the set of machine-admitting (AnyPrincipal) handlers changed"
unset FIX_HANDLERS_DIR FIX_API_SRC_DIR

# --- (n) a raw caller proof read from extensions outside middleware/ fails (g) ---
# Two spellings: the field form on one line, and a method form split across lines with a
# path-qualified type — the second is what a line grep for a bare type name misses.
for spelling in field split; do
  FIX_SRC="${FIXTURE_DIR}/src_raw_proof_${spelling}"
  cp -R "$REAL_API_SRC" "$FIX_SRC"
  python3 - "$FIX_SRC/handlers/health.rs" "$spelling" <<'PYEOF'
import sys
p, spelling = sys.argv[1], sys.argv[2]
s = open(p).read()
if spelling == "field":
    s += "\nfn peek(parts: &axum::http::request::Parts) -> bool { parts.extensions.get::<Caller>().is_some() }\n"
else:
    s += "\nfn peek(req: &axum::http::Request<axum::body::Body>) -> bool {\n    req.extensions()\n        .get::<temper_services::auth::Caller>()\n        .is_some()\n}\n"
open(p, "w").write(s)
PYEOF
  FIX_HANDLERS_DIR="$FIX_SRC/handlers" FIX_API_SRC_DIR="$FIX_SRC"
  run_test "a raw Caller read from extensions outside middleware (${spelling}): fails" "$REAL_ROUTES" 1 \
      "a raw caller proof read from request extensions outside middleware/"
  unset FIX_HANDLERS_DIR FIX_API_SRC_DIR
done

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
