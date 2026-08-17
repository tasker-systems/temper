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


def test_the_witness_column_index_does_not_latch_across_tables():
    """A second measured bug. The index persisted after its table ended, so an unrelated later table
    contributed phantom tokens — on the real register, a numbered table hundreds of lines below the
    coverage table donated `sal_norm` as a witness for a clause called `2`.

    The bite: with latching this yields a phantom entry from the second table.
    """
    body = """
### `a-real-clause`

| Clause | Witnesses | State |
|---|---|---|
| **a-real-clause** | `a_real_test` | covered |

Some prose between the tables.

| # | Thing | Note |
|---|---|---|
| **2** | `sal_norm` | unrelated |
"""
    witnesses, _, _ = parse_witness_tables(body)
    assert witnesses == {"a-real-clause": ["a_real_test"]}, (
        "a table with no witness column must contribute nothing, even after a table that had one"
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
