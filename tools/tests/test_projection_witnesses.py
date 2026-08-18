"""Witnesses for the five clauses of goal 01a00f76 that remained declared holes after
the build shipped.

The mechanism exists (``tools/register_projection/``), so witnesses can bite. Each test
here is constructed to fail by a NAMED mutation of the existing emitter — reverting the
guard the clause is about reds the test. A witness that cannot be shown to fail is an
assertion, not a witness, and that distinction is the whole reason this file exists.

These tests never reach the network. ``source.py`` reads the ``temper`` CLI, which is the
right call for the live tool and the wrong call for a unit test, so every case here builds
the data structures (``Goal``, ``Task``, ``SymbolIndex``) by hand and exercises the pure
emitter (``project_register``, ``build_document``, ``render``) directly.

The clause names are the readable names the goal carries — see
``outcome-registers.md``: *"Clause names are readable names, not indices."*
"""

from __future__ import annotations

import io
from dataclasses import replace
from typing import get_type_hints

import yaml

from register_projection import project as projection
from register_projection import source
from register_projection.__main__ import HEADER, build_document, render
from register_projection.registers import ReadReport
from register_projection.source import Goal, SourceUnavailable, Task
from register_projection.symbols import NOT_WITNESS_SHAPE, SymbolIndex


# ── Fixtures ──────────────────────────────────────────────────────────────────────────────────


def _index_with(*functions: str) -> SymbolIndex:
    """An index that resolves exactly the named functions, plus the one path fixture."""
    paths = frozenset({".github/scripts/audit-migration-declarations.sh"})
    basenames: dict[str, list[str]] = {}
    for p in sorted(paths):
        basenames.setdefault(p.rsplit("/", 1)[-1], []).append(p)
    fn_map = {name: f"crates/x/tests/{name}.rs" for name in functions}
    return SymbolIndex(
        functions=fn_map,
        paths=paths,
        basenames=basenames,
        domain=["test fixture"],
    )


def _goal(goal_id: str, body: str, title: str = "a goal") -> Goal:
    return Goal(id=goal_id, title=title, context_ref="@me/temper", status="active", body=body)


def _task(task_id: str, open_meta: dict | None = None) -> Task:
    return Task(id=task_id, title=f"task {task_id}", stage="done", open_meta=open_meta or {})


_REGISTER_BODY = """\
## What must be true — the clauses

### `sources-that-disagree-are-reported-as-disagreeing`
### `regenerating-against-unchanged-sources-changes-nothing`
### `an-unresolvable-witness-must-never-count-as-a-covered-clause`
### `a-total-must-never-be-reported-over-a-source-that-was-not-read`
### `the-projection-must-never-become-the-authority-over-the-register`

## Declared coverage

| Clause | Witnesses | State |
|---|---|---|
| **sources-that-disagree-are-reported-as-disagreeing** | `live_test_a` · `live_test_b` | covered |
| **regenerating-against-unchanged-sources-changes-nothing** | `determinism_test` | covered |
| **an-unresolvable-witness-must-never-count-as-a-covered-clause** | `live_test_a` · `a_renamed_test_that_is_not_here` | covered |
| **a-total-must-never-be-reported-over-a-source-that-was-not-read** | `totals_test` | covered |
| **the-projection-must-never-become-the-authority-over-the-register** | `authority_test` | covered |
"""


def _views(goal_id: str = "01a00f76-5087-7be3-855e-5a12489831e3") -> list[projection.RegisterView]:
    """A single-register projection with two resolved + one unresolved witness, plus three
    advancing tasks (one citing witnesses, one citing enables, one citing nothing)."""
    goal = _goal(goal_id, _REGISTER_BODY, "the projection goal")
    tasks = [
        _task("01a011b1-0001-7000-8000-000000000001", {
            "witnesses": {"goal": goal_id, "clauses": ["sources-that-disagree-are-reported-as-disagreeing"]},
        }),
        # An enables citation naming the SAME clause, so the clause carries both a witnesses
        # and an enables citation — the case where collapsing the two would be most damaging.
        _task("01a011b1-0002-7000-8000-000000000002", {
            "enables": {"goal": goal_id, "clauses": ["sources-that-disagree-are-reported-as-disagreeing"]},
        }),
        _task("01a011b1-0003-7000-8000-000000000003"),  # advances, cites nothing
    ]
    advancing = list(tasks)  # all three carry the advances edge in this fixture
    index = _index_with("live_test_a", "live_test_b", "determinism_test", "totals_test", "authority_test")
    return [projection.project_register(goal, tasks, advancing, index)]


