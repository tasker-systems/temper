"""The Python client for the Temper knowledge base API.

Sibling of `temper-rb` and `temper-ts`; the three are pinned to the same wire
contracts. `temper/generated/` is emitted from the repo-root `openapi.json` — itself
a product of the Axum router — and is NEVER hand-edited: `cargo make openapi`
regenerates it, and `openapi-py-drift` (in `cargo make check`, and in CI) fails if the
committed copy has fallen behind the contract.

    import temper
    from temper.generated.api.resources_api import ResourcesApi

    temper.configure(base_url="https://temperkb.io")
    client = temper.Client(temper.ClientCredentials(
        token_url="https://temperkb.io/oauth/token",
        client_id="tmpr_...",
        client_secret="...",
    ))

    client.whoami()
    client.call(lambda api: ResourcesApi(api).get_resource(uuid), idempotent=True)
"""

from temper.act import Act
from temper.client import MAX_READ_ATTEMPTS, Client, default_backoff
from temper.connection import (
    SURFACE,
    Connection,
    api_client,
    configure,
    current_connection,
    current_token,
    reset_connection,
    with_token,
)
from temper.credentials import BearerToken, ClientCredentials, Credentials
from temper.errors import (
    BadRequest,
    Conflict,
    Forbidden,
    NotFound,
    PermanentError,
    RateLimited,
    ServerError,
    SystemAccessRequired,
    TemperError,
    TransientError,
    TransportError,
    Unauthorized,
    map_error,
)

# The contract this package was generated against. `generate-temper-py.sh` passes
# openapi.json's info.version to the generator as `packageVersion`, so the generated
# tree already carries it — we alias it rather than reasserting it, and callers never
# reach into `temper.generated` for it. Independent of `__version__`, which is the
# SDK's own SemVer: a package version and an API version answer different questions.
from temper.generated import __version__ as CONTRACT_VERSION
from temper.refs import parse_ref
from temper.version import __version__

__all__ = [
    "CONTRACT_VERSION",
    "MAX_READ_ATTEMPTS",
    "SURFACE",
    "Act",
    "BadRequest",
    "BearerToken",
    "Client",
    "ClientCredentials",
    "Conflict",
    "Connection",
    "Credentials",
    "Forbidden",
    "NotFound",
    "PermanentError",
    "RateLimited",
    "ServerError",
    "SystemAccessRequired",
    "TemperError",
    "TransientError",
    "TransportError",
    "Unauthorized",
    "__version__",
    "api_client",
    "configure",
    "current_connection",
    "current_token",
    "default_backoff",
    "map_error",
    "parse_ref",
    "reset_connection",
    "with_token",
]
