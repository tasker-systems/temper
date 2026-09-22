#!/usr/bin/env bash
# check-openapi-pin.sh — the pinned-contract gate: the committed openapi.json stays
# inside the current pinned minor.
#
# WHY THIS EXISTS
# ---------------
# Pete, 2026-09-16: the deploy base has grown past the point where a wire-shape
# breaking change can meaningfully ship even as a minor bump — the widest enterprise
# adoption is current as of the v0.5.1 wire shapes, and a minor release does not
# reach that fleet on any predictable cadence. The declared-class gate
# (wire-class-crosscheck.sh) makes a break SIGNALLED; this gate makes it AVOIDABLE:
# each released minor pins its contract under schemas/versions/<M.m>/openapi.json,
# and every patch-level change to the committed contract must be ADDITIVE against
# the current pin. A client built at the pin interoperates with every later
# release of the era, in both skew directions. The genuinely breaking change is
# not forbidden — but since the compat-deprecation regime (2026-09-22, spec of
# record temper-artifacts/specs/2026-09-22-compat-semver-policy-amendment-design.md)
# it has exactly two routes: the deprecation path — keep serving the existing
# rendering under a record and land the corrected signal additively (D-C2/D-C3) —
# or the retirement release train, where M moves and the next pin is cut in the
# same release PR (D-C1/D-C4). Until one of those, this gate reads red, which is
# the discipline speaking: that movement cannot merge to main.
#
# The additive standard is the declared-class gate's own (spec §4: shapes only
# grow; tolerant in both skew directions), computed by the SAME comparator —
# wire-shape.jq / wire-shape-lib.jq, one definition, never a second copy. The
# verdict is that comparator's; this script owns only pin selection and the
# failure face.
#
# PIN SELECTION
# -------------
# The current pin is the pin whose <M.m> has the SAME major as the repo-root
# VERSION and the GREATEST minor ≤ VERSION's minor. So during minor 0.5 the 0.5
# pin applies; when the release train bumps VERSION to 0.6 and cuts the 0.6 pin in
# the same PR, the 0.6 pin applies from that commit onward. A VERSION bumped past
# the newest pin (the interim before the pin PR merges) keeps judging against the
# newest pin ≤ VERSION — additive growth passes, real movement still fails.
#
# FAIL CLOSED
# -----------
# No pin for VERSION's major, no committed openapi.json, unreadable JSON on either
# side, a missing jq, or a pin directory without its provenance README.md — every
# one is a FAIL with the remedy named. "I could not look" is never "nothing moved".
#
# ACCEPTED RESIDUE
# ----------------
# * The pin records the HTTP contract only. MCP output and CLI stdout are contract
#   surface (spec §3) whose shape no committed artifact records — their classes are
#   owned by review, exactly as in wire-class-crosscheck.sh (spec §4's residue).
#   D-S6's other pinned surfaces — the three generated skins and the in-tree ts-rs
#   trees — track this contract transitively through their drift gates in the same
#   CI; they are not pinned separately.
# * The comparator's whitelisted prose edits and its born-node tolerance are the
#   spec's additive definition, not this script's opinion.
#
# USAGE
#   bash .github/scripts/check-openapi-pin.sh                 # repo defaults
#   cargo make openapi-pin-check                              # wired into `check`
#
# Fixture seams (each replaces one repo derivation; what the harness feeds):
#   --emitted FILE       the committed contract (default: openapi.json at repo root)
#   --pin-dir DIR        the pin tree (default: schemas/versions)
#   --version-file FILE  the VERSION file (default: VERSION at repo root)
#
# Exit 0 = the committed contract is unchanged or grew against the current pin.
# Exit 1 = it moved, or the gate could not look.

set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

EMITTED_ARG=""
PIN_DIR_ARG=""
VERSION_FILE_ARG=""

