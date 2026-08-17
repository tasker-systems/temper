"""Reading a register body.

Every case here is a failure that was MEASURED against a real register on 2026-08-17, not an
invented shape. Two of them are silent-wrong-answer bugs found in the first draft of this reader:
each returned a smaller number with no error, which is the exact failure mode this tool exists to
catch in registers and is therefore the one it must not commit itself.
"""

from register_projection.registers import (
    normalize_clause_name,
    parse_clauses,
    parse_witness_tables,
    read_register,
)


# ── The two measured bugs ───────────────────────────────────────────────────────────────────────


def test_a_data_row_whose_prose_mentions_clauses_and_witnesses_is_not_read_as_a_header():
    """THE BUG THIS TOOL ALMOST SHIPPED.

    Header detection was lexical — a row containing the words "clause" and "witness" was treated as
    a header. Register prose discusses clauses and witnesses constantly, so on the search-acts
    register two DATA rows matched. Each moved the column index onto its own Coverage column, which
    meant that row's witnesses were never collected and the row AFTER it was read from the wrong
    column. Fourteen witnesses for `composition-is-legible` vanished and nothing reported anything.

    The bite: under the lexical test this returns 1 witness (from the wrong column of the second
    row); under the structural test it returns both rows' real witnesses.
    """
    body = """
## Declared coverage

| Clause | Coverage | Witnesses |
|---|---|---|
| **composition-is-legible** | **covered** — the MIRROR of the defect this clause's other witnesses guard | `first_real_test` · `second_real_test` |
| **refusal-is-distinct** | **covered** | `third_real_test` |

### `composition-is-legible`
### `refusal-is-distinct`
"""
    witnesses, _, found = parse_witness_tables(body)
    assert found
    assert witnesses["composition-is-legible"] == ["first_real_test", "second_real_test"], (
        "the row's OWN witnesses must be collected; a data row mentioning 'clause' and 'witnesses' "
        "in prose is not a header"
    )
    assert witnesses["refusal-is-distinct"] == ["third_real_test"]


def test_the_witness_column_index_does_not_latch_past_the_end_of_its_table():
    """A second measured bug: the index persisted after its table ended, so later rows were read
    through a column mapping that no longer applied.

    **This test was rewritten because its first version did not bite.** It used a second table WITH
    its own header row — and a header row resets the index by itself, so reverting the reset changed
    nothing and 29 tests stayed green. It was passing for a reason unrelated to what it named, which
    is the precise failure this whole tool exists to catch, committed inside its own test suite.

    The discriminating case is a table row with NO header of its own: a stray row after prose, which
    is what a hand-written register produces when a table is edited. Only the end-of-table reset can
    stop the previous table's column mapping being applied to it.
    """
    body = """
### `a-real-clause`

| Clause | Witnesses | State |
|---|---|---|
| **a-real-clause** | `a_real_test` | covered |

Some prose, and then a stray row that never declared a header of its own.

| **2** | `sal_norm` | unrelated |
"""
    witnesses, _, _ = parse_witness_tables(body)
    assert witnesses == {"a-real-clause": ["a_real_test"]}, (
        "once a table ends, its witness column must not be applied to anything that follows"
    )


# ── Clause forms ────────────────────────────────────────────────────────────────────────────────


def test_prose_form_clauses_are_read_because_the_heading_only_version_went_unrun_for_six_weeks():
    body = """
## What must be true — the clauses

- **no-cross-act-ranking** — No single ordered result ranks across acts.
**`subject-decides-the-door`** — an act's door set follows from its subject.
- ~~**claims-carry-standing**~~ — RETIRED, relocated.
"""
    clauses, withdrawn, report = parse_clauses(body)
    assert clauses == [
        "no-cross-act-ranking",
        "subject-decides-the-door",
        "claims-carry-standing",
    ]
    assert withdrawn == {"claims-carry-standing"}
    assert report.clause_form == "prose"


def test_a_withdrawn_heading_clause_is_still_parsed_so_a_citation_to_it_is_distinguishable():
    """A citation pointing at a withdrawn clause is a finding. Dropping the clause would make that
    citation look like a dangling name instead, which is a different and less useful thing to say."""
    body = """
## The clauses
### `live-clause`
### ~~`retired-clause`~~ — WITHDRAWN 2026-07-27
"""
    clauses, withdrawn, _ = parse_clauses(body)
    assert clauses == ["live-clause", "retired-clause"]
    assert withdrawn == {"retired-clause"}


def test_the_em_dash_separator_keeps_a_coverage_table_row_from_becoming_a_clause():
    """Bold kebab-case inside a clause section over-matches without the trailing separator: the
    register's own `| **clause-name** | covered |` rows would each parse as a new clause."""
    body = """
## The clauses

- **a-genuine-clause** — states something.

## Declared coverage
| Clause | Witnesses | State |
|---|---|---|
| **a-genuine-clause** | `t` | covered |
"""
    clauses, _, _ = parse_clauses(body)
    assert clauses == ["a-genuine-clause"]


# ── Not inferring coverage from absence ─────────────────────────────────────────────────────────


def test_a_body_that_is_not_a_register_says_so_rather_than_reporting_zero_clauses():
    read = read_register("# Just a document\n\nSome prose.\n")
    assert read.clauses == []
    assert read.report.unreadable_reason is not None
    assert "does not parse as a register" in read.report.unreadable_reason


