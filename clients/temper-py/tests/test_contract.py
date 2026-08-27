"""Pin the M2M mint against the cross-language wire contract.

`tests/contracts/m2m-token-request.json` exists because a contract asserted only
against itself is not asserted at all: the Ruby gem minted with a JSON body and proved
it with a stub that parsed JSON, while temper's AS read the body with
`req.formData()` and proved THAT with a form-encoded request. Both suites were green
and no client could mint against temper's issuer.

The contract's own `$comment` names this file's obligation: "Adding a client
(temper-py) means pinning it against this file too."
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from temper import ClientCredentials

# Anchored at the repo root rather than relative to each test, so a test that moves a
# directory deeper does not silently point at the wrong file.
_REPO_ROOT = Path(__file__).resolve().parents[3]
CONTRACT_PATH = _REPO_ROOT / "tests" / "contracts" / "m2m-token-request.json"


@pytest.fixture(scope="module")
def contract() -> dict[str, Any]:
    parsed: dict[str, Any] = json.loads(CONTRACT_PATH.read_text())
    return parsed


@pytest.fixture
def minted(server, contract):
    server.respond_json({"access_token": "tok-1", "expires_in": 3600})
    ClientCredentials(
        token_url=f"{server.url}/oauth/token",
        client_id="cid",
        client_secret="sec",
        audience="https://api.test",
    ).token()
    return server.requests[-1]


def test_the_contract_file_is_where_we_think_it_is():
    # `Path.read_text` on a missing file raises, but a moved contract would otherwise
    # only surface as an unhelpful failure inside a fixture.
    assert CONTRACT_PATH.is_file(), f"wire contract not found at {CONTRACT_PATH}"


def test_sends_the_content_type_the_token_endpoint_actually_parses(minted, contract):
    # THE regression test. JSON is what Auth0 tolerates and temper's AS does not: its
    # handleToken reads the body with `req.formData()`, so a JSON mint never reaches
    # the client_credentials branch at all.
    assert minted.headers["content-type"] == contract["content_type"]


def test_the_content_type_is_one_the_server_accepts(minted, contract):
    assert minted.headers["content-type"] in contract["accepted_content_types"]


def test_emits_exactly_the_params_the_contract_requires(minted, contract):
    form = minted.form()
    for name in contract["required_params"]:
        assert name in form, f"{name} is required by the contract"
    assert form["grant_type"] == contract["grant_type"]


def test_carries_the_credential_in_the_form_body_not_a_basic_header(minted, contract):
    # client_secret_post: both issuers accept it, and it is what temper's own clients
    # emit. RFC 6749 §2.3.1 makes Basic take precedence where both are present, so
    # sending both would silently change which one the server reads.
    assert "client_secret_post" in contract["credential_transport"]
    assert minted.form()["client_secret"] == "sec"
    assert "authorization" not in minted.headers
