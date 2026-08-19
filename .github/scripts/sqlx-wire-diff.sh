#!/usr/bin/env bash
# sqlx-wire-diff.sh — did this change move the wire contract between the binary and the schema?
#
# WHY THIS EXISTS
# ---------------
# Spec §2 (internal/superpowers/specs/2026-07-30-schema-binary-pairing-design.md), verbatim:
#   "Diff the `.sqlx` caches against the merge-base. A change to `describe.columns[].type_info`
#    **or** `describe.parameters.Left[]` means this PR moves the wire contract."
#
# The compiler already wrote down what every checked query expects from the schema. That record is
# the only artifact in the repo that knows the binary's side of the contract, which is what makes it
# the right thing to diff — and what makes Arc A a hard precondition: 56 production queries were
# absent from these caches until it converted them, so before Arc A this detector would have read as
# coverage over a corpus it could not see.
#
# This script produces a VERDICT. It does not decide whether the verdict is acceptable — that is the
# asymmetric cross-check against the migration's declaration, which consumes this.
#
# WHAT THE KEYING MAKES TRUE
# --------------------------
# A cache entry's filename is `query-<sha256 of the query text>.json`, and that is the whole key.
# Verified 2026-07-31 across `.sqlx`: filename hash == the `hash` field == sha256(query), 351
# distinct queries, zero duplicate query texts.
#
# So a changed query text can never appear as a MODIFIED entry — it is a new file plus a deleted
# one. A modified entry means the same query text now describes differently, and the only thing that
# can cause that is the schema moving underneath it. The keying does the discrimination that would
# otherwise need a heuristic:
#
#   modified entry  -> the schema moved under an unchanged query   -> WIRE MOVE
#   new entry       -> someone wrote a query                       -> not a move
#   deleted entry   -> someone removed a query                     -> not a move
#
# The deleted case is a decision, not an oversight: a query that no longer exists cannot be decoded
# by the new binary, and the old binary's copy of it is unaffected by anything in the cache. Removal
# is a code change. The migration-side risk of dropping what a query read is covered by the
# declaration, not here.
#
# WHAT COUNTS AS A MOVE
# ---------------------
# The verdict is triggered by `describe.columns[]` compared as the full `(ordinal, name, type_info)`
# list, and by `describe.parameters.Left[]`. That is §2's rule read completely rather than narrowed
# to one field: sqlx's macros bind results BY NAME and parameters BY POSITION, so a renamed column
# and a reordered column list break a deployed binary exactly as a retyped one does.
#
# `describe.nullable[]` is reported but does NOT trigger the verdict. A nullability flip is a real
# decode risk (`T` vs `Option<T>`), but sqlx infers nullability heuristically and conservatively, and
# folding a noisy signal into the verdict trains people to ignore the verdict. §2 did not decide it;
# this is a declared hole, printed under its own heading, not a silent narrowing.
#
# SCOPE — TWO CACHES IN, ONE OUT, ANNOUNCED EITHER WAY
# ----------------------------------------------------
# In scope: `.sqlx` and `crates/temper-services/.sqlx`. Out of scope: `tests/e2e/.sqlx`. The wire
# contract that can break a deploy is the RUNNING BINARY'S, and tests/e2e is not deployed — the same
# scoping spec §6 applies to the macro allow-list. The exclusion is printed on every run, because a
# silently-omitted cache is indistinguishable from a bug, and temper-services alone holds more
# entries than the workspace cache.
#
# USAGE
#   .github/scripts/sqlx-wire-diff.sh                              # git mode: resolve base, compare
#   .github/scripts/sqlx-wire-diff.sh --base-dir A --head-dir B    # directory mode (fixture seam)
#   WIRE_DIFF_BASE=<rev> .github/scripts/sqlx-wire-diff.sh         # pin the base explicitly
#
# EXIT CODES — a verdict, not a pass/fail
#    0  the wire contract did not move
#   10  the wire contract moved (see stdout for which entries)
#    2  the base could not be determined or read. NEVER collapsed into 0: "I could not look" is not
#      "nothing moved". This is the canary's lesson carried over — a safeguard that derives its
#      input from the healthy state cannot bootstrap from the unhealthy one.

