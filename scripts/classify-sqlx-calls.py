#!/usr/bin/env python3
r"""Enumerate sqlx query call sites, split by macro-vs-runtime and production-vs-test.

Supports internal/development/sqlx-macro-exception-classification.md, whose counts are
otherwise unverifiable assertions. Run it: `python3 scripts/classify-sqlx-calls.py`.

Four things a flat grep gets wrong, each found by getting them wrong first:

1. `#[cfg(test)] mod tests { ... }` blocks live inside the same files as production code,
   so test-module membership needs BRACE MATCHING. Matching only
   `#[cfg(all(test, feature = "test-db"))]` and missing plain `#[cfg(test)]` undercounts
   test modules badly — there are 213 of the latter and 22 of the former.

2. A path-less `\bquery\s*\(` matches reqwest's `RequestBuilder::query`. temper-client
   has NO sqlx dependency and still reported 7 "sqlx calls". Non-macro calls must
   therefore carry the `sqlx::` path — safe here because nothing bare-imports sqlx's
   query fns.

3. Macros may be bare (`query_as!`), since no other macro of that name is in scope. So
   the two spellings need two patterns, not one with an optional `!`.

4. A TURBOFISH CANNOT BE MATCHED WITH A CHARACTER CLASS. The original runtime pattern
   ended `(?:::<[^>]*>)?\s*\(`, and `[^>]*` cannot cross a nested generic: in
   `sqlx::query_as::<_, (Uuid, Option<String>, i32)>(`, it halts at the `>` closing
   `Option<String>`, the optional group then backtracks away, and `\s*\(` meets a `:`.
   The whole call site becomes invisible. That hid **10 production runtime sites** — 8 in
   `graph_service.rs`, 2 in `context_graph_service.rs` — every one of them a
   `query_as::<_, (…)>` projecting into a tuple.

   This was not a harmless undercount. The number this script reports is what Arc A's
   "the count must be 46 before the allow-list is seeded" instruction keys on, and after
   the 56 classified sites were converted the OLD pattern reported exactly 46 — the
   done-number, reached while ten unconverted sites sat behind the blind spot. An
   enumerator that backs an enforcement gate has to see the whole corpus, or the gate
   reads as coverage over a corpus it cannot see, which is the defect the gate exists to
   prevent. Angle brackets nest: count depth, don't character-class.
"""
import os, re, subprocess, sys, json
from pathlib import Path


def _repo_root():
    """The tree to scan. `SQLX_SCAN_ROOT` overrides — that is the seam a guard test points at a
    fixture directory, mirroring `MIGRATIONS_DIR` in `.github/scripts/audit-grant-sinks.sh`.

    This was a hardcoded absolute path to one developer's checkout until 2026-07-30, which meant
    the script could not run in CI at all — worth knowing, because the allow-list check is
    specified to shell out to it."""
    if env := os.environ.get("SQLX_SCAN_ROOT"):
        return Path(env)
    try:
        top = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        return Path(top)
    except (subprocess.CalledProcessError, FileNotFoundError):
        # Not in a git tree (a tarball, a fixture dir): fall back to this file's parent repo.
        return Path(__file__).resolve().parent.parent


ROOT = _repo_root()

# Production source only. `tests/` dirs anywhere and the standalone e2e crate are excluded
# by the spec: "the wire contract that can break a deploy is the running binary's".
def production_files():
    for crate in sorted((ROOT / "crates").iterdir()):
        src = crate / "src"
        if not src.is_dir():
            continue
        for f in sorted(src.rglob("*.rs")):
            yield f

CFG_TEST = re.compile(r'#\[cfg\((?:[^)]*\b)?test\b[^\]]*\]\s*(?:pub\s+)?mod\s+\w+\s*\{')

def test_spans(text):
    """Byte spans of #[cfg(...test...)] mod blocks, via brace matching."""
    spans = []
    for m in CFG_TEST.finditer(text):
        i = text.index("{", m.start())
        depth, j = 0, i
        while j < len(text):
            c = text[j]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        spans.append((m.start(), j))
    return spans

