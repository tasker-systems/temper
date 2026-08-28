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

from temper._validate import require_endpoint, require_opaque
from temper.errors import TransportError, Unauthorized

#: The mint's response is a small JSON object — an access token, a type, a lifetime.
#: Read past this and stop: a client pointed at a hostile or simply broken endpoint
#: should not stream an unbounded body into memory while holding the mint lock.
MAX_TOKEN_RESPONSE_BYTES = 64 * 1024


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
        # `require_opaque` rather than a bare emptiness check: this string becomes an
        # `Authorization: Bearer ...` header verbatim, and the way it actually arrives
        # malformed is a trailing newline off `open(path).read()` or a shell
        # `$(...)`. urllib3 v2 rejects the resulting header, but it does so on the
        # request, blaming the request.
        self._token = require_opaque(token, "token")

    def __repr__(self) -> str:
        # The default repr is already opaque; this one stays opaque on purpose, so
        # that adding a field later cannot start printing a credential into a log
        # line or a pytest failure dump.
        return "BearerToken(token=<redacted>)"

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
    #: The mint runs under `self._lock`, so a token endpoint that accepts the
    #: connection and then never answers does not block one caller — it blocks every
    #: thread that needs a token, for as long as the socket stays open. urllib3's
    #: default is the socket default, which is "forever".
    DEFAULT_TIMEOUT = urllib3.Timeout(connect=5.0, read=10.0)

    def __init__(
        self,
        *,
        token_url: str,
        client_id: str,
        client_secret: str,
        audience: str | None = None,
        clock: Callable[[], float] = time.time,
        http: urllib3.PoolManager | None = None,
        timeout: urllib3.Timeout | float | None = None,
        allow_insecure_http: bool = False,
    ) -> None:
        # The client_secret goes on the wire to this URL on every mint, so plaintext
        # http off the loopback interface is refused unless the caller says otherwise.
        require_endpoint(token_url, name="token_url", allow_insecure_http=allow_insecure_http)
        self._token_url = token_url
        self._client_id = require_opaque(client_id, "client_id")
        self._client_secret = _require_secret(client_secret)
        self._audience = None if audience is None else require_opaque(audience, "audience")
        self._clock = clock
        self._timeout = self.DEFAULT_TIMEOUT if timeout is None else timeout
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
        # Dropped BEFORE the request, not after a success. `refresh()` is called
        # because the server rejected the cached token; if the re-mint then fails, a
        # caller must not go on presenting the token this object already knows is
        # dead — that turns one legible 401 into a loop of them.
        self._token = None
        self._expires_at = 0.0

        status, raw = self._post_token_request()
        try:
            body = json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as exc:
            raise Unauthorized(
                f"token mint returned a non-JSON body ({status})",
                status=status,
            ) from exc
        if not isinstance(body, dict):
            raise Unauthorized(
                f"token mint returned a JSON {type(body).__name__}, not an object ({status})",
                status=status,
            )

        # `.get` and an explicit shape check, not `body["access_token"]`. A KeyError
        # out of `token()` is a defect report about this package; what actually
        # happened is that the issuer answered 200 with something else — an HTML login
        # page from a captive portal, an `{"error": ...}` some issuers send with a 200
        # — and the caller needs to be told THAT.
        token = body.get("access_token")
        if not isinstance(token, str) or not token:
            raise Unauthorized(
                f"token mint response carried no access_token ({status})", status=status
            )
        # It becomes an `Authorization` header, and it came off the network. A value
        # with a `\r\n` in it is a second header. Reported as an Unauthorized rather
        # than the ValueError the constructors raise: that distinction is the whole
        # point — a ValueError from this package means the CALLER passed something
        # wrong, and here the caller did nothing wrong.
        if token.strip() != token or any(c.isspace() for c in token) or not token.isprintable():
            raise Unauthorized(
                f"token mint returned an access_token containing whitespace or control "
                f"characters, which cannot be an Authorization header ({status})",
                status=status,
            )

        # RFC 6749 §4.4.3 and the shared contract both say Bearer. Checked only when
        # present, because it is the ONE field of the three that an issuer may omit —
        # but a `token_type` that says something else means this token does not belong
        # in a `Bearer` header at all.
        token_type = body.get("token_type")
        if isinstance(token_type, str) and token_type.casefold() != "bearer":
            raise Unauthorized(
                f"token mint returned token_type {token_type!r}, not Bearer", status=status
            )

        try:
            lifetime = int(body["expires_in"])
        except (KeyError, TypeError, ValueError) as exc:
            raise Unauthorized(
                f"token mint response carried no usable expires_in ({status})", status=status
            ) from exc
        if lifetime <= 0:
            raise Unauthorized(
                f"token mint returned expires_in={lifetime}, which is already expired",
                status=status,
            )

        self._token = token
        # Absolute, not relative: a duration cannot survive being cached.
        self._expires_at = self._clock() + lifetime
        return token

    def _post_token_request(self) -> tuple[int, bytes]:
        """POST the grant; return the status and a bounded body."""
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
            #
            # `redirect=False` is the security-relevant one. urllib3 follows a 3xx by
            # default, and following it here would re-POST the form — client_secret
            # and all — to whatever origin the Location header names. A token endpoint
            # that has genuinely moved is a configuration change, not something to
            # chase at runtime with a credential in hand.
            #
            # `preload_content=False` hands back the stream so the read below can be
            # bounded, and `retries=False` keeps the one attempt one attempt: a POST
            # is outside urllib3's default retry set anyway, and `Retry` is also what
            # implements redirect-following.
            response = self._http.request(
                "POST",
                self._token_url,
                fields=params,
                encode_multipart=False,
                redirect=False,
                retries=False,
                timeout=self._timeout,
                preload_content=False,
            )
        except urllib3.exceptions.HTTPError as exc:
            # Safe to name the URL: `require_endpoint` refused a token_url carrying
            # userinfo, so there is no secret in this string.
            raise TransportError(f"token mint could not reach {self._token_url}: {exc}") from exc

        try:
            raw = response.read(MAX_TOKEN_RESPONSE_BYTES + 1)
        except urllib3.exceptions.HTTPError as exc:
            raise TransportError(
                f"token mint response from {self._token_url} was truncated: {exc}"
            ) from exc
        finally:
            # `preload_content=False` leaves the connection checked out. A body that
            # read to EOF goes back to the pool; an over-long one is dropped, which is
            # the right outcome for a response this client refuses to finish reading.
            response.release_conn()

        status = response.status
        if len(raw) > MAX_TOKEN_RESPONSE_BYTES:
            raise Unauthorized(
                f"token mint returned more than {MAX_TOKEN_RESPONSE_BYTES} bytes; "
                f"{self._token_url} does not look like a token endpoint",
                status=status,
            )
        if 300 <= status < 400:
            raise Unauthorized(
                f"token mint was redirected ({status}); refusing to re-send the "
                f"client_secret to a location this client did not configure",
                status=status,
            )
        if not 200 <= status < 300:
            raise Unauthorized(
                f"token mint failed ({status})",
                status=status,
                details=raw.decode("utf-8", "replace"),
            )
        return status, raw

    def __repr__(self) -> str:
        """Never the secret.

        The default repr is opaque, which is exactly why this is worth writing down:
        the moment someone adds a `dataclass` decorator or a debugging `__repr__` for
        the fields that ARE safe, the secret goes with it into every log line and
        pytest failure dump that touches this object.
        """
        return (
            f"ClientCredentials(token_url={self._token_url!r}, "
            f"client_id={self._client_id!r}, client_secret=<redacted>)"
        )


def _require_secret(value: str) -> str:
    """The client_secret, checked for the two ways it actually arrives wrong.

    Both failures present identically — `invalid_client`, from an issuer that will not
    say more, about a value nobody can safely paste into a bug report.

    ONE: whitespace. `TEMPER_M2M_CLIENT_SECRET=$(cat secret.txt)` keeps the trailing
    newline; so does a copy out of a terminal that soft-wrapped. `require_opaque`
    rejects it and names it.

    TWO: the fields are swapped. A temper-minted client_id is `tmpr_` + base64url
    (`crates/temper-services/src/auth/secret.rs`), and a temper-minted SECRET is
    base64url of 32 random bytes with no prefix at all — the chance one begins with
    those exact five characters is about 2^-30. So a secret that starts with `tmpr_`
    is a client id in the secret's slot, every time.
    """
    secret = require_opaque(value, "client_secret")
    if secret.startswith("tmpr_"):
        raise ValueError(
            "client_secret starts with `tmpr_`, which is the prefix on a temper "
            "client_id — the client_id and client_secret look swapped"
        )
    return secret
