# temper-py

The Python client for the [Temper](https://github.com/tasker-systems/temper)
knowledge-base API. Sibling of `temper-rb` and `temper-ts` — all three are pinned to
the same OpenAPI contract. Workspace-isolated: this package is not a cargo member and
not a bun workspace member, so nothing in the repo builds it implicitly.

```python
import temper
from temper.generated.api.resources_api import ResourcesApi

temper.configure(base_url="https://temperkb.io")

client = temper.Client(
    temper.ClientCredentials(
        token_url="https://temperkb.io/oauth/token",
        client_id="tmpr_...",
        client_secret="...",
    )
)

client.whoami()
client.call(lambda api: ResourcesApi(api).get_resource(resource_id), idempotent=True)
```

## What is generated and what is not

`temper/generated/**` is emitted from the repo-root `openapi.json` — itself a product
of the Axum router — by a pinned `openapi-generator`. **Never hand-edit it.** A new
field on a response DTO restales it exactly as it restales `openapi.json` itself, the
`temper-rb` gem, and `temper-ts`'s `schema.ts`:

```bash
cargo make openapi      # regenerates the spec and all three SDKs
cargo make openapi-py   # just this package
```

`cargo make check` runs `openapi-py-drift`, which regenerates and diffs. The
`test-python` CI job runs the same check and never skips, so a contract change that
leaves this package behind cannot merge.

Everything outside `temper/generated/` is hand-written, and
`.openapi-generator-ignore` keeps the generator off it.

| Module | What it is |
|---|---|
| `temper.connection` | One `ApiClient` (one urllib3 pool) per process; the token is call-scoped via a `ContextVar` |
| `temper.credentials` | `BearerToken` and `ClientCredentials`, pinned to `tests/contracts/m2m-token-request.json` |
| `temper.errors` | The transient/permanent split, and `map_error` |
| `temper.client` | `Client.call()` — the one seam carrying retry policy and 401 repair |
| `temper.act` | `ActInput`'s seven wire keys, with the confidence invariant enforced at construction |
| `temper.refs` | `parse_ref` — a port of `temper_workflow::operations::parse_ref` |

### There are no per-endpoint wrapper methods, deliberately

The gem hand-writes `Resources`, `Contexts`, `CognitiveMaps` because a Ruby caller
otherwise passes an untyped hash. The generated Python core already answers that:
every operation is a typed method over pydantic models. A hand-written
`resources.create(...)` would be a second, worse spelling of something already
correct — and a place for the two to drift. `temper-ts` declines the same wrappers
for the same reason.

What the generated core does *not* answer is which failures are worth retrying and
who repairs a dead token. That is what `Client.call()` is:

```python
client.call(fn, idempotent=True)  # a safe method: 5xx and transport failures retry
client.call(fn)  # a write: NEVER auto-retried
```

A 401 is repaired once either way — re-authenticating is not re-submitting. A
`BearerToken` cannot mint, so its 401 comes back untouched rather than being replaced
by a message about the client's own plumbing.

## Connections and forking

`temper.configure()` installs a process-global connection: one `ApiClient`, one
urllib3 pool, one TLS handshake amortized across every call. The access token is *not*
on it — it is bound per call from the `Client`'s credential, through a `ContextVar`,
so one connection serves every concurrent caller with their own identity.

urllib3 has no fork hook (the gem gets one free from `connection_pool >= 2.4`), so a
forking server must drop the inherited sockets itself:

```python
import os, temper

os.register_at_fork(after_in_child=temper.reset_connection)
```

## Errors

`map_error` translates the generated core's `ApiException` — and the raw `urllib3`
errors a transport failure raises — into a tree whose top-level split is the one that
matters operationally:

- `TransientError` → `ServerError`, `RateLimited` (with `retry_after`), `TransportError`.
  Let these escape a job; a retry is what fixes them.
- `PermanentError` → `Unauthorized`, `Forbidden`, `SystemAccessRequired`, `NotFound`,
  `Conflict`, `BadRequest`. Catch these and dead-letter them.

`SystemAccessRequired.refusal` returns the typed refusal the server sent (`Denied`,
`Revoked`, `IllegalTransition`, …) so a worker can tell "never granted" from "granted
and then revoked" without matching on a message string. `refusal_kind` gives the raw
discriminator when this build predates the kind the server named.

> The gem calls the transport failure `Temper::ConnectionError`. Here it is
> `TransportError`, because `ConnectionError` is a Python builtin and shadowing it
> would make `except ConnectionError` silently catch the wrong thing.

## Development

```bash
uv sync --group dev                              # honours .python-version
uv run ruff check . && uv run ruff format --check .
uv run mypy
uv run pytest
uv build                                          # the gemspec-equivalent smoke test
```

There is no committed lockfile — a library states what it works with rather than
freezing what its consumers resolve, the same call `temper-rb` makes by gitignoring
`Gemfile.lock`. What a lockfile would have bought is bought instead by pinning `ruff`
and `mypy` EXACTLY in `pyproject.toml`: those two can redden CI on their own release
schedule, and nothing else in the dev group can.

Supported interpreters are **3.10+**. That is above the generated package's own `>= 3.9`
on purpose: 3.9 is end-of-life, and mypy 2.x already requires 3.10, so a 3.9 floor would
promise a version whose type-check we could not run.

Regenerating needs no Python at all — a Rust developer who changed a DTO can run
`cargo make openapi-py` with either Docker or a JVM. The generator pin and its
parameters live in one place, `.github/scripts/generate-temper-py.sh`, shared by
cargo-make and the drift gate.
