#!/usr/bin/env bash
# .github/scripts/test-audit-shape-schema-compile.sh
#
# Test harness for audit-shape-schema-compile.sh. The claims under test, in the order that matters:
#   1. THE BITE: a `jsonschema::validator_for` added under crates/*/src makes the guard FAIL and
#      names the file. This is the whole point — a new default-options compile is the exposure.
#   2. Every entry-point spelling the guard claims to see is seen (options, validate, async,
#      single-line imports), so the bite is not specific to one spelling.
#   3. The guard stays GREEN on what it says it ignores: doc/line comments, tests/ directories, and
#      the real tree, so prose churn never teaches anyone to re-baseline reflexively.
#   4. An empty scan fails rather than passing vacuously.
#
#   bash .github/scripts/test-audit-shape-schema-compile.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-shape-schema-compile.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
# The in-repo probe crate for claim 1 — always removed, including on failure.
PROBE_CRATE="${REPO_ROOT}/crates/zz-shape-schema-compile-probe"
trap 'rm -rf "$FIXTURE_DIR" "$PROBE_CRATE"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# new_src NAME FILE BODY — write BODY to a fixture crate's src/FILE and echo the scan root.
new_src() {
    local root="${FIXTURE_DIR}/$1/crates/demo/src"
    mkdir -p "$root"
    printf '%s\n' "$3" > "${root}/$2"
    echo "${FIXTURE_DIR}/$1/crates"
}

# expect_sites NAME SCAN_ROOT EXPECTED_FILE_COUNT — the detector's own output, via --list.
# Fixtures are asserted through --list, never the exit code: a fixture contains none of the real
# allowlist, so the diff always fires, and an exit-code assertion would pass whether or not the
# detector saw anything.
expect_sites() {
    local name="$1" root="$2" expected="$3" actual
    actual="$(SCAN_ROOT="$root" bash "$AUDIT_SCRIPT" --list 2>&1 | grep -c . || true)"
    if [ "$actual" = "$expected" ]; then ok "$name"
    else bad "$name" "expected ${expected} site file(s), saw ${actual}" "$(SCAN_ROOT="$root" bash "$AUDIT_SCRIPT" --list 2>&1)"; fi
}

echo "Running audit-shape-schema-compile.sh tests..."
echo ""

# --- the real tree matches the reviewed allowlist ---
set +e
OUT="$(bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 0 ]; then ok "real repo: the allowlist matches"
else bad "real repo: the allowlist matches" "exit=${RC}" "output: ${OUT}"; fi

# --- 1. THE BITE: validator_for under crates/*/src fails the guard and names the file ---
mkdir -p "${PROBE_CRATE}/src"
# A valid manifest, so a concurrent cargo or rust-analyzer workspace load (members are
# `crates/*`) does not fail on a member without one while the probe exists.
cat > "${PROBE_CRATE}/Cargo.toml" <<'TOML'
[package]
name = "zz-shape-schema-compile-probe"
version = "0.0.0"
edition = "2021"
publish = false
TOML
cat > "${PROBE_CRATE}/src/lib.rs" <<'RS'
pub fn compile(schema: &serde_json::Value) {
    let _ = jsonschema::validator_for(schema);
}
RS
set +e
OUT="$(bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "zz-shape-schema-compile-probe/src/lib.rs"; then
    ok "BITE: jsonschema::validator_for under crates/*/src fails the guard, naming the file"
else
    bad "BITE: jsonschema::validator_for under crates/*/src fails the guard, naming the file" "exit=${RC}" "output: ${OUT}"
fi
rm -rf "$PROBE_CRATE"
set +e
bash "$AUDIT_SCRIPT" >/dev/null 2>&1; RC=$?
set -e
if [ "$RC" -eq 0 ]; then ok "  ...and removing the file restores green"
else bad "  ...and removing the file restores green" "exit=${RC}"; fi

# --- 2. every spelling is seen ---
expect_sites "options(): detected" \
    "$(new_src options a.rs 'fn f() { let _ = jsonschema::options().build(&s); }')" 1
expect_sites "validate(): detected" \
    "$(new_src validate a.rs 'fn f() { let _ = jsonschema::validate(&s, &i); }')" 1
