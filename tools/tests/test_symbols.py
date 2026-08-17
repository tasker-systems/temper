"""Token shape and resolution.

Every token in this file was pulled from a real register's witness cell on 2026-08-17, and every
false-positive case here was actually reported as a missing witness by an earlier draft. That is the
point: the cost of this tool being wrong is not a wrong number, it is that people stop running it.
"""

import pytest

from register_projection.symbols import NOT_WITNESS_SHAPE, Resolution, SymbolIndex, classify


# ── Shape ───────────────────────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize(
    "token",
    [
        "the_stranded_mechanic_declares_no_door_at_all",
        "a_refused_plan_prints_every_refusal_and_exits_non_zero",
        "t_one",
    ],
)
def test_lowercase_snake_case_is_a_function_citation(token):
    assert classify(token) == "function"


@pytest.mark.parametrize(
    "token,why",
    [
        ("ApiError", "a type name, cited in prose about the refusal"),
        ("VERCEL_GIT_COMMIT_SHA", "an environment variable"),
        ("20260730000010", "a migration number"),
        ("temper", "the product"),
        ("temper-client", "a crate, kebab-case"),
    ],
)
def test_prose_terms_in_a_witness_cell_are_not_witnesses(token, why):
    """ALL FIVE were reported as unresolved witnesses by an earlier draft. None is evidence, and
    reporting a type name as missing evidence is how a tool earns the reputation that retires it."""
    assert classify(token) is NOT_WITNESS_SHAPE, why


@pytest.mark.parametrize(
    "token",
    [
        ".github/scripts/audit-migration-declarations.sh",
        "sqlx-wire-diff.sh",
        "crates/temper-api/src/handlers/health.rs",
        "test-rust.yml",
    ],
)
def test_paths_and_bare_filenames_are_path_citations(token):
    assert classify(token) == "path"


# ── Resolution ──────────────────────────────────────────────────────────────────────────────────


def _index() -> SymbolIndex:
    paths = frozenset({".github/scripts/sqlx-wire-diff.sh", "crates/a/src/dup.rs", "crates/b/src/dup.rs"})
    basenames: dict[str, list[str]] = {}
    for p in sorted(paths):
        basenames.setdefault(p.rsplit("/", 1)[-1], []).append(p)
    return SymbolIndex(
        functions={"a_live_test": "crates/x/tests/t.rs"},
        paths=paths,
        basenames=basenames,
        domain=["test fixture"],
    )


def test_a_function_that_exists_resolves_and_says_where():
    r = _index().resolve("a_live_test")
    assert (r.resolved, r.kind, r.found_in, r.matched_by) == (
        True,
        "function",
        "crates/x/tests/t.rs",
        "exact",
    )


def test_a_renamed_test_does_not_resolve_which_is_the_founding_defect():
    """`the_search_family_declares_SEVEN_acts...` while the live test says EIGHT — the family grew,
    the test was renamed to encode the new count, and the citation was left pointing at the old
    name. Coverage intact, pointer rotted."""
    assert _index().resolve("the_search_family_declares_seven_acts").resolved is False


def test_a_bare_filename_resolves_by_basename_and_declares_the_weaker_match():
    """Registers cite scripts by filename alone. Four such citations were reported as missing while
    the files sat in `.github/scripts/`. A basename hit is weaker than an exact one and says so."""
    r = _index().resolve("sqlx-wire-diff.sh")
    assert r.resolved is True
    assert r.matched_by == "basename"
    assert r.found_in == ".github/scripts/sqlx-wire-diff.sh"


def test_an_ambiguous_basename_resolves_without_claiming_a_single_location():
    r = _index().resolve("dup.rs")
    assert r.resolved is True
    assert r.matched_by == "basename"
    assert "2 candidates" in r.found_in


def test_a_path_that_exists_nowhere_does_not_resolve():
    """`temper-substrate/src/migrate_ledger.rs` — a real citation whose file was moved to
    `crates/temper-migrate/src/ledger.rs` by PR #591. The register records the move in its own prose
    and the citation was never updated."""
    assert _index().resolve("temper-substrate/src/migrate_ledger.rs").resolved is False


def test_a_non_witness_token_is_never_reported_as_unresolved():
    """It is not resolved, and it is not a FAILURE to resolve. The distinction is what keeps the
    unresolved count meaningful — it must contain only things that claimed to be evidence."""
    r = _index().resolve("ApiError")
    assert isinstance(r, Resolution)
    assert r.kind is NOT_WITNESS_SHAPE
    assert r.resolved is False