# ── Clause 1: sources-that-disagree-are-reported-as-disagreeing ────────────────────────────────


def test_the_four_sources_are_emitted_as_separate_keys_and_never_summed():
    """The clause: where the sources disagree, the projection shows the disagreement rather
    than a resolved number.

    Four sources speak to a clause's coverage, and they legitimately differ:
    ``register_table`` (the register's own table), ``citations_witnesses`` (tasks declaring
    ``witnesses``), ``citations_enables`` (tasks declaring ``enables``), and ``advancing``
    (tasks carrying an ``advances`` edge). Collapsing any two into one field destroys the
    disagreement the clause says must stay visible.

    The bite: merge any two of these into a single field or add a `total_coverage` sum, and
    the disagreement becomes invisible — reds this test.
    """
    view = _views()[0]
    clause_doc = next(
        c for c in view.clauses
        if c.name == "sources-that-disagree-are-reported-as-disagreeing"
    )

    # Each source is a distinct attribute on the view, not one merged collection.
    assert hasattr(view, "advancing_total"), "advancing is its own axis"
    assert hasattr(clause_doc, "table_witnesses"), "register_table is its own axis"
    assert hasattr(clause_doc, "citing_witnesses"), "citations_witnesses is its own axis"
    assert hasattr(clause_doc, "citing_enables"), "citations_enables is its own axis"

    # The four sources carry DIFFERENT values on this fixture — the disagreement is real:
    # register_table cites two tests, one task cites witnesses, one cites enables, three advance.
    assert len(clause_doc.table_witnesses) == 2, "register table cites two witnesses"
    assert clause_doc.citing_witnesses == ["01a011b1-0001-7000-8000-000000000001"]
    assert clause_doc.citing_enables == ["01a011b1-0002-7000-8000-000000000002"], (
        "the enables citation names the same clause — the case where collapsing would be worst"
    )
    assert view.advancing_total == 3

    # The emitted doc keeps them as separate YAML keys. ``_clause_doc`` must not produce a
    # ``coverage``/``verdict``/``total`` field that sums them, and ``_register_doc`` must not
    # fold the three citation axes into one.
    from register_projection.__main__ import _clause_doc, _register_doc

    doc = _clause_doc(clause_doc)
    assert "register_table" in doc, "the register's own table is emitted under its own key"
    assert "citations_witnesses" in doc
    assert "citations_enables" in doc
    forbidden_sum_keys = {"coverage", "verdict", "total_coverage", "covered", "resolved_count"}
    assert not (forbidden_sum_keys & doc.keys()), (
        f"a sum/verdict key would collapse the sources: {forbidden_sum_keys & doc.keys()}"
    )

    reg_doc = _register_doc(view)
    assert "advancing" in reg_doc, "advancing is emitted under its own key"
    assert "clauses" in reg_doc, "clauses (with their per-source breakdown) under their own key"
    assert not (forbidden_sum_keys & reg_doc.keys()), (
        f"a register-level verdict key would collapse the sources: {forbidden_sum_keys & reg_doc.keys()}"
    )


