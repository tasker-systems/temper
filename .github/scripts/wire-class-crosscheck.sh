#!/usr/bin/env bash
# wire-class-crosscheck.sh — every PR that moves a wire surface declares its compat class in
# RELEASE_REGISTER.md, and CI fails the PRs that say nothing, or whose declaration their own
# openapi diff contradicts.
#
# WHY THIS EXISTS
# ---------------
# The shared-semver policy (spec of record:
# temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md, §4, decision D-S4) makes the
# compatibility gate a declared-class mechanism: per-PR class declaration, CI verifying only the
# mechanical half, human judgment owning the behavioral half, and a release-verdict register that
# gates releases. The register row is the DURABLE declaration, because a merged PR body is not in
# the tree and the release calculator diffs the tree — declare_migration's lesson carried over:
# "the declaration must live in the artifact the tooling reads".
#
# The pattern's incumbents are the two scripts this one extends to the openapi/register side:
# audit-migration-declarations.sh (silence fails) and sqlx-schema-crosscheck.sh (the claim is
# cross-checked against what the wire actually did). This is that logic pointed at
# crates/temper-api/, crates/temper-mcp/, crates/temper-client/, openapi.json, clients/, and
# packages/. The diff base comes from the same mechanism those use: WIRE_DIFF_BASE /
# GITHUB_BASE_SHA / GITHUB_BASE_REF / merge-base with origin/main / HEAD~1.
#
# THE STRUCTURAL BAR — READ BEFORE "IMPROVING" THIS SCRIPT
# --------------------------------------------------------
# Spec §4, verbatim:
#
#   "CI is structurally barred from certifying the behavioral class green — behavior is not in
#    the diff, and a green check that claims it is lying. The behavioral half is owned by
#    review (the gate question) and by the merge decision (briefing duty)."
#
# All nine behavioral citations absorbed into the register (spec §7) passed green CI. That is
# not a CI deficiency; it is the class boundary. A green run here proves row presence, register
# well-formedness, and shape honesty — it proves NOTHING about behavior behind unchanged
# shapes, which is exactly the class CI could never see.
#
# WHAT IS CHECKED
# ---------------
#   1. PRESENCE — a wire-touched path changed and the diff adds no register row naming this
#      PR: FAIL, naming the wire files. The row, not the file, is what is checked: a PR that
#      edits the register without adding its own row still fails.
#   2. PARSE — when RELEASE_REGISTER.md changed, every machine line (`pr:`, `classes:`,
#      `surfaces:`, `status:`) must be well-formed against the register header's own
#      vocabulary. Malformed: FAIL, naming the line.
#   3. SHAPE HONESTY — when openapi.json changed, this PR's own row(s) must declare a shape
#      class, and the declaration must survive the actual diff:
#        shape moved (anything beyond `.info.version`, jq-compared) and no `shape-breaking`
#            declared: FAIL — the #858 class;
#        no shape movement and `additive` declared: PASS — the D-S3 baseline (a version bump
#            re-stales the doc and the generated cores; that is what `additive` means);
#        no shape movement and `shape-breaking` declared: PASS, noted — honest
#            over-declaration never fails here (the sqlx asymmetry; failing it would train
#            under-declaration);
#        neither `additive` nor `shape-breaking` while openapi.json changed: FAIL —
#            `behavioral` does not answer the shape question, and the P floor is owed.
#
# THE pr: self SENTINEL
# ---------------------
# The row describing the PR that carries it is written `pr: self` — the number is unknowable
# before push, and presence is what CI verifies, not the number. `pr: <the real number>` is
# equally legal once known; CI (which knows the number, via GITHUB_PR_NUMBER) and --pr accept
# either.
#
# ACCEPTED RESIDUE
# ----------------
# * Shape movement is measured on openapi.json only. MCP output and CLI stdout are contract
#   surface (spec §3) whose shape no committed artifact records, so no mechanical half exists
#   for them — their classes are owned by review, exactly like the behavioral class.
# * An unknown machine-looking field (`claas: additive`) is not flagged; only the four
#   documented fields are parsed. The release calculator reads the same four, so a typo'd
#   field surfaces as a missing declaration at release time — and check 1 already fails the
#   row this PR owes.
# * `surfaces:` is validated beyond the three fields the predicate names, because the register
#   header documents its vocabulary and a machine-line validator that skips a machine line is
#   a hole the file's own contract already closes.
#
# USAGE
#   .github/scripts/wire-class-crosscheck.sh                # CI / local: derive from git
#   GITHUB_PR_NUMBER=123 .github/scripts/wire-class-crosscheck.sh
#
# Fixture seams (each replaces one git derivation; what the harness feeds):
#   --wire-touched FILE      file of changed paths, one per line
#   --added-register FILE    file of added register machine lines, verbatim (`pr: self`, ...)
#   --shape-verdict moved|unchanged|undeterminable
#   --register FILE          parse target (default: RELEASE_REGISTER.md at the repo root;
#                            the diff/presence half always reads the repo's own register)
#   --pr N                   this PR's number (else GITHUB_PR_NUMBER; else `self` only)
#   --base REF               diff base (else the incumbents' candidate list)
#
# Exit 0 = presence, parse, and shape honesty hold. Exit 1 = they do not (every face reported).

