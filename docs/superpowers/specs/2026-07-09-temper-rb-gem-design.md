# temper-rb — the gem, and the contract repair it needs first

**Date:** 2026-07-09
**Status:** Design approved. Supersedes the P4 section of
[2026-07-09-temper-rb-ruby-bindings-design.md](2026-07-09-temper-rb-ruby-bindings-design.md), which
scoped four preamble beats but deliberately left the gem unspecced.
**Beats:** P0 ✅ (#341) · P1 ✅ (#340) · P2 ✅ (#343) · P3 ✅ (#345) · **P5 (new)** · then the gem.

This document does two things. It specifies **P5**, a fifth preamble beat that discovery forced into
existence, and it records the **full design of the gem** so the decisions behind it survive the gap.

Decisions continue the prior doc's numbering (D1–D7).

---

## What discovery established

The prior doc's design was approved on reasoning. This one was written after running the actual
generator against the actual contract and reading the emitted Ruby. Two of the prior doc's premises
did not survive that.

### The contract does not generate. It is not valid OpenAPI.

`openapi-generator` 7.24 (run via Docker; no JVM required) against `openapi.json` emits **zero
files**. Two defects, both ours — utoipa behaves correctly in each case. P0 made the spec
*authoritative*. It never made it *correct*, and the `openapi-check` gate compares router paths to
spec paths, so it is blind to both.

This is the subject of P5, below.

### "Ruby never needs `ort`" and "cogmaps are in scope" cannot both hold as written

The prior doc rejected treating cogmaps as a peripheral tier: *"shipping a client that reads a map
but cannot author into one is not a smaller scope; it is a broken one."* It also argued Ruby never
needs an embedder, because *"the server recomputes them as an unconditional fallback."*

The fallback is not unconditional. It exists on ingest and nowhere near reconcile.
`crates/temper-core/src/types/reconcile.rs:1` opens:

> The PUT body is a PRE-EMBEDDED desired-state manifest: the CLI embeds (`compute_body_chunks`)
> before sending, so the server stays embed-free on the request path.
>
> These are Rust-only CLI↔API wire types.

`ReconcileEntry.chunks_packed: String` and `ReconcileTelosBlock.chunks_packed: String` are required,
not `Option`. `prepare_telos_blocks` (`crates/temper-services/src/backend/db_backend.rs:201`) unpacks
them verbatim — its doc-comment says *"client-embedded chunks, carried verbatim — NO server-side
ONNX."* And `reconcile.rs:20` states `chunks_packed` is *"the SOLE, AUTHORITATIVE body content — the
entry carries no raw `body`."* There is no text for a server to fall back on.

So bulk cogmap authoring is the one path a Ruby client physically cannot reach without a 768-dim BGE
embedder. Resolved by **D9**.

### Corrections to the prior doc, recorded so they are not re-derived

- `HttpClient::new` calls `Client::builder().timeout(Duration::from_secs(30)).build()`
  (`crates/temper-client/src/http.rs:125`), not the bare `.build()` the prior doc quotes. A timeout
  *is* set. The client also retries — `MAX_ATTEMPTS = 3`, 200ms/400ms backoff, and `should_retry`
  (`http.rs:52`) fires only on **safe methods (GET/HEAD)** for transport errors and 5xx. Writes are
  never retried.
- The fresh-pool-per-construction trap is **real but bounded**: `with_token_override` is called once
  per CLI process (via `build_client_from`, `config.rs:100`), not per request. It bites a long-lived
  host, which is exactly what the gem is.
- Auth env vars are `JWKS_URL`, `AUTH_ISSUER`, `AUTH_AUDIENCE`
  (`crates/temper-services/src/config.rs:20`) — **not** `AUTH0_*`.
- Machine profiles **auto-provision on first M2M call** under the `auth0-m2m` namespace
  (`profile_service::resolve_machine_from_claims` → `create_agent_profile_and_link`). There is no
  registration step. `normalize_machine` (`crates/temper-services/src/auth/normalize.rs:43`) keys on
  `gty == "client-credentials"` — hyphen, not underscore.
- `temper-client` has **no refresh-and-retry on 401**. Its refreshing method exists but no sub-client
  calls it on the request path. It is a poor template for the gem's auth.
- P3 **has landed** (#345). `ActContext.correlation` and `ActInput.correlation_id` both exist, typed
  `Option<CorrelationId>`, and `CorrelationId` is a registered component schema. The gem designs
  against it directly rather than waiting for it.

---

# Part 1 — P5: make the emitted contract generate

## Defect 1 — two `$ref`s point at nothing

`GET /api/resources` declares `sort` as `oneOf: [{type: null}, {$ref: ".../ResourceSortField"}]`, and
`order` against `SortOrder`. Neither component exists — post-P3, 153 schemas are defined and those
two are still missing.

Both enums derive `ToSchema` (`crates/temper-workflow/src/types/resource.rs:108,126`), so utoipa
correctly emits a `$ref`. But `components(schemas(...))` (`crates/temper-api/src/openapi.rs:28`) is a
hand-maintained ~95-entry list, and they are not in it. `.routes()` auto-collects schemas reachable
from request and response **bodies**; these two are reachable only through an `IntoParams` query
struct (`ResourceListParams`), which it does not walk.

**P3 supplies the control case.** It added a `correlation_id` query parameter to
`DELETE /api/resources/{id}` and `PUT /api/cognitive-maps/{id}` whose schema is
`oneOf: [{type: null}, {$ref: ".../CorrelationId"}]` — structurally identical to the two broken
params. Yet `CorrelationId` resolves, and nobody hand-registered it. It resolves because it *also*
hangs off `ActInput`, a request-**body** schema, and collection is transitive from bodies.
`ResourceSortField` and `SortOrder` hang off nothing but an `IntoParams` struct.

So the rule is exact: **a schema reachable only from `IntoParams` is never collected.** Every
query-only enum added in future re-breaks the contract, which is why the fix ships with a test rather
than only two registrations.

> **This settles an open thread from P2.** That session note suspected `components(schemas(...))` was
> *"plausibly deletable now that `.routes()` auto-collects schemas."* It is not. `.routes()` does not
> cover `IntoParams`, and deleting the list would dangle far more than two refs.

The 3.1 dereferencer throws rather than warns:

```
ERROR ReferenceVisitor - Error resolving schema #/components/schemas/ResourceSortField
java.lang.RuntimeException: Could not find /components/schemas/ResourceSortField
```

Zero files are emitted.

## Defect 2 — `operationId` is not unique

OpenAPI requires `operationId` to be unique across the document. We never set one, so utoipa falls
back to the handler's fn name. 79 operations collapse to **62 unique ids** — 17 excess across 7
colliding ids, involving 24 operations:

| opId | operations |
| --- | --- |
| `create` | POST contexts, ingest, resources, teams, teams/{id}/invite |
| `list` | GET contexts, invocations, resources, **resources/{id}/edges**, teams, teams/{id}/invitations |
| `update` | PUT ingest/{id}; PATCH profile, resources/{id}, teams/{id} |
| `get` | GET contexts/{id}, profile, resources/{id} |
| `grant` | POST cognitive-maps/{id}/grants, resources/{id}/grants |
| `revoke` | DELETE cognitive-maps/{id}/grants, resources/{id}/grants |
| `delete` | DELETE resources/{id}, teams/{id} |

Generation requires `--skip-validate-spec` (18 reported errors). That flag would also suppress the
next dangling `$ref` and the next malformed schema — it is a permanent blindfold, not a workaround.

**Exactly one collision is within a tag**, which is the painful kind, because openapi-generator
partitions methods into one class per tag:

- `crates/temper-api/src/handlers/resources.rs:59` — `pub async fn list`, `tag = "Resources"`
- `crates/temper-api/src/handlers/edges.rs:35` — `pub async fn list`, `tag = "Resources"`

Both want to be `ResourcesApi#list`. The generator emits the second as **`list_0`** — the only `_N`
artifact in the whole client.

## D8 — the contract is repaired at the source, and CI learns to validate it

Not patched by a gem-side overlay, and not worked around with `--skip-validate-spec`. `openapi.json`
is a published artifact; a third party generating any client hits the same crash, and `temper-py` and
`temper-ts` inherit whatever we do here.

**Scope of P5:**

1. Register `ResourceSortField` and `SortOrder` in `components(schemas(...))`.
2. Set explicit `operation_id` on the 24 colliding operations, named for what they do
   (`list_resources`, `list_edges`, `create_context`, `grant_resource_access`, …). Also fix the three
   Ingest ops whose fn names leak a `_handler` suffix into the contract (`list_blocks_handler`,
   `append_block_handler`, `finalize_handler`).
3. Regenerate `openapi.json`.
4. Extend the `openapi-check` gate from *diff* to *validate*: every `$ref` resolves; every
   `operationId` is present and unique; `openapi-generator validate` passes with no
   `--skip-validate-spec`. The gate cannot invoke `cargo make` (it needs `./.env`), so the logic
   lives in `.github/scripts/` beside the existing scripts.

**Acceptance:** `openapi-generator generate -g ruby --library=faraday` succeeds with no
`--skip-validate-spec` and emits a non-empty client; no emitted method is named `list_0`; CI goes red
if either defect is reintroduced, proven by temporarily removing a schema registration.

No behavior changes. Routes, handlers, and wire types are untouched — only annotations, the schema
registration list, the emitted artifact, and the gate.

---

# Part 2 — the gem

## D9 — cogmap authoring is incremental; reconcile stays a Rust/CLI path

The gem authors into a cognitive map through **server-recompute paths only**:

- `POST /api/ingest` with `home_cogmap_id` (`crates/temper-core/src/types/ingest.rs:22`) — the server
  chunks and embeds when `chunks_packed`/`content_hash` are absent, which they always will be.
- `POST /api/relationships/assert` and `POST /api/facets/set` — no body content, so no embedding.
- `POST /api/cognitive-maps` with `telos: None` — genesis takes an `Option<ReconcileTelos>`, so a
  charter-less map is creatable.

`PUT /api/cognitive-maps/{id}` (reconcile) and genesis-with-charter remain reachable through
`Temper::Generated::CognitiveMapsApi` for anyone who wants to hand-pack chunks, but **the skin does
not surface them**, and the README says why.

This preserves "Ruby never needs `ort`" at zero cost to temper, and it holds the prior doc's line
honestly: the gem *can* author into a map. It cannot perform bulk desired-state reconciliation, which
is a CLI operator's job, not a Rails request's.

*Condition for revisiting:* if a Ruby caller ever needs bulk reconcile, the fix is to give
`ReconcileEntry`/`ReconcileTelosBlock` an optional `chunks_packed` plus a raw body, and let the server
chunk and embed as ingest already does. That is a temper beat, not a gem feature — and it would put
ONNX back on the reconcile request path, which `reconcile.rs:1` deliberately designed away.

## D17 — the gem lives at `clients/temper-rb/`

The root `Cargo.toml` workspace glob is `members = ["crates/*", "tests/e2e"]`. The root
`package.json` `workspaces` is the explicit list `["packages/temper-cloud", "packages/temper-ui"]`.
A top-level `clients/` directory is therefore **inert to both toolchains** with no ignore file, no
exclusion, and no config.

This is a cleaner variant of the `tasker-rb` precedent, which lives under `crates/` and survives only
because tasker-core uses an explicit `members` list that names the gem's inner `ext/` crate while
omitting its parent. Temper's glob would swallow anything under `crates/` carrying a `Cargo.toml`.
`clients/` also names the slot the family grows into: `clients/temper-py`, `clients/temper-ts`.

## D10 — generated core in its own namespace, committed, drift-gated

```
clients/temper-rb/
  lib/temper/
    generated/            # rake generate is the ONLY writer
      models/*.rb         #   Temper::Generated::ResourceRow
      api/*.rb            #   Temper::Generated::ResourcesApi
      api_client.rb
      configuration.rb
    client.rb             # hand-written skin, Temper::
    credentials.rb
    errors.rb
    act.rb
    refs.rb
    resources.rb  contexts.rb  cognitive_maps.rb  …
  .openapi-generator-ignore
  Rakefile                # rake generate
```

The skin never lives inside `generated/`, so clobbering is **structurally impossible** rather than
prevented by an ignore file that someone must keep correct. The generator's default flat layout
(models and skin sharing `lib/temper/`) was rejected for exactly that reason: one missed ignore entry
silently overwrites hand-written code.

The generated tree is **committed**. CI runs `rake generate && git diff --exit-code` — the same drift
gate `openapi-check` applies one level up — so a contract change surfaces as a reviewable diff in the
PR that causes it. Contributors need no Docker and no JVM to build or test the gem.

The generator's 171 rspec stubs and 170 markdown docs are dropped via `.openapi-generator-ignore`.
What remains is ~40k LOC worth keeping: 152 models (33.6k) carrying `openapi_types`, `attribute_map`,
and `openapi_nullable`, and 18 tag-partitioned API classes (6.0k).

## D15 — generate every operation; the skin covers a subset

All 18 API classes, all 79 operations. Surface size is not a cost driver, and the prior doc already
rejected shrinking by method count.

The skin covers resources, contexts, ingest, relationships, facets, meta, search, graph, profile, and
cognitive maps (per D9). Steward, admin, and SAML stay deferred.

## D14 — no `Dry::Struct` over responses

The generated models already carry `openapi_types`, `attribute_map`, and `openapi_nullable`, and they
deserialize correctly. Hand-writing 152 `Dry::Struct` mirrors of Rust structs is precisely what
CLAUDE.md forbids — *"never define a zod schema that mirrors a Rust struct manually"* — and the rule
does not care that the second language is Ruby. `tasker-rb` hand-writes its `Dry::Struct` types
because it has no contract to generate from; we do.

The skin returns generated model instances directly. It does not re-wrap them.

Corollary: the gem takes **no** `dry-*` runtime dependency.

## D11 — pure-Ruby faraday with a persistent adapter

`--library=faraday`, with `net_http_persistent` swapped in for the default `net_http` adapter.

Faraday memoizes its connection (`@connection_regular ||= build_connection`), so one `ApiClient` means
one connection — but `Faraday.default_adapter` is `net_http`, which does **not** keepalive, so a naive
setup still pays a TLS handshake per request. `net-http-persistent` pools per thread and is documented
thread-safe. Both hooks needed already exist: `Configuration#configure_connection` and
`#configure_middleware`.

`typhoeus` (the generator's default) gets keepalive free from libcurl's thread-local connection cache,
but pulls `ethon` → the `ffi` gem, which compiles a native extension at install and needs libcurl in
the image. That is a smaller version of the toolchain cost the codegen decision exists to avoid.
`httpx` is pure Ruby and HTTP/2-capable, but its generated adapter was not audited and its Rails
deployment track record is the thinnest of the three.

Known cost: faraday is widely present in Rails apps transitively, so the version pin
(`>= 1.0.1, < 3.0`) is a real, manageable conflict surface.

## D12 — credentials are call-scoped; the connection is process-global

The generated `Configuration.default` and `ApiClient.default` are class-variable singletons holding a
**single** `access_token`. Under Puma, two threads serving two users would clobber each other's token.
The obvious fix — a fresh `Configuration` + `ApiClient` per request — is the pooling trap: fresh
client, fresh connection, TLS handshake per request.

`Configuration#access_token_getter` accepts a callable, invoked per request. That is the seam.

- **One process-wide `Generated::ApiClient`**, holding one Faraday connection and one pool.
  `X-Temper-Surface: sdk` is set once in its `default_headers`. (The path-item-level header on all 64
  paths makes the generator emit an optional `x_temper_surface` kwarg on all 79 operations; the skin
  never exposes it.)
- **`Temper::Client` is a cheap façade** holding a credentials object and a reference to that shared
  client. It sets a fiber-local around each call; `access_token_getter` reads it.

The `.default` singletons are never touched.

```ruby
Temper.configure do |c|
  c.base_url = ENV.fetch("TEMPER_API_URL")
  c.device_id = ENV["TEMPER_DEVICE_ID"]   # optional; X-Temper-Device-Id
end

# Puma — per request, zero I/O
client = Temper::Client.new(
  credentials: Temper::Credentials::BearerToken.new(session_token))

# Sidekiq — memoized per process, refreshes itself
client = Temper::Client.new(
  credentials: Temper::Credentials::ClientCredentials.new(
    client_id:, client_secret:, audience: ENV.fetch("TEMPER_AUTH_AUDIENCE")))
```

Two strategies behind one interface:

- **`BearerToken`** — a token the caller already holds. No I/O, no refresh.
- **`ClientCredentials`** — POSTs Auth0 `client_credentials`, caches until near expiry, refreshes under
  a mutex. The `audience` must equal the API's configured `AUTH_AUDIENCE` or the minted token fails
  validation before `normalize_machine` ever runs. The machine profile auto-provisions server-side on
  first call.

**On 401:** `ClientCredentials` re-fetches once and retries, then raises. `BearerToken` cannot refresh
and raises `Unauthorized` immediately. **Idempotent reads (GET/HEAD)** retry on 5xx and transport
failures, three attempts at 200ms/400ms — mirroring `should_retry` in the Rust client. **Writes never
auto-retry.**

**Fork safety** is a genuine gap with no precedent to copy: `tasker-rb` sidesteps it by never forking
(its example app runs single-mode Puma, and nothing hooks `Process._fork`). The gem exposes
`Temper.reset_connection!`, and the README documents calling it from Puma's `on_worker_boot` and from
`Sidekiq.configure_server`, so a forked worker never inherits its parent's sockets.

## The write path, and `Temper::Act`

```ruby
client.resources.create(
  title: "Postmortem", context_ref: "@dana/incidents",
  doc_type_name: "note", content: markdown,
  act: Temper::Act.new(confidence: :probable, reasoning: "…"))
```

The skin builds the `IngestPayload` and **flattens the six `ActInput` keys into the body** — that is
how ~30 write endpoints accept act context (`#[serde(flatten)] pub act: ActInput`). The exception is
`DELETE /api/resources/{id}`, which takes `Query<ActInput>`; the skin routes the same six keys onto
the query string there. (Reconcile is the other `Query<ActInput>` endpoint and is out of scope per D9.)

No `chunks_packed`, no `content_hash` — both are `Option` on `IngestPayload`
(`crates/temper-core/src/types/ingest.rs:33,46`), and the server computes them.

`Temper::Act` is a value object with a **constructor invariant** mirroring `ActInput::into_act_context`:
supplying `reasoning`, `rationale`, `persona`, or `model` without `confidence` raises `ArgumentError`
locally instead of earning a 400. That is the parse-don't-validate answer to `AgentAuthorship.confidence`
being non-`Option`. `Act` is optional on every call; in v1 `authorship` is normally absent, per D5.

**P3 has landed**, so `Act` carries `correlation:` from the gem's first commit — there is no follow-up
regeneration to remember. It maps to `ActInput.correlation_id`, which the contract carries as a body
field on the flattened write endpoints and as a query parameter on `DELETE /api/resources/{id}` and
`PUT /api/cognitive-maps/{id}`. The enqueue boundary then reads as the goal describes: the Puma request
mints a correlation UUID, serializes it into the job arguments as a bare UUID that outlives any
credential, and the worker stamps the same value while writing as a different principal
(`dana@sdk` → `acme-app@sdk`).

Correlation is provenance, never authorization — nothing gates on it — and an act with no supplied
correlation self-roots to its own event id. So the gem may always omit it.

`Temper.parse_ref` is a ~10-line pure Ruby port of `temper_workflow::operations::parse_ref`
(`crates/temper-workflow/src/operations/refs.rs:94`): bare UUID, or trailing-UUID-only from the last
five hyphen groups. The gem does **not** port `sluggify` — the server derives the slug from the title.

## D13 — typed errors, split transient vs permanent

The generated core raises one flat `Temper::Generated::ApiError` carrying `code` (the status),
`response_headers`, and `response_body`. The server always speaks one envelope
(`crates/temper-services/src/error.rs:28`, referenced by 71 operations):

```json
{ "error": { "code": "CONFLICT", "message": "…", "details": { } } }
```

The skin parses that envelope with the status and raises:

```
Temper::Error
├─ Temper::TransientError            # re-raise → Sidekiq retries
│  ├─ RateLimited      (429, #retry_after)
│  ├─ ServerError      (5xx)
│  └─ ConnectionError  (timeout / refused)
└─ Temper::PermanentError            # rescue → dead-letter
   ├─ Unauthorized     (401)
   ├─ Forbidden        (403)
   │  └─ SystemAccessRequired   (error.code == SYSTEM_ACCESS_REQUIRED)
   ├─ NotFound         (404)
   ├─ Conflict         (409)
   └─ BadRequest       (400 / 422)
```

Every exception carries `#status`, `#code`, `#message`, `#details`. The split is load-bearing rather
than decorative: Sidekiq retries a job whose exception escapes, so a 409 that is classified transient
spins forever and a 503 that is classified permanent is silently dropped. `SystemAccessRequired` is
discriminated on `error.code`, matching what the Rust client already special-cases.

The skin **classifies**; it does not auto-retry writes. Same rule as the Rust client, same reason.

Note that `422` and `500` are declared nowhere in the contract, so those bodies arrive unparsed. The
skin degrades to `#message = nil` with the raw body on `#details`. Widening the contract to declare
them is a candidate follow-up, not a blocker.

## D16 — gem version is independent; the contract version is recorded

`Temper::VERSION` is ordinary SemVer for the gem. `Temper::CONTRACT_VERSION` is written by
`rake generate` from `openapi.json`'s `info.version`, so a released gem always names the contract it
was built against.

This deliberately avoids `tasker-rb`'s drift, where `lib/tasker_core/version.rb` reads `0.1.10` while
`ext/tasker_core/Cargo.toml` reads `0.1.8`, reconciled only by a release script at publish time. A gem
version and an API version answer different questions and should not be forced to agree.

## Testing, packaging, CI

**Tests.** RSpec with `webmock` for the skin: credential strategies, the 401 retry-once path, error
classification, `Act`'s constructor invariant, `parse_ref`, and act-context carriage (body-flatten vs
query-string on delete). The generated core is **not** unit-tested — testing generated code tests the
generator. A live integration tier runs against a real server, gated on an env var, following
`tasker-rb`'s `FFI_CLIENT_TESTS` precedent.

**CI.** `ruby/setup-ruby` with `bundler-cache: true`; rubocop; rspec; and the
`rake generate && git diff --exit-code` drift gate. The gem's jobs are scoped to `clients/temper-rb/**`
plus `openapi.json`, so they stay off the critical path of unrelated PRs — and `openapi.json` is in
that trigger set precisely because a contract change must be seen to move the gem.

**Release.** `gem push` to RubyGems with `rubygems_mfa_required`, an idempotency guard that queries
RubyGems before building, and `allowed_push_host` pinned — following `tools/scripts/release/publish-ruby.sh`
in tasker-core. There is no native extension, so there is **no platform gem matrix and no cross-compile**:
one source gem, and no cargo on the install box. That was the whole point.

**Ruby floor.** `>= 3.1`. The generator's default (`>= 2.7`) is older than anything we intend to
support; `tasker-rb` pins `>= 3.4.0`, which is tighter than a library needs to be.

---

## Scope

### Rejected

- **A gem-side JSON overlay that patches the spec before generation.** It forks the contract: the
  published `openapi.json` stays broken for every third party and every future SDK, and the overlay
  rots silently. See D8.
- **`--skip-validate-spec` as a permanent flag.** It suppresses the next dangling `$ref` too. See D8.
- **Generating at build time without committing.** Every contributor and CI job would need Docker or a
  JVM to run the tests, `gem build` would stop being hermetic from a checkout, and contract drift would
  stay invisible until release. See D10.
- **`Dry::Struct` over responses.** Manual mirroring of Rust structs. See D14.
- **A global client with a middleware-set thread-local token.** Ambient global principal; a spawned
  thread inherits nothing and silently writes as the wrong writer. That is the G1 mis-attribution bug
  class P2 just spent a PR eliminating. See D12.
- **`typhoeus` / `httpx`.** See D11.
- **Ruby-side embeddings via `onnxruntime`.** Reintroduces the native toolchain cost that motivated
  codegen over magnus in the first place. See D9.
- **Shrinking the generated surface by method count.** Already rejected in the prior doc; reaffirmed
  by D15.

### Deferred

- **`ContextOwnerRef` is a string-or-object `oneOf` with no discriminator**
  (`crates/temper-core/src/context_ref.rs:20` — an externally-tagged serde enum mixing a unit variant
  with newtype variants). It generates a heuristic `build` wrapper that the generated code itself
  admits is best-effort: *"we do not attempt to check whether exactly one item matches."* It rides in
  the `contexts.create` request body. Not a generation blocker; the skin hand-constructs the payload.
  Fixing it properly means making the serde representation uniform (adjacently tagged, or a decorated
  string with a custom impl) and reviewing every downstream consumer — a change with a blast radius
  well beyond the gem.
- **Declaring `422` and `500` responses** on the ~79 operations, so error bodies for those statuses
  parse. See D13.
- **`temper-py` and `temper-ts`.** They inherit P0–P3 and P5 for free, and D9's incremental-authoring
  constraint applies to them identically.
- **Steward, admin, and SAML surfaces.** Operator surfaces, not library surfaces.
- **Invocation envelopes and `authorship`.** Needed when a Ruby *agent* authors into a cogmap. See D5,
  D6 in the prior doc.
- **A Rails railtie / generator.** No precedent exists in tasker-contrib to copy; the initializer
  pattern is enough for v1.

---

## Open threads

- **P3 landed and is merged into the P5 branch** (#345). Its `correlation` field rode the existing
  `params(ActInput)` with no annotation churn, exactly as predicted, and it registered `CorrelationId`
  as a component without touching `components(schemas(...))` — the control case that pins down why
  `ResourceSortField`/`SortOrder` dangle. It also left a follow-up task (`019f4a19`): threading a
  segmented ingest session's `block_created` + `resource_finalized` events onto one correlation now
  that a caller can supply one, which needs a precedence rule against the caller's value.
- **`components(schemas(...))` stays hand-maintained.** P5 adds two entries rather than deleting the
  list. The deletion P2 hoped for requires `.routes()` to walk `IntoParams` schemas, which it does not.
  Worth an upstream utoipa issue.
- **The faraday version pin is a live conflict surface.** Rails apps carry faraday transitively. If
  the `< 3.0` ceiling bites a real consumer, the fallback is `httpx` (pure Ruby, persistent by default)
  rather than typhoeus.
- **Fork safety is designed, not proven.** `Temper.reset_connection!` plus documented hooks is the
  plan; no existing gem in either repo demonstrates it, so it needs a real forked-Puma test before v1.
