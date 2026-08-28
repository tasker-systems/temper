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
| `temper._validate` | The admission checks the seams share: endpoints, and values that become headers |

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


### What `configure()` will pass through

Keyword arguments beyond `base_url` and `device_id` reach the generated
`Configuration`, and only the ones on an **allowlist** do:

```python
temper.configure(
    base_url="https://temper.internal",
    ssl_ca_cert="/etc/ssl/private-ca.pem",  # trust a private CA
    tls_server_name="temper.internal",  # the SNI name the cert carries
    connection_pool_maxsize=32,
    proxy="http://proxy.internal:3128",
)
```

`ssl_ca_cert`, `ca_cert_data`, `cert_file`, `key_file`, `tls_server_name`,
`connection_pool_maxsize`, `proxy`, `proxy_headers`, `socket_options`,
`datetime_format`, `date_format` — that is the whole list, and an unrecognised name
is a `TypeError` rather than a silent passthrough. Three of the arguments it refuses
are why the list is an allowlist:

| Refused | What it would have done |
|---|---|
| `debug=True` | Sets `httplib.HTTPConnection.debuglevel = 1`, a **class** attribute — every HTTP request in the process starts printing its request headers to stdout, `Authorization: Bearer …` included. Raise the level on the `temper.generated` or `urllib3` logger instead; neither touches httplib. |
| `verify_ssl=False` | `ssl.CERT_NONE` on the pool: any certificate from anything that answers, and the bearer token goes to whoever intercepted the connection. Use `ssl_ca_cert` / `ca_cert_data`. |
| `assert_hostname=False` | Keeps verification on but stops checking the certificate is for the host you dialled. Use `tls_server_name`. |

The rest are refused because this module owns them (`host`, `retries`, the
`server_*` family) or because credentials are call-scoped, not connection-scoped
(`access_token`, `api_key`, `username`/`password`). Every refusal names its reason,
and all of them fire at `configure()` — not at the first API call, which is when a
lazily-built `Configuration` would have raised.

### Endpoints, and plaintext http

`base_url` and `ClientCredentials(token_url=...)` must be absolute `https` URLs, with
no userinfo (`https://id:secret@host` puts the secret in every error message that
names the URL) and no query or fragment. Plaintext `http` is accepted for the
loopback interface — a test server, a `temper serve` on your laptop — and refused
anywhere else, because a bearer token and a `client_secret` both travel in the clear
over it. Where TLS genuinely terminates elsewhere, say so:

```python
temper.configure(base_url="http://temper.internal", allow_insecure_http=True)
```

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

### Composition bounds raise BEFORE the request `[2026-08-28]`

`/api/query`'s contract publishes ceilings on what one composition may declare, and the
generated pydantic models enforce them locally — so these surface as a `ValidationError`
at construction (and, since `validate_assignment` is on, at mutation) rather than as a
`BadRequest` carrying a typed refusal:

| field | ceiling |
|---|---|
| `Composition.stages` | 64 |
| `Intention.query` | 4096 |
| `IdSet.ids` | 256 |
| `ResourceFilter.doc_type` / `.tags`, `EdgeFilter.labels` | 64 |

**This is a behaviour change for code that already builds large plans**: a 300-id `IdSet`
used to construct fine and reach the server. It now raises before any HTTP call.

**The client counts characters; the server counts bytes.** A 4096-character CJK question is
8192 bytes — it constructs cleanly here and is refused server-side as `intention_too_long`.
The skew is one-directional by construction (a UTF-8 string is never fewer bytes than
characters), so the client can only ever under-enforce, never refuse something the server
would have run.

Two ceilings are deliberately NOT enforced here, because neither is a contract fact: the
per-stage predicate and probe caps, and the aggregate embed budget
(`intention_budget_exceeded`) — what a deployment can embed in one request is a property of
that deployment. Those arrive as refusals.


## Credentials

`BearerToken(token)` and `ClientCredentials(...)` check their inputs at construction,
because the two ways a credential arrives wrong both surface as an unexplained
`invalid_client` or 401 hours later:

- **Whitespace.** `TEMPER_M2M_CLIENT_SECRET=$(cat secret.txt)` keeps the trailing
  newline. Rejected rather than stripped — a stripped value is a guess, and the same
  guess is wrong for a space in the middle of a secret.
- **Swapped fields.** A `client_secret` beginning `tmpr_` is a temper *client id* in
  the secret's slot; temper mints secrets as bare base64url.

The mint itself is deliberately unadventurous. It does not follow redirects (that
would re-POST the `client_secret` to whatever origin the `Location` names), it reads
at most 64 KiB of response, it applies a connect/read timeout — the mint runs under a
lock, so an issuer that accepts the connection and never answers would otherwise
block every thread in the process, not one — and it treats a 200 that carries no
usable `access_token`, `token_type` or `expires_in` as a credential failure rather
than letting a `KeyError` out of `token()`. A mint that fails drops the cached token
first, so a caller never goes on presenting one the server has already rejected.

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