set -uo pipefail

# Caches whose entries describe the deployed binary's contract with the schema.
IN_SCOPE_CACHES=( ".sqlx" "crates/temper-services/.sqlx" )
EXCLUDED_CACHES=( "tests/e2e/.sqlx" )

BASE_DIR=""
HEAD_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --base-dir) BASE_DIR="${2:-}"; shift 2 ;;
        --head-dir) HEAD_DIR="${2:-}"; shift 2 ;;
        *) echo "sqlx-wire-diff: unknown argument: $1" >&2; exit 2 ;;
    esac
done

# ── Git mode: materialize the base caches into a temp tree, then fall through to directory mode ──
if [ -z "$BASE_DIR" ]; then
    cd "$(git rev-parse --show-toplevel)" || exit 2

    # The base ladder. Each rung is tried in turn; the first that names a commit we can actually
    # read wins. The plan's warning is earned — the predecessor safeguard assumed a git base was
    # available in a build environment and shipped twice before measuring, because
    # VERCEL_GIT_PREVIOUS_SHA was unset exactly when it mattered and then merge-base could not
    # resolve against a shallow clone at all. So: try, verify the object is present, and say so
    # loudly when none is.
    BASE=""
    for candidate in \
        "${WIRE_DIFF_BASE:-}" \
        "${GITHUB_BASE_SHA:-}" \
        "$( [ -n "${GITHUB_BASE_REF:-}" ] && git merge-base "origin/${GITHUB_BASE_REF}" HEAD 2>/dev/null )" \
        "$(git merge-base origin/main HEAD 2>/dev/null)" \
        "$(git rev-parse HEAD~1 2>/dev/null)"
    do
        [ -z "$candidate" ] && continue
        if ! git cat-file -e "${candidate}^{commit}" 2>/dev/null; then
            # A shallow clone can name a commit it does not hold. Ask for just that one.
            git fetch --depth=1 origin "$candidate" >/dev/null 2>&1 || true
        fi
        if git cat-file -e "${candidate}^{commit}" 2>/dev/null; then BASE="$candidate"; break; fi
    done

    if [ -z "$BASE" ]; then
        echo "sqlx-wire-diff: UNDETERMINABLE — no base revision could be resolved or fetched."
        echo "  Tried, in order: \$WIRE_DIFF_BASE, \$GITHUB_BASE_SHA,"
        echo "  merge-base origin/\$GITHUB_BASE_REF, merge-base origin/main, HEAD~1."
        echo "  This is NOT a pass. A detector that cannot read its input must say so."
        exit 2
    fi

    echo "sqlx-wire-diff: base ${BASE} ($(git rev-parse --short "$BASE"))"

    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    for cache in "${IN_SCOPE_CACHES[@]}"; do
        mkdir -p "${TMP}/${cache}"
        # `git archive` over a subtree is the cheapest faithful copy; a missing subtree at the base
        # (the cache did not exist yet) is legitimate and leaves an empty dir, which reads as "every
        # entry is new" — correctly, not a move.
        git archive "${BASE}" -- "$cache" 2>/dev/null | tar -x -C "$TMP" 2>/dev/null || true
    done
    BASE_DIR="$TMP"
    HEAD_DIR="$(pwd)"
fi

if [ -z "$HEAD_DIR" ]; then
    echo "sqlx-wire-diff: --base-dir given without --head-dir" >&2
    exit 2
fi
if [ ! -d "$BASE_DIR" ] || [ ! -d "$HEAD_DIR" ]; then
    echo "sqlx-wire-diff: UNDETERMINABLE — base or head tree is not readable."
    echo "  base: ${BASE_DIR}"
    echo "  head: ${HEAD_DIR}"
    echo "  This is NOT a pass."
    exit 2