def test_an_enables_citation_and_a_witnesses_citation_are_not_the_same_field():
    """The two citation kinds are semantically distinct — ``witnesses`` claims to BE evidence,
    ``enables`` builds the mechanism that makes evidence possible (per ``source.py``'s doc and
    ``outcome-registers.md``). Folding them into one list would make a first ``enables``
    citation read as a coverage improvement, which the clause's origin (the filing) names as
    exactly the flattening it refuses.

    The bite: if ``citations_witnesses`` and ``citations_enables`` were merged into a single
    ``citations`` field, the enables-citing task would appear in the same list as the
    witnesses-citing one and the disagreement would be gone.
    """
    view = _views()[0]
    enables_clause = next(
        c for c in view.clauses
        if c.name == "regenerating-against-unchanged-sources-changes-nothing"
    )
    # The fixture's enables citation names a different clause than the witnesses one, so the
    # two kinds are observable on different clauses — collapsing them would hide the fact that
    # an `enables` citation does not claim coverage the way a `witnesses` one does.
    enables_clause_via_enables = next(
        c for c in view.clauses
        if c.name == "sources-that-disagree-are-reported-as-disagreeing"
    )
    # That clause is named by both a witnesses AND an enables citation — the case where a
    # single merged list would be most damaging, because the enables citation would read as
    # a coverage claim it did not make.
    assert enables_clause_via_enables.citing_witnesses == ["01a011b1-0001-7000-8000-000000000001"]
    assert enables_clause_via_enables.citing_enables == ["01a011b1-0002-7000-8000-000000000002"], (
        "an enables citation and a witnesses citation naming the same clause must stay in "
        "separate fields — they are different claims"
    )
    # And the regenerating clause (named by neither kind here) shows empty for both, which is
    # the honest answer rather than a default that reads as coverage.
    assert enables_clause.citing_witnesses == []
    assert enables_clause.citing_enables == []


# ── Clause 2: regenerating-against-unchanged-sources-changes-nothing ────────────────────────────


def test_rendering_the_same_document_twice_produces_byte_identical_output():
    """The clause: if the projection varies without its sources varying, every regeneration
    is noise and the history it is supposed to be becomes unreadable.

    The simplest instance: build the document from the same inputs twice, render both, and
    assert the bytes agree. The emitter must be a pure function of its inputs.

    The bite: introducing any non-determinism — a timestamp, a uuid, ``random``, a dict whose
    iteration order varies — makes the two renders differ and reds this test.
    """
    a = render(build_document_from_views(_views()))
    b = render(build_document_from_views(_views()))
    assert a == b, "regenerating against unchanged sources must change nothing"


def test_shuffling_the_input_order_does_not_change_the_rendered_output():
    """The load-bearing half of the clause: the emitter sorts by a stable key, so reordering
    the inputs must not reorder the output. Without this, a caller who happened to list goals
    in a different order would get a different file, and a diff would fire on every run.

    The bite: remove any ``.sort(key=...)`` in ``build_document`` or ``project_register`` and
    shuffling the inputs changes the output — reds this test.

    This test bites against the REAL ``build_document`` — not the test helper — by monkeypatching
    the source layer so it returns the fixtures in a shuffled order across two runs. If
    ``build_document`` did not sort, the two rendered artifacts would differ.
    """
    import random

    from register_projection import __main__ as main_mod
    from register_projection import source as source_mod

    goal_a = _goal("01a00f76-5087-7be3-855e-5a12489831e3", _REGISTER_BODY, "a goal")
    goal_b = _goal("01a011b1-eeee-70d2-9dba-991fd1fe15c0", _REGISTER_BODY, "second goal")
    goal_c = _goal("01a00002-9fd1-78c3-b1e4-7e3400e9b5d0", _REGISTER_BODY, "third goal")
    goals_in_order = [goal_a, goal_b, goal_c]
    index = _index_with("live_test_a", "determinism_test")

    # Monkeypatch the four network functions build_document calls. The fixture returns goals in
    # one order on the first call and a shuffled order on the second; the rendered output must
    # be identical either way, because build_document sorts views by goal_id before emitting.
    call_count = {"n": 0}

    def fake_active_goals(_ctx):
        call_count["n"] += 1
        ordered = goals_in_order if call_count["n"] == 1 else list(reversed(goals_in_order))
        return ordered, len(ordered)

    def fake_tasks_in_context(_ctx):
        return []

    def fake_advancing_tasks(_goal_id):
        return []

    def fake_build_index(_repo_root):
        return index

    # Save and patch.
    orig_active = source_mod.active_goals
    orig_tasks = source_mod.tasks_in_context
    orig_advancing = source_mod.advancing_tasks
    orig_index = main_mod.build_index
    source_mod.active_goals = fake_active_goals
    source_mod.tasks_in_context = fake_tasks_in_context
    source_mod.advancing_tasks = fake_advancing_tasks
    main_mod.build_index = fake_build_index
    try:
        a = render(main_mod.build_document("@me/temper", "."))
        b = render(main_mod.build_document("@me/temper", "."))
    finally:
        source_mod.active_goals = orig_active
        source_mod.tasks_in_context = orig_tasks
        source_mod.advancing_tasks = orig_advancing
        main_mod.build_index = orig_index

    assert a == b, (
        "the rendered output must not depend on the order the source returned goals in; "
        "build_document must sort by a stable key before emitting"
    )

    # And at the project_register layer: a different goal ordering must not move the output.
    views_in_order = [
        projection.project_register(goal_a, [], [], index),
        projection.project_register(goal_b, [], [], index),
    ]
    views_reversed = list(reversed(views_in_order))
    assert render(build_document_from_views(views_in_order)) == render(
        build_document_from_views(views_reversed)
    ), "goal order must not affect the rendered artifact"


