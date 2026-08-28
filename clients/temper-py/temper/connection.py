"""One connection per process, one token per call.

The generated `Configuration` holds `access_token` as a plain attribute, and
`ApiClient` builds its urllib3 `PoolManager` in `__init__`. Put those two facts
together and the obvious design — a fresh `Configuration` + `ApiClient` per request,
so each caller gets its own token — is a trap: fresh client, fresh pool, TLS
handshake per request. The gem's `connection.rb` header documents the same trap for
Faraday.

So the CONNECTION is process-global and the TOKEN is call-scoped, via a property on
a `Configuration` subclass that reads a `ContextVar`. `Configuration.__setattr__`
delegates to `object.__setattr__`, which honours a data descriptor on the class, so
the base class's own ``self.access_token = access_token`` in `__init__` goes through
the setter like any other write.

WHY A ContextVar AND NOT threading.local. A fresh OS thread starts with an empty
context, so a `ContextVar` is thread-local for free — and it is ALSO correct under
asyncio and greenlet-based servers, where a `threading.local` is shared by every task
on the thread and would leak one caller's token into another's request. The gem uses
Ruby's fiber-local `Thread.current[]` for exactly this reason.

The token never lands in `Configuration.__dict__`, so `copy.deepcopy(configuration)`
(which the generated `__deepcopy__` drives off `__dict__`) yields a configuration with
no token. Nothing on the request path deep-copies it; do not start.

WHAT `configure()` WILL AND WILL NOT PASS THROUGH. Its `**configuration_kwargs` used
to reach the generated `Configuration` untouched, and three of that constructor's
arguments turn off a security control from a keyword one typo wide. `debug=True` sets
`httplib.HTTPConnection.debuglevel`, a CLASS attribute, so every HTTP request in the
process starts printing its headers — Authorization included — to stdout.
`verify_ssl=False` reaches `ssl.CERT_NONE`, and `assert_hostname=False` keeps
verification on while dropping the half that ties a certificate to a host. All three
fail OPEN: the request still succeeds, so nothing goes red. `FORWARDABLE_SETTINGS`
below is therefore an allowlist rather than a denylist, and it fails closed on a name
the generator adds later.
"""

from __future__ import annotations

import threading
from collections.abc import Iterator
from contextlib import contextmanager
from contextvars import ContextVar
from typing import Any

from temper._validate import require_endpoint, require_opaque
from temper.generated.api_client import ApiClient
from temper.generated.configuration import Configuration as GeneratedConfiguration

_TOKEN: ContextVar[str | None] = ContextVar("temper_access_token", default=None)

#: The attribution marker, sent on every request. It names the KIND of surface,
#: never the client's language — the gem sends this same `sdk`
#: (clients/temper-rb/lib/temper/connection.rb) and so does temper-ts
#: (src/auth-fetch.ts). There is deliberately no override: the server trusts
#: `{sdk, cli}` and attributes a `cli` write to the `<handle>@cli` emitter, so a knob
#: here would be a knob for writing a lie into the event ledger.
SURFACE = "sdk"

#: The generated `Configuration` settings `configure()` will pass through.
#:
#: An ALLOWLIST, not a denylist, and that direction is the whole point. `Configuration`
#: takes twenty-odd keyword arguments; three of them silently disable a security
#: control, several more are owned by this module, and the generator adds to the list
#: on its own schedule. A denylist would be correct only until the next regeneration.
#:
#: What survives the filter is the set a real deployment needs and nothing else: how to
#: TRUST a peer (a private CA, a client certificate, an SNI name), how to REACH it (a
#: proxy, a pool size, socket options), and how to spell a date.
FORWARDABLE_SETTINGS = frozenset(
    {
        "ssl_ca_cert",
        "ca_cert_data",
        "cert_file",
        "key_file",
        "tls_server_name",
        "connection_pool_maxsize",
        "proxy",
        "proxy_headers",
        "socket_options",
        "datetime_format",
        "date_format",
    }
)