set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

REGISTER_FILE="RELEASE_REGISTER.md"
REGISTER_EXPLICIT=0
PR_NUMBER="${GITHUB_PR_NUMBER:-}"
BASE_ARG=""
WIRE_TOUCHED_FILE=""
ADDED_REGISTER_FILE=""
SHAPE_VERDICT_ARG=""

while [ $# -gt 0 ]; do
    case "$1" in
        --register)       REGISTER_FILE="${2:-}"; REGISTER_EXPLICIT=1; shift 2 ;;
        --pr)             PR_NUMBER="${2:-}"; shift 2 ;;
        --base)           BASE_ARG="${2:-}"; shift 2 ;;
        --wire-touched)   WIRE_TOUCHED_FILE="${2:-}"; shift 2 ;;
        --added-register) ADDED_REGISTER_FILE="${2:-}"; shift 2 ;;
        --shape-verdict)  SHAPE_VERDICT_ARG="${2:-}"; shift 2 ;;
        *) echo "wire-class-crosscheck: unknown argument: $1" >&2; exit 1 ;;
    esac
done

# The wire surface, per spec §4/§5 — the same trees detect-changes.sh classes as WIRE,
# CLIENTS, and PACKAGES. openapi.json is exact; the rest are prefixes.
WIRE_PREFIXES="crates/temper-api/ crates/temper-mcp/ crates/temper-client/ clients/ packages/"

# ── The diff base — the incumbents' candidate list, copied ──────────────────────────────────────
derive_base() {
    local candidate
    for candidate in \
        "${BASE_ARG:-}" \
        "${WIRE_DIFF_BASE:-}" \
        "${GITHUB_BASE_SHA:-}" \
        "$( [ -n "${GITHUB_BASE_REF:-}" ] && git merge-base "origin/${GITHUB_BASE_REF}" HEAD 2>/dev/null )" \
        "$(git merge-base origin/main HEAD 2>/dev/null)" \
        "$(git rev-parse HEAD~1 2>/dev/null)"
    do
        [ -z "$candidate" ] && continue
        git cat-file -e "${candidate}^{commit}" 2>/dev/null || git fetch --depth=1 origin "$candidate" >/dev/null 2>&1 || true
        if git cat-file -e "${candidate}^{commit}" 2>/dev/null; then BASE="$candidate"; return 0; fi
    done
    return 1
}

CHANGED_FILE="$(mktemp)"
WIRE_FILES="$(mktemp)"
ADDED_LINES="$(mktemp)"
trap 'rm -f "$CHANGED_FILE" "$WIRE_FILES" "$ADDED_LINES"' EXIT

if [ -n "$WIRE_TOUCHED_FILE" ]; then
    cp "$WIRE_TOUCHED_FILE" "$CHANGED_FILE"
else
    if ! derive_base; then
        echo "wire-class-crosscheck: FAILED — could not resolve a diff base against which to"
        echo "list changed paths. 'I could not look' is not 'nothing moved'; resolve the base"
        echo "(--base REF or WIRE_DIFF_BASE) and re-run."
        exit 1
    fi
    git diff --name-only "$BASE" > "$CHANGED_FILE" 2>/dev/null || : > "$CHANGED_FILE"
fi

# ── Which changed paths are wire-touched ────────────────────────────────────────────────────────
while IFS= read -r p; do
    [ -z "$p" ] && continue
    if [ "$p" = "openapi.json" ]; then
        printf '%s\n' "$p" >> "$WIRE_FILES"
        continue
    fi
    for pre in $WIRE_PREFIXES; do
        case "$p" in "$pre"*) printf '%s\n' "$p" >> "$WIRE_FILES"; break ;; esac
    done
done < "$CHANGED_FILE"

REGISTER_IN_DIFF=0
if [ -z "$WIRE_TOUCHED_FILE" ] && grep -qx "RELEASE_REGISTER.md" "$CHANGED_FILE"; then
    REGISTER_IN_DIFF=1