fi

for cache in "${EXCLUDED_CACHES[@]}"; do
    echo "sqlx-wire-diff: ${cache} is OUT of scope — the wire contract that breaks a deploy is the"
    echo "                running binary's, and this cache is not deployed. Announced, not silent."
done

python3 - "$BASE_DIR" "$HEAD_DIR" "${IN_SCOPE_CACHES[@]}" <<'PY'
import json, os, sys

base_root, head_root = sys.argv[1], sys.argv[2]
caches = sys.argv[3:]

def load(root, cache):
    d = os.path.join(root, cache)
    out = {}
    if not os.path.isdir(d):
        return out
    for name in sorted(os.listdir(d)):
        if not name.endswith('.json'):
            continue
        try:
            with open(os.path.join(d, name)) as fh:
                out[name] = json.load(fh)
        except (OSError, ValueError) as e:
            print(f"  !! {cache}/{name}: unreadable ({e})")
    return out

def columns(entry):
    # The full identity of a result column, in order. sqlx binds results BY NAME, so a rename is a
    # break; positions shift, so a reorder or an insertion is a break.
    return [(c.get('ordinal'), c.get('name'), c.get('type_info'))
            for c in entry.get('describe', {}).get('columns', [])]

def params(entry):
    return entry.get('describe', {}).get('parameters', {}).get('Left', [])

def nullable(entry):
    return entry.get('describe', {}).get('nullable', [])

moves, notes, new_n, gone_n = [], [], 0, 0

for cache in caches:
    b, h = load(base_root, cache), load(head_root, cache)
    new_n  += len(set(h) - set(b))
    gone_n += len(set(b) - set(h))
    for name in sorted(set(b) & set(h)):
        be, he = b[name], h[name]
        bc, hc = columns(be), columns(he)
        bp, hp = params(be), params(he)
        why = []
        if bc != hc:
            bd = {(o, n): t for o, n, t in bc}
            hd = {(o, n): t for o, n, t in hc}
            for k in sorted(set(bd) | set(hd), key=lambda x: (x[0] if x[0] is not None else -1, str(x[1]))):
                if bd.get(k) != hd.get(k):
                    why.append(f"column {k[0]} {k[1]!r}: {bd.get(k, '<absent>')} -> {hd.get(k, '<absent>')}")
        if bp != hp:
            why.append(f"parameters.Left[]: {bp} -> {hp}")
        if why:
            moves.append((cache, name, he.get('query', '').strip().replace('\n', ' ')[:110], why))
        elif nullable(be) != nullable(he):
            notes.append((cache, name, f"nullable[]: {nullable(be)} -> {nullable(he)}"))

print(f"sqlx-wire-diff: {len(caches)} cache(s) in scope; "
      f"{new_n} new entr{'y' if new_n == 1 else 'ies'}, "
      f"{gone_n} deleted (neither is a move).")

if notes:
    print()
    print(f"NOTED, not counted as a move ({len(notes)}) — nullability only. Spec §2 decided on")
    print("type_info and parameters.Left[]; sqlx infers nullability heuristically, so folding it")
    print("into the verdict would train people to ignore the verdict.")
    for cache, name, w in notes:
        print(f"  {cache}/{name}")
        print(f"      {w}")

if moves:
    print()
    print(f"THE WIRE CONTRACT MOVED — {len(moves)} entr{'y' if len(moves) == 1 else 'ies'}.")
    print("Each is a query whose text did not change while what the schema returns for it did.")
    for cache, name, query, why in moves:
        print(f"  {cache}/{name}")
        print(f"      query: {query}")
        for w in why:
            print(f"      {w}")
    print()
    print("verdict: moved")
    sys.exit(10)

print()
print("verdict: unchanged")
sys.exit(0)
PY
