#!/usr/bin/env python3
"""Harness for junit-summary.py — prove a flake is named, and that an empty report is never a pass.

The fixtures are nextest's real JUnit shape, captured from nextest 0.9.132 under a `ci` profile with
`retries = 1`: a flaky test carries `<flakyFailure>` and no `<failure>`; a test that failed every
attempt carries `<failure>` and `<rerunFailure>`.

Exit codes alone cannot carry this harness — a script that ignored `<flakyFailure>` entirely would
still exit 0 — so each arm also asserts what the reader is shown.
Run: python3 .github/scripts/test-junit-summary.py
"""

import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "junit-summary.py"
passed = 0
failed = 0


def suite(*cases: str) -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<testsuites name="nextest-run"><testsuite name="nx">' + "".join(cases) + "</testsuite></testsuites>"
    )


CLEAN = '<testcase name="tests::steady" classname="nx" time="0.008"></testcase>'
FLAKY = (
    '<testcase name="tests::flaky_once" classname="nx" time="0.007">'
    '<flakyFailure time="0.008" message="panicked" type="test failure with exit code 101">boom</flakyFailure>'
    "</testcase>"
)
FAILED_AFTER_RETRY = (
    '<testcase name="tests::always_fails" classname="nx" time="2.003">'
    '<failure type="test failure"/><rerunFailure time="2.006" type="test failure"></rerunFailure>'
    "</testcase>"
)


def run(xml: str | None):
    with tempfile.TemporaryDirectory() as tmp:
        junit = Path(tmp) / "junit.xml"
        if xml is not None:
            junit.write_text(xml)
        summary = Path(tmp) / "summary.md"
        r = subprocess.run(
            [sys.executable, str(SCRIPT), "--junit", str(junit), "--name", "shard-x", "--summary", str(summary)],
            capture_output=True,
            text=True,
        )
        return r.returncode, r.stdout, summary.read_text() if summary.exists() else ""


def arm(name: str, xml: str | None, code: int, stdout_has=(), stdout_lacks=(), summary_has=(), summary_lacks=()):
    global passed, failed
    got, out, summ = run(xml)
    problems = []
    if got != code:
        problems.append(f"exit {got}, expected {code}")
    problems += [f"stdout lacks {s!r}" for s in stdout_has if s not in out]
    problems += [f"stdout has {s!r}" for s in stdout_lacks if s in out]
    problems += [f"summary lacks {s!r}" for s in summary_has if s not in summ]
    problems += [f"summary has {s!r}" for s in summary_lacks if s in summ]
    if problems:
        failed += 1
        print(f"  FAIL: {name} — {'; '.join(problems)}")
    else:
        passed += 1
        print(f"  ok: {name}")


print("=== junit-summary harness ===\n")

arm("a clean report passes, says so explicitly, and warns about nothing", suite(CLEAN), 0,
    stdout_lacks=["::warning"], summary_has=["1 testcases: 1 passed first try, 0 flaky, 0 failed", "No flaky tests."])

arm("a flaky test is a ::warning naming it, and a summary line — never a silent pass", suite(CLEAN, FLAKY), 0,
    stdout_has=["::warning title=Flaky test (shard-x)::nx tests::flaky_once"],
    summary_has=["1 flaky", "`nx tests::flaky_once`", "Not a clean pass"],
    summary_lacks=["No flaky tests."])

arm("a test that failed every attempt counts as failed, not flaky", suite(CLEAN, FAILED_AFTER_RETRY), 0,
    stdout_lacks=["::warning"], summary_has=["0 flaky, 1 failed", "No flaky tests."])

arm("a flake does not fail the step (the verdict stays nextest's)", suite(FLAKY), 0)

arm("a missing report is unusable, not a pass", None, 2, stdout_has=["::error"])
arm("a report with no testcases is unusable — the job tested nothing", suite(), 2, stdout_has=["::error"])
arm("an unparseable report is unusable", "<testsuites><testcase", 2, stdout_has=["::error"])

print(f"\n{passed} passed, {failed} failed")
sys.exit(1 if failed else 0)
