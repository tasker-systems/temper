#!/usr/bin/env python3
"""Check per-crate coverage floors against an lcov.info report.

Reads lcov.info (as emitted by `cargo llvm-cov --lcov`), groups every `SF:` record
by the crate its path belongs to (`crates/<crate>/...` or `tests/e2e/...`), computes
per-crate line coverage, and fails — naming crate, measured coverage, and floor —
when a crate drops below the floor recorded in coverage-thresholds.json.

Floors are ADVISORY in placement: the only caller is the nightly coverage workflow
(`.github/workflows/coverage.yml`), never the PR path. A floor raise/lower is a
visible diff to the thresholds file, never a silent edit.

A crate with no floor recorded is REPORTED and treated as missing — a new crate
must be added to coverage-thresholds.json in the commit that adds it, so its
coverage is never averaged away behind the workspace aggregate.

Usage:
    python3 check-coverage-floors.py [--lcov lcov.info] [--thresholds coverage-thresholds.json]

Exit codes:
    0 - every crate at or above its floor
    1 - one or more crates below floor (or a floor is missing)
    2 - the report or thresholds file is unusable
"""

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

SF_PATH = re.compile(r"^SF:(.*)$")
DA_LINE = re.compile(r"^DA:\d+,\d+")


def parse_args():
    parser = argparse.ArgumentParser(description="Check per-crate coverage floors")
    parser.add_argument("--lcov", type=Path, default=Path("lcov.info"))
    parser.add_argument("--thresholds", type=Path, default=Path("coverage-thresholds.json"))
    return parser.parse_args()


def floor_for(thresholds: dict, crate: str) -> int | None:
    return thresholds.get("rust", {}).get(crate)


def per_crate_coverage(lcov_path: Path) -> dict[str, tuple[int, int]]:
    """crate -> (covered_lines, total_lines) from an lcov.info report."""
    if not lcov_path.exists():
        print(f"Error: lcov report not found: {lcov_path}", file=sys.stderr)
        sys.exit(2)

    stats: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    current_crate: str | None = None

    with lcov_path.open() as f:
        for raw in f:
            line = raw.rstrip("\n")
            if line == "end_of_record":
                current_crate = None
                continue
            sf = SF_PATH.match(line)
            if sf:
                current_crate = crate_from_path(sf.group(1))
                if current_crate is None:
                    # Not a workspace-crate source (e.g. build script output); ignore its lines.
                    pass
                continue
            if current_crate is not None and DA_LINE.match(line):
                count = int(line.split(",")[1])
                stats[current_crate][1] += 1
                if count > 0:
                    stats[current_crate][0] += 1
    return {crate: (covered, total) for crate, (covered, total) in stats.items() if total > 0}


def crate_from_path(sf_path: str) -> str | None:
    """The crate a source path belongs to, or None when it is not crate source.

    llvm-cov emits paths relative to the repo root in CI but ABSOLUTE from a local run;
    anchor on the first `crates/` (or `tests/e2e/`) segment either way.
    """
    idx = sf_path.find("crates/")
    if idx >= 0:
        parts = sf_path[idx:].split("/")
        if len(parts) >= 3:
            return parts[1]
        return None
    if "tests/e2e/" in sf_path:
        return "tests-e2e"
    return None


def main():
    args = parse_args()

    if not args.thresholds.exists():
        print(f"Error: thresholds file not found: {args.thresholds}", file=sys.stderr)
        sys.exit(2)

    thresholds = json.loads(args.thresholds.read_text())
    coverage = per_crate_coverage(args.lcov)

    if not coverage:
        print("Error: no per-crate coverage data parsed from the report", file=sys.stderr)
        sys.exit(2)

    floored = set(thresholds.get("rust", {}))
    measured = set(coverage)
    all_pass = True
    failures: list[tuple[str, float, int]] = []

    print("=" * 70)
    print("Per-crate coverage floors (nightly, advisory)")
    print("=" * 70)
    print()

    for crate in sorted(measured):
        covered, total = coverage[crate]
        pct = 100.0 * covered / total
        floor = floor_for(thresholds, crate)
        if floor is None:
            print(f"[?] {crate:<24} {pct:>6.2f}%  no floor recorded — add one to "
                  f"coverage-thresholds.json")
            all_pass = False
            continue
        status = "PASS" if pct >= floor else "FAIL"
        if status == "FAIL":
            all_pass = False
            failures.append((crate, pct, floor))
        print(f"[{'+' if status == 'PASS' else 'x'}] {crate:<24} {pct:>6.2f}% >= {floor:>3d}%  "
              f"[{status}]")

    floored_but_unmeasured = sorted(floored - measured)
    for crate in floored_but_unmeasured:
        print(f"[?] {crate:<24} no source in the report — is the crate still in the workspace?")
        all_pass = False

    print()
    if failures:
        print("FAILURE: crates below floor:")
        print()
        for crate, pct, floor in failures:
            print(f"  {crate}: {pct:.2f}% (need {floor - pct:.2f}% more to reach {floor}%)")
        print()
    elif all_pass:
        print("All crates pass coverage thresholds")
        print()

    sys.exit(0 if all_pass else 1)


if __name__ == "__main__":
    main()