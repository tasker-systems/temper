"""The SDK's error tree, and the mapper that produces it.

The generated core raises exactly two shapes: `ApiException` (an HTTP status the
server answered with) and, for a transport failure, whatever `urllib3` raised —
the generated `rest.py` translates only `SSLError`, so a refused connection or a
read timeout escapes as a raw `urllib3.exceptions.HTTPError`. Neither shape says
whether retrying would help. This module answers that.

The split is load-bearing rather than decorative, and it is the same split the gem
draws (`clients/temper-rb/lib/temper/errors.rb`): a 409 classified transient spins
a worker forever, and a 503 classified permanent is silently dropped.

ONE DELIBERATE DIVERGENCE FROM THE GEM'S NAMES. The gem calls the transport failure
`Temper::ConnectionError`; here it is `TransportError`, because `ConnectionError`
is a Python builtin (an `OSError` subclass). A `temper.ConnectionError` would shadow
it inside every module that did `from temper import *` or `from temper.errors import
ConnectionError`, and an `except ConnectionError` written against either meaning
would silently catch the other. The tree is a port, not a transliteration.
"""

from __future__ import annotations

import json
from typing import Any

import urllib3.exceptions

from temper.generated.exceptions import ApiException
from temper.generated.models.refusal import Refusal


class TemperError(Exception):
    """Base of the tree. Carries the server's envelope, not just a message."""

    def __init__(
        self,
        message: str | None = None,
        *,
        status: int | None = None,
        code: str | None = None,
        details: Any = None,
    ) -> None:
        super().__init__(message)
        self.status = status
        self.code = code
        self.details = details


class TransientError(TemperError):
    """Let these escape a job: a retry is what fixes them."""


class RateLimited(TransientError):
    def __init__(
        self,
        message: str | None = None,
        *,
        retry_after: int | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(message, **kwargs)
        self.retry_after = retry_after


class ServerError(TransientError):
    pass


class TransportError(TransientError):
    """The request never got an HTTP answer: DNS, connect, TLS, read timeout."""


class PermanentError(TemperError):
    """Catch these: retrying will not help. Dead-letter them."""


class Unauthorized(PermanentError):
    pass


class Forbidden(PermanentError):
    pass


class SystemAccessRequired(Forbidden):
    """The one error whose `details` is a typed payload rather than a free-form blob.

    The server says *why* it refused, so a worker can tell "never granted" from
    "granted and then revoked" without matching on the message string.
    """

    @property
    def refusal(self) -> Refusal | None:
        """The refusal as a generated model — `Denied`, `Revoked`, `IllegalTransition`, …

        `None` means this build cannot name the refusal: either the server sent none
        (it predates the typed 403) or it sent a kind added after this package was
        generated. Reach for `refusal_kind` in that case — a name we cannot resolve is
        still worth logging.

        The gem has to `transform_keys(&:to_sym)` here, because its ApiClient
        deserializes with `symbolize_names` while the error envelope arrives
        string-keyed, and feeding string keys to the oneOf dispatcher resolves to nil
        SILENTLY. Python has no such split: `Refusal.from_dict` reads the same string
        keys the envelope carries. It raises `ValueError` on no match rather than
        returning None, so the unresolvable case is caught rather than conflated with
        "the server sent nothing".
        """
        raw = self._refusal_dict()
        if raw is None:
            return None
        try:
            return Refusal.from_dict(raw)
        except ValueError:
            return None

    @property
    def refusal_kind(self) -> str | None:
        """The refusal's discriminator as the server sent it, resolvable or not."""
        raw = self._refusal_dict()
        if raw is None:
            return None
        kind = raw.get("kind")
        return kind if isinstance(kind, str) else None

    def _refusal_dict(self) -> dict[str, Any] | None:
        if not isinstance(self.details, dict):
            return None
        raw = self.details.get("refusal")
        return raw if isinstance(raw, dict) else None


class NotFound(PermanentError):
    pass


class Conflict(PermanentError):
    pass


class BadRequest(PermanentError):
    pass


SYSTEM_ACCESS_REQUIRED = "SYSTEM_ACCESS_REQUIRED"

# 422 is declared on no operation but is what a serde rejection surfaces as.
# 403 and 429 are absent on purpose: each needs more than the status to classify.
_STATUS_CLASSES: dict[int, type[TemperError]] = {
    400: BadRequest,
    401: Unauthorized,
    404: NotFound,
    409: Conflict,
    422: BadRequest,
}


def _parse_envelope(body: Any) -> tuple[str | None, str | None, Any]:
    """The server speaks exactly one envelope: ``{"error":{code,message,details}}``.

    Anything else — an HTML 502 from a proxy, an undeclared 500 — degrades to a raw
    body on `details` rather than raising inside the error path.

    422/429/500/503 are declared on NO operation, so those bodies are parsed
    opportunistically and classified off the raw HTTP status.
    """
    if body is None or body == "":
        return None, None, None
    try:
        parsed = json.loads(body)
    except (ValueError, TypeError):
        return None, None, body
    if not isinstance(parsed, dict):
        return None, None, body
    error = parsed.get("error")
    if not isinstance(error, dict):
        return None, None, body
    return error.get("code"), error.get("message"), error.get("details")


def _retry_after(exc: ApiException) -> int | None:
    headers = getattr(exc, "headers", None) or {}
    raw = headers.get("Retry-After") or headers.get("retry-after")
    if raw is None:
        return None
    try:
        seconds = int(raw)
    except (TypeError, ValueError):
        # RFC 9110 also allows an HTTP-date here, and a proxy in the path may send
        # anything at all. `None` says "no usable hint", which is honest; guessing a
        # duration from an unparseable header is not.
        return None
    # Clamped, because this value's whole purpose is to be passed to a sleep, and a
    # clock-skewed intermediary answering a negative delay should not turn a 429 into
    # a ValueError several frames away from here.
    return max(seconds, 0)


def map_error(exc: BaseException) -> TemperError:
    """Translate one generated-core failure into the tree above.

    Accepts both shapes the core can raise. A `urllib3` error never reached an HTTP
    status at all, so it classifies as `TransportError` — transient — without
    consulting a body it does not have.
    """
    if isinstance(exc, urllib3.exceptions.HTTPError):
        return TransportError(str(exc) or type(exc).__name__)

    if not isinstance(exc, ApiException):
        raise TypeError(f"map_error expects ApiException or urllib3 HTTPError, got {type(exc)!r}")

    status = exc.status
    code, message, details = _parse_envelope(exc.body)
    # ApiException.__str__ decorates itself with status/headers/body when it has
    # them, so it is only a useful fallback once the envelope yielded nothing.
    if message is None:
        message = str(exc)

    # status 0 is what the generated rest.py assigns to an SSL failure and to a
    # request it could not even build: no HTTP answer, so transport.
    if status is None or status == 0:
        return TransportError(message, status=status, code=code, details=details)

    if status == 403:
        cls = SystemAccessRequired if code == SYSTEM_ACCESS_REQUIRED else Forbidden
        return cls(message, status=status, code=code, details=details)

    if status == 429:
        return RateLimited(
            message,
            retry_after=_retry_after(exc),
            status=status,
            code=code,
            details=details,
        )

    if status in _STATUS_CLASSES:
        return _STATUS_CLASSES[status](message, status=status, code=code, details=details)

    if 500 <= status <= 599:
        return ServerError(message, status=status, code=code, details=details)

    return TemperError(message, status=status, code=code, details=details)
