import pytest

from temper import Act


def test_an_empty_act_sends_nothing():
    assert Act().to_dict() == {}


def test_authorship_without_confidence_cannot_be_constructed():
    # Rust's AgentAuthorship.confidence is non-Option, so this shape is a 400.
    # Rejecting it here means no call site can send one.
    with pytest.raises(ValueError, match="confidence"):
        Act(reasoning="because")


def test_correlation_and_invocation_are_exempt_from_the_confidence_rule():
    act = Act(correlation="c-1", invocation="i-1")
    assert act.to_dict() == {"correlation_id": "c-1", "invocation_id": "i-1"}


def test_confidence_is_stringified_for_the_wire():
    assert Act(confidence=0.9).to_dict()["confidence"] == "0.9"


def test_omits_absent_keys_rather_than_sending_null():
    # The server distinguishes an absent key from an explicit null.
    act = Act(confidence="high", reasoning="because")
    assert act.to_dict() == {"confidence": "high", "reasoning": "because"}


def test_to_dict_is_a_copy():
    act = Act(confidence="high")
    act.to_dict()["confidence"] = "mutated"
    assert act.to_dict()["confidence"] == "high"
