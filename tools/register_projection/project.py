"""Assemble the projection: registers × clauses × the three sources that speak to coverage.

## Three sources, reported separately, never summed

They legitimately disagree, and the disagreement is the useful signal:

1. **`register_table`** — what the register's own declared-coverage table cites. Where the sources
   disagree, this is the record; the others report on the habit of citing, not on coverage.
2. **`citations`** — `open_meta.witnesses` / `open_meta.enables` on tasks. Sparse by construction:
   a whole seven-PR arc once accumulated ~235 tests with **zero** tasks declaring `witnesses`.
3. **`advancing`** — tasks carrying an `advances` edge. Per goal rather than per clause, because an
   edge names a goal and never a clause.

Collapsing these into one number destroys what makes them worth having, and picking a winner makes a
first `open_meta` citation read as a coverage improvement when nothing about coverage changed.

## Nothing here decides

An uncovered clause is not a defect — frequently it is correct, because a witness may not precede
its mechanism. An unresolved witness is not adjudicated either: this reports that a name resolves to
nothing in the searched domain, and stops. Whether that is a stale citation, a renamed test or a term
of art is a judgment, and judgment is the reader's.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .registers import read_register
from .source import Goal, Task
from .symbols import NOT_WITNESS_SHAPE, SymbolIndex


@dataclass
class WitnessView:
    name: str
    kind: str
    resolved: bool
    found_in: str | None
    matched_by: str | None = None


@dataclass
class ClauseView:
    name: str
    withdrawn: bool
    interrogatable: bool
    table_witnesses: list[WitnessView] = field(default_factory=list)
    # Backticked tokens in the witness cell that are not witness-shaped — type names, env vars,
    # migration numbers. Carried rather than discarded: a token this could not classify must not
    # vanish silently, and a reader disagreeing with the classification needs to see what was skipped.
    unclassified_tokens: list[str] = field(default_factory=list)
    citing_witnesses: list[str] = field(default_factory=list)
    citing_enables: list[str] = field(default_factory=list)


@dataclass
class RegisterView:
    goal_id: str
    goal_title: str
    context_ref: str
    clause_form: str
    witness_table: bool
    unreadable_reason: str | None
    clauses: list[ClauseView] = field(default_factory=list)
    unmatched_coverage_rows: list[str] = field(default_factory=list)
    advancing_total: int = 0
    advancing_citing_nothing: list[str] = field(default_factory=list)
    dangling_citations: list[dict] = field(default_factory=list)


def project_register(
    goal: Goal,
    tasks: list[Task],
    advancing: list[Task],
    index: SymbolIndex,
) -> RegisterView:
    read = read_register(goal.body)
    declared = set(read.clauses)

    view = RegisterView(
        goal_id=goal.id,
        goal_title=goal.title,
        context_ref=goal.context_ref,
        clause_form=read.report.clause_form,
        witness_table=read.report.witness_table_found,
        unreadable_reason=read.report.unreadable_reason,
        unmatched_coverage_rows=read.unmatched_rows,
        advancing_total=len(advancing),
    )

    citing_w: dict[str, list[str]] = {}
    citing_e: dict[str, list[str]] = {}
    for task in tasks:
        for kind, clauses in task.citations(goal.id):
            for clause in clauses:
                if clause not in declared:
                    # A citation naming a clause the register does not contain. Reported, because a
                    # pointer to nothing is a finding about the pair, not noise.
                    view.dangling_citations.append(
                        {"task": task.id, "clause": clause, "kind": kind}
                    )
                    continue
                (citing_w if kind == "witnesses" else citing_e).setdefault(clause, []).append(task.id)

    for name in read.clauses:
        tokens = read.table_witnesses.get(name, [])
        resolutions = [index.resolve(t) for t in tokens]
        witnesses = [r for r in resolutions if r.kind is not NOT_WITNESS_SHAPE]
        unclassified = [r.token for r in resolutions if r.kind is NOT_WITNESS_SHAPE]
        view.clauses.append(
            ClauseView(
                name=name,
                withdrawn=name in read.withdrawn,
                # "Interrogatable" is a property of THIS register's shape, not of the clause: it says
                # whether the register names its evidence somewhere a machine can read. A clause whose
                # witnesses live in prose is not less witnessed — it is less checkable, and saying so
                # is the difference between a gap and a trap.
                interrogatable=read.report.witness_table_found,
                table_witnesses=[
                    WitnessView(r.token, r.kind, r.resolved, r.found_in, r.matched_by)
                    for r in witnesses
                ],
                unclassified_tokens=unclassified,
                citing_witnesses=sorted(citing_w.get(name, [])),
                citing_enables=sorted(citing_e.get(name, [])),
            )
        )

    view.advancing_citing_nothing = sorted(t.id for t in advancing if not t.cites_anything())
    view.dangling_citations.sort(key=lambda d: (d["task"], d["clause"]))
    return view


@dataclass
class Totals:
    goals_in_context: int
    goals_projected: int
    goals_unreadable: int
    clauses: int
    clauses_withdrawn: int
    witness_citations: int
    witness_citations_unresolved: int
    registers_without_witness_table: int


def totals(views: list[RegisterView], goals_in_context: int) -> Totals:
    clauses = sum(len(v.clauses) for v in views)
    withdrawn = sum(1 for v in views for c in v.clauses if c.withdrawn)
    cited = [w for v in views for c in v.clauses for w in c.table_witnesses]
    return Totals(
        goals_in_context=goals_in_context,
        goals_projected=len(views),
        goals_unreadable=sum(1 for v in views if not v.clauses),
        clauses=clauses,
        clauses_withdrawn=withdrawn,
        witness_citations=len(cited),
        witness_citations_unresolved=sum(1 for w in cited if not w.resolved),
        registers_without_witness_table=sum(1 for v in views if not v.witness_table),
    )
