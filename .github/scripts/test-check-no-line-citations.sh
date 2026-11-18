#!/usr/bin/env bash
# .github/scripts/test-check-no-line-citations.sh
#
# Test harness for check-no-line-citations.sh. Runs the checker against the real tree and against
# synthetic fixture trees (SCAN_ROOT), asserting the exit code and, where it matters, the message.
#   bash .github/scripts/test-check-no-line-citations.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK_SCRIPT="${SCRIPT_DIR}/check-no-line-citations.sh"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# tree NAME — a fresh fixture root with crates/ and an empty ratchet baseline; prints its path.
tree() {
    local root="${FIXTURE_DIR}/$1"
    mkdir -p "${root}/crates/demo/src" "${root}/crates/demo/tests" "${root}/.github/scripts"
    printf '# baseline\n' > "${root}/.github/scripts/line-citation-ratchet.tsv"
    printf '%s' "$root"
}

# put ROOT RELPATH — write stdin to ROOT/RELPATH.
put() {
    mkdir -p "$(dirname "$1/$2")"
    cat > "$1/$2"
}

# run_test NAME ROOT EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_test() {
    local test_name="$1" root="$2" expected_exit="$3" expected_text="${4:-}"
    local output actual_exit
    set +e
    if [ -n "$root" ]; then
        output="$(SCAN_ROOT="$root" bash "$CHECK_SCRIPT" 2>&1)"
    else
        output="$(bash "$CHECK_SCRIPT" 2>&1)"
    fi
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "  FAIL: ${test_name}"
        echo "    expected exit=${expected_exit} actual exit=${actual_exit}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
    elif [ -n "$expected_text" ] && ! printf '%s' "$output" | grep -qF -- "$expected_text"; then
        echo "  FAIL: ${test_name}"
        echo "    expected output to contain: ${expected_text}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
    else
        echo "  PASS: ${test_name}"
        PASS=$((PASS + 1))
    fi
}

echo "Running check-no-line-citations.sh tests..."
echo ""

# --- the real tree passes. Runs with NO SCAN_ROOT: the default is what CI scans. ---
run_test "real tree: passes" "" 0

# --- a symbol citation in src passes ---
R="$(tree symbol)"
put "$R" crates/demo/src/lib.rs <<'EOF'
/// Resolved server-side (`temper_services::authz::audit_gate::AuditAuthority`).
fn f() {}
EOF
run_test "symbol citation in src: passes" "$R" 0

# --- one of the fourteen citations from the 2026-08-26 review, re-added in src: fails ---
R="$(tree doc_comment)"
put "$R" crates/demo/src/lib.rs <<'EOF'
/// The subject is resolved from `block_id` (`temper-services/src/authz/audit_gate.rs:65-77`).
fn f() {}
EOF
run_test "/// citation in src: fails, naming the site" "$R" 1 "crates/demo/src/lib.rs:1:"

# --- //! module docs and trailing // comments count too ---
R="$(tree module_doc)"
put "$R" crates/demo/src/lib.rs <<'EOF'
//! Checked at `mod.rs:54`.
EOF
run_test "//! citation in src: fails" "$R" 1

R="$(tree trailing)"
put "$R" crates/demo/src/lib.rs <<'EOF'
fn f() -> u8 { 1 } // mirrors `other.rs:12`
EOF
run_test "trailing // citation in src: fails" "$R" 1

# --- a nested module file is src too ---
R="$(tree nested)"
put "$R" crates/demo/src/a/b/c.rs <<'EOF'
// see `writes.rs:538-541`
EOF
run_test "citation in a nested src module: fails" "$R" 1

# --- an inline #[cfg(test)] module and a tests.rs sibling inside src are src: strict ---
R="$(tree cfg_test)"
put "$R" crates/demo/src/lib.rs <<'EOF'
fn f() {}
#[cfg(test)]
mod tests {
    // pattern from `write_path_mutations.rs:19`
    #[test]
    fn t() {}
}
EOF
run_test "citation inside an inline #[cfg(test)] module in src: fails" "$R" 1

R="$(tree tests_sibling)"
put "$R" crates/demo/src/foo/tests.rs <<'EOF'
// `registry.rs:392`
EOF
run_test "citation in a src/**/tests.rs sibling: fails" "$R" 1

# --- not a comment: a string literal is not scanned (declared NOT CHECKED) ---
R="$(tree literal)"
put "$R" crates/demo/src/lib.rs <<'EOF'
fn f() -> &'static str { "foo.rs:12" }
EOF
run_test "a string literal carrying .rs:NN: passes" "$R" 0

