#!/usr/bin/env python3

"""Lint the operation summaries in an emitted OpenAPI document.

An operation's `summary` is not internal prose. It is the page title, the nav
label, the URL slug and the meta-description on the published Apidog site --
which means a handler doc comment written for the next implementer is one
generation step away from being a public heading. This script reads the
*emitted* `openapi.json`, so it makes no assumption about how the summary got
there (utoipa doc comments, hand-authored spec, anything else). It checks what
actually ships.

Discipline, inherited verbatim from `scripts/register-coverage.py`:

    It detects; it does not decide.
    Coverage is never inferred from absence.

Concretely:

  * A summary this script cannot read is reported as UNKNOWN, never as clean.
  * If the document will not parse, or contains no operations, the script
    REFUSES to report rather than reporting zero findings. Reporting zero would
    be inferring cleanliness from absence.
  * Findings are split into DEFECTS (factual: the published title is wrong or
    leaks) and REPORTS (editorial: the title is inelegant). `--strict` fails on
    defects only. A report is never an error, for the same reason an uncovered
    page is not: the summary may be long because the endpoint genuinely needs
    it, and a count is not a verdict.
  * Existing defects are baselined and only *growth* fails, so the gate can be
    adopted before the backlog is cleared. The baseline is never silently
    rewritten -- a stale entry is reported, and `--update-baseline` is an
    explicit act.

Exit codes:
    0  no new defects (and, under --strict, no defects at all)
    1  new defects against the baseline, or any defect under --strict
    2  refusal -- the document could not be read honestly

Usage:
    openapi-summary-lint.py openapi.json
    openapi-summary-lint.py openapi.json query-openapi.json --strict
    openapi-summary-lint.py openapi.json --update-baseline
    openapi-summary-lint.py openapi.json --json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterator

# --------------------------------------------------------------------------
# Thresholds
# --------------------------------------------------------------------------

# Editorial ceiling for a nav label. Above this a title stops working as a
# sidebar entry; it is not by itself a defect.
NAV_LABEL_MAX = 60

# Apidog truncates long titles. The observed cut on docs.temperkb.io landed
# around 250 characters (mid-word, in the middle of a leaked directive). This
# threshold is calibrated from that observation, NOT from documented behaviour
# -- it is set below the observed cut to leave margin. A summary past it will
# be published truncated, which makes it a factual defect rather than a style
# note: the page title will not be a complete sentence.
TRUNCATION_RISK = 200

HTTP_METHODS = (
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
)

DEFAULT_BASELINE = Path(".github/baselines/openapi-summaries.json")

# --------------------------------------------------------------------------
# Detection patterns
# --------------------------------------------------------------------------

# A Rust path (`handlers::edges::assert`, `temper_substrate::keys`). A summary
# that names a module path is addressed to whoever maintains the code, not to
# whoever reads the docs. This is the single strongest signal that internal
# prose has reached a public title, and it is what would have caught the
# auditor-verdict leak.
RUST_PATH = re.compile(r"\b[a-z_][a-z0-9_]*::[a-z_][a-z0-9_]*", re.IGNORECASE)

# All-caps directives addressed to an implementer. Deliberately a closed list:
# "MUST" and "SHALL" are excluded because they are legitimate RFC-2119 contract
# language in an API description.
DIRECTIVE = re.compile(
    r"\b(CONFORM|TODO|FIXME|XXX|HACK|NOTE|WARNING|SAFETY|INVARIANT)\b"
)

# Rust attribute syntax leaking through.
ATTRIBUTE = re.compile(r"#\[")

# Markdown that will render literally, because summary is a plain-text field.
# Underscores are NOT matched: snake_case identifiers are common and legitimate.
MARKDOWN = re.compile(r"`|\*\*|\[[^\]]+\]\([^)]*\)")

# A bare code identifier standing in for a title: `list_teams`, `auditorDispatch`.
# Detecting the *shape* rather than comparing against the operation id matters --
# comparing normalised strings flags "List teams" as an echo of `list_teams`,
# which is a false positive on a perfectly good title. An identifier has no
# whitespace and carries a case or underscore boundary; a title has words.
BARE_IDENTIFIER = re.compile(
    r"^(?:[a-z0-9]+(?:_[a-z0-9]+)+|[a-z]+(?:[A-Z][a-z0-9]*)+)$"
)

# A summary that opens by restating the method and path Apidog already displays.
METHOD_PATH_PREFIX = re.compile(
    r"^\s*(GET|PUT|POST|DELETE|OPTIONS|HEAD|PATCH|TRACE)\s+/", re.IGNORECASE
)

# Sentence boundary -- the signature of a whole doc-comment paragraph having
# been swallowed into the summary instead of just its opening line.
SENTENCE_BREAK = re.compile(r"[.!?]\s+\S")


# --------------------------------------------------------------------------
# Findings
# --------------------------------------------------------------------------

DEFECTS = {
    "missing": "no summary \u2014 the published title falls back to the operation id",
    "identifier_title": "summary is a bare code identifier, not a written title",
    "internal_reference": "summary names internal code or addresses an implementer",
    "markdown": "summary contains markdown, which renders literally in a title",
    "truncation_risk": f"summary exceeds {TRUNCATION_RISK} chars and will publish truncated",
}

REPORTS = {
    "over_nav_length": f"summary exceeds {NAV_LABEL_MAX} chars; long for a nav label",
    "trailing_period": "summary ends in a period; titles are not sentences",
    "method_path_prefix": "summary restates the method and path already shown",
    "multi_sentence": "summary spans more than one sentence; likely a swallowed paragraph",
}


@dataclass
class Finding:
    key: str          # "POST /api/resources/{id}/audits"
    operation_id: str | None
    rule: str
    detail: str
    summary: str | None

    @property
    def is_defect(self) -> bool:
        return self.rule in DEFECTS


@dataclass
class Scan:
    findings: list[Finding] = field(default_factory=list)
    operations: int = 0
    unknown: list[str] = field(default_factory=list)
    sources: list[str] = field(default_factory=list)

    @property
    def defects(self) -> list[Finding]:
        return [f for f in self.findings if f.is_defect]

    @property
    def reports(self) -> list[Finding]:
        return [f for f in self.findings if not f.is_defect]

    @property
    def defect_keys(self) -> set[str]:
        return {f.key for f in self.defects}


class Refusal(Exception):
    """The document cannot be read honestly. Never downgrade this to a pass."""


# --------------------------------------------------------------------------
# Reading
# --------------------------------------------------------------------------

def load_spec(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise Refusal(f"{path}: no such file")
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise Refusal(f"{path}: unreadable ({exc})") from exc
    try:
        spec = json.loads(text)
    except json.JSONDecodeError as exc:
        raise Refusal(f"{path}: not valid JSON ({exc})") from exc
    if not isinstance(spec, dict):
        raise Refusal(f"{path}: top level is {type(spec).__name__}, expected an object")
    if "paths" not in spec:
        raise Refusal(f"{path}: no `paths` member -- this is not an OpenAPI document")
    if not isinstance(spec["paths"], dict):
        raise Refusal(f"{path}: `paths` is not an object")
    return spec


def iter_operations(spec: dict[str, Any], source: str) -> Iterator[tuple[str, dict, str | None]]:
    """Yield (key, operation, unknown_reason) for each operation in the document."""
    for path, item in spec["paths"].items():
        if not isinstance(item, dict):
            yield (f"{source}:{path}", {}, "path item is not an object")
            continue
        if "$ref" in item:
            # We do not resolve external path-item refs. Saying so is the point:
            # an operation we cannot see is unknown, never assumed clean.
            yield (f"{source}:{path}", {}, "path item is a $ref; not resolved")
            continue
        for method in HTTP_METHODS:
            op = item.get(method)
            if op is None:
                continue
            if not isinstance(op, dict):
                yield (f"{method.upper()} {path}", {}, "operation is not an object")
                continue
            yield (f"{method.upper()} {path}", op, None)


# --------------------------------------------------------------------------
# Rules
# --------------------------------------------------------------------------

def check(key: str, op: dict[str, Any]) -> list[Finding]:
    op_id = op.get("operationId")
    op_id = op_id if isinstance(op_id, str) else None
    raw = op.get("summary")

    if raw is None or not isinstance(raw, str) or not raw.strip():
        return [Finding(key, op_id, "missing", DEFECTS["missing"], None)]

    summary = raw.strip()
    out: list[Finding] = []

    def flag(rule: str) -> None:
        table = DEFECTS if rule in DEFECTS else REPORTS
        out.append(Finding(key, op_id, rule, table[rule], summary))

    if BARE_IDENTIFIER.match(summary) or (op_id and summary == op_id):
        flag("identifier_title")

    if RUST_PATH.search(summary) or DIRECTIVE.search(summary) or ATTRIBUTE.search(summary):
        flag("internal_reference")

    if MARKDOWN.search(summary):
        flag("markdown")

    if len(summary) > TRUNCATION_RISK:
        flag("truncation_risk")
    elif len(summary) > NAV_LABEL_MAX:
        flag("over_nav_length")

    if summary.endswith("."):
        flag("trailing_period")

    if METHOD_PATH_PREFIX.match(summary):
        flag("method_path_prefix")

    if SENTENCE_BREAK.search(summary):
        flag("multi_sentence")

    return out


def scan(paths: list[Path]) -> Scan:
    result = Scan()
    for path in paths:
        spec = load_spec(path)
        source = path.name
        result.sources.append(source)
        for key, op, unknown in iter_operations(spec, source):
            if unknown:
                result.unknown.append(f"{key}: {unknown}")
                continue
            result.operations += 1
            result.findings.extend(check(key, op))

    if result.operations == 0:
        raise Refusal(
            "no operations found across "
            f"{', '.join(result.sources)} -- refusing to report zero findings, "
            "because that would infer cleanliness from absence"
        )
    return result


# --------------------------------------------------------------------------
# Baseline
# --------------------------------------------------------------------------

def load_baseline(path: Path) -> set[str]:
    if not path.exists():
        return set()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise Refusal(f"{path}: baseline unreadable ({exc}) -- refusing to treat it as empty")
    keys = data.get("known_defects") if isinstance(data, dict) else data
    if not isinstance(keys, list):
        raise Refusal(f"{path}: baseline has no `known_defects` list")
    return set(keys)


def write_baseline(path: Path, result: Scan) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "_comment": (
            "Operations with known summary defects. Only growth fails the gate. "
            "Regenerate with: openapi-summary-lint.py <spec> --update-baseline"
        ),
        "sources": sorted(result.sources),
        "operations_scanned": result.operations,
        "known_defects": sorted(result.defect_keys),
    }
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------

def truncate(text: str, width: int = 96) -> str:
    return text if len(text) <= width else text[: width - 1] + "\u2026"


def render(result: Scan, new: set[str], stale: set[str], strict: bool) -> None:
    print(f"openapi summary lint \u2014 {result.operations} operations "
          f"across {', '.join(result.sources)}")
    print()

    for label, findings in (("DEFECTS", result.defects), ("REPORTS", result.reports)):
        if not findings:
            print(f"{label}: none")
            print()
            continue
        by_rule: dict[str, list[Finding]] = {}
        for f in findings:
            by_rule.setdefault(f.rule, []).append(f)
        print(f"{label} ({len(findings)} across {len({f.key for f in findings})} operations)")
        for rule, group in sorted(by_rule.items()):
            print(f"\n  {rule} \u2014 {group[0].detail}")
            for f in sorted(group, key=lambda x: x.key):
                marker = " NEW" if (label == "DEFECTS" and f.key in new) else ""
                print(f"    {f.key}{marker}")
                if f.summary is not None:
                    print(f"      {truncate(f.summary)}")
        print()

    if result.unknown:
        print(f"UNKNOWN ({len(result.unknown)}) \u2014 not inspected, not assumed clean")
        for u in result.unknown:
            print(f"    {u}")
        print()

    if stale:
        print(f"BASELINE STALE ({len(stale)}) \u2014 fixed, still listed. "
              f"Re-baseline with --update-baseline.")
        for k in sorted(stale):
            print(f"    {k}")
        print()

    if new:
        print(f"FAIL: {len(new)} operation(s) gained a summary defect since the baseline.")
    elif strict and result.defects:
        print(f"FAIL (--strict): {len(result.defect_keys)} operation(s) carry a summary defect.")
    else:
        print("PASS: no new summary defects.")


# --------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Lint operation summaries in an emitted OpenAPI document.",
    )
    ap.add_argument("spec", nargs="+", type=Path,
                    help="one or more emitted OpenAPI JSON documents")
    ap.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE,
                    help=f"baseline file (default: {DEFAULT_BASELINE})")
    ap.add_argument("--update-baseline", action="store_true",
                    help="rewrite the baseline from the current scan and exit 0")
    ap.add_argument("--strict", action="store_true",
                    help="fail on any defect, not only on growth")
    ap.add_argument("--json", action="store_true",
                    help="emit machine-readable findings instead of a report")
    args = ap.parse_args(argv)

    try:
        result = scan(args.spec)
        if args.update_baseline:
            write_baseline(args.baseline, result)
            print(f"baseline written: {args.baseline} "
                  f"({len(result.defect_keys)} known defects, "
                  f"{result.operations} operations scanned)")
            return 0
        baseline = load_baseline(args.baseline)
    except Refusal as exc:
        print(f"REFUSING TO REPORT: {exc}", file=sys.stderr)
        return 2

    new = result.defect_keys - baseline
    stale = baseline - result.defect_keys

    if args.json:
        print(json.dumps({
            "operations": result.operations,
            "sources": result.sources,
            "unknown": result.unknown,
            "new_defects": sorted(new),
            "stale_baseline": sorted(stale),
            "findings": [
                {
                    "key": f.key,
                    "operation_id": f.operation_id,
                    "rule": f.rule,
                    "severity": "defect" if f.is_defect else "report",
                    "detail": f.detail,
                    "summary": f.summary,
                }
                for f in sorted(result.findings, key=lambda x: (x.key, x.rule))
            ],
        }, indent=2))
    else:
        render(result, new, stale, args.strict)

    if new:
        return 1
    if args.strict and result.defects:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
