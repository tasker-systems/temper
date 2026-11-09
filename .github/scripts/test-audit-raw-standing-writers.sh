#!/usr/bin/env bash
# .github/scripts/test-audit-raw-standing-writers.sh
#
# Test harness for audit-raw-standing-writers.sh — the guard that pins production code calling the
# standing committer `principal_standing_apply` directly instead of through `standing_service`.
#
# Each red case violates exactly one of the guard's claims on a fixture that satisfies the rest;
# each green case pins a boundary of its field of view, so its silence is never broader than its
# header says:
#
#   (a) RED   a new raw writer under crates/*/src, naming the file
#   (b) RED   a second raw call in an already-baselined file (the count is pinned, not the path)
#   (c) RED   a raw call in an operator-run SQL template, naming the file
#   (d) RED   a writer after an item-level `#[cfg(test)] use` — only a test MODULE ends the scan
#   (e) RED   an empty scan fails as broken, it does not pass
#   (f) RED   a file the scan cannot read fails the scan rather than shrinking it
#   (g) GREEN a raw call inside a `#[cfg(test)] mod` is not a production writer
#   (h) GREEN the seam itself (`standing_service.rs`) calls the committer
#   (i) GREEN a `//` comment line naming the call
#   (j) GREEN an integration test under crates/*/tests
#
# Fixtures are COPIES of the files the baseline names, plus the seam, so the unedited fixture
# reproduces the reviewed set exactly and every red below is its one edit.
#
#   bash .github/scripts/test-audit-raw-standing-writers.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT="${SCRIPT_DIR}/audit-raw-standing-writers.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'chmod -R u+rwX "$WORK" 2>/dev/null; rm -rf "$WORK"' EXIT

FIXTURE_FILES=(
    crates/temper-services/src/services/standing_service.rs
    crates/temper-services/src/services/machine_registration_service.rs
    crates/temper-substrate/src/scenario/access/loader.rs
    crates/temper-substrate/src/scenario/bootseed.rs
    crates/temper-substrate/src/scenario/loader.rs
    migrations/templates/system_initialization.sql
)

