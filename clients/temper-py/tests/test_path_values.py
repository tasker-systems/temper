"""A path value of `.` or `..` would climb to the parent route.

The generate script escapes `.` at the one path-substitution site and refuses the two dot segments
there; both are post-generation patches, so these fail if a regeneration drops either.
"""

from __future__ import annotations

import pytest

from temper.generated.api_client import ApiClient
from temper.generated.configuration import Configuration

TEMPLATE = "/api/schema/doc-types/{name}"


def serialize(name: str) -> str:
    client = ApiClient(Configuration(host="https://temper.example"))
    _method, url, *_ = client.param_serialize("GET", TEMPLATE, path_params={"name": name})
    return url


def test_a_dotted_value_stays_one_literal_segment():
    assert serialize("a.b") == "https://temper.example/api/schema/doc-types/a%2Eb"


@pytest.mark.parametrize("name", [".", ".."])
def test_a_dot_segment_is_refused_before_a_request_exists(name):
    with pytest.raises(ValueError, match="parent route"):
        serialize(name)