def test_a_register_with_no_witness_column_is_declared_unreadable_not_reported_as_uncovered():
    """Live shape: several registers state witnesses in prose and carry a `| Clause | State |`
    table. Reporting those clauses as having no witnesses would be a false negative that reads as a
    finding — the noise that trains a reader to stop looking."""
    body = """
## The clauses
### `some-clause`

## Declared coverage
| Clause | State |
|---|---|
| `some-clause` | witnessed, described in the prose above |
"""
    read = read_register(body)
    assert read.clauses == ["some-clause"]
    assert read.report.witness_table_found is False
    assert "not machine-readable" in read.report.unreadable_reason


def test_a_coverage_row_naming_no_declared_clause_is_reported_rather_than_dropped():
    body = """
## The clauses
### `declared-clause`

| Clause | Witnesses | State |
|---|---|---|
| **declared-clause** | `t_one` | covered |
| **never-declared** | `t_two` | covered |
"""
    read = read_register(body)
    assert read.unmatched_rows == ["never-declared"]
    assert "never-declared" not in read.table_witnesses


# ── Token and name handling ─────────────────────────────────────────────────────────────────────


def test_path_shaped_witnesses_are_read_because_registers_cite_scripts_as_evidence():
    body = """
## The clauses
### `a-clause`

| Clause | Witnesses | State |
|---|---|---|
| **a-clause** | `.github/scripts/audit-migration-declarations.sh` · `a_test_fn` | covered |
"""
    read = read_register(body)
    assert read.table_witnesses["a-clause"] == [
        ".github/scripts/audit-migration-declarations.sh",
        "a_test_fn",
    ]


def test_clause_cell_decoration_is_stripped_so_a_table_row_matches_its_heading():
    assert normalize_clause_name("**every-act-is-situated**") == "every-act-is-situated"
    assert normalize_clause_name("~~**claims-carry-standing**~~") == "claims-carry-standing"
    assert normalize_clause_name("`subject-decides-the-door`") == "subject-decides-the-door"


def test_a_repeated_token_is_recorded_once_so_a_count_is_not_inflated_by_restatement():
    body = """
## The clauses
### `a-clause`

| Clause | Witnesses | State |
|---|---|---|
| **a-clause** | `t_one` · `t_one` · `t_two` | covered |
"""
    read = read_register(body)
    assert read.table_witnesses["a-clause"] == ["t_one", "t_two"]


# ── The fourth clause form ──────────────────────────────────────────────────────────────────────


def test_clauses_declared_only_in_a_table_are_read():
    """MEASURED, 2026-08-17. An August register — outcome-shaped, eight clauses, meaning test applied
    in its own prose — declared every clause as a table row and parsed as ZERO. It was about to be
    counted among the pre-register-era goals needing a retrofit. It needed nothing.

    Fourth counterexample to this register's own equivalence claim that all registers can be read the
    same way. The claim keeps failing in a NEW way each time, which is the argument for reporting the
    form rather than assuming it."""
    body = """
## Clauses

Invariants. No mechanism named.

| Clause | States |
|---|---|
| `a-write-returns-without-waiting-on-projection` | A act completes without waiting |
| `projection-lag-is-readable` | A reader can determine how far a shape trails |
"""
    clauses, _, report = parse_clauses(body)
    assert clauses == [
        "a-write-returns-without-waiting-on-projection",
        "projection-lag-is-readable",
    ]
    assert "table" in report.clause_form


def test_a_coverage_table_reports_on_clauses_and_does_not_declare_them():
    """The discriminator is the witness column, and it is load-bearing rather than cosmetic. Reading
    a COVERAGE table as a declaration destroys the only check that catches a coverage row naming a
    clause the register does not have — including a typo'd restatement of a real one, which is the
    case most worth catching because it reads as coverage.

    The bite: without the witness-column discriminator, `a-clause-that-was-never-declared` becomes a
    clause and the unmatched-row finding disappears."""
    body = """
## The clauses
### `a-real-clause`

## Declared coverage
| Clause | Witnesses | State |
|---|---|---|
| `a-real-clause` | `t_one` | covered |
| `a-clause-that-was-never-declared` | `t_two` | covered |
"""
    read = read_register(body)
    assert read.clauses == ["a-real-clause"]
    assert read.unmatched_rows == ["a-clause-that-was-never-declared"]


def test_a_two_column_table_that_is_not_about_clauses_declares_nothing():
    """MEASURED, 2026-08-17 — a false positive this reader introduced and then removed.

    The first version of the table form required only "no witness column", which made EVERY
    two-column table a declaration. The OTel register's table of six DEPLOYABLES then contributed
    `temper-ui` as a clause, because a deployable name is kebab-case and backticked exactly like a
    clause name is. One spurious clause is worse than a missing one here: it inflates the denominator
    and it looks like a real finding.

    The bite: without the clause-column requirement, `temper-ui` is returned as a clause."""
    body = """
## The surface — six deployables

| Deployable | Note |
|---|---|
| `temper-ui` | separate Vercel project — the "two projects, one trace" problem |
| `temper-api` | the Rust half |
"""
    clauses, _, report = parse_clauses(body)
    assert clauses == [], "a table of deployables is not a table of clauses"
    assert report.clause_form == "none"
