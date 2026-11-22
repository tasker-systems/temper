#!/usr/bin/env bash
# .github/scripts/test-audit-erasure-callers.sh
#
# Test harness for audit-erasure-callers.sh — the guard that pins production code calling an
# erasure act or redaction function, or setting the replay setting or marker.
#
# Each red case violates exactly one of the guard's claims on a fixture that satisfies the rest;
# each green case pins a boundary of its field of view:
#
#   (a) RED   a new caller of an erasure act under crates/*/src, naming the file
#   (b) RED   a second call in an already-baselined file (the count is pinned, not the path)
#   (c) RED   a moved caller: the act called from another file than the baselined one
#   (d) RED   each other class, one witness each: a redaction apply, a projector, the replay
#             setting, a marker write, a marker-writer call; and the spellings SQL treats alike
#             (lowercase and schema-qualified, COPY, SET LOCAL, an uppercase call)
#   (e) RED   a caller in api/*.rs, naming the file
#   (f) RED   a caller in an operator-run SQL template, naming the file
#   (g) RED   an empty scan fails as broken, it does not pass
#   (h) RED   a file the scan cannot read fails the scan rather than shrinking it
#   (i) GREEN a call inside a `#[cfg(test)]` mod is not production
#   (j) GREEN a `//` comment line, and a string that names a function without calling it
#   (k) GREEN an integration test under crates/*/tests
#   (l) GREEN a migration (definitions live there)
#
# Fixtures are COPIES of the files the baseline names, so the unedited fixture reproduces the
# reviewed set exactly and every red below is its one edit.
#
#   bash .github/scripts/test-audit-erasure-callers.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT="${SCRIPT_DIR}/audit-erasure-callers.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'chmod -R u+rwX "$WORK" 2>/dev/null; rm -rf "$WORK"' EXIT

FIXTURE_FILES=(
    crates/temper-services/src/services/erasure_service.rs
    crates/temper-services/src/services/resource_erasure_service.rs
    crates/temper-substrate/src/replay.rs
    crates/temper-substrate/src/test_schema.rs
)

make_fixture() {
    local dest="$1" f
    for f in "${FIXTURE_FILES[@]}"; do
        mkdir -p "$dest/$(dirname "$f")"
        cp "$REPO_ROOT/$f" "$dest/$f"
    done
    mkdir -p "$dest/migrations/templates" "$dest/api"
}

# run_case NAME ROOT EXPECTED_EXIT [EXPECTED_SUBSTRING…]
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

CHANGED='FAIL — the set of erasure, redaction or replay-setting callers changed'
OK='audit-erasure-callers: OK'
SVC=crates/temper-services/src/services

echo "test-audit-erasure-callers"

# ── the controls ───────────────────────────────────────────────────────────────────────────────
run_case "real tree passes" "$REPO_ROOT" 0 "$OK"

F0="$WORK/f0"; make_fixture "$F0"
run_case "unedited fixture passes" "$F0" 0 "$OK"

# ── (a) a new caller ───────────────────────────────────────────────────────────────────────────
FA="$WORK/fa"; make_fixture "$FA"
cat > "$FA/$SVC/widget_service.rs" <<'RS'
pub async fn erase(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4, $5)").bind(id).execute(pool).await?;
    Ok(())
}
RS
run_case "(a) a new erasure-act caller reds, naming the file" \
    "$FA" 1 "$CHANGED" "> 1 execute $SVC/widget_service.rs"

# ── (b) one more call in a baselined file ──────────────────────────────────────────────────────
#
# The target must have no test gate: the scan stops at the first one, so a call appended below
# it is never read and the case would stay green whatever the guard counted.
FB="$WORK/fb"; make_fixture "$FB"
TB="$FB/crates/temper-substrate/src/replay.rs"
if grep -qE '^[[:space:]]*#\[cfg\((all\()?test' "$TB"; then
    echo "  FAIL  (b) — fixture target already has a test gate; the appended call would be unread"
    FAIL=$((FAIL + 1))
else
    cat >> "$TB" <<'RS'

async fn walk_again(tx: &mut PgConnection) {
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)").execute(tx).await;
}
RS
    run_case "(b) a second call in a baselined file reds" \
        "$FB" 1 "$CHANGED" "3 replay_setting crates/temper-substrate/src/replay.rs"
fi

# ── (c) a moved caller ─────────────────────────────────────────────────────────────────────────
FC="$WORK/fc"; make_fixture "$FC"
mv "$FC/$SVC/resource_erasure_service.rs" "$FC/$SVC/resource_service.rs"
run_case "(c) a caller that moved files reds, naming both" \
    "$FC" 1 "$CHANGED" "< 1 execute $SVC/resource_erasure_service.rs" \
    "> 1 execute $SVC/resource_service.rs"

# ── (d) every other class, one witness each ────────────────────────────────────────────────────
D_N=0
d_case() {
    local label="$1" class="$2" line="$3" dir
    D_N=$((D_N + 1)); dir="$WORK/fd-$D_N"; make_fixture "$dir"
    printf 'pub async fn f(c: &mut PgConnection) {\n    %s\n}\n' "$line" > "$dir/$SVC/d_service.rs"
    run_case "(d) $label reds" "$dir" 1 "$CHANGED" "> 1 $class $SVC/d_service.rs"
}
# The lines are Rust source: `$1` is a bind placeholder, not a shell expansion.
# shellcheck disable=SC2016
d_case "a redaction apply" apply_redaction \
    'sqlx::query("SELECT _edge_erasure_apply_redaction($1, $2)");'
