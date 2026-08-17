"""Read a register's markdown body: its clause names, and the witnesses its own table names.

An outcome register is the body of a `goal` resource. Two things are extracted here and they come
from different places in the document, which is the whole reason this module exists:

- **Clause names** — the invariants the goal declares, written as headings or in prose.
- **Table witnesses** — the evidence the register's own *declared coverage state* table cites for
  each clause. This is the register's self-report, and it is the thing that rots.

Both are read; neither is adjudicated. Where this module cannot see, it says so — `ReadReport`
carries an explicit reason rather than letting an empty result read as "nothing there". Coverage is
never inferred from absence, and that rule applies to the instrument as hard as to the register.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

# ── Clause names ────────────────────────────────────────────────────────────────────────────────
#
# Carried forward from `scripts/register-coverage.py`, whose comments record why each form is here.
# **Do not narrow these to the heading form.** That version caused a register written entirely in
# prose form to parse as ZERO clauses; the tool reported it honestly, was therefore never wrong, and
# for that reason went unrun and unnoticed for six weeks. A tool that fails closed is safe, silent,
# and reads as a pass.

CLAUSE_HEADING_RE = re.compile(r"^#{2,4}\s+(~~)?`([a-z0-9]+(?:-[a-z0-9]+)+)`(~~)?")

# The same clause written in prose. Three forms are in live use and none is wrong:
#   - **no-cross-act-ranking** — No single ordered result ranks…      (bulleted, plain bold)
#   - ~~**claims-carry-standing**~~ — RETIRED …                        (bulleted, withdrawn)
#   **`subject-decides-the-door`** — an act's door set follows…        (unbulleted, backticked)
#
# The trailing ` — ` is load-bearing. Without it the pattern over-matches every bold hyphenated
# phrase after the first clause section — decision titles, terms of art, and the `| **name** |`
# cells of the register's own coverage table.
CLAUSE_PROSE_RE = re.compile(
    r"^\s*(?:[-*]\s+)?(~~)?\*\*(~~)?`?([a-z0-9]+(?:-[a-z0-9]+)+)`?(~~)?\*\*(~~)?\s+—\s"
)

CLAUSE_SECTION_RE = re.compile(
    r"^#{1,3}\s+.*(what must be true|what must never become true|the clauses|negative face)",
    re.IGNORECASE,
)

# A FOURTH form: the clause declared as a table row whose first cell IS the name.
#
#     | Clause | States |
#     |---|---|
#     | `a-write-returns-without-waiting-on-projection` | A resource-lifecycle act completes … |
#
# `[measured — 2026-08-17]` Found because a register authored in August — outcome-shaped, eight
# clauses, meaning test applied in its own prose — parsed as ZERO clauses and was about to be counted
# among the "pre-register-era goals needing retrofit". It needed nothing. **Fourth instance of this
# register's own equivalence claim failing**: *"all registers are interchangeable for the purpose of
# reading their clauses"* is false, and it keeps producing new counterexamples rather than one.
#
# The rule is deliberately strict — the WHOLE cell must be the name, once decoration is stripped. A
# looser "contains a kebab-case token" would swallow every coverage table's prose and every numbered
# table's index. It is not gated on `saw_clause_section` because a coverage table routinely sits far
# below the clause sections, and picking clause names up from there is a feature: a clause that
# appears only in a coverage table is still a clause the register declares.
CLAUSE_TABLE_CELL_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)+$")

# ── Tables ──────────────────────────────────────────────────────────────────────────────────────

_SEPARATOR_CELL_RE = re.compile(r"^:?-{2,}:?$")

# A witness token as registers actually write them: a backticked bare identifier (a test function)
# or a backticked path (a script or a file cited as the evidence). Both forms are in live use.
#
# The leading `.` is deliberate and was caught by a test: the most-cited script witness in the corpus
# is `.github/scripts/audit-migration-declarations.sh`, and a first-character class that excluded `.`
# dropped it while happily keeping the bare identifiers beside it — a partial read that looks total.
_TOKEN_RE = re.compile(r"`([A-Za-z0-9_.][A-Za-z0-9_./-]*)`")

_WITNESS_HEADER_RE = re.compile(r"witness", re.IGNORECASE)
_CLAUSE_HEADER_RE = re.compile(r"clause", re.IGNORECASE)


def _cells(line: str) -> list[str]:
    return [c.strip() for c in line.strip().strip("|").split("|")]


def _is_table_row(line: str) -> bool:
    return line.strip().startswith("|")


def _is_separator_row(line: str) -> bool:
    if not _is_table_row(line):
        return False
    cells = [c for c in _cells(line) if c]
    return bool(cells) and all(_SEPARATOR_CELL_RE.match(c) for c in cells)


def normalize_clause_name(cell: str) -> str:
    """A table's Clause cell as the clause name it refers to.

    Registers decorate the name in the cell — `**bold**`, `~~withdrawn~~`, backticks — while the
    clause heading carries only backticks. Comparing the two requires stripping both.
    """
    text = cell.strip()
    text = re.sub(r"^~~|~~$", "", text)
    text = re.sub(r"^\*\*|\*\*$", "", text)
    text = text.strip().strip("`").strip()
    text = re.sub(r"^~~|~~$", "", text)
    return text


@dataclass
class ReadReport:
    """What the reader could and could not see. Never let an empty result speak for itself."""

    clause_form: str = "none"  # heading | prose | both | none
    saw_clause_section: bool = False
    witness_table_found: bool = False
    unreadable_reason: str | None = None


@dataclass
class RegisterRead:
    clauses: list[str] = field(default_factory=list)
    withdrawn: set[str] = field(default_factory=set)
    # clause name -> witness tokens the register's own table cites for it
    table_witnesses: dict[str, list[str]] = field(default_factory=dict)
    # Rows whose Clause cell matched no declared clause. Reported, never dropped: a coverage row for
    # a clause that does not exist is a finding about the register, not noise about the reader.
    unmatched_rows: list[str] = field(default_factory=list)
    report: ReadReport = field(default_factory=ReadReport)


def parse_clause_declaration_tables(body: str) -> list[tuple[str, bool]]:
    """Clause names declared by a table that has NO witness column.

    `[measured — 2026-08-17]` A register authored in August declares its eight clauses as
    `| `clause-name` | States |` rows and nothing else. It parsed as ZERO clauses and was about to be
    counted among the pre-register-era goals needing a retrofit. It needed nothing.

    The whole cell must be the name once decoration is stripped — a looser rule swallows every
    coverage table's prose and every numbered table's index.
    """
    lines = body.splitlines()
    found: list[tuple[str, bool]] = []
    in_declaration_table = False

    for i, line in enumerate(lines):
        if not _is_table_row(line):
            in_declaration_table = False
            continue
        if _is_separator_row(line):
            continue
        following = lines[i + 1] if i + 1 < len(lines) else ""
        if _is_separator_row(following):
            cells = _cells(line)
            # TWO conditions, and the first was learned the hard way. Requiring only "no witness
            # column" made every two-column table a declaration: the OTel register's table of six
            # DEPLOYABLES contributed `temper-ui` as a clause, because a deployable name is
            # kebab-case too. The table must SAY its first column is clauses.
            in_declaration_table = bool(cells) and bool(
                _CLAUSE_HEADER_RE.search(cells[0])
            ) and not any(_WITNESS_HEADER_RE.search(c) for c in cells)
            continue
        if not in_declaration_table:
            continue
        cell = _cells(line)[0]
        name = normalize_clause_name(cell)
        if CLAUSE_TABLE_CELL_RE.fullmatch(name):
            found.append((name, cell.strip().startswith("~~")))
    return found


def parse_clauses(body: str) -> tuple[list[str], set[str], ReadReport]:
    """Clause names in declaration order, plus the withdrawn ones and what the reader saw."""
    clauses: list[str] = []
    withdrawn: set[str] = set()
    saw_section = False
    from_heading = False
    from_prose = False

    # A DECLARATION table is one with no witness column; a COVERAGE table has one. The distinction is
    # semantic, not cosmetic: the first declares clauses, the second reports on clauses declared
    # elsewhere. Treating both as declarations destroys the only check that catches a coverage row
    # naming a clause the register does not have — including a typo'd restatement of a real one.
    declared_in_tables = parse_clause_declaration_tables(body)
    from_table = bool(declared_in_tables)
    for name, is_withdrawn in declared_in_tables:
        if name not in clauses:
            clauses.append(name)
            if is_withdrawn:
                withdrawn.add(name)

    for line in body.splitlines():
        if CLAUSE_SECTION_RE.match(line):
            saw_section = True

        heading = CLAUSE_HEADING_RE.match(line)
        if heading:
            name = heading.group(2)
            if name not in clauses:
                clauses.append(name)
                from_heading = True
                if heading.group(1):
                    withdrawn.add(name)
            continue

        # Gated on having seen a clause section AND on the ` — ` separator. The section flag alone is
        # not enough: it latches and never resets, so every bold hyphenated phrase downstream of the
        # first clause section would otherwise become a candidate.
        if saw_section:
            prose = CLAUSE_PROSE_RE.match(line)
            if prose:
                name = prose.group(3)
                if name not in clauses:
                    clauses.append(name)
                    from_prose = True
                    if prose.group(1) or prose.group(2):
                        withdrawn.add(name)

    forms = [
        n for n, seen in (("heading", from_heading), ("prose", from_prose), ("table", from_table))
        if seen
    ]
    form = "+".join(forms) if forms else "none"
    return clauses, withdrawn, ReadReport(clause_form=form, saw_clause_section=saw_section)


def parse_witness_tables(body: str) -> tuple[dict[str, list[str]], list[str], bool]:
    """Witness tokens per clause, from any table declaring a witness column.

    Returns (clause -> tokens, rows whose first cell is present, whether such a table was found).

    **Header detection is structural, not lexical, and that is not a stylistic preference.** A
    markdown header row is the row immediately followed by a separator row; nothing else is. Testing
    instead for the *words* "clause" and "witness" in a row misidentifies any DATA row whose prose
    happens to discuss clauses and witnesses — which register prose does constantly. Measured on the
    search-acts register: two data rows were read as headers, the column index moved to their
    Coverage column, and one clause's fourteen witnesses silently vanished from the count while the
    row after each was read from the wrong column. Nothing said anything was wrong.

    **The column index resets when a table ends.** Latching it across tables leaks the witness column
    of one table onto the unrelated rows of the next — measured on the same register, where a
    numbered table hundreds of lines later contributed a phantom token.
    """
    lines = body.splitlines()
    witnesses: dict[str, list[str]] = {}
    first_cells: list[str] = []
    found_table = False
    witness_idx: int | None = None

    for i, line in enumerate(lines):
        if not _is_table_row(line):
            witness_idx = None  # the table ended
            continue
        if _is_separator_row(line):
            continue

        following = lines[i + 1] if i + 1 < len(lines) else ""
        if _is_separator_row(following):
            cells = _cells(line)
            hits = [j for j, c in enumerate(cells) if _WITNESS_HEADER_RE.search(c)]
            witness_idx = hits[0] if hits else None
            if witness_idx is not None:
                found_table = True
            continue

        if witness_idx is None:
            continue
        cells = _cells(line)
        if len(cells) <= witness_idx or not cells[0]:
            continue

        name = normalize_clause_name(cells[0])
        first_cells.append(name)
        tokens = _TOKEN_RE.findall(cells[witness_idx])
        if tokens:
            witnesses.setdefault(name, [])
            for tok in tokens:
                if tok not in witnesses[name]:
                    witnesses[name].append(tok)

    return witnesses, first_cells, found_table


def read_register(body: str) -> RegisterRead:
    """Read a register body, reporting what could not be read rather than returning empty."""
    clauses, withdrawn, report = parse_clauses(body)
    table_witnesses, row_names, found_table = parse_witness_tables(body)
    report.witness_table_found = found_table

    if not clauses:
        report.unreadable_reason = (
            "no clause declarations parsed"
            if report.saw_clause_section
            else "body does not parse as a register: no clause section found"
        )
    elif not found_table:
        report.unreadable_reason = (
            "no coverage table declares a witness column; this register's witnesses, if any, "
            "are stated in prose and are not machine-readable here"
        )

    declared = set(clauses)
    unmatched = sorted({n for n in row_names if n and n not in declared})

    return RegisterRead(
        clauses=clauses,
        withdrawn=withdrawn,
        table_witnesses={k: v for k, v in table_witnesses.items() if k in declared},
        unmatched_rows=unmatched,
        report=report,
    )
