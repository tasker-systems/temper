"""Properties of the GENERATED core that the skin — and callers — depend on.

Nothing here tests the generator. It tests the contract's rendering into Python:
each assertion below is a shape that, if the spec changed under it, would break a
caller silently rather than loudly.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

import temper
from temper.generated.api.resources_api import ResourcesApi
from temper.generated.configuration import Configuration
from temper.generated.models.illegal_transition import IllegalTransition
from temper.generated.models.ingest_payload import IngestPayload
from temper.generated.models.insufficient_authority import InsufficientAuthority
from temper.generated.models.refusal import Refusal
from temper.generated.models.system_access_details import SystemAccessDetails

CONTRACT_PATH = Path(__file__).resolve().parents[3] / "openapi.json"


@pytest.fixture(scope="module")
def contract() -> dict[str, Any]:
    parsed: dict[str, Any] = json.loads(CONTRACT_PATH.read_text())
    return parsed


def test_records_the_contract_version_it_was_generated_from(contract):
    assert contract["info"]["version"] == temper.CONTRACT_VERSION


def test_keeps_the_package_version_independent_of_the_contract_version():
    # A package version and an API version answer different questions.
    assert temper.__version__.count(".") == 2
    assert all(part.isdigit() for part in temper.__version__.split("."))


def test_exposes_collision_free_resource_operations():
    # Every operation has a unique operationId. If a future contract change
    # reintroduces a collision, the generator silently emits `list_0` again.
    names = [n for n in dir(ResourcesApi) if not n.startswith("_")]
    assert "list_resources" in names
    assert "list_resource_edges" in names
    assert [n for n in names if n.rstrip("0123456789") != n and n[-1].isdigit()] == []


def test_flattens_the_seven_act_input_keys_onto_ingest_payload():
    act_keys = {
        "confidence",
        "correlation_id",
        "invocation_id",
        "model",
        "persona",
        "rationale",
        "reasoning",
    }
    assert act_keys <= set(IngestPayload.model_fields)


def test_the_act_keys_this_package_emits_are_the_keys_the_payload_declares():
    # `temper.Act` hand-writes the wire key names. Nothing else connects the two, so
    # a renamed key on the contract would leave Act emitting a field the server drops.
    every_key = temper.Act(
        confidence="high",
        reasoning="r",
        rationale="ra",
        persona="p",
        model="m",
        correlation="c",
        invocation="i",
    ).to_dict()
    assert set(every_key) <= set(IngestPayload.model_fields)


def test_exposes_the_seam_the_skin_overrides():
    # TokenScopedConfiguration replaces `access_token` with a call-scoped property.
    # If the generator ever stopped putting the token there, the client would
    # authenticate with nothing and every test that asserts a header would be the
    # only thing that noticed.
    assert "access_token" in Configuration.__init__.__code__.co_varnames
    assert "bearer_auth" in Configuration(access_token="t").auth_settings()


class TestTypedRefusal:
    """The admission 403 carries a typed refusal.

    It reaches Python as an anonymous `oneOf`, which the generator resolves by trying
    each branch until one validates — so "it discriminates" is a property of the
    emitted schema, not something the template guarantees.
    """

    @pytest.mark.parametrize(
        "kind",
        ["no_standing", "denied", "requested", "revoked", "deactivated", "no_prior_standing"],
    )
    def test_resolves_each_unit_variant(self, kind):
        # Named branches, not RefusalOneOf4 — see the `schema(title = …)` note on the
        # Rust enum.
        refusal = Refusal.from_dict({"kind": kind})
        variant = refusal.actual_instance
        assert variant is not None
        assert type(variant).__name__ == "".join(part.capitalize() for part in kind.split("_"))
        assert variant.kind == kind

    def test_carries_the_payload_of_a_data_bearing_variant(self):
        refusal = Refusal.from_dict(
            {"kind": "illegal_transition", "act": "approve", "from": "denied"}
        )
        assert isinstance(refusal.actual_instance, IllegalTransition)
        assert refusal.actual_instance.act == "approve"

    def test_admits_a_null_from_an_act_can_be_illegal_with_no_standing_at_all(self):
        refusal = Refusal.from_dict({"kind": "illegal_transition", "act": "approve", "from": None})
        assert isinstance(refusal.actual_instance, IllegalTransition)

    def test_distinguishes_the_two_authority_sides(self):
        refusal = Refusal.from_dict(
            {"kind": "insufficient_authority", "required": "admin", "actual": "self_principal"}
        )
        assert isinstance(refusal.actual_instance, InsufficientAuthority)
        assert refusal.actual_instance.required == "admin"
        assert refusal.actual_instance.actual == "self_principal"

    def test_refuses_a_kind_this_build_does_not_know_rather_than_mis_casting_it(self):
        # The server may be newer than this package. Casting an unknown kind into
        # whichever branch happens to match first would report the wrong reason to the
        # operator — worse than reporting none.
        with pytest.raises(ValueError):
            Refusal.from_dict({"kind": "something_new"})

    def test_resolves_through_the_details_envelope_the_403_actually_sends(self):
        details = SystemAccessDetails.from_dict(
            {
                "email": "p@example.com",
                "display_name": "Pete",
                "refusal": {"kind": "revoked"},
                "request_url": "https://temperkb.io/request-access",
                "cli_command": "temper auth request-access",
            }
        )
        assert details is not None
        assert details.refusal is not None
        variant = details.refusal.actual_instance
        assert variant is not None
        assert variant.kind == "revoked"
        assert details.email == "p@example.com"
