"""Emit the register-coverage projection as YAML.

Usage:
    uv run --project tools register-projection --repo-root . --out docs/registers/coverage.yaml
    uv run --project tools register-projection --check    # exit non-zero if the committed file drifts

## Determinism is a clause, not a nicety

`regenerating-against-unchanged-sources-changes-nothing` is what makes a history of diffs mean
anything. If the output varied on its own, every regeneration would diff and the record this artifact
is supposed to be would become noise. So: everything is sorted by a stable key, **no timestamp is
emitted**, and the client version is deliberately kept OUT of the document — it would churn the file
on every CLI bump while saying nothing about coverage. Git supplies the timestamp; `--verbose`
supplies the client version to stderr, where it cannot pollute the artifact.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import yaml

from . import project as projection
from . import source
from .symbols import build_index

SCHEMA_VERSION = 1
DEFAULT_CONTEXT = "@me/temper"
DEFAULT_OUT = "docs/registers/coverage.yaml"

HEADER = """\
# Register coverage — GENERATED, do not edit by hand.
#
# Regenerate:  uv run --project tools register-projection
# Check:       bash .github/scripts/check-register-coverage-drift.sh
#
# WHAT THIS IS: a projection of what each outcome register CLAIMS as evidence, and whether that
# evidence resolves to anything that exists in this repository. It reports; it does not adjudicate.
#
# An `uncovered` clause is NOT a defect — a witness may not precede its mechanism, so an uncovered
# clause is frequently correct. An `unresolved` witness is a name that matched nothing in the
# searched domain (recorded below as `search_domain`); whether that is a stale citation, a renamed
# test, or a term of art is a judgment this file does not make.
#
# WHERE THE SOURCES DISAGREE, THE REGISTER'S OWN TABLE IS THE RECORD. `citations` and `advancing`
# report the habit of declaring, not coverage. They are never summed.
#
# A diff here can mean the REMOTE changed, not this repository — unlike every other drift-checked
# artifact in this tree, whose source is the tree itself.
"""


def _witness_doc(w: projection.WitnessView) -> dict:
    doc: dict = {"name": w.name, "kind": w.kind, "resolves": w.resolved}
    if w.found_in:
        doc["found_in"] = w.found_in
    if w.matched_by and w.matched_by != "exact":
        # A weaker match than an exact one, and it says so rather than reading as equivalent.
        doc["matched_by"] = w.matched_by
    return doc


def _clause_doc(c: projection.ClauseView) -> dict:
    doc: dict = {"name": c.name}
    if c.withdrawn:
        doc["withdrawn"] = True
    doc["interrogatable"] = c.interrogatable
    if c.table_witnesses:
        doc["register_table"] = [_witness_doc(w) for w in c.table_witnesses]
    if c.unclassified_tokens:
        doc["unclassified_tokens"] = c.unclassified_tokens
    if c.citing_witnesses:
        doc["citations_witnesses"] = c.citing_witnesses
    if c.citing_enables:
        doc["citations_enables"] = c.citing_enables
    return doc


def _register_doc(v: projection.RegisterView) -> dict:
    doc: dict = {
        "goal_id": v.goal_id,
        "goal_title": v.goal_title,
        "context": v.context_ref,
        "read": {
            "clause_form": v.clause_form,
            "witness_table": v.witness_table,
        },
    }
    if v.unreadable_reason:
        doc["read"]["unreadable"] = v.unreadable_reason
    doc["clauses"] = [_clause_doc(c) for c in v.clauses]
    doc["advancing"] = {
        "total": v.advancing_total,
        "citing_nothing": v.advancing_citing_nothing,
    }
    if v.unmatched_coverage_rows:
        doc["coverage_rows_naming_no_clause"] = v.unmatched_coverage_rows
    if v.dangling_citations:
        doc["citations_naming_no_clause"] = v.dangling_citations
    return doc


def build_document(context_ref: str, repo_root: str) -> dict:
    index = build_index(repo_root)
    goals, goals_in_context = source.active_goals(context_ref)
    tasks = source.tasks_in_context(context_ref)

    views = [
        projection.project_register(g, tasks, source.advancing_tasks(g.id), index) for g in goals
    ]
    views.sort(key=lambda v: v.goal_id)
    t = projection.totals(views, goals_in_context)

    return {
        "schema_version": SCHEMA_VERSION,
        "source_system": "https://temperkb.io",
        "context": context_ref,
        "search_domain": index.domain,
        "totals": {
            # Every count states its denominator. A subset reported as a bare number reads as a
            # total to the next person, and the metric that conflates them kills healthy things.
            "goals_in_context": t.goals_in_context,
            "goals_projected": t.goals_projected,
            "goals_projected_note": "active goals only; non-active goals are not read",
            "goals_yielding_no_clauses": t.goals_unreadable,
            "registers_without_a_machine_readable_witness_table": t.registers_without_witness_table,
            "clauses": t.clauses,
            "clauses_withdrawn": t.clauses_withdrawn,
            "witness_citations": t.witness_citations,
            "witness_citations_unresolved": t.witness_citations_unresolved,
        },
        "registers": [_register_doc(v) for v in views],
    }


def render(document: dict) -> str:
    body = yaml.safe_dump(
        document,
        sort_keys=False,          # insertion order is the designed order; alphabetising destroys it
        default_flow_style=False,
        allow_unicode=True,
        width=100,
    )
    return HEADER + body


def main() -> int:
    parser = argparse.ArgumentParser(description="Project register coverage into a YAML artifact.")
    parser.add_argument("--context", default=DEFAULT_CONTEXT)
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--out", default=DEFAULT_OUT)
    parser.add_argument(
        "--check",
        action="store_true",
        help="regenerate in memory and exit non-zero if the committed artifact differs",
    )
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    if args.verbose:
        print(f"client: {source.client_version()}", file=sys.stderr)

    try:
        rendered = render(build_document(args.context, args.repo_root))
    except source.SourceUnavailable as exc:
        # Loudly, and never as a pass disguised as silence.
        print(f"register-projection: SOURCE UNAVAILABLE: {exc}", file=sys.stderr)
        return 2

    out_path = Path(args.repo_root) / args.out
    if args.check:
        if not out_path.exists():
            print(f"register-projection: {args.out} does not exist", file=sys.stderr)
            return 1
        if out_path.read_text() != rendered:
            print(
                f"register-projection: {args.out} is stale — regenerate with "
                f"`uv run --project tools register-projection`",
                file=sys.stderr,
            )
            return 1
        print(f"register-projection: {args.out} is current")
        return 0

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(rendered)
    print(f"register-projection: wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
