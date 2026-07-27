#!/usr/bin/env python3
"""Report clause coverage and citation integrity for an outcome register.

An outcome register is the body of a `goal` resource. Its clauses state what must
be true (and what must never become true); tasks cite those clauses through
`open_meta.witnesses` / `open_meta.enables`. This script answers the question the
discipline requires a register to answer about itself:

    which clauses have a witness, which do not, and is that declared?

Discipline this script obeys, and why it matters here
-----------------------------------------------------

**It detects; it does not decide.** An uncovered clause is never an error. A
clause with no witness is frequently *correct* — the mechanism it needs may not
exist yet, in which case filing a witness against it would violate
`no-witness-precedes-its-mechanism`. So uncovered clauses are reported and never
set a non-zero exit. Only *factual* defects do, under --strict: a citation naming
a clause the register does not contain, or a citation whose goal link is missing
or points somewhere else.

**Coverage is never inferred from absence.** Every place this script cannot see
is reported as unknown rather than as clean. If the clause headings cannot be
parsed it says so and refuses to report zero; if a check is outside its domain it
says that too.

Checks
------

1. **Coverage** — each clause in the register, with the tasks citing it, split by
   `witnesses` (evidence) and `enables` (builds the mechanism; not evidence, and
   deliberately not counted as coverage).

2. **Dangling citations** — a task citing a clause name that does not appear in
   the register. This is the check that caught a real defect on its first run.

3. **Citation integrity** — goal membership has two spellings: the `advances`
   edge (the only thing `resource list --goal` filters on) and
   `open_meta.witnesses.goal` (the citation). Nothing ties them, so this compares
   them per task and reports MISSING (no advances edge) separately from DIVERGES
   (an advances edge that points at a different goal). The distinction is
   load-bearing — see the note on 019f975e below.

4. **Closure staleness** — set difference of the register's closure declaration
   against the project taxonomy. Reported as OUT OF DOMAIN when the register
   enumerates its actors in place, which is a complete and valid register. It is
   never silently counted clean.

Usage:
  python3 scripts/register-coverage.py <goal-ref>
  python3 scripts/register-coverage.py <goal-ref> --strict   # non-zero on integrity defects
  python3 scripts/register-coverage.py <goal-ref> --json
"""

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field

# Clause headings in a register body, e.g.
#   ### `no-clause-names-a-mechanism`
#   ### ~~`correction-costs-less-than-the-drift`~~ — WITHDRAWN 2026-07-27
# The strikethrough marks a clause withdrawn from force. It stays parsed, because
# a citation still pointing at a withdrawn clause is a finding, not a dangling
# name — a reader must be able to tell those two apart.
CLAUSE_RE = re.compile(r"^#{2,4}\s+(~~)?`([a-z0-9]+(?:-[a-z0-9]+)+)`(~~)?")

# Section headings that introduce clauses. Used only to sanity-check that the
# body looks like a register at all before reporting a clause count.
CLAUSE_SECTION_RE = re.compile(
    r"^#{1,3}\s+.*(what must be true|what must never become true|the clauses|negative face)",
    re.IGNORECASE,
)


# A cancelled witness is a **named remainder**, not coverage. Retiring a witness is legitimate and
# often correct; what must never happen is the clause it pointed at silently reading as covered
# afterwards. So retired witnesses stay *listed* (the remainder must remain visible) and are excluded
# from the covered/uncovered verdict. Every other stage — backlog, in-progress, done — counts: a filed
# witness is a live claim of coverage even before it runs.
RETIRED_STAGES = frozenset({"cancelled"})


@dataclass
class Clause:
    name: str
    withdrawn: bool = False
    witnesses: list = field(default_factory=list)
    enables: list = field(default_factory=list)

    @property
    def live_witnesses(self) -> list:
        return [w for w in self.witnesses if w.stage not in RETIRED_STAGES]

    @property
    def retired_witnesses(self) -> list:
        return [w for w in self.witnesses if w.stage in RETIRED_STAGES]

    @property
    def covered(self) -> bool:
        return bool(self.live_witnesses)


@dataclass
class Citation:
    task_id: str
    title: str
    stage: str
    kind: str  # "witnesses" | "enables"
    goal: str
    clauses: list


