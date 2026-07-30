#!/usr/bin/env python3
r"""Enumerate sqlx query call sites, split by macro-vs-runtime and production-vs-test.

Supports docs/development/sqlx-macro-exception-classification.md, whose counts are
otherwise unverifiable assertions. Run it: `python3 scripts/classify-sqlx-calls.py`.

Three things a flat grep gets wrong, each found by getting them wrong first:

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
"""
import re, sys, json
from pathlib import Path

ROOT = Path("/Users/petetaylor/projects/tasker-systems/temper")

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
CALL_RUNTIME = re.compile(r'\bsqlx::(' + QFN + r')\s*(?:::<[^>]*>)?\s*\(')
CALL_MACRO = re.compile(r'\b(?:sqlx::)?(' + QFN + r')!\s*[({\[]')

def main():
    rows = []
    for f in production_files():
        text = f.read_text(encoding="utf-8", errors="replace")
        spans = test_spans(text)
        for rx, is_macro in ((CALL_RUNTIME, False), (CALL_MACRO, True)):
            for m in rx.finditer(text):
                pos = m.start()
                rows.append({
                    "file": str(f.relative_to(ROOT)),
                    "line": text.count("\n", 0, pos) + 1,
                    "fn": m.group(1),
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
