#!/usr/bin/env python3
"""Fail when a dependency of the Vercel-built package needs a newer rustc than it declares.

Vercel's `@vercel/rust` builder compiles `temper-cloud` (bins `axum`, `mcp`, `internal`) with the
rustc it ships, not with CI's `stable`. `temper-cloud` records that compiler as its `rust-version`
in the root Cargo.toml. A dependency bump whose new version declares a higher `rust-version` builds
on CI's stable and then fails the production build: the cargo group bump that moved
html-to-markdown-rs to 3.22.0 (needs 1.99, and its new astral-html dependency 1.97) did exactly
that while CI stayed green. Resolver 3 steers a plain `cargo update` away from such versions, but
Dependabot and `cargo update --precise` do not go through it, so the lock itself is checked here.

What it does: runs `cargo metadata --format-version 1 --locked`, walks the NORMAL and BUILD
dependency closure of the package (default `temper-cloud`; dev edges are dropped, every target
platform is kept), and fails naming each package in that closure whose `rust_version` is higher
than the package's own. Build dependencies count because the builder compiles them with the same
rustc. Nothing is compiled; cargo only reads manifests.

It also fails, rather than passing, when it has nothing to compare against: the package is
missing, it declares no `rust-version`, or no package in its closure declares one (the metadata
format moved, or the walk read nothing).

NOT CHECKED:
  - A dependency that declares no `rust-version` but needs a newer compiler anyway. Cargo has
    nothing to read for it, and only a build on the declared toolchain would catch it.
  - That the declared `rust-version` is still the rustc Vercel ships. It is kept in step with
    `scripts/vercel-build.sh` by hand.
  - Per-package feature resolution. `cargo metadata` resolves features across the whole workspace,
    so the closure can include an optional dependency only another member enables. That errs
    toward failing, never toward passing.
  - Other packages' toolchains (temper-cli's release binaries build on stable).

Usage:
    python3 .github/scripts/check-deploy-msrv.py [--package temper-cloud] [--metadata FILE] [--list]

  --metadata FILE  read `cargo metadata` JSON from FILE instead of running cargo (the harness's
                   fixture override; `-` reads stdin).
  --list           print every package in the closure with its declared rust-version, then run
                   the check as usual.

Exit codes: 0 every declared rust-version fits; 1 a violation, or nothing to check; 2 usage or
cargo error.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

DEFAULT_PACKAGE = "temper-cloud"


def parse_version(text: str) -> tuple[int, int, int]:
    """`1.92` and `1.92.0` compare equal; a pre-release suffix is not a valid rust-version."""
    parts = text.strip().split(".")
    if not 1 <= len(parts) <= 3 or not all(p.isdigit() for p in parts):
        raise ValueError(f"not a rust-version: {text!r}")
    nums = [int(p) for p in parts] + [0] * (3 - len(parts))
    return (nums[0], nums[1], nums[2])


def load_metadata(source: str | None) -> dict:
    if source == "-":
        return json.load(sys.stdin)
    if source is not None:
        return json.loads(Path(source).read_text())
    root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True
    ).stdout.strip()
    proc = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        print("check-deploy-msrv: cargo metadata failed (see above)", file=sys.stderr)
        sys.exit(2)
    return json.loads(proc.stdout)


def closure(metadata: dict, root_id: str) -> list[str]:
    """Package ids reachable from root_id over normal and build edges, root included."""
    nodes = {node["id"]: node for node in (metadata.get("resolve") or {}).get("nodes", [])}
    if root_id not in nodes:
        raise SystemExit(f"check-deploy-msrv: FAIL — {root_id} has no node in the resolve graph")
    seen = {root_id}
    stack = [root_id]
    while stack:
        for dep in nodes[stack.pop()].get("deps", []):
            # `kind` is null for a normal edge, "build" or "dev" otherwise. An edge is followed
            # when any of its kinds is not dev.
            if not any(k.get("kind") != "dev" for k in dep.get("dep_kinds", [])):
                continue
            if dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    return sorted(seen)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--package", default=DEFAULT_PACKAGE)
    ap.add_argument("--metadata", help="cargo metadata JSON file instead of running cargo")
    ap.add_argument("--list", action="store_true", help="print the closure before checking")
    args = ap.parse_args()

    metadata = load_metadata(args.metadata)
    packages = {p["id"]: p for p in metadata.get("packages", [])}
    members = set(metadata.get("workspace_members", []))
    roots = [p for p in packages.values() if p["name"] == args.package and p["id"] in members]
    if len(roots) != 1:
        print(
            f"check-deploy-msrv: FAIL — expected one workspace member named {args.package}, "
            f"found {len(roots)}",
            file=sys.stderr,
        )
        return 1
    root = roots[0]
    if not root.get("rust_version"):
        print(
            f"check-deploy-msrv: FAIL — {args.package} declares no rust-version, so there is "
            "nothing to check its dependencies against. Declare the rustc Vercel builds it with.",
            file=sys.stderr,
        )
        return 1
    ceiling = parse_version(root["rust_version"])

    ids = closure(metadata, root["id"])
    declared = 0
    offenders = []
    for pkg_id in ids:
        pkg = packages[pkg_id]
        rv = pkg.get("rust_version")
        if args.list:
            print(f"{pkg['name']}\t{pkg['version']}\t{rv or '-'}")
        if pkg_id == root["id"] or not rv:
            continue
        declared += 1
        if parse_version(rv) > ceiling:
            offenders.append((pkg["name"], pkg["version"], rv))

    if declared == 0:
        print(
            f"check-deploy-msrv: FAIL — none of the {len(ids) - 1} packages in {args.package}'s "
            "closure declares a rust-version; the walk read nothing to check.",
            file=sys.stderr,
        )
        return 1

    if offenders:
        print(
            f"check-deploy-msrv: FAIL — {args.package} declares rust-version "
            f"{root['rust_version']} (the rustc Vercel builds it with), but these packages in its "
            "normal/build dependency closure need newer:",
            file=sys.stderr,
        )
        for name, version, rv in offenders:
            print(f"  {name} {version} requires rustc {rv}", file=sys.stderr)
        print(
            "Hold each at its newest release that fits (`cargo update -p <crate>` picks it under "
            "resolver 3), and add a Dependabot ignore for the versions above.",
            file=sys.stderr,
        )
        return 1

    print(
        f"check-deploy-msrv: ok — {declared} of {len(ids) - 1} packages in {args.package}'s "
        f"closure declare a rust-version, none above {root['rust_version']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
