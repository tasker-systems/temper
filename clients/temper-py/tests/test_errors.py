import pytest
import urllib3.exceptions

from temper import (
    BadRequest,
    Conflict,
    Forbidden,
    NotFound,
    PermanentError,
    PreconditionFailed,
    RateLimited,
    ServerError,
    SystemAccessRequired,
    TemperError,
    TransportError,
    Unauthorized,
    map_error,
)
from temper.generated.exceptions import ApiException


def api_error(status, body=None, headers=None):
    exc = ApiException(status=status, reason="test", body=body)
    exc.headers = headers or {}
    return exc


def envelope(code=None, message="nope", details=None):
    error = {"message": message}
    if code is not None:
        error["code"] = code
    if details is not None:
        error["details"] = details
    return __import__("json").dumps({"error": error})


@pytest.mark.parametrize(
    ("status", "expected"),
    [
        (400, BadRequest),
        (401, Unauthorized),
        (404, NotFound),
        (409, Conflict),
        # 422 is declared on no operation but is what a serde rejection surfaces as.
        (422, BadRequest),
        (500, ServerError),
        (503, ServerError),
    ],
)
def test_classifies_off_the_status(status, expected):
    error = map_error(api_error(status, envelope()))
    assert isinstance(error, expected)
    assert error.status == status
    assert str(error) == "nope"


def test_a_403_without_the_system_code_is_plain_forbidden():
    error = map_error(api_error(403, envelope(code="FORBIDDEN")))
    assert type(error) is Forbidden


def test_a_403_carrying_the_system_code_is_the_typed_refusal():
    body = envelope(code="SYSTEM_ACCESS_REQUIRED", details={"refusal": {"kind": "revoked"}})
    error = map_error(api_error(403, body))
    assert isinstance(error, SystemAccessRequired)
    assert error.refusal_kind == "revoked"
    # A worker can tell "never granted" from "granted and then revoked" without
    # matching on the message string.
    assert error.refusal is not None
    variant = error.refusal.actual_instance
    assert variant is not None
    assert variant.kind == "revoked"


def test_an_unresolvable_refusal_kind_still_reports_its_name():
    # A kind added to the server after this package was generated: the oneOf cannot
    # resolve it, but the discriminator is still worth logging.
    body = envelope(
        code="SYSTEM_ACCESS_REQUIRED",
        details={"refusal": {"kind": "invented_after_this_build"}},
    )
    error = map_error(api_error(403, body))
    assert isinstance(error, SystemAccessRequired)
    assert error.refusal is None
    assert error.refusal_kind == "invented_after_this_build"


def test_a_403_with_no_refusal_payload_degrades_rather_than_raising():
    error = map_error(api_error(403, envelope(code="SYSTEM_ACCESS_REQUIRED")))
    assert isinstance(error, SystemAccessRequired)
    assert error.refusal is None
    assert error.refusal_kind is None


def _refused(precondition):
    error = {"code": "PRECONDITION_FAILED", "message": "precondition failed: open.claim_id"}
    if precondition is not None:
        error["precondition"] = precondition
    return map_error(api_error(412, __import__("json").dumps({"error": error})))


# A 412 used to fall through to the bare TemperError, neither transient nor permanent, and the
# payload naming who holds a claim was dropped with the envelope's unread `precondition` member.
def test_a_412_is_a_permanent_precondition_failed():
    error = _refused({"failed": []})
    assert isinstance(error, PreconditionFailed)
    assert isinstance(error, PermanentError)
    assert error.status == 412
    assert error.code == "PRECONDITION_FAILED"


def test_a_412_carries_every_failed_expectation_with_its_current_value():
    error = _refused(
        {
            "failed": [
                {"tier": "open", "key": "claim_id", "expected": None, "current": "claim-a"},
                {"tier": "body", "expected": "bt1.old", "current": "bt1.new"},
            ]
        }
    )
    assert error.precondition is not None
    assert [f.key for f in error.failed] == ["claim_id", None]
    assert [f.current for f in error.failed] == ["claim-a", "bt1.new"]
    assert error.failed[0].expected is None


def test_a_412_with_no_payload_is_still_precondition_failed_with_nothing_failed():
    error = _refused(None)
    assert isinstance(error, PreconditionFailed)
    assert error.precondition is None
    assert error.failed == []


def test_a_429_carries_retry_after():
    error = map_error(api_error(429, envelope(), headers={"Retry-After": "30"}))
    assert isinstance(error, RateLimited)
    assert error.retry_after == 30


def test_a_429_with_an_unparseable_retry_after_is_still_rate_limited():
    error = map_error(api_error(429, envelope(), headers={"Retry-After": "in a bit"}))
    assert isinstance(error, RateLimited)
    assert error.retry_after is None


def test_a_non_envelope_body_degrades_to_details_rather_than_raising():
    # An HTML 502 from a proxy, or an undeclared 500.
    error = map_error(api_error(502, "<html>bad gateway</html>"))
    assert isinstance(error, ServerError)
    assert error.details == "<html>bad gateway</html>"
    assert error.code is None


def test_an_unclassified_status_stays_at_the_root_of_the_tree():
    error = map_error(api_error(418, envelope()))
    assert type(error) is TemperError


def test_a_urllib3_failure_is_transport_not_a_status():
    # The generated rest.py translates only SSLError, so a refused connection or a
    # read timeout escapes as a raw urllib3 error with no HTTP answer behind it.
    error = map_error(urllib3.exceptions.ProtocolError("connection refused"))
    assert isinstance(error, TransportError)


def test_status_zero_is_transport_too():
    # What the generated rest.py assigns to an SSL failure and to a request it could
    # not even build.
    assert isinstance(map_error(api_error(0, None)), TransportError)


def test_refuses_to_guess_at_an_exception_it_does_not_own():
    with pytest.raises(TypeError):
        map_error(ValueError("not ours"))
