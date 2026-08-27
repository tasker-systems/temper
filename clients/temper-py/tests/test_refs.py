import pytest

from temper import parse_ref

UUID = "019f4912-3f20-7fd3-814f-13a5ddbe3cd7"


def test_accepts_a_bare_uuid():
    assert parse_ref(UUID) == UUID


def test_strips_surrounding_whitespace():
    assert parse_ref(f"  {UUID}\n") == UUID


def test_resolves_the_decorated_slug_form():
    assert parse_ref(f"a-resource-{UUID}") == UUID


def test_a_stale_slug_half_is_harmless():
    assert parse_ref(f"whatever-it-used-to-be-called-{UUID}") == UUID


@pytest.mark.parametrize("bad", ["", "not-a-ref", "a-resource-019f4912", UUID.replace("-", "")])
def test_rejects_rather_than_guesses(bad):
    with pytest.raises(ValueError):
        parse_ref(bad)


def test_rejects_a_non_string():
    with pytest.raises(TypeError):
        parse_ref(None)  # type: ignore[arg-type]
