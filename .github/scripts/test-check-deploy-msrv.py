#!/usr/bin/env python3
"""Harness for check-deploy-msrv.py — prove it fails where it must and passes where it may.

Each case feeds the checker a synthetic `cargo metadata` document through --metadata and asserts
the exit code and, for a failing case, the reason. No cargo, no network.
Run: python3 .github/scripts/test-check-deploy-msrv.py
"""

import json
import subprocess
import sys
import tempfile
from pathlib import Path

CHECKER = Path(__file__).resolve().parent / "check-deploy-msrv.py"
ROOT = "path+file:///ws#temper-cloud@0.6.4"


def pkg_id(name: str, version: str = "1.0.0") -> str:
    return f"registry+https://github.com/rust-lang/crates.io-index#{name}@{version}"


def metadata(root_rv, packages, edges, members_extra=()):
    """packages: {name: rust_version|None}; edges: [(from, to, kinds)], kinds e.g. [None, "dev"]."""
    ids = {"temper-cloud": ROOT, **{n: pkg_id(n) for n in packages}}
    pkgs = [{"id": ROOT, "name": "temper-cloud", "version": "0.6.4", "rust_version": root_rv}]
    pkgs += [
        {"id": ids[n], "name": n, "version": "1.0.0", "rust_version": rv}
        for n, rv in packages.items()
    ]
    deps = {i: [] for i in ids.values()}
    for src, dst, kinds in edges:
        deps[ids[src]].append(
            {"pkg": ids[dst], "dep_kinds": [{"kind": k, "target": None} for k in kinds]}
        )
    return {
        "packages": pkgs,
        "workspace_members": [ROOT, *members_extra],
        "resolve": {"nodes": [{"id": i, "deps": d} for i, d in deps.items()]},
    }


def run(doc):
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "metadata.json"
        path.write_text(json.dumps(doc))
        proc = subprocess.run(
            [sys.executable, str(CHECKER), "--metadata", str(path)],
            capture_output=True,
            text=True,
        )
    return proc.returncode, proc.stdout + proc.stderr


results: list[bool] = []


def case(label, doc, want_code, want_text=None, absent_text=None):
    code, out = run(doc)
    ok = code == want_code
    if ok and want_text is not None:
        ok = want_text in out
    if ok and absent_text is not None:
        ok = absent_text not in out
    results.append(ok)
    if ok:
        print(f"  ok   {label}")
    else:
        print(f"  FAIL {label}: exit {code}, want {want_code}\n{out}")


GOOD = {"a": "1.88", "b": "1.92.0", "c": None}
GOOD_EDGES = [("temper-cloud", "a", [None]), ("a", "b", ["build"]), ("a", "c", [None])]

print("check-deploy-msrv harness")

case(
    "every declared rust-version fits: passes",
    metadata("1.92", GOOD, GOOD_EDGES),
    0,
    "none above 1.92",
)

case(
    "a normal dependency two hops down needs newer: fails naming it",
    metadata("1.92", {**GOOD, "d": "1.99"}, [*GOOD_EDGES, ("b", "d", [None])]),
    1,
    "d 1.0.0 requires rustc 1.99",
)

case(
    "a build dependency needs newer: fails (the builder compiles it too)",
    metadata("1.92", {**GOOD, "d": "1.93"}, [*GOOD_EDGES, ("temper-cloud", "d", ["build"])]),
    1,
    "d 1.0.0 requires rustc 1.93",
)

case(
    "a patch-level rust-version above the ceiling: fails",
    metadata("1.92", {**GOOD, "d": "1.92.1"}, [*GOOD_EDGES, ("a", "d", [None])]),
    1,
    "requires rustc 1.92.1",
)

case(
    "a dev-only dependency needs newer: not flagged",
    metadata("1.92", {**GOOD, "d": "1.99"}, [*GOOD_EDGES, ("temper-cloud", "d", ["dev"])]),
    0,
    absent_text="requires rustc",
)

case(
    "an edge that is dev and normal at once: followed",
    metadata("1.92", {**GOOD, "d": "1.99"}, [*GOOD_EDGES, ("temper-cloud", "d", ["dev", None])]),
    1,
    "d 1.0.0 requires rustc 1.99",
)

case(
    "a package outside the closure needs newer: not flagged",
    metadata("1.92", {**GOOD, "d": "1.99"}, GOOD_EDGES),
    0,
    absent_text="requires rustc",
)

case(
    "temper-cloud declares no rust-version: fails",
    metadata(None, GOOD, GOOD_EDGES),
    1,
    "declares no rust-version",
)

case(
    "no package in the closure declares a rust-version: fails",
    metadata("1.92", {"a": None, "c": None}, [("temper-cloud", "a", [None]), ("a", "c", [None])]),
    1,
    "the walk read nothing",
)

no_root = metadata("1.92", GOOD, GOOD_EDGES)
no_root["workspace_members"] = []
case("temper-cloud is not a workspace member: fails", no_root, 1, "expected one workspace member")

failed = results.count(False)
print(f"{len(results) - failed} passed, {failed} failed")
sys.exit(1 if failed else 0)
