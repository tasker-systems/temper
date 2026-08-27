"""Two credential strategies behind one interface.

Precedence is not discovered from the environment — it is the caller's explicit
choice. That avoids, structurally, the drift the steward's `temper-auth.ts` header
documents: its schedules went Connect-first while its MCP connection went M2M-first,
so on the Auth0-fronted prod instance the schedules' REST calls silently failed while
MCP worked.

Ported from `clients/temper-rb/lib/temper/credentials.rb`, which is itself a port of
`packages/agent-workflows/steward/agent/lib/temper-auth.ts` — the machine-principal
caller already running in production. The mint request is pinned against
`tests/contracts/m2m-token-request.json`, the cross-language wire contract; see
`tests/test_contract.py`.
"""

from __future__ import annotations

import json
import threading
import time
from collections.abc import Callable
from typing import Protocol, runtime_checkable

import urllib3

from temper.errors import TransportError, Unauthorized


@runtime_checkable
class Credentials(Protocol):
    """What `Client` needs from a credential, and nothing more."""

    @property
    def can_refresh(self) -> bool:
        """Whether `refresh()` can produce a NEW token, or only re-raise.

        `Client` consults this before spending its one 401 repair: asking a
        `BearerToken` to refresh replaces the server's real answer — the body a human
        is trying to read — with a message about the client's own plumbing.
        """

    def token(self) -> str:
        """The current access token, minting one if the strategy can and must."""

    def refresh(self) -> str:
        """Mint unconditionally, discarding any cached token."""


class BearerToken:
    """A token the caller already holds — a web request serving a signed-in user.

    No I/O, no refresh.
    """

    def __init__(self, token: str) -> None:
        if not isinstance(token, str) or not token:
            raise ValueError("token must be a non-empty str")
        self._token = token

    @property
    def can_refresh(self) -> bool:
        return False

    def token(self) -> str:
        return self._token

    def refresh(self) -> str:
        raise Unauthorized(
            "BearerToken cannot refresh; mint a new token upstream",
            status=401,
        )


class ClientCredentials:
    """A `client_credentials` machine principal — a background worker.

    Works against BOTH issuers a temper instance can be fronted by:

    * Auth0 (`temper admin machine provision`), where `token_url` is your Auth0
      tenant's ``/oauth/token`` and `audience` must equal the API's AUTH_AUDIENCE.
    * temper's own AS (`temper admin machine issue`, a ``tmpr_*`` client id), where
      `token_url` is your own instance's ``/oauth/token`` and `audience` is omitted —
      that AS mints with its server-side AS_AUDIENCE and ignores a request-supplied
      one entirely.

    Two properties carried over from the gem because both were bought with an
    incident:

    * The cache is lock-guarded. The steward's is a bare module global, sound only
      because a serverless function is single-threaded. Under a threaded server every
      in-flight thread races to mint at expiry.
    * `refresh()` exists. Refresh-ahead-of-expiry alone is insufficient: a caller
      resolves a token once and then holds it across a long unit of work, so a token
      that dies mid-work takes a 401 nothing recovers. Re-mint ON 401.
    """

    SKEW_SECONDS = 60
    #: RFC 6749 §4 mandates form encoding at the token endpoint. Auth0 also accepts
    #: JSON, which is why the gem sent JSON while Auth0 was the only issuer it faced —
    #: and why every test stayed green. Temper's own AS reads the body with
    #: `req.formData()`, so a JSON mint never reaches its client_credentials branch.
    TOKEN_REQUEST_CONTENT_TYPE = "application/x-www-form-urlencoded"

    def __init__(
        self,
        *,
        token_url: str,
        client_id: str,
        client_secret: str,
        audience: str | None = None,
        clock: Callable[[], float] = time.time,
        http: urllib3.PoolManager | None = None,
    ) -> None:
        self._token_url = _require(token_url, "token_url")
        self._client_id = _require(client_id, "client_id")
        self._client_secret = _require(client_secret, "client_secret")
        self._audience = None if audience is None else _require(audience, "audience")
        self._clock = clock
        # urllib3 rather than a second HTTP client: the generated core already
        # depends on it, so the mint costs no new dependency. Its own PoolManager,
        # deliberately — the token endpoint is a different origin from the API on the
        # Auth0-fronted deployment, and sharing the API's pool would tie the mint's
        # lifetime to `reset_connection()`.
        self._http = http if http is not None else urllib3.PoolManager()
        self._lock = threading.Lock()
        self._token: str | None = None
        self._expires_at = 0.0

    @property
    def can_refresh(self) -> bool:
        return True

    def token(self) -> str:
        with self._lock:
            if self._expired():
                return self._mint()
            # `_expired()` is false only when a mint has already landed a token.
            return self._token  # type: ignore[return-value]

    def refresh(self) -> str:
        with self._lock:
            return self._mint()

    # -- internals -----------------------------------------------------------

    def _expired(self) -> bool:
        return self._token is None or self._clock() >= (self._expires_at - self.SKEW_SECONDS)

    def _mint(self) -> str:
        """Caller holds `self._lock`."""
        response = self._post_token_request()
        try:
            body = json.loads(response.data.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as exc:
            raise Unauthorized(
                f"token mint returned a non-JSON body ({response.status})",
                status=response.status,
            ) from exc

        self._token = body["access_token"]
        # Absolute, not relative: a duration cannot survive being cached.
        self._expires_at = self._clock() + int(body["expires_in"])
        return self._token

    def _post_token_request(self) -> urllib3.BaseHTTPResponse:
        params = {
            "grant_type": "client_credentials",
            "client_id": self._client_id,
            "client_secret": self._client_secret,
        }
        # Auth0 requires it; temper's AS ignores a request-supplied audience and mints
        # with its own AS_AUDIENCE. Sending an empty one would be a lie, so omit it.
        if self._audience is not None:
            params["audience"] = self._audience

        try:
            # `encode_multipart=False` is what makes this form-encoded rather than
            # multipart. Both are on the contract's accepted list, but the gem and the
            # TS client both emit form encoding and the contract names it as THE
            # content type, so all three clients send the same bytes.
            response = self._http.request(
                "POST",
                self._token_url,
                fields=params,
                encode_multipart=False,
            )
        except urllib3.exceptions.HTTPError as exc:
            raise TransportError(f"token mint could not reach {self._token_url}: {exc}") from exc

        if not 200 <= response.status < 300:
            raise Unauthorized(
                f"token mint failed ({response.status})",
                status=response.status,
                details=response.data.decode("utf-8", "replace"),
            )
        return response


def _require(value: str, name: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name} must be a non-empty str")
    return value