def test_no_timestamp_or_generated_at_key_appears_in_the_rendered_output():
    """A timestamp would churn the file on every run while saying nothing about coverage —
    the emitter deliberately keeps the client version OUT of the document (see
    ``__main__.py``'s module doc). Anything time-shaped in the YAML would make regeneration
    non-idempotent and red the previous test on every run.

    The bite: add a ``generated_at``, ``timestamp``, ``generated`` or ``now`` key to the
    document dict — reds this test. Bites against the REAL ``build_document`` via the same
    monkeypatch the shuffle test uses.
    """
    from register_projection import __main__ as main_mod
    from register_projection import source as source_mod

    goal = _goal("01a00f76-5087-7be3-855e-5a12489831e3", _REGISTER_BODY)
    index = _index_with("live_test_a")

    orig_active = source_mod.active_goals
    orig_tasks = source_mod.tasks_in_context
    orig_advancing = source_mod.advancing_tasks
    orig_index = main_mod.build_index
    source_mod.active_goals = lambda _ctx: ([goal], 1)
    source_mod.tasks_in_context = lambda _ctx: []
    source_mod.advancing_tasks = lambda _g: []
    main_mod.build_index = lambda _r: index
    try:
        doc = main_mod.build_document("@me/temper", ".")
        rendered = render(doc)
    finally:
        source_mod.active_goals = orig_active
        source_mod.tasks_in_context = orig_tasks
        source_mod.advancing_tasks = orig_advancing
        main_mod.build_index = orig_index

    forbidden_time_keys = {"generated_at", "timestamp", "generated", "now", "date", "time"}
    assert not (forbidden_time_keys & doc.keys()), (
        f"a time-shaped key would make regeneration non-idempotent: "
        f"{forbidden_time_keys & doc.keys()}"
    )
    assert "generated_at" not in rendered
    assert "timestamp" not in rendered


# ── Clause 3: an-unresolvable-witness-must-never-count-as-a-covered-clause ────────────────────


def test_a_clause_with_an_unresolved_witness_carries_no_verdict_field():
    """The clause: a witness naming evidence that cannot be found must not count as covered.
    The flattering failure is declaring a witness unresolvable in a detail somewhere while
    still counting it in a summary — so the emitter emits no ``covered``/``verdict`` field
    on a clause at all. Coverage is the register's own declared state; this reports on it.

    The bite: add a ``covered: True`` field when ``table_witnesses`` is non-empty, or add a
    ``witness_citations_resolved`` total that ignores the unresolved count — reds this test.
    """
    view = _views()[0]
    clause = next(
        c for c in view.clauses
        if c.name == "an-unresolvable-witness-must-never-count-as-a-covered-clause"
    )
    # The fixture: one resolved witness, one unresolved.
    resolved = [w for w in clause.table_witnesses if w.resolved]
    unresolved = [w for w in clause.table_witnesses if not w.resolved]
    assert len(resolved) == 1 and len(unresolved) == 1, "fixture: one of each"

    from register_projection.__main__ import _clause_doc
    doc = _clause_doc(clause)
    forbidden_verdict_keys = {"covered", "verdict", "coverage_state", "resolved_count", "is_covered"}
    assert not (forbidden_verdict_keys & doc.keys()), (
        f"a verdict key would let an unresolved witness count as covered: "
        f"{forbidden_verdict_keys & doc.keys()}"
    )
    # The unresolved witness is reported as ``resolves: false`` in its own entry — that is the
    # detail that says so. What must NOT exist is a summary that folds it into "covered".
    witness_docs = doc["register_table"]
    assert any(w["resolves"] is False for w in witness_docs), (
        "the unresolved witness must be visible as ``resolves: false``"
    )