expect_sites "async_validator_for(): detected" \
    "$(new_src asyncfn a.rs 'async fn f() { let _ = jsonschema::async_validator_for(&s).await; }')" 1
expect_sites "Validator::new via the full path: detected" \
    "$(new_src vnew a.rs 'fn f() { let _ = jsonschema::Validator::new(&s); }')" 1
expect_sites "single-line import of an entry point: detected" \
    "$(new_src import a.rs 'use jsonschema::{validator_for, Draft};')" 1

expect_sites "evaluate(): detected" \
    "$(new_src evaluate a.rs 'fn f() { let _ = jsonschema::evaluate(&s, &i); }')" 1
expect_sites "bundle(): detected" \
    "$(new_src bundle a.rs 'fn f() { let _ = jsonschema::bundle(&s); }')" 1
expect_sites "async_bundle(): detected" \
    "$(new_src asyncbundle a.rs 'async fn f() { let _ = jsonschema::async_options().async_bundle(&s).await; }')" 1
expect_sites "ValidationOptions::default(): detected" \
    "$(new_src vodefault a.rs 'fn f() { let _ = ValidationOptions::default(); }')" 1
expect_sites "ValidationOptions::<R>::default(): detected" \
    "$(new_src vodefaultgen a.rs 'fn f() { let _ = ValidationOptions::<Arc<dyn Retrieve>>::default(); }')" 1
expect_sites "Registry::try_new: detected" \
    "$(new_src registry a.rs 'fn f() { let _ = Registry::try_new(uri, res); }')" 1
expect_sites "Registry::options: detected" \
    "$(new_src registryopts a.rs 'fn f() { let _ = Registry::options().build(pairs); }')" 1
expect_sites "bare Validator::new (no jsonschema:: prefix): detected" \
    "$(new_src barenew a.rs 'fn f() { let _ = Validator::new(&s); }')" 1
expect_sites "bare Validator::options: detected" \
    "$(new_src bareopts a.rs 'fn f() { let _ = Validator::options().build(&s); }')" 1
expect_sites "use jsonschema as alias: detected" \
    "$(new_src alias a.rs 'use jsonschema as js;')" 1
expect_sites "use jsonschema::* glob: detected" \
    "$(new_src glob a.rs 'use jsonschema::*;')" 1
expect_sites "extern crate jsonschema: detected" \
    "$(new_src externcrate a.rs 'extern crate jsonschema as js;')" 1

expect_sites "Registry::try_from_resources: detected" \
    "$(new_src regfromres a.rs 'fn f() { let _ = Registry::try_from_resources(pairs); }')" 1
expect_sites "RegistryOptions::new: detected" \
    "$(new_src regoptsnew a.rs 'fn f() { let _ = RegistryOptions::new().build(pairs); }')" 1
expect_sites "RegistryOptions::default: detected" \
    "$(new_src regoptsdef a.rs 'fn f() { let _ = RegistryOptions::default(); }')" 1
expect_sites "bare Validator::async_new: detected" \
    "$(new_src asyncnew a.rs 'async fn f() { let _ = Validator::async_new(&s).await; }')" 1
expect_sites "bare Validator::async_options: detected" \
    "$(new_src asyncopts a.rs 'async fn f() { let _ = Validator::async_options().build(&s).await; }')" 1
expect_sites "evaluate inside a single-line use import: detected" \
    "$(new_src useevaluate a.rs 'use jsonschema::{evaluate, Draft};')" 1
expect_sites "jsonschema::async_bundle on its own: detected" \
    "$(new_src asyncbundlealone a.rs 'async fn f() { let _ = jsonschema::async_bundle(&s).await; }')" 1

# --- 3. what the guard says it ignores ---
expect_sites "a whole-line comment naming an entry point: ignored" \
    "$(new_src comment a.rs '// jsonschema::validator_for is forbidden here')" 0
expect_sites "a doc comment naming an entry point: ignored" \
    "$(new_src doc a.rs '/// Never call `jsonschema::options()` directly.')" 0
expect_sites "an import of only the types: ignored" \
    "$(new_src types a.rs 'use jsonschema::{Resource, Validator};')" 0