while [ $# -gt 0 ]; do
    case "$1" in
        --emitted)      EMITTED_ARG="${2:-}"; shift 2 ;;
        --pin-dir)      PIN_DIR_ARG="${2:-}"; shift 2 ;;
        --version-file) VERSION_FILE_ARG="${2:-}"; shift 2 ;;
        *) echo "check-openapi-pin: unknown argument: $1" >&2; exit 1 ;;
    esac
done

fail() { echo "check-openapi-pin: FAILED — $*"; exit 1; }

command -v jq >/dev/null 2>&1 || fail "jq is not installed; the shape verdict cannot be computed, and 'I could not look' is not 'nothing moved'."

VERSION_FILE="${VERSION_FILE_ARG:-VERSION}"
[ -f "$VERSION_FILE" ] || fail "no VERSION file at ${VERSION_FILE} — the current minor is unknowable, so no pin can be selected."
RAW_VERSION="$(tr -d '[:space:]' < "$VERSION_FILE")"
case "$RAW_VERSION" in
    *.*.*) : ;;
    *) fail "VERSION does not read as <major>.<minor>.<patch> of bare numbers (read: '${RAW_VERSION}')." ;;
esac
VMAJOR="${RAW_VERSION%%.*}"
REM="${RAW_VERSION#*.}"
VMINOR="${REM%%.*}"
VPATCH="${REM#*.}"
case "$VMAJOR" in ''|*[!0-9]*) fail "VERSION's major is not a bare number (read: '${RAW_VERSION}')." ;; esac
case "$VMINOR" in ''|*[!0-9]*) fail "VERSION's minor is not a bare number (read: '${RAW_VERSION}')." ;; esac
case "$VPATCH" in ''|*[!0-9]*|*.*) fail "VERSION does not read as <major>.<minor>.<patch> of bare numbers (read: '${RAW_VERSION}')." ;; esac

PIN_DIR="${PIN_DIR_ARG:-schemas/versions}"
[ -d "$PIN_DIR" ] || fail "no pin tree at ${PIN_DIR}/ — cut a pin for the current minor: schemas/versions/${VMAJOR}.${VMINOR}/openapi.json plus a provenance README.md (see the 0.5 pin's README for the form)."