fi

# ── Check 2: parse — every machine line in the register is well-formed ──────────────────────────
# Runs when the register changed in this diff, or when a parse target was named explicitly
# (the fixture seam). A malformed register is FATAL and first: a declaration nobody can parse
# is not a declaration, and the shape ruling below would be reading garbage.
if [ "$REGISTER_IN_DIFF" -eq 1 ] || [ "$REGISTER_EXPLICIT" -eq 1 ]; then
    if [ ! -f "$REGISTER_FILE" ]; then
        echo "wire-class-crosscheck: FAILED — register file not found: ${REGISTER_FILE}" >&2
        exit 1
    fi
    PARSE_OUT="$(awk '
        function flag(msg) {
            printf "%s:%d: %s\n", FILENAME, FNR, orig
            printf "        %s\n", msg
        }
        {
            orig = $0
            if ($0 !~ /^(pr|classes|surfaces|status):/) next
            field = $0; sub(/:.*/, "", field)
            value = $0; sub(/^[^:]*:[[:space:]]*/, "", value)
            if (field == "pr") {
                if (value == "") flag("pr: is empty — a row must name its PR (a number, or a sentinel: pre-policy, goal, self)")
                else if (value !~ /^(self|goal|pre-policy|[0-9]+)$/) flag("pr: \x27" value "\x27 — expected a PR number or one of the sentinels: pre-policy, goal, self")
            } else if (field == "classes") {
                if (value == "") { flag("classes: is empty — silence is not a classification"); next }
                n = split(value, toks, /[[:space:],]+/)
                for (i = 1; i <= n; i++)
                    if (toks[i] != "" && toks[i] !~ /^(additive|shape-breaking|behavioral)$/)
                        flag("classes: \x27" toks[i] "\x27 is not one of: additive, shape-breaking, behavioral")
            } else if (field == "surfaces") {
                if (value == "") { flag("surfaces: is empty — name the surfaces this row speaks for"); next }
                n = split(value, toks, /[[:space:],]+/)
                for (i = 1; i <= n; i++)
                    if (toks[i] != "" && toks[i] !~ /^(http|mcp|cli-stdout|clients|schema|internal)$/)
                        flag("surfaces: \x27" toks[i] "\x27 is not one of: http, mcp, cli-stdout, clients, schema, internal")
            } else {
                if (value == "") { flag("status: is empty"); next }
                if (value == "open" || value == "signal-only" || value == "satisfied") next
                if (value ~ /^blocked:/) {
                    if (substr(value, 9) == "") flag("status: \x27blocked:\x27 names no release class — everything after \x27blocked:\x27 is the class name, free text, never tokenized")
                    next
                }
                flag("status: \x27" value "\x27 — expected open, signal-only, satisfied, or blocked:<release-class>")
            }
        }
    ' "$REGISTER_FILE")"
    if [ -n "$PARSE_OUT" ]; then
        echo "wire-class-crosscheck: FAILED — the register (${REGISTER_FILE}) is malformed."
        echo
        printf '%s\n' "$PARSE_OUT"
        echo
        echo "The register header documents the machine fields; fix the lines named above first —"
        echo "a declaration nobody can parse is not a declaration (spec §4.1)."
        exit 1
    fi
fi

# ── Not wire-touched: nothing to cross-check ────────────────────────────────────────────────────
WIRE_COUNT="$(grep -c . "$WIRE_FILES" || true)"
WIRE_COUNT="${WIRE_COUNT:-0}"
if [ "$WIRE_COUNT" -eq 0 ]; then
    if [ "$REGISTER_IN_DIFF" -eq 1 ] || [ "$REGISTER_EXPLICIT" -eq 1 ]; then
        echo "wire-class-crosscheck: PASS — no wire paths changed; the register edit validated."
    else
        echo "wire-class-crosscheck: PASS — no wire paths changed and the register is untouched; nothing to check."
    fi
    exit 0
fi

echo "── wire paths ──"
WIRE_LIST=""
grep -qx "openapi.json" "$WIRE_FILES" && WIRE_LIST="  openapi.json"$'\n'
for pre in $WIRE_PREFIXES; do
    c="$(grep -c "^${pre}" "$WIRE_FILES" || true)"
    c="${c:-0}"
    if [ "$c" -gt 0 ]; then
        WIRE_LIST="${WIRE_LIST}  ${pre} (${c} file(s))"$'\n'
    fi
done
printf '%s' "$WIRE_LIST"