expect_sites "a Validator type annotation (no constructor call): ignored" \
    "$(new_src typeann a.rs 'fn f(v: &Validator) -> Option<Validator> { None }')" 0
expect_sites "an unrelated type ending in Validator::new: ignored" \
    "$(new_src otherval a.rs 'fn f() { let _ = JwtValidator::new(); }')" 0
TESTS_ONLY="${FIXTURE_DIR}/testsdir/crates/demo"
mkdir -p "${TESTS_ONLY}/tests"
printf '%s\n' 'fn f() { let _ = jsonschema::validator_for(&s); }' > "${TESTS_ONLY}/tests/t.rs"
expect_sites "a tests/ directory: out of the field of view" "${FIXTURE_DIR}/testsdir/crates" 0

# --- 3b. layer 2: every jsonschema dependency edge must say default-features = false ---
# The Rust scan runs on the real tree (green), the manifest scan on a fixture via MANIFEST_ROOT, so
# these assert the manifest half alone: exit code AND the output naming the offending file.
manifest_case() {  # NAME CONTENT EXPECTED_EXIT [EXPECTED_SUBSTRING]
    local name="$1" content="$2" want="$3" substr="${4:-}" root out rc
    root="${FIXTURE_DIR}/manifest-${name// /-}"
    mkdir -p "${root}/crates/demo"
    printf '%s\n' "$content" > "${root}/crates/demo/Cargo.toml"
    set +e
    out="$(MANIFEST_ROOT="$root" bash "$AUDIT_SCRIPT" 2>&1)"; rc=$?
    set -e
    if [ "$rc" -eq "$want" ] && { [ -z "$substr" ] || printf '%s' "$out" | grep -qF -- "$substr"; }; then
        ok "manifest: $name"
    else
        bad "manifest: $name" "expected exit=${want} ${substr:+containing: ${substr}}" "exit=${rc}" "output: ${out}"
    fi
}
manifest_case "BITE: a one-line dependency without default-features = false fails, naming the file" \
'[dependencies]
jsonschema = "0.45"' 1 "crates/demo/Cargo.toml"
manifest_case "BITE: an inline table that leaves the defaults on fails" \
'[dependencies]
jsonschema = { version = "0.45", features = ["arbitrary-precision"] }' 1 "crates/demo/Cargo.toml"
manifest_case "BITE: a [dependencies.jsonschema] table without it fails" \
'[dependencies.jsonschema]
version = "0.45"' 1 "crates/demo/Cargo.toml"
manifest_case "BITE: a [workspace.dependencies] line without it fails" \
'[workspace.dependencies]
jsonschema = "0.45"' 1 "crates/demo/Cargo.toml"
manifest_case "green: a one-line dependency with default-features = false" \
'[dependencies]
jsonschema = { version = "0.45", default-features = false }' 0
manifest_case "green: a table with default-features = false" \
'[dependencies.jsonschema]
version = "0.45"
default-features = false' 0
manifest_case "green: workspace inheritance (the root line is checked on its own)" \
'[dependencies]
jsonschema = { workspace = true }' 0
manifest_case "green: dotted workspace inheritance" \
'[dependencies]
jsonschema.workspace = true' 0

# A fixture with no jsonschema dependency at all fails: the scan would have found the real ones.
NOMANIFEST="${FIXTURE_DIR}/manifest-none"
mkdir -p "${NOMANIFEST}/crates/demo"
printf '%s\n' '[dependencies]' 'serde = "1"' > "${NOMANIFEST}/crates/demo/Cargo.toml"
set +e
OUT="$(MANIFEST_ROOT="$NOMANIFEST" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "found NO jsonschema dependency"; then
    ok "manifest: no jsonschema dependency found fails rather than passing"
else
    bad "manifest: no jsonschema dependency found fails rather than passing" "exit=${RC}" "output: ${OUT}"
fi

# --- 4. an empty scan fails rather than passing ---
EMPTY="${FIXTURE_DIR}/empty/crates"
mkdir -p "$EMPTY"
set +e
OUT="$(SCAN_ROOT="$EMPTY" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "found NO jsonschema entry points"; then
    ok "empty scan: fails rather than passing vacuously"
else
    bad "empty scan: fails rather than passing vacuously" "exit=${RC}" "output: ${OUT}"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
