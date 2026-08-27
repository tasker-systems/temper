"""The call seam: retry policy, 401 repair, and error translation in one place.

A cheap façade over the process-global connection. Holds a credential and nothing
else; constructing one does no I/O, so a threaded server can build one per user and a
worker process can memoize one.

WHY THERE ARE NO PER-ENDPOINT METHODS HERE. The gem hand-writes `Resources`,
`Contexts`, `CognitiveMaps` because a Ruby caller otherwise passes an untyped hash.
The generated Python core already answers that: every operation is a typed method
taking pydantic models, so a hand-written `resources.create(...)` would be a second,
worse spelling of something already correct — and a place for the two to drift. This
is the same call temper-ts makes, for the same reason, in `src/client.ts`.

What the generated core does NOT answer is which failures are worth retrying and who
repairs a dead token. That is what `call()` is.
"""

from __future__ import annotations

import time
from collections.abc import Callable
from typing import Any, TypeVar

import urllib3.exceptions

from temper import connection as _connection
from temper.credentials import Credentials
from temper.errors import TemperError, TransientError, Unauthorized, map_error
from temper.generated.api.profile_api import ProfileApi
from temper.generated.api.search_api import SearchApi
from temper.generated.api_client import ApiClient
from temper.generated.exceptions import ApiException
from temper.generated.models.profile_with_entitlements import ProfileWithEntitlements
from temper.generated.models.search_params import SearchParams
from temper.generated.models.search_response import SearchResponse

T = TypeVar("T")

#: 200ms, 400ms — mirroring MAX_ATTEMPTS and the backoff in
#: crates/temper-client/src/http.rs, and the gem's Client::DEFAULT_BACKOFF.
MAX_READ_ATTEMPTS = 3


def default_backoff(attempt: int) -> None:
    time.sleep(0.2 * (2 ** (attempt - 1)))


class Client:
    def __init__(
        self,
        credentials: Credentials,
        *,
        backoff: Callable[[int], None] = default_backoff,
    ) -> None:
        self._credentials = credentials
        self._backoff = backoff

    def call(self, fn: Callable[[ApiClient], T], *, idempotent: bool = False) -> T:
        """The one seam every call goes through.

        ``idempotent=True``  — a safe method. 5xx and transport failures retry.
        ``idempotent=False`` — a write. NEVER auto-retried.

        A 401 is repaired once, for reads and writes alike: re-authenticating is not
        re-submitting. A credential that cannot mint gets its 401 back untouched —
        `BearerToken.refresh()` raises, and raising here would replace temper's real
        answer with a message about the client's own plumbing.

        Usage::

            client.call(
                lambda api: ResourcesApi(api).get_resource(resource_id),
                idempotent=True,
            )
        """
        attempt = 0
        reminted = False
        api = _connection.api_client()

        while True:
            attempt += 1
            try:
                with _connection.with_token(self._credentials.token()):
                    return fn(api)
            except (ApiException, urllib3.exceptions.HTTPError) as caught:
                # Bound outside the `except` so the retry/repair decisions below read as
                # straight-line code rather than as branches nested in a handler. The
                # cost is that Python's implicit chaining does not apply out here — hence
                # the explicit `from raw` on the raise, without which the caller's
                # traceback would stop at the mapped error and lose the urllib3 or
                # ApiException frame that says what actually happened.
                error, raw = map_error(caught), caught

            if self._repair_credentials(error, reminted):
                reminted = True
                continue

            if not self._retryable_read(error, idempotent, attempt):
                raise error from raw

            self._backoff(attempt)

    # -- conveniences the gem also puts on Client itself ----------------------

    def whoami(self) -> ProfileWithEntitlements:
        """Assert the machine profile resolved, and report what it can reach.

        Authentication is not authorization. A minted M2M token does not even yield a
        profile on its own: the client_id must already be registered (lookup-or-401 —
        there is no JIT-create branch), and then, without a cogmap write grant and team
        membership, every call authenticates cleanly and 403s. Discovering that here
        beats discovering it on the first write.
        """
        return self.call(lambda api: ProfileApi(api).get_profile(), idempotent=True)

    def search(self, query: str, **opts: Any) -> SearchResponse:
        """`SearchParams` names the field `query`, not `q`."""
        params = SearchParams(query=query, **opts)
        return self.call(lambda api: SearchApi(api).search(params), idempotent=True)

    # -- internals -----------------------------------------------------------

    def _repair_credentials(self, error: TemperError, reminted: bool) -> bool:
        if not isinstance(error, Unauthorized) or reminted:
            return False
        if not self._credentials.can_refresh:
            # The gem reaches the same outcome by letting BearerToken#refresh! raise
            # its own Unauthorized. Asking first is the same decision made where it
            # can be read, and it keeps the SERVER's 401 — body and all — as the
            # error the caller sees.
            return False
        self._credentials.refresh()
        return True

    @staticmethod
    def _retryable_read(error: TemperError, idempotent: bool, attempt: int) -> bool:
        return idempotent and isinstance(error, TransientError) and attempt < MAX_READ_ATTEMPTS