# ── This PR's own rows, from the register lines this diff ADDS ──────────────────────────────────
# A row counts only when the diff adds its `pr:` line — declaring by editing someone else's
# row is not declaring. Fields are read as a consecutive block, so classes land with the pr:
# they belong to.
added_register_lines() {
    if [ -n "$ADDED_REGISTER_FILE" ]; then
        cat "$ADDED_REGISTER_FILE"
        return 0
    fi
    derive_base || return 1
    git diff "$BASE" -- RELEASE_REGISTER.md 2>/dev/null \
        | grep -E '^\+' | grep -vE '^\+\+\+ ' | sed 's/^+//'
}

added_register_lines > "$ADDED_LINES" || ADDED_LINES_DERIV_FAILED=1

# Added lines → records of `pr<TAB>classes`. A `classes:` line attaches to the most recent
# added `pr:` line, so an edit to someone else's row never masquerades as this PR's row;
# a row with no classes reads "-", which no vocabulary check can mistake for a class.
added_records() {
    awk '
        function trim(s) { sub(/^[[:space:]]+/, "", s); sub(/[[:space:]]+$/, "", s); return s }
        function flush() { if (pr != "") printf "%s\t%s\n", pr, (cls == "" ? "-" : cls) }
        /^pr:/      { flush(); pr = trim(substr($0, 4)); cls = ""; next }
        /^classes:/ { if (pr != "") { c = trim(substr($0, 9)); cls = (cls == "" ? c : cls ", " c) } next }
        { next }
        END { flush() }
    ' "$ADDED_LINES"
}

SELF_CLASSES=""
HAS_OWN_ROW=0
if [ "${ADDED_LINES_DERIV_FAILED:-0}" -eq 1 ]; then
    SELF_CLASSES="<deriv-failed>"
else
    while IFS=$'\t' read -r r_pr r_cls; do
        [ -z "${r_pr:-}" ] && continue
        if [ "$r_pr" = "self" ] || { [ -n "$PR_NUMBER" ] && [ "$r_pr" = "$PR_NUMBER" ]; }; then
            HAS_OWN_ROW=1
            SELF_CLASSES="${SELF_CLASSES} ${r_cls}"
        fi
    done < <(added_records)
fi

classes_contain() {
    case ",$(printf '%s' "$1" | tr -d ' ')," in
        *",$2,"*) return 0 ;;
        *) return 1 ;;
    esac
}

echo "── register row for this PR ──"
if [ "$HAS_OWN_ROW" -eq 1 ]; then
    printf '  found — declared classes:%s\n' "$SELF_CLASSES"
else
    echo "  none — no added register row names this PR (pr: self$( [ -n "$PR_NUMBER" ] && printf ' or pr: %s' "$PR_NUMBER" ))"
fi

PROBLEMS=""

# ── Check 1: presence ───────────────────────────────────────────────────────────────────────────
if [ "$HAS_OWN_ROW" -eq 0 ]; then
    PROBLEMS="  RELEASE_REGISTER.md adds no register row for this PR, while wire-touched paths changed:"$'\n'
    PROBLEMS="${PROBLEMS}${WIRE_LIST}"
    PROBLEMS="${PROBLEMS}  Every wire-touching PR lands its own declaration row in RELEASE_REGISTER.md, under"$'\n'
    PROBLEMS="${PROBLEMS}  '## Since <version> — unreleased', with the four machine fields (pr: self is legal —"$'\n'
    PROBLEMS="${PROBLEMS}  the number is unknowable before push): pr:, classes:, surfaces:, status:. The register"$'\n'
    PROBLEMS="${PROBLEMS}  row is the durable declaration; the PR template field is only the authoring act (spec §4)."$'\n'
fi

# ── Check 3: shape honesty ──────────────────────────────────────────────────────────────────────
derive_shape() {
    command -v jq >/dev/null 2>&1 || { echo "undeterminable"; return 0; }
    # openapi.json absent from the base: the whole contract is new in this PR — maximal movement.
    if ! git cat-file -e "${BASE}:openapi.json" 2>/dev/null; then
        echo "moved"; return 0
    fi
    base_s="$(git show "${BASE}:openapi.json" 2>/dev/null | jq -S 'del(.info.version)' 2>/dev/null)" || { echo "undeterminable"; return 0; }
    head_s="$(jq -S 'del(.info.version)' openapi.json 2>/dev/null)" || { echo "undeterminable"; return 0; }
    if [ -z "$base_s" ] || [ -z "$head_s" ]; then echo "undeterminable"; return 0; fi
    if [ "$base_s" = "$head_s" ]; then echo "unchanged"; else echo "moved"; fi
}

