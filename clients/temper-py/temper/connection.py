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
"""

from __future__ import annotations

import threading
from collections.abc import Iterator
from contextlib import contextmanager
from contextvars import ContextVar
from typing import Any
from urllib.parse import urlsplit

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
    """Bind `token` for the duration of the block, restoring what was there before."""
    reset = _TOKEN.set(token)
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
        **configuration_kwargs: Any,
    ) -> None:
        self.base_url = _normalize_host(base_url)
        self.device_id = device_id
        self._configuration_kwargs = configuration_kwargs
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


def _normalize_host(base_url: str) -> str:
    """The instance origin, trailing slash stripped.

    The generated core joins `host` with a path that already starts with `/`, so a
    trailing slash yields `//api/...` — which temper's router answers with a 404 that
    reads like a missing route rather than like a config typo.
    """
    if not isinstance(base_url, str) or not base_url:
        raise ValueError("base_url must be a non-empty str")
    parts = urlsplit(base_url)
    if parts.scheme not in ("http", "https") or not parts.netloc:
        raise ValueError(f"base_url must be an absolute http(s) URL, got {base_url!r}")
    return base_url.rstrip("/")


_default_lock = threading.Lock()
_default: Connection | None = None


def configure(
    *,
    base_url: str,
    device_id: str | None = None,
    **configuration_kwargs: Any,
) -> Connection:
    """Install the process-global connection. Credentials are NOT here — they are per-call."""
    global _default
    connection = Connection(base_url=base_url, device_id=device_id, **configuration_kwargs)
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
