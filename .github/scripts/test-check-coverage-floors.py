#!/usr/bin/env python3
"""Harness for check-coverage-floors.py — prove it fails where it must and passes where it may.

Every failing arm is a MUTATION: break exactly one thing the checker claims to catch, and
require it to exit 1 for THAT reason. Run: python3 .github/scripts/test-check-coverage-floors.py
"""

import subprocess
import sys
import tempfile
from pathlib import Path

PASS = 0
FAIL = 1
passed = 0
failed = 0

CHECKER = Path(__file__).resolve().parent / "check-coverage-floors.py"


def run(lcov_text: str, thresholds: dict):
    with tempfile.TemporaryDirectory() as tmp:
        lcov = Path(tmp) / "lcov.info"
        lcov.write_text(lcov_text)
        thr = Path(tmp) / "coverage-thresholds.json"
        thr.write_text(_json(thresholds))
        return subprocess.run(
            [sys.executable, str(CHECKER), "--lcov", str(lcov), "--thresholds", str(thr)],
            capture_output=True,
            text=True,
        ).returncode


def _json(d: dict) -> str:
    import json

    return json.dumps(d)


LCOV = """TN:
SF:crates/alpha/src/lib.rs
DA:1,1
DA:2,0
DA:3,1
end_of_record
SF:crates/beta/src/lib.rs
DA:1,1
DA:2,1
end_of_record
SF:crates/gamma/src/lib.rs
DA:1,0
DA:2,0
end_of_record
"""


def arm(name: str, expected: int, lcov: str, thresholds: dict) -> None:
    global passed, failed
    got = run(lcov, thresholds)
    if got == expected:
        passed += 1
        print(f"  ok: {name}")
    else:
        failed += 1
        print(f"  FAIL: {name} — expected exit {expected}, got {got}")


print("=== check-coverage-floors harness ===\n")

arm("all above floor passes", PASS, LCOV, {"rust": {"alpha": 50, "beta": 50, "gamma": 0}})
arm("a crate below floor fails, named by the checker", FAIL, LCOV,
    {"rust": {"alpha": 50, "beta": 50, "gamma": 50}})
arm("floor exactly at measured coverage passes", PASS, LCOV, {"rust": {"alpha": 66, "beta": 100, "gamma": 0}})
arm("a floor LOWERED below measurement is a pass, never a failure", PASS, LCOV, {"rust": {"alpha": 60, "beta": 90, "gamma": 0}})
arm("a new crate with no floor recorded fails (coverage may never be averaged away)", FAIL, LCOV, {"rust": {"alpha": 50, "beta": 50}})
arm("a floored crate absent from the report fails", FAIL, LCOV, {"rust": {"alpha": 50, "beta": 50, "gamma": 0, "delta": 40}})
arm("an empty report is unusable, not a pass", 2, "", {"rust": {"alpha": 50}})

# Non-crate sources are excluded from the computation, not averaged in.
arm("non-crate SF paths are ignored", PASS,
    """TN:
SF:crates/alpha/src/lib.rs
DA:1,1
end_of_record
SF:target/debug/build/out.rs
DA:1,0
end_of_record
""", {"rust": {"alpha": 100}})

# llvm-cov emits ABSOLUTE paths from a local run; the crate anchor must survive that.
arm("absolute SF paths anchor on crates/", PASS,
    """TN:
SF:/Users/somebody/temper/crates/alpha/src/lib.rs
DA:1,1
DA:2,0
end_of_record
""", {"rust": {"alpha": 50}})

# tests/e2e is a crate-shaped member with its own floor vocabulary.
arm("tests/e2e reports under its own name", PASS,
    """TN:
SF:tests/e2e/tests/t.rs
DA:1,1
DA:2,0
end_of_record
""", {"rust": {"tests-e2e": 50}})

print(f"\npassed={passed} failed={failed}")
raise SystemExit(1 if failed else 0)