#: Settings that are refused BY NAME, each with the reason it is refused.
#:
#: Everything here would otherwise be reachable as a keyword one character away from
#: something harmless, and every one of them fails open — the request still succeeds,
#: so nothing in a test suite goes red.
REFUSED_SETTINGS: dict[str, str] = {
    "debug": (
        "`debug=True` sets httplib.HTTPConnection.debuglevel = 1, which is a CLASS "
        "attribute: every HTTP request in the process — this package's and anyone "
        "else's — starts printing its request headers to stdout, Authorization "
        "bearer token included, into whatever collects that process's logs. Raise "
        "the level on logging.getLogger('temper.generated') or 'urllib3' instead; "
        "neither touches httplib"
    ),
    "verify_ssl": (
        "`verify_ssl=False` sets ssl.CERT_NONE on the pool, which accepts any "
        "certificate from anything that answers — the bearer token then goes to "
        "whoever intercepted the connection. To trust a private CA pass "
        "`ssl_ca_cert` (a PEM path) or `ca_cert_data`; for a name mismatch pass "
        "`tls_server_name`"
    ),
    "assert_hostname": (
        "`assert_hostname=False` keeps certificate verification on but stops "
        "checking that the certificate is for the host you dialled, which is the "
        "half that makes it mean anything. Pass `tls_server_name` to name the SNI "
        "host the certificate actually carries"
    ),
    "access_token": (
        "the token is call-scoped, not connection-scoped — one connection serves "
        "every concurrent caller with their own identity. Pass a credential to "
        "Client, or use temper.with_token()"
    ),
    "api_key": "temper authenticates with a bearer token; pass a credential to Client",
    "api_key_prefix": "temper authenticates with a bearer token; pass a credential to Client",
    "username": "temper has no HTTP basic auth; pass a credential to Client",
    "password": "temper has no HTTP basic auth; pass a credential to Client",
    "host": "the origin is `base_url`",
    "server_index": "the origin is `base_url`",
    "server_variables": "the origin is `base_url`",
    "server_operation_index": "the origin is `base_url`",
    "server_operation_variables": "the origin is `base_url`",
    "ignore_operation_servers": "the origin is `base_url`",
    "retries": (
        "retry policy belongs in Client.call, which knows whether the operation is "
        "idempotent; urllib3 does not, and its defaults re-send PUT and DELETE"
    ),
    "client_side_validation": (
        "disabling it moves a malformed request from a local error to a server "
        "round-trip that fails less legibly"
    ),
    "safe_chars_for_path_param": (
        "widening path-parameter escaping lets a path segment carry `/` or `?` into "
        "the request line"
    ),
}


def _admissible_settings(settings: dict[str, Any]) -> dict[str, Any]:
    """Filter `configure()`'s passthrough kwargs, or explain why one cannot pass.

    Fails CLOSED on a name it does not recognise. A setting the generator adds
    tomorrow arrives here unreviewed, and "unreviewed" is how `debug` would have got
    in; a caller who needs one says so in a diff to `FORWARDABLE_SETTINGS`, where the
    question of what it does gets asked once.
    """
    for name in settings:
        reason = REFUSED_SETTINGS.get(name)
        if reason is not None:
            raise ValueError(f"{name} is not configurable here: {reason}")
        if name not in FORWARDABLE_SETTINGS:
            raise TypeError(
                f"configure() got an unexpected keyword argument {name!r}; "
                f"the settings it forwards are {sorted(FORWARDABLE_SETTINGS)}"
            )
    return settings


class TokenScopedConfiguration(GeneratedConfiguration):
    """A generated `Configuration` whose `access_token` is per-call, not per-object."""

    @property
    def access_token(self) -> str | None:
        return _TOKEN.get()

    @access_token.setter
    def access_token(self, value: str | None) -> None:
        # Reached once, from the base class's __init__. A caller setting a token here
        # would be setting it for whatever context happens to be current, which is
        # never what they meant — `with_token` is the seam.
        if value is not None:
            raise AttributeError(
                "access_token is call-scoped on this configuration; use temper.with_token()"
            )


def current_token() -> str | None:
    """The token the current context will authenticate with, if any."""
    return _TOKEN.get()


@contextmanager
def with_token(token: str) -> Iterator[None]:
    """Bind `token` for the duration of the block, restoring what was there before.

    The value becomes an `Authorization: Bearer ...` header verbatim, so it is checked
    for the whitespace and control characters a header cannot carry. `Client` binds
    tokens that were already checked at construction or at the mint; this is the seam
    a caller can reach directly.
    """
    reset = _TOKEN.set(require_opaque(token, "token"))
    try:
        yield
    finally:
        _TOKEN.reset(reset)