make_fixture() {
    local dest="$1" f
    for f in "${FIXTURE_FILES[@]}"; do
        mkdir -p "$dest/$(dirname "$f")"
        cp "$REPO_ROOT/$f" "$dest/$f"
    done
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

CHANGED='FAIL — the set of raw principal_standing_apply callers changed'
OK='audit-raw-standing-writers: OK'

echo "test-audit-raw-standing-writers"

# ── the controls ───────────────────────────────────────────────────────────────────────────────
run_case "real tree passes" "$REPO_ROOT" 0 "$OK"

F0="$WORK/f0"; make_fixture "$F0"
run_case "unedited fixture passes" "$F0" 0 "$OK"

# ── (a) a new raw writer ───────────────────────────────────────────────────────────────────────
FA="$WORK/fa"; make_fixture "$FA"
mkdir -p "$FA/crates/temper-services/src/services"
cat > "$FA/crates/temper-services/src/services/widget_service.rs" <<'RS'
pub async fn approve(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("SELECT principal_standing_apply($1,'approve','approved',NULL,NULL)")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
RS
run_case "(a) a new raw writer reds, naming the file" \
    "$FA" 1 "$CHANGED" "widget_service.rs"

# ── (b) one more call in a file already on the baseline ────────────────────────────────────────
FB="$WORK/fb"; make_fixture "$FB"
cat >> "$FB/crates/temper-substrate/src/scenario/loader.rs" <<'RS'

async fn reseed(tx: &mut PgConnection, id: Uuid) -> Result<()> {
    sqlx::query("SELECT principal_standing_apply($1,'provision','approved',NULL,NULL)")
        .bind(id)
        .execute(tx)
        .await?;
    Ok(())
}
RS
run_case "(b) a second raw call in a baselined file reds" \
    "$FB" 1 "$CHANGED" "2 crates/temper-substrate/src/scenario/loader.rs"

# ── (c) operator-run SQL ───────────────────────────────────────────────────────────────────────
FC="$WORK/fc"; make_fixture "$FC"
cat > "$FC/migrations/templates/reinstate_operator.sql" <<'SQL'
SELECT principal_standing_apply(:'profile_id', 'reactivate', 'approved', NULL, 'operator');
SQL
run_case "(c) a raw call in an SQL template reds, naming it" \
    "$FC" 1 "$CHANGED" "migrations/templates/reinstate_operator.sql"

# ── (d) an item-level test gate does not end the scan ──────────────────────────────────────────
FD="$WORK/fd"; make_fixture "$FD"
mkdir -p "$FD/crates/temper-core/src"
cat > "$FD/crates/temper-core/src/gated_use.rs" <<'RS'
#[cfg(test)]
use crate::test_support::seed;

pub async fn apply(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("SELECT principal_standing_apply($1,'approve','approved',NULL,NULL)")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
RS
run_case "(d) a writer after an item-level #[cfg(test)] use reds" \
    "$FD" 1 "$CHANGED" "gated_use.rs"

# ── (e) a scan that finds nothing is broken ────────────────────────────────────────────────────
FE="$WORK/fe"
mkdir -p "$FE/crates/temper-core/src" "$FE/migrations/templates"
echo 'pub fn nothing() {}' > "$FE/crates/temper-core/src/lib.rs"
run_case "(e) an empty scan reds as broken" \
    "$FE" 1 "the scan found no callers at all"

# ── (f) an unreadable file fails the scan ──────────────────────────────────────────────────────
FF="$WORK/ff"; make_fixture "$FF"
mkdir -p "$FF/crates/temper-core/src"
echo 'pub fn sealed() {}' > "$FF/crates/temper-core/src/sealed.rs"
chmod 000 "$FF/crates/temper-core/src/sealed.rs"
if [[ -r "$FF/crates/temper-core/src/sealed.rs" ]]; then
    # Running as root: permission bits do not bind, so this fixture cannot be built here.
    echo "  skip  (f) an unreadable file fails the scan — permission bits do not bind for this user"
else
    run_case "(f) an unreadable file fails the scan" \
        "$FF" 1 "the scan errored on" "SCAN-ERROR crates/temper-core/src/sealed.rs"
fi
chmod 644 "$FF/crates/temper-core/src/sealed.rs"

# ── (g) a test module is not production ────────────────────────────────────────────────────────
FG="$WORK/fg"; make_fixture "$FG"
cat >> "$FG/crates/temper-services/src/services/machine_registration_service.rs" <<'RS'

#[cfg(test)]
mod raw_seed_tests {
    async fn seed(pool: &PgPool, id: Uuid) {
        sqlx::query("SELECT principal_standing_apply($1,'provision','approved',NULL,NULL)")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }
}
RS
run_case "(g) a raw call inside a #[cfg(test)] mod stays green" "$FG" 0 "$OK"

# ── (h) the seam is the sanctioned caller ──────────────────────────────────────────────────────
FH="$WORK/fh"; make_fixture "$FH"
cat >> "$FH/crates/temper-services/src/services/standing_service.rs" <<'RS'

pub async fn commit_again(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("SELECT principal_standing_apply($1,'approve','approved',NULL,NULL)")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
RS
run_case "(h) a call in the seam itself stays green" "$FH" 0 "$OK"

# ── (i) prose is not a call ────────────────────────────────────────────────────────────────────
FI="$WORK/fi"; make_fixture "$FI"
printf '\n// Never call principal_standing_apply( directly from here.\n' \
    >> "$FI/crates/temper-substrate/src/scenario/bootseed.rs"
run_case "(i) a // comment naming the call stays green" "$FI" 0 "$OK"

# ── (j) integration tests are outside the field of view on purpose ─────────────────────────────
FJ="$WORK/fj"; make_fixture "$FJ"
mkdir -p "$FJ/crates/temper-services/tests"
cat > "$FJ/crates/temper-services/tests/standing_seed_test.rs" <<'RS'
async fn seed(pool: &PgPool, id: Uuid) {
    sqlx::query("SELECT principal_standing_apply($1,'provision','approved',NULL,NULL)")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}
RS
run_case "(j) an integration test under crates/*/tests stays green" "$FJ" 0 "$OK"

echo
echo "  $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
