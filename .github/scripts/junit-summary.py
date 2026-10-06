#!/usr/bin/env python3
"""Summarize one nextest JUnit report, and name every flaky test where a reader of the run sees it.

CI runs nextest under `--profile ci`, which sets `retries = 1` (`.config/nextest.toml`). A test that
fails and then passes on the retry therefore no longer turns the run red — so unless something says
otherwise, it reads exactly like a first-try pass. This script is what says otherwise:

  * every flaky testcase becomes a `::warning` annotation (shown on the run and the PR checks page)
    and a line in the job summary;
  * the summary states the counts, and says "no flaky tests" explicitly when there are none, so an
    absent list is never mistaken for an unexamined one.

nextest's JUnit shape (verified against nextest 0.9.132): a flaky test is a `<testcase>` carrying one
`<flakyFailure>` per failed attempt and no `<failure>`; a test that failed every attempt carries
`<failure>` (plus `<rerunFailure>` for the retries).

A flake is reported, never failed: failing on it would re-create the red-on-flake the retry exists
to remove. The run's verdict stays nextest's.

Usage:
    python3 junit-summary.py --junit junit/unit.xml --name unit [--summary "$GITHUB_STEP_SUMMARY"]

Exit codes:
    0 - the report was read (flakes, if any, are listed — not failed)
    2 - the report is missing, unparseable, or holds no testcases. An empty report is a job that
        tested nothing, which is a finding, not a pass.
"""

import argparse
import os
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def parse_args():
    parser = argparse.ArgumentParser(description="Summarize a nextest JUnit report")
    parser.add_argument("--junit", type=Path, required=True)
    parser.add_argument("--name", required=True, help="the job/invocation label, e.g. integration-2")
    parser.add_argument(
        "--summary",
        type=Path,
        default=Path(os.environ["GITHUB_STEP_SUMMARY"]) if os.environ.get("GITHUB_STEP_SUMMARY") else None,
        help="markdown file to append to (defaults to $GITHUB_STEP_SUMMARY; stdout if unset)",
    )
    return parser.parse_args()


def unusable(msg: str) -> None:
    print(f"::error title=JUnit unusable::{msg}")
    print(f"Error: {msg}", file=sys.stderr)
    sys.exit(2)


def main() -> None:
    args = parse_args()
    if not args.junit.is_file():
        unusable(f"no JUnit report at {args.junit} for {args.name} (did nextest run under --profile ci?)")
    try:
        root = ET.parse(args.junit).getroot()
    except ET.ParseError as e:
        unusable(f"{args.junit} is not parseable XML: {e}")

    cases = list(root.iter("testcase"))
    if not cases:
        unusable(f"{args.junit} holds no testcases — {args.name} tested nothing")

    passed, failed, flaky = 0, 0, []
    for case in cases:
        label = f"{case.get('classname', '?')} {case.get('name', '?')}"
        if case.find("failure") is not None or case.find("error") is not None:
            failed += 1
        elif case.find("flakyFailure") is not None:
            flaky.append((label, len(case.findall("flakyFailure"))))
        else:
            passed += 1

    for label, attempts in flaky:
        print(f"::warning title=Flaky test ({args.name})::{label} passed only after {attempts} failed attempt(s)")

    lines = [
        f"### Tests — {args.name}",
        "",
        f"{len(cases)} testcases: {passed} passed first try, {len(flaky)} flaky, {failed} failed.",
        "",
    ]
    if flaky:
        lines.append("**Flaky — passed only on retry. Not a clean pass:**")
        lines.append("")
        lines += [f"- `{label}` ({attempts} failed attempt(s))" for label, attempts in flaky]
    else:
        lines.append("No flaky tests.")
    text = "\n".join(lines) + "\n\n"

    if args.summary:
        with args.summary.open("a") as f:
            f.write(text)
    else:
        sys.stdout.write(text)


if __name__ == "__main__":
    main()