# Two distinct spellings, and conflating them is a precision bug:
#
#   NON-MACRO must carry the `sqlx::` path. `.query(...)` alone is overwhelmingly
#   reqwest's RequestBuilder::query — temper-client has no sqlx dependency at all, yet a
#   path-less pattern reports 7 "sqlx calls" there. Verified safe: nothing in the tree
#   bare-imports sqlx's query fns (`rg 'use sqlx::.*query'` is empty), so requiring the
#   path loses nothing.
#
#   MACRO may be bare — `query_as!` is unambiguous, since no other macro of that name is
#   in scope.
QFN = r'query(?:_as|_scalar|_file|_as_with|_with|_file_as|_file_scalar)?'
# The runtime scan is two-stage — see note 4 in the module docstring. Stage 1 finds the
# path + function name; stage 2 walks forward over an OPTIONAL turbofish with angle-bracket
# depth counting, then requires the open paren. A regex cannot do stage 2.
CALL_RUNTIME_HEAD = re.compile(r'\bsqlx::(' + QFN + r')\b')
CALL_MACRO = re.compile(r'\b(?:sqlx::)?(' + QFN + r')!\s*[({\[]')


def _skip_ws(text, i):
    while i < len(text) and text[i].isspace():
        i += 1
    return i


def _skip_turbofish(text, i):
    """Advance past a `::<...>` turbofish, counting `<`/`>` depth so nested generics are
    crossed. Returns the index after it, i unchanged when there is no turbofish, or None
    when the brackets never balance (malformed source — the call is not counted)."""
    i = _skip_ws(text, i)
    if not text.startswith("::<", i):
        return i
    depth, j = 0, i + 2
    while j < len(text):
        if text[j] == "<":
            depth += 1
        elif text[j] == ">":
            depth -= 1
            if depth == 0:
                return j + 1
        j += 1
    return None


def runtime_calls(text):
    """(offset, fn_name) for each runtime `sqlx::query*(...)` call — macros excluded."""
    for m in CALL_RUNTIME_HEAD.finditer(text):
        i = m.end()
        if i < len(text) and text[i] == "!":
            continue  # a macro; CALL_MACRO counts it
        j = _skip_turbofish(text, i)
        if j is None:
            continue
        j = _skip_ws(text, j)
        if j < len(text) and text[j] == "(":
            yield m.start(), m.group(1)

def main():
    rows = []
    for f in production_files():
        text = f.read_text(encoding="utf-8", errors="replace")
        spans = test_spans(text)
        found = [(pos, fn, False) for pos, fn in runtime_calls(text)]
        found += [(m.start(), m.group(1), True) for m in CALL_MACRO.finditer(text)]
        for pos, fn, is_macro in found:
            rows.append({
                "file": str(f.relative_to(ROOT)),
                "line": text.count("\n", 0, pos) + 1,
                "fn": fn,
                "macro": is_macro,
                "in_test": any(a <= pos <= b for a, b in spans),
            })
    prod = [r for r in rows if not r["in_test"]]
    macro = [r for r in prod if r["macro"]]
    runtime = [r for r in prod if not r["macro"]]
    tests = [r for r in rows if r["in_test"]]

    print(f"macro calls, production source        : {len(macro)}")
    print(f"non-macro calls, production code paths: {len(runtime)}")
    print(f"  macro calls in test modules         : {sum(1 for r in tests if r['macro'])}")
    print(f"  NON-macro calls in test modules     : {sum(1 for r in tests if not r['macro'])}")
    print(f"TOTAL call sites                      : {len(rows)}")
    print()
    from collections import Counter
    by_file = Counter(r["file"] for r in runtime)
    print("NON-MACRO by file:")
    for fpath, n in by_file.most_common():
        print(f"  {n:3}  {fpath}")

    if len(sys.argv) > 1:
        json.dump(runtime, open(sys.argv[1], "w"), indent=1)

main()