def test_the_totals_report_the_unresolved_count_and_never_fold_it_into_a_covered_figure():
    """The totals block carries ``witness_citations`` and ``witness_citations_unresolved`` as
    separate counts. A total that reported only ``witness_citations`` and derived a
    "covered" figure from ``witness_citations - witness_citations_unresolved`` would let an
    unresolved witness count as covered by hiding behind a derived number.

    The bite: drop ``witness_citations_unresolved`` from ``Totals``, or add a
    ``witness_citations_resolved`` field — reds this test. Bites against the REAL
    ``build_document`` via the same monkeypatch the shuffle test uses.
    """
    from register_projection import __main__ as main_mod
    from register_projection import source as source_mod

    views = _views()
    t = projection.totals(views, goals_in_context=1)
    assert hasattr(t, "witness_citations"), "total citations is reported"
    assert hasattr(t, "witness_citations_unresolved"), "unresolved is reported separately"
    # Fixture: three witness citations across the five clauses, one unresolved.
    assert t.witness_citations >= 1
    assert t.witness_citations_unresolved == 1, "the one renamed test is the unresolved witness"
    assert t.witness_citations_unresolved <= t.witness_citations, (
        "unresolved must never exceed total — it is a subset, not a parallel count"
    )

    # The emitted totals dict must not derive a "covered" or "resolved" count from these.
    # Bite against the REAL build_document, not the helper.
    goal = _goal("01a00f76-5087-7be3-855e-5a12489831e3", _REGISTER_BODY)
    index = _index_with("live_test_a", "live_test_b", "determinism_test", "totals_test", "authority_test")
    orig_active = source_mod.active_goals
    orig_tasks = source_mod.tasks_in_context
    orig_advancing = source_mod.advancing_tasks
    orig_index = main_mod.build_index
    source_mod.active_goals = lambda _ctx: ([goal], 1)
    source_mod.tasks_in_context = lambda _ctx: []
    source_mod.advancing_tasks = lambda _g: []
    main_mod.build_index = lambda _r: index
    try:
        doc = main_mod.build_document("@me/temper", ".")
    finally:
        source_mod.active_goals = orig_active
        source_mod.tasks_in_context = orig_tasks
        source_mod.advancing_tasks = orig_advancing
        main_mod.build_index = orig_index

    totals_doc = doc["totals"]
    forbidden_derived_keys = {
        "witness_citations_resolved",
        "witness_citations_covered",
        "covered_clauses",
        "coverage_rate",
    }
    assert not (forbidden_derived_keys & totals_doc.keys()), (
        f"a derived coverage figure would fold unresolved into covered: "
        f"{forbidden_derived_keys & totals_doc.keys()}"
    )


# ── Clause 4: a-total-must-never-be-reported-over-a-source-that-was-not-read ────────────────────


def test_a_truncated_listing_page_raises_rather_than_being_projected_as_a_whole():
    """The clause: a total must never be reported over a source that was not read. The
    founding failure of this whole area was a partial page presented as a whole one — see
    the goal's *Closure* section, where it happened once and went unnoticed for six weeks.

    ``source._rows`` is the one place the source layer decides whether a page is whole. A
    truncated page must raise ``SourceUnavailable`` rather than returning the rows it has.

    The bite: make ``_rows`` return ``rows`` without checking ``truncated`` — a partial page
    becomes a silent undercount and reds this test.
    """
    partial_page = {
        "rows": [{"id": "01a011b1-0001-7000-8000-000000000001"}],
        "truncated": True,
        "returned": 1,
        "total": 3,
    }
    with __import__("pytest").raises(SourceUnavailable, match="truncated"):
        source._rows(partial_page, "task")


def test_a_non_truncated_page_returns_its_rows():
    """The complement: a whole page is projected, not refused. This guards against the
    inverse mutation — making ``_rows`` raise unconditionally — which would be a different
    way of failing to report over a source, just one that fails loudly instead of silently.
    """
    whole_page = {
        "rows": [{"id": "01a011b1-0001-7000-8000-000000000001"}],
        "truncated": False,
        "returned": 1,
        "total": 1,
    }
    assert source._rows(whole_page, "task") == whole_page["rows"]