# ── Pin selection: same major, greatest minor ≤ VERSION's minor ─────────────────────────────────
CURRENT_PIN=""
CURRENT_PIN_MINOR=-1
for pin_json in "$PIN_DIR"/*/openapi.json; do
    [ -f "$pin_json" ] || continue
    pin_mm="$(basename "$(dirname "$pin_json")")"
    case "$pin_mm" in
        *.*) pmajor="${pin_mm%%.*}"; pminor="${pin_mm##*.}" ;;
        *) continue ;;
    esac
    case "${pmajor}${pminor}" in
        *[!0-9]*) continue ;;  # not an <M.m> directory; not ours to judge
    esac
    if [ "$pmajor" = "$VMAJOR" ] && [ "$pminor" -le "$VMINOR" ] && [ "$pminor" -gt "$CURRENT_PIN_MINOR" ]; then
        CURRENT_PIN="$pin_json"
        CURRENT_PIN_MINOR="$pminor"
    fi
done
[ -n "$CURRENT_PIN" ] || fail "no pin for major ${VMAJOR} at or below minor ${VMINOR} under ${PIN_DIR}/ (found: $(ls "$PIN_DIR" 2>/dev/null | tr '\n' ' ')). Cut schemas/versions/${VMAJOR}.${VMINOR}/openapi.json with a provenance README.md — the discipline lives in RELEASING.md."

[ -f "$(dirname "$CURRENT_PIN")/README.md" ] || fail "pin ${CURRENT_PIN} carries no provenance README.md beside it — a pin without its source named is a snapshot nobody can trust. See the 0.5 pin's README for the form."

EMITTED="${EMITTED_ARG:-openapi.json}"
[ -f "$EMITTED" ] || fail "no committed contract at ${EMITTED} — nothing to compare against the pin at ${CURRENT_PIN}. If the emit moved, update this gate; if the contract was deleted, that is the movement."

TMP_BASE="$(mktemp)"; TMP_HEAD="$(mktemp)"
trap 'rm -f "$TMP_BASE" "$TMP_HEAD"' EXIT

# .info.version is stripped on both sides, exactly as wire-class-crosscheck.sh does:
# the version bump itself is the D-S3 baseline, never shape movement.
jq -S 'del(.info.version)' "$CURRENT_PIN" > "$TMP_BASE" 2>/dev/null || fail "the pin ${CURRENT_PIN} is not readable JSON — a pin that cannot be parsed cannot be enforced."
jq -S 'del(.info.version)' "$EMITTED"     > "$TMP_HEAD" 2>/dev/null || fail "the committed contract ${EMITTED} is not readable JSON — resolve before judging."
[ -s "$TMP_BASE" ] && [ -s "$TMP_HEAD" ] || fail "a stripped side came back empty (pin or contract) — refusing to compare."

if cmp -s "$TMP_BASE" "$TMP_HEAD"; then
    echo "check-openapi-pin: PASS — the committed contract is identical to the current pin ($(basename "$(dirname "$CURRENT_PIN")")), version stripped."
    exit 0
fi

# ── The verdict — the shared comparator, one definition ─────────────────────────────────────────
VERDICT="$(jq -n -r -f "${SCRIPT_DIR}/wire-shape.jq" -L "$SCRIPT_DIR" \
    --slurpfile base "$TMP_BASE" --slurpfile head "$TMP_HEAD" 2>/dev/null)"
VERDICT="${VERDICT//\"/}"
[ -z "$VERDICT" ] && VERDICT="moved"   # fail closed: an unreadable verdict is never a pass

case "$VERDICT" in
    grew)
        echo "check-openapi-pin: PASS — the committed contract GREW against the current pin ($(basename "$(dirname "$CURRENT_PIN")")): shapes only grew, tolerant in both skew directions. The P floor is owed at the next release (spec §4)."
        exit 0
        ;;
    moved)
        echo "check-openapi-pin: FAILED — the committed contract MOVED beyond the current pin ($(basename "$(dirname "$CURRENT_PIN")")). Patch releases may only grow the shape; this movement strands clients built at the pin."
        echo
        echo "── breaking movement (removed or changed, named) ──"
        MOVEMENTS="$(jq -n -r -f "${SCRIPT_DIR}/wire-pin-movements.jq" -L "$SCRIPT_DIR" \
            --slurpfile base "$TMP_BASE" --slurpfile head "$TMP_HEAD" 2>/dev/null \
            | jq -r '.[]' 2>/dev/null)"
        BREAKING="$(printf '%s\n' "$MOVEMENTS" | grep -E '(removed|changed):' || true)"
        BORN="$(printf '%s\n' "$MOVEMENTS" | grep -E 'born:' || true)"
        if [ -n "$BREAKING" ]; then
            printf '%s\n' "$BREAKING" | sed 's/^/  /'
        else
            echo "  (the naming pass could not enumerate it — treat the whole diff as the movement; fail closed)"
        fi
        if [ -n "$BORN" ]; then
            echo
            echo "── tolerated growth alongside the movement ──"
            printf '%s\n' "$BORN" | sed 's/^/  /'
        fi
        echo
        echo "This movement is a shape break — below the era level it is a refusal, not a schedule"
        echo "(D-C2). Convert it to the deprecation path: keep serving the existing rendering under a"
        echo "record, land the corrected signal additively, and re-declare the register row"
        echo "(additive + behavioral — the #906 pattern). Or wait for the retirement release train,"
        echo "where M moves and this release PR cuts the next pin (schemas/versions/<M.m>/ per"
        echo "RELEASING.md), which turns this gate green. A version bump alone never launders a break"
        echo "into safety. Until one of those, this red is the discipline speaking: this diff cannot"
        echo "merge to main."
        exit 1
        ;;
    *)
        fail "the shape verdict '${VERDICT}' is neither grew nor moved — refusing to render it as a pass."
        ;;
esac