# --- a migration cited by file name passes ---
R="$(tree migration)"
put "$R" crates/demo/src/lib.rs <<'EOF'
// See migration `20260826000110_context_retire.sql` and `.rs` files generally.
EOF
run_test "migration file name and bare .rs: passes" "$R" 0

# --- RATCHET: a citation in tests/ that the baseline pins passes ---
R="$(tree ratchet_ok)"
put "$R" crates/demo/tests/a.rs <<'EOF'
// mirrors `content_mutation.rs:290-344`
EOF
printf 'crates/demo/tests/a.rs\t1\n' >> "$R/.github/scripts/line-citation-ratchet.tsv"
run_test "pinned tests/ citation: passes" "$R" 0

# --- RATCHET: a new citation in an unlisted test file fails ---
R="$(tree ratchet_new_file)"
put "$R" crates/demo/tests/a.rs <<'EOF'
// mirrors `content_mutation.rs:290-344`
EOF
run_test "new citation in an unlisted tests/ file: fails" "$R" 1 "grew past the ratchet"

# --- RATCHET: a listed file whose count grows fails ---
R="$(tree ratchet_grow)"
put "$R" crates/demo/tests/a.rs <<'EOF'
// mirrors `content_mutation.rs:290-344`
// and `writes.rs:297`
EOF
printf 'crates/demo/tests/a.rs\t1\n' >> "$R/.github/scripts/line-citation-ratchet.tsv"
run_test "a tests/ file's count growing: fails" "$R" 1 "crates/demo/tests/a.rs: 2 citation(s), baseline 1"

# --- RATCHET: a count that fell must be recorded ---
R="$(tree ratchet_fell)"
put "$R" crates/demo/tests/a.rs <<'EOF'
// mirrors `content_mutation::create_then_revise`
EOF
printf 'crates/demo/tests/a.rs\t1\n' >> "$R/.github/scripts/line-citation-ratchet.tsv"
run_test "a tests/ count that fell without lowering the baseline: fails" "$R" 1 "fell below"

# --- RATCHET: examples/ and build.rs are ratchet, not strict ---
R="$(tree examples)"
put "$R" crates/demo/examples/x.rs <<'EOF'
//! see `search.rs:12`
EOF
printf 'crates/demo/examples/x.rs\t1\n' >> "$R/.github/scripts/line-citation-ratchet.tsv"
run_test "pinned examples/ citation: passes" "$R" 0

# --- the repo-level tests/ crate: its src/ is strict, its tests/ ratchet ---
R="$(tree e2e_src)"
put "$R" tests/e2e/src/lib.rs <<'EOF2'
// spawns the server as `main.rs:40` does
EOF2
run_test "citation in tests/<crate>/src: fails" "$R" 1 "tests/e2e/src/lib.rs:1:"

R="$(tree e2e_tests)"
put "$R" tests/e2e/tests/a_test.rs <<'EOF2'
// see `access_service.rs:866`
EOF2
run_test "new citation in an unlisted tests/e2e/tests file: fails" "$R" 1 "tests/e2e/tests/a_test.rs: 1 citation(s), baseline 0"

# --- UPDATE_BASELINE=1 writes a baseline the next run accepts ---
R="$(tree update)"
put "$R" crates/demo/tests/a.rs <<'EOF'
// `a.rs:1` and `b.rs:2`
// `c.rs:3`
EOF
SCAN_ROOT="$R" UPDATE_BASELINE=1 bash "$CHECK_SCRIPT" > /dev/null 2>&1 || true
run_test "UPDATE_BASELINE=1 then check: passes" "$R" 0
if grep -qF "$(printf 'crates/demo/tests/a.rs\t2')" "$R/.github/scripts/line-citation-ratchet.tsv"; then
    echo "  PASS: UPDATE_BASELINE=1 counts lines, not matches"
    PASS=$((PASS + 1))
else
    echo "  FAIL: UPDATE_BASELINE=1 counts lines, not matches"
    FAIL=$((FAIL + 1))
fi

# --- TW-3: a tree with no Rust fails rather than passing on nothing ---
R="${FIXTURE_DIR}/empty"
mkdir -p "$R/crates" "$R/.github/scripts"
printf '# baseline\n' > "$R/.github/scripts/line-citation-ratchet.tsv"
run_test "no .rs files at all: fails" "$R" 1 "matched nothing"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