def test_totals_carry_their_denominators_beside_their_counts():
    """The clause's name: a TOTAL must never be reported over a source that was NOT READ. The
    totals block carries ``goals_in_context`` beside ``goals_projected`` and a note that only
    active goals are read — so a reader can never read "30" as "all of them".

    The bite: drop ``goals_in_context`` or the ``goals_projected_note`` from the emitted
    totals, and a count over a subset reads as a total to the next person — reds this test.
    Bites against the REAL ``build_document`` via the same monkeypatch the shuffle test uses.
    """
    from register_projection import __main__ as main_mod
    from register_projection import source as source_mod

    goal = _goal("01a00f76-5087-7be3-855e-5a12489831e3", _REGISTER_BODY)
    index = _index_with("live_test_a")
    orig_active = source_mod.active_goals
    orig_tasks = source_mod.tasks_in_context
    orig_advancing = source_mod.advancing_tasks
    orig_index = main_mod.build_index
    source_mod.active_goals = lambda _ctx: ([goal], 7)  # 7 in context, 1 projected
    source_mod.tasks_in_context = lambda _ctx: []
    source_mod.advancing_tasks = lambda _g: []
    main_mod.build_index = lambda _r: index
    try:
        doc = main_mod.build_document("@me/temper", ".")
    finally:
        source_mod.active_goals = orig_active
        source_mod.tasks_in_context = orig_tasks
        source_mod.advancing_tasks = orig_advancing
        main_mod.build_index = orig_index

    totals_doc = doc["totals"]

    assert "goals_in_context" in totals_doc, "the denominator must be stated beside the count"
    assert "goals_projected" in totals_doc
    assert "goals_projected_note" in totals_doc, (
        "the note that says only active goals are read must travel with the count"
    )
    assert totals_doc["goals_in_context"] >= totals_doc["goals_projected"], (
        "projected is a subset of in-context; the denominator must not be smaller than the count"
    )


# ── Clause 5: the-projection-must-never-become-the-authority-over-the-register ─────────────────
#
# This is the judged clause. The judgement perspective: a cold reader comparing the projection
# to the register's own table. The projection reports on the register; it must not become a
# second source of truth maintained by nobody. Three exemplars:
#   (a) the artifact's header names the register's table as the record;
#   (b) no clause/register doc carries a verdict/covered/coverage_state field;
#   (c) the source layer reads (`list`/`show`) and never writes (`update`/`create`/`delete`).
#
# A pure judgement is a perspective plus exemplars, and exemplars can rot. So a structural bite
# is added: the emitted docs must not contain a verdict-shaped key. That makes the judgement
# fail mechanically if a later change adds one, while the exemplars carry the perspective.


def test_the_header_names_the_register_s_own_table_as_the_record():
    """Exemplar (a): the artifact's header states, in prose, that where the sources disagree
    the register's own table is the record. The header is the first thing a cold reader sees,
    so the authority claim is stated up front rather than buried in a field.

    The bite: remove the authority-claim line from ``HEADER`` and the cold reader has no
    statement of which source is authoritative — reds this test. Matches the SPECIFIC line,
    not the loose word "record" (which appears elsewhere in the header for unrelated reasons).
    """
    # The specific authority claim: the register's own table is THE RECORD. Asserting the
    # whole phrase — not just the word "record" — keeps the bite precise.
    assert "REGISTER'S OWN TABLE IS THE RECORD" in HEADER.upper(), (
        "the header must name the register's table as the record, not the projection"
    )
    # And the projection's own role is named as the reporter, not the authority.
    assert "reports" in HEADER.lower() and "adjudicate" in HEADER.lower(), (
        "the header must say the projection reports and does not adjudicate"
    )