SHAPE_NOTE=""
if grep -qx "openapi.json" "$WIRE_FILES"; then
    if [ -n "$SHAPE_VERDICT_ARG" ]; then
        SHAPE_VERDICT="$SHAPE_VERDICT_ARG"
    else
        derive_base || SHAPE_VERDICT="undeterminable"
        [ -n "${SHAPE_VERDICT:-}" ] || SHAPE_VERDICT="$(derive_shape)"
    fi
    echo "── shape (openapi.json, .info.version stripped) ──"
    echo "  verdict: ${SHAPE_VERDICT}"
    case "$SHAPE_VERDICT" in
        moved)
            if classes_contain "$SELF_CLASSES" "shape-breaking"; then
                SHAPE_NOTE="  shape moved and the register declares shape-breaking — the M question is on the record."
            else
                PROBLEMS="${PROBLEMS}  openapi.json moved beyond .info.version and this PR's register row(s) declare no"
                PROBLEMS="${PROBLEMS}"$'\n'
                PROBLEMS="${PROBLEMS}  shape-breaking class (declared:${SELF_CLASSES:- <none>}). A rename, removal, type change, or"
                PROBLEMS="${PROBLEMS}"$'\n'
                PROBLEMS="${PROBLEMS}  envelope change is the M class: the bump, the changelog, and the client-release plan are"
                PROBLEMS="${PROBLEMS}"$'\n'
                PROBLEMS="${PROBLEMS}  owed before merge (spec §4; the PR #858 class)."
                PROBLEMS="${PROBLEMS}"$'\n'
            fi
            ;;
        unchanged)
            if classes_contain "$SELF_CLASSES" "additive"; then
                SHAPE_NOTE="  no shape movement (.info.version only) with additive declared — the D-S3 baseline: the"
                SHAPE_NOTE="${SHAPE_NOTE}"$'\n'
                SHAPE_NOTE="${SHAPE_NOTE}  version bump re-staled the doc and the generated cores; the P floor is on the record."
            elif classes_contain "$SELF_CLASSES" "shape-breaking"; then
                SHAPE_NOTE="  no shape movement, but the register declares shape-breaking: PASS, NOTED. The stripped"
                SHAPE_NOTE="${SHAPE_NOTE}"$'\n'
                SHAPE_NOTE="${SHAPE_NOTE}  diff is the only shape record CI has, so a genuine break can hide from it; failing an"
                SHAPE_NOTE="${SHAPE_NOTE}"$'\n'
                SHAPE_NOTE="${SHAPE_NOTE}  honest over-declaration would train under-declaration (the sqlx gate's asymmetry)."
            else
                PROBLEMS="${PROBLEMS}  openapi.json changed and this PR's register row(s) declare neither additive nor"
                PROBLEMS="${PROBLEMS}"$'\n'
                PROBLEMS="${PROBLEMS}  shape-breaking (declared:${SELF_CLASSES:- <none>}). A version bump alone is additive — it"
                PROBLEMS="${PROBLEMS}"$'\n'
                PROBLEMS="${PROBLEMS}  forces the P floor at the next release — and 'behavioral' does not answer the shape"
                PROBLEMS="${PROBLEMS}"$'\n'
                PROBLEMS="${PROBLEMS}  question the openapi diff poses (spec §4)."
                PROBLEMS="${PROBLEMS}"$'\n'
            fi
            ;;
        *)
            PROBLEMS="${PROBLEMS}  openapi.json changed but the shape diff could not be computed (jq missing, unreadable"
            PROBLEMS="${PROBLEMS}"$'\n'
            PROBLEMS="${PROBLEMS}  base, or invalid JSON). 'I could not look' is not 'nothing moved' — resolve it and re-run."
            PROBLEMS="${PROBLEMS}"$'\n'
            ;;
    esac
fi

echo
if [ -n "$PROBLEMS" ]; then
    echo "wire-class-crosscheck: FAILED"
    echo
    printf '%s' "$PROBLEMS"
    if [ -n "$SHAPE_NOTE" ]; then printf '%s\n' "$SHAPE_NOTE"; echo; fi
    echo
    echo "The declared-class gate: spec of record temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md §4."
    echo "The register header (RELEASE_REGISTER.md) documents the machine fields and the vocabularies."
    exit 1
fi

echo "wire-class-crosscheck: PASS — presence, parse, and shape honesty hold (register row and wire diff agree)."
[ -n "$SHAPE_NOTE" ] && { printf '%s\n' "$SHAPE_NOTE"; echo; }
echo "Verified: the mechanical half only. CI is structurally barred from certifying the behavioral"
echo "class — behavior is not in the diff; that half is owned by review and the merge decision (spec §4)."
exit 0