def run_temper(args: list[str]) -> dict:
    """Invoke the temper CLI and parse its JSON record.

    `temper` writes logs to stderr and may append trailing output after the JSON
    document, so this decodes the first complete value rather than the whole
    stream.
    """
    proc = subprocess.run(
        ["temper", *args, "--format", "json"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise SystemExit(
            f"temper {' '.join(args)} failed ({proc.returncode}):\n{proc.stderr.strip()}"
        )
    try:
        return json.JSONDecoder().raw_decode(proc.stdout)[0]
    except json.JSONDecodeError as exc:
        raise SystemExit(f"temper {' '.join(args)} returned unparseable output: {exc}")


def parse_clauses(body: str) -> tuple[dict, bool]:
    """Extract clause names from a register body.

    Returns (clauses, looked_like_a_register). The second value exists so the
    caller can distinguish "this register declares no clauses" from "this body
    was not parseable as a register" — reporting the latter as zero coverage
    would be inferring coverage from absence.
    """
    clauses: dict = {}
    saw_clause_section = False
    for line in body.splitlines():
        if CLAUSE_SECTION_RE.match(line):
            saw_clause_section = True
        match = CLAUSE_RE.match(line)
        if match:
            name = match.group(2)
            if name not in clauses:
                clauses[name] = Clause(name=name, withdrawn=bool(match.group(1)))
    return clauses, saw_clause_section


def collect_citations(context_ref: str, goal_id: str) -> list:
    """Every task in the context whose open_meta cites this goal.

    Deliberately reads the *citation*, not `list --goal`. A query built on
    `--goal` reads only the edge, and the two spellings are exactly what check 3
    exists to compare — sourcing the population from one of them would make the
    check unable to fail in one direction.
    """
    listing = run_temper(
        ["resource", "list", "--type", "task", "--context", context_ref, "--meta-only", "--all"]
    )
    if listing.get("truncated"):
        raise SystemExit(
            "task listing was truncated; refusing to report coverage from a partial page"
        )

    citations = []
    for row in listing.get("rows", []):
        open_meta = row.get("open_meta") or {}
        for kind in ("witnesses", "enables"):
            block = open_meta.get(kind)
            if not isinstance(block, dict) or block.get("goal") != goal_id:
                continue
            citations.append(
                Citation(
                    task_id=row["id"],
                    title=row.get("title", ""),
                    stage=row.get("stage") or "-",
                    kind=kind,
                    goal=block["goal"],
                    clauses=list(block.get("clauses") or []),
                )
            )
    return citations


def advances_targets(task_id: str) -> list:
    """Resource ids this task points at with an `advances` edge."""
    record = run_temper(["resource", "show", task_id, "--edges"])
    edges = (record.get("edges") or {}).get("outgoing", [])
    return [e["peer_resource_id"] for e in edges if e.get("label") == "advances"]


def build_report(goal_ref: str) -> dict:
    goal = run_temper(["resource", "show", goal_ref])
    if goal.get("doc_type_name") != "goal":
        raise SystemExit(
            f"{goal_ref} is a {goal.get('doc_type_name')}, not a goal; a register lives on a goal"
        )

    clauses, saw_clause_section = parse_clauses(goal.get("content") or "")
    citations = collect_citations(goal["context_ref"], goal["id"])

    dangling = []
    for citation in citations:
        for clause_name in citation.clauses:
            clause = clauses.get(clause_name)
            if clause is None:
                dangling.append((citation, clause_name))
            elif citation.kind == "witnesses":
                clause.witnesses.append(citation)
            else:
                clause.enables.append(citation)

    integrity = []
    for citation in citations:
        targets = advances_targets(citation.task_id)
        if citation.goal in targets:
            verdict = "OK"
        elif not targets:
            verdict = "MISSING"
        else:
            verdict = "DIVERGES"
        integrity.append((citation, verdict, targets))

    return {
        "goal": goal,
        "clauses": clauses,
        "saw_clause_section": saw_clause_section,
        "citations": citations,
        "dangling": dangling,
        "integrity": integrity,
    }


def print_report(report: dict) -> None:
    goal = report["goal"]
    clauses = report["clauses"]

    print(f"Register: {goal['title']}")
    print(f"          {goal['id']}  ({goal['context_ref']})")
    print()

    if not clauses:
        if report["saw_clause_section"]:
            print("CLAUSES: none parsed, though a clause section is present.")
            print("  The register may use a heading form this script does not recognise.")
        else:
            print("CLAUSES: this body does not parse as a register.")
        print("  Reporting no coverage would infer coverage from absence. Nothing is claimed here.")
        return

    print("── Coverage " + "─" * 68)
    print("  `enables` tasks build a clause's mechanism. They are listed, and they are")
    print("  deliberately NOT counted as coverage — that would be the false positive")
    print("  `no-clause-is-uncovered-silently` forbids.")
    print()
    for clause in clauses.values():
        mark = "WITHDRAWN" if clause.withdrawn else ("covered" if clause.covered else "UNCOVERED")
        suffix = ""
        if not clause.withdrawn and clause.retired_witnesses:
            suffix = f"  ({len(clause.retired_witnesses)} retired — remainder, not coverage)"
        print(f"  {clause.name}  [{mark}]{suffix}")
        for citation in clause.live_witnesses:
            print(f"      witness  {citation.task_id}  ({citation.stage})  {citation.title[:60]}")
        for citation in clause.retired_witnesses:
            print(f"      RETIRED  {citation.task_id}  ({citation.stage})  {citation.title[:60]}")
        for citation in clause.enables:
            print(f"      enables  {citation.task_id}  ({citation.stage})  {citation.title[:60]}")
        if not clause.witnesses and not clause.enables:
            print("      (no citations)")
    print()
    uncovered = [c for c in clauses.values() if not c.covered and not c.withdrawn]
    print(f"  {len(clauses)} clauses · {len(uncovered)} uncovered")
    if uncovered:
        print("  An uncovered clause is not a defect. Check the register declares each one,")
        print("  and says why — that declaration is the discipline, not this script's output.")
    print()

    print("── Dangling citations " + "─" * 58)
    if report["dangling"]:
        for citation, clause_name in report["dangling"]:
            print(f"  DANGLING  {citation.task_id} cites `{clause_name}`, absent from the register")
            print(f"            {citation.title[:70]}")
    else:
        print("  none")
    print()

    print("── Citation integrity " + "─" * 58)
    print("  The citation (open_meta) and the link (advances edge) are two spellings of")
    print("  goal membership with nothing tying them. Compared per task:")
    print()
    for citation, verdict, targets in report["integrity"]:
        if verdict == "OK":
            continue
        print(f"  {verdict}  {citation.task_id}  ({citation.stage})")
        print(f"            {citation.title[:70]}")
        print(f"            cites goal   {citation.goal}")
        if targets:
            for target in targets:
                print(f"            advances     {target}")
        else:
            print("            advances     (no advances edge)")
    clean = sum(1 for _, v, _ in report["integrity"] if v == "OK")
    print(f"  {clean}/{len(report['integrity'])} citations agree with their edge")
    print()

    print("── Closure staleness " + "─" * 59)
    print("  OUT OF DOMAIN — this check is a set difference against the project taxonomy")
    print("  (`domain` resources), which does not exist yet. A register that enumerates its")
    print("  situated actors in place is complete and valid; it is simply outside what this")
    print("  check can read. Not counted clean.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("goal_ref", help="goal ref — a UUID or the decorated slug-<uuid> form")
    parser.add_argument(
        "--strict",
        action="store_true",
        help="exit non-zero on integrity defects (dangling citations, missing/diverging edges). "
        "Uncovered clauses never affect the exit code — they are frequently correct.",
    )
    parser.add_argument("--json", action="store_true", help="emit the report as JSON")
    args = parser.parse_args()

    report = build_report(args.goal_ref)

    if args.json:
        print(
            json.dumps(
                {
                    "goal": {"id": report["goal"]["id"], "title": report["goal"]["title"]},
                    "clauses": [
                        {
                            "name": c.name,
                            "withdrawn": c.withdrawn,
                            "witnesses": [w.task_id for w in c.live_witnesses],
                            "retired_witnesses": [w.task_id for w in c.retired_witnesses],
                            "enables": [e.task_id for e in c.enables],
                            "covered": c.covered,
                        }
                        for c in report["clauses"].values()
                    ],
                    "dangling": [
                        {"task": c.task_id, "clause": name} for c, name in report["dangling"]
                    ],
                    "integrity": [
                        {"task": c.task_id, "verdict": v, "advances": t}
                        for c, v, t in report["integrity"]
                    ],
                    "closure_staleness": "out-of-domain: no project taxonomy",
                },
                indent=2,
            )
        )
    else:
        print_report(report)

    if args.strict:
        defects = len(report["dangling"]) + sum(
            1 for _, v, _ in report["integrity"] if v != "OK"
        )
        if defects:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