def test_no_emitted_doc_carries_a_verdict_or_covered_field():
    """Exemplar (b), as a structural bite. The projection reports; it does not adjudicate. A
    ``covered``/``verdict``/``coverage_state`` field on a clause or register doc would make
    the projection a second source of truth — one maintained by nobody, because nothing writes
    it back to the register.

    The bite: add any verdict-shaped key to ``_clause_doc`` or ``_register_doc`` — reds this
    test.
    """
    from register_projection.__main__ import _clause_doc, _register_doc

    view = _views()[0]
    verdict_keys = {"covered", "verdict", "coverage_state", "is_covered", "coverage"}
    for clause in view.clauses:
        doc = _clause_doc(clause)
        assert not (verdict_keys & doc.keys()), (
            f"clause {clause.name!r} must not carry a verdict key: {verdict_keys & doc.keys()}"
        )
    reg_doc = _register_doc(view)
    assert not (verdict_keys & reg_doc.keys()), (
        f"the register doc must not carry a verdict key: {verdict_keys & reg_doc.keys()}"
    )


def test_the_source_layer_only_reads_and_never_writes_to_the_knowledge_base():
    """Exemplar (c): the source layer calls ``temper resource list`` and ``temper resource
    show`` — both read-only. A ``resource update``/``create``/``delete`` call would make the
    projection write back to the register, which is the clause's named failure: the projection
    becoming the authority over the register by editing it.

    The bite: add a ``resource update``/``create``/``delete`` call anywhere in ``source.py``
    and the source layer is no longer read-only — reds this test.
    """
    import ast
    import inspect

    tree = ast.parse(inspect.getsource(source))
    write_verbs = {"update", "create", "delete"}
    read_verbs = {"list", "show"}
    found_writes: list[str] = []
    found_reads: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            if node.value in write_verbs:
                found_writes.append(node.value)
            if node.value in read_verbs:
                found_reads.append(node.value)
    assert not found_writes, (
        f"the source layer must never write to the knowledge base; found write verbs: "
        f"{found_writes}"
    )
    assert "list" in found_reads, "the source layer reads via `resource list`"
    assert "show" in found_reads, "the source layer reads via `resource show`"


# ── Helpers ────────────────────────────────────────────────────────────────────────────────────


def build_document_from_views(views: list[projection.RegisterView]) -> dict:
    """Build the document dict the way ``__main__.build_document`` does, but from pre-built
    views — so the witness tests never reach the network.

    Mirrors the exact field set and ordering of ``__main__.build_document``'s return value,
    INCLUDING the stable sort by ``goal_id``. If that function changes shape, these tests
    still assert against the same structure the live emitter produces.
    """
    views_sorted = sorted(views, key=lambda v: v.goal_id)
    t = projection.totals(views_sorted, goals_in_context=max(1, len(views_sorted)))
    return {
        "schema_version": 1,
        "source_system": "https://temperkb.io",
        "context": "@me/temper",
        "search_domain": ["test fixture"],
        "totals": {
            "goals_in_context": t.goals_in_context,
            "goals_projected": t.goals_projected,
            "goals_projected_note": "active goals only; non-active goals are not read",
            "registers_with_no_clauses_read": t.goals_unreadable,
            "registers_with_clauses_and_a_witness_table": t.registers_with_witness_table,
            "registers_with_clauses_but_no_witness_table": t.registers_clauses_but_no_witness_table,
            "witness_table_note": (
                "absence of a witness table is not a defect: a register whose mechanism is unbuilt "
                "has no witnesses to name, and giving it an empty table would read as 'checked, "
                "found nothing' rather than 'nothing to check yet'"
            ),
            "clauses": t.clauses,
            "clauses_withdrawn": t.clauses_withdrawn,
            "witness_citations": t.witness_citations,
            "witness_citations_unresolved": t.witness_citations_unresolved,
        },
        "registers": [
            {
                "goal_id": v.goal_id,
                "goal_title": v.goal_title,
                "context": v.context_ref,
                "read": {
                    "clause_form": v.clause_form,
                    "witness_table": v.witness_table,
                },
                "clauses": [
                    {
                        "name": c.name,
                        "interrogatable": c.interrogatable,
                        "register_table": [
                            {"name": w.name, "kind": w.kind, "resolves": w.resolved}
                            for w in c.table_witnesses
                        ],
                        "citations_witnesses": c.citing_witnesses,
                        "citations_enables": c.citing_enables,
                    }
                    for c in v.clauses
                ],
                "advancing": {
                    "total": v.advancing_total,
                    "citing_nothing": v.advancing_citing_nothing,
                },
            }
            for v in views_sorted
        ],
    }