# shellcheck disable=SC2016
d_case "a redaction projector" projector \
    'sqlx::query("SELECT _project_edge_erased_redactions($1, $2)");'
d_case "the replay setting" replay_setting \
    "sqlx::query(\"SELECT set_config('temper.replaying', 'on', false)\");"
d_case "a marker write" replay_marker \
    'sqlx::query("INSERT INTO kb_replay_target (id) VALUES (true)");'
d_case "a marker-writer call" replay_marker \
    'sqlx::query("SELECT _replay_target_mark()");'

# Spellings SQL treats alike: case, a schema qualifier, COPY, and SET in place of set_config.
d_case "a lowercase, schema-qualified marker write" replay_marker \
    'sqlx::query("insert into public.kb_replay_target (id) values (true)");'
d_case "a COPY into the marker" replay_marker \
    'sqlx::query("COPY kb_replay_target FROM STDIN");'
d_case "SET LOCAL of the replay setting" replay_setting \
    "sqlx::query(\"SET LOCAL temper.replaying = 'on'\");"
# shellcheck disable=SC2016
d_case "an uppercase act call" execute \
    'sqlx::query("SELECT RESOURCE_ERASURE_EXECUTE($1, $2, $3, $4, $5)");'

# ── (e) api/*.rs ───────────────────────────────────────────────────────────────────────────────
FE="$WORK/fe"; make_fixture "$FE"
cat > "$FE/api/erase.rs" <<'RS'
async fn handler(pool: PgPool) {
    sqlx::query("SELECT principal_erasure_execute($1, $2, $3, $4)").execute(&pool).await;
}
RS
run_case "(e) a caller in api/*.rs reds, naming it" \
    "$FE" 1 "$CHANGED" "> 1 execute api/erase.rs"

# ── (f) operator-run SQL ───────────────────────────────────────────────────────────────────────
FF="$WORK/ff"; make_fixture "$FF"
cat > "$FF/migrations/templates/replay_by_hand.sql" <<'SQL'
SELECT set_config('temper.replaying', 'on', false);
SQL
run_case "(f) a caller in an SQL template reds, naming it" \
    "$FF" 1 "$CHANGED" "> 1 replay_setting migrations/templates/replay_by_hand.sql"

# ── (g) a scan that finds nothing is broken ────────────────────────────────────────────────────
FG="$WORK/fg"
mkdir -p "$FG/crates/temper-core/src" "$FG/migrations/templates"
echo 'pub fn nothing() {}' > "$FG/crates/temper-core/src/lib.rs"
run_case "(g) an empty scan reds as broken" "$FG" 1 "the scan found no callers at all"

# ── (h) an unreadable file fails the scan ──────────────────────────────────────────────────────
FH="$WORK/fh"; make_fixture "$FH"
echo 'pub fn sealed() {}' > "$FH/$SVC/sealed.rs"
chmod 000 "$FH/$SVC/sealed.rs"
if [[ -r "$FH/$SVC/sealed.rs" ]]; then
    echo "  skip  (h) an unreadable file fails the scan — permission bits do not bind for this user"
else
    run_case "(h) an unreadable file fails the scan" \
        "$FH" 1 "the scan errored on" "SCAN-ERROR $SVC/sealed.rs"
fi
chmod 644 "$FH/$SVC/sealed.rs"

# ── (i) a test module is not production ────────────────────────────────────────────────────────
FI="$WORK/fi"; make_fixture "$FI"
cat > "$FI/$SVC/gated_service.rs" <<'RS'
pub fn production() {}

#[cfg(test)]
mod tests {
    async fn seed(pool: &PgPool) {
        sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4, $5)").execute(pool).await;
        sqlx::query("SELECT set_config('temper.replaying', 'on', true)").execute(pool).await;
    }
}
RS
run_case "(i) calls inside a #[cfg(test)] mod stay green" "$FI" 0 "$OK"

# ── (j) prose and names are not calls ──────────────────────────────────────────────────────────
FJ="$WORK/fj"; make_fixture "$FJ"
cat > "$FJ/$SVC/prose_service.rs" <<'RS'
// Never call resource_erasure_execute( from here, nor set_config('temper.replaying', …).
const PREFIX: &str = "resource_erasure_execute: ";
fn note() -> &'static str { "the replay walk marks kb_replay_target" }
RS
run_case "(j) a comment, and a name without a call, stay green" "$FJ" 0 "$OK"

# ── (k) integration tests are outside the field of view on purpose ─────────────────────────────
FK="$WORK/fk"; make_fixture "$FK"
mkdir -p "$FK/crates/temper-services/tests"
cat > "$FK/crates/temper-services/tests/erase_test.rs" <<'RS'
async fn erase(pool: &PgPool) {
    sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4, $5)").execute(pool).await;
}
RS
run_case "(k) an integration test under crates/*/tests stays green" "$FK" 0 "$OK"

# ── (l) a migration is not scanned ─────────────────────────────────────────────────────────────
FL="$WORK/fl"; make_fixture "$FL"
cat > "$FL/migrations/20990101000000_defs.sql" <<'SQL'
SELECT set_config('temper.replaying', 'on', true);
SELECT resource_erasure_execute(NULL, NULL, NULL, NULL, NULL);
SQL
run_case "(l) a migration stays green" "$FL" 0 "$OK"

echo
echo "  $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