class Connection:
    """The process-global `ApiClient`, plus the settings that built it.

    Constructed through `configure()`; reached through `api_client()`.
    """

    def __init__(
        self,
        *,
        base_url: str,
        device_id: str | None = None,
        allow_insecure_http: bool = False,
        **configuration_kwargs: Any,
    ) -> None:
        self.base_url = _normalize_host(base_url, allow_insecure_http=allow_insecure_http)
        # Straight into an `X-Temper-Device-Id` header, so it gets the header-safety
        # check rather than urllib3's, which fires per request and blames the request.
        self.device_id = None if device_id is None else require_opaque(device_id, "device_id")
        # Filtered EAGERLY: `_build()` is lazy, so an unfiltered kwarg would raise on
        # the first API call rather than at the `configure()` that wrote it.
        self._configuration_kwargs = _admissible_settings(configuration_kwargs)
        self._lock = threading.Lock()
        self._api_client: ApiClient | None = None

    def api_client(self) -> ApiClient:
        # Double-checked under a lock: the fast path is an attribute read, and the
        # slow path builds exactly one pool no matter how many threads arrive at once.
        client = self._api_client
        if client is not None:
            return client
        with self._lock:
            if self._api_client is None:
                self._api_client = self._build()
            return self._api_client

    def close(self) -> None:
        """Drop the pooled sockets. Call from a post-fork hook.

        A forked worker inherits its parent's open sockets, and two processes reading
        one TLS connection is a corrupted stream, not a slow one. Unlike the gem —
        whose `connection_pool >= 2.4` drops pooled sockets from a `Process._fork` hook
        by itself — urllib3 has no fork hook, so this is the ONLY thing standing
        between a forked worker and its parent's sockets. Register it:

            os.register_at_fork(after_in_child=temper.reset_connection)
        """
        with self._lock:
            client = self._api_client
            self._api_client = None
        if client is not None:
            # The generated ApiClient has no `close()` — its `__exit__` is a no-op —
            # so the sockets are reached through the pool it built. `clear()` closes
            # every idle connection and empties the pool; a request in flight on
            # another thread keeps its own connection and is unaffected.
            client.rest_client.pool_manager.clear()

    def _build(self) -> ApiClient:
        configuration = TokenScopedConfiguration(
            host=self.base_url,
            # urllib3 retries on its own by default (`Retry(3)`), and its default
            # allowed-methods set includes PUT and DELETE — so a read timeout on a
            # temper write would be re-sent, silently, one layer below the seam that
            # exists to decide exactly that. Retry policy belongs in `Client.call`,
            # which knows whether the operation is idempotent. `False` makes urllib3
            # re-raise the original error rather than wrapping it in MaxRetryError.
            retries=False,
            **self._configuration_kwargs,
        )
        client = ApiClient(configuration)
        client.default_headers["X-Temper-Surface"] = SURFACE
        if self.device_id:
            client.default_headers["X-Temper-Device-Id"] = self.device_id
        return client


def _normalize_host(base_url: str, *, allow_insecure_http: bool = False) -> str:
    """The instance origin, trailing slash stripped.

    The generated core joins `host` with a path that already starts with `/`, so a
    trailing slash yields `//api/...` — which temper's router answers with a 404 that
    reads like a missing route rather than like a config typo.

    A path PREFIX survives, because the same join makes `https://host/temper` +
    `/api/...` the correct address for an instance mounted under one. A query or a
    fragment does not survive, because that same join would bury it mid-URL.
    """
    require_endpoint(base_url, name="base_url", allow_insecure_http=allow_insecure_http)
    return base_url.rstrip("/")


_default_lock = threading.Lock()
_default: Connection | None = None


def configure(
    *,
    base_url: str,
    device_id: str | None = None,
    allow_insecure_http: bool = False,
    **configuration_kwargs: Any,
) -> Connection:
    """Install the process-global connection. Credentials are NOT here — they are per-call.

    `base_url` must be https unless it names the loopback interface;
    `allow_insecure_http=True` accepts plaintext elsewhere, deliberately.

    The remaining keyword arguments reach the generated `Configuration`, and only the
    ones in `FORWARDABLE_SETTINGS` do: see `REFUSED_SETTINGS` for what the others
    would have switched off.
    """
    global _default
    connection = Connection(
        base_url=base_url,
        device_id=device_id,
        allow_insecure_http=allow_insecure_http,
        **configuration_kwargs,
    )
    with _default_lock:
        previous, _default = _default, connection
    if previous is not None:
        previous.close()
    return connection


def current_connection() -> Connection:
    """The installed process-global connection, or a refusal to guess at one.

    NAMED `current_connection`, NOT `connection`. `temper/__init__.py` re-exports the
    names in this module, and a function called `connection` would rebind the
    `temper.connection` attribute from this MODULE to itself — so
    `from temper import connection` would hand a caller (and a test's monkeypatch) the
    function while every reader assumed the module. That is not hypothetical: this
    package's own test isolation fixture set `_default` on the function object and the
    leak it was written to prevent went on leaking.
    """
    with _default_lock:
        current = _default
    if current is None:
        raise RuntimeError("temper is not configured; call temper.configure(base_url=...) first")
    return current


def api_client() -> ApiClient:
    return current_connection().api_client()


def reset_connection() -> None:
    """Drop the process-global connection's sockets. Safe to call when unconfigured."""
    with _default_lock:
        current = _default
    if current is not None:
        current.close()
