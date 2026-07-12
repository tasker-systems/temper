# temper-rb

A pure-Ruby SDK for the [Temper](https://temperkb.io) knowledge-base API.

There is no native extension: one source gem, no compiler on the install box, no
platform matrix. The client core is generated from the API's `openapi.json` and
committed, so building and testing this gem needs neither Docker nor a JVM.

> **The integration how-to lives in the repo:**
> [docs/guides/temper-rb.md](https://github.com/tasker-systems/temper/blob/main/docs/guides/temper-rb.md).
> It covers the two callers, both machine-credential mint paths, the token
> lifecycle, attribution across the enqueue boundary, the error taxonomy, and
> going live. This README is the quickstart.

## Install

```ruby
gem 'temper-rb'
```

Ruby >= 3.1.

## Configure

```ruby
Temper.configure do |c|
  c.base_url  = ENV.fetch('TEMPER_API_URL')
  c.device_id = ENV['TEMPER_DEVICE_ID']   # optional; sets X-Temper-Device-Id
end
```

The connection is **process-global**. The **token is per call**. Constructing a
`Temper::Client` does no I/O, so build one per request if you like.

## Puma — a token the caller already holds

```ruby
client = Temper::Client.new(
  credentials: Temper::Credentials::BearerToken.new(session_token))

client.resources.create(
  title: 'Postmortem', context_ref: '@dana/incidents',
  doc_type_name: 'note', content: markdown,
  act: Temper::Act.new(confidence: :probable, reasoning: 'summarised the incident'))
```

A `BearerToken` does no I/O and **cannot refresh** — `refresh!` raises. A user's
token is not the app's to re-mint.

`Temper::Act` refuses to be built with `reasoning`, `rationale`, `persona`, or
`model` unless you also give `confidence` — the server would reject it with a
400, so the gem rejects it locally instead.

## Sidekiq — a machine principal

```ruby
CREDENTIALS = Temper::Credentials::ClientCredentials.new(
  token_url:     ENV.fetch('TEMPER_M2M_TOKEN_URL'),
  client_id:     ENV.fetch('TEMPER_M2M_CLIENT_ID'),
  client_secret: ENV.fetch('TEMPER_M2M_CLIENT_SECRET'),
  audience:      ENV['TEMPER_M2M_AUDIENCE'])   # Auth0 only — omit for temper-issued
```

It caches the token until 60 seconds before its absolute expiry, mints under a
mutex (Puma threads race a cold cache), and re-mints once on a 401 — because a job
that holds a token across a long unit of work can outlive it, and
refresh-ahead-of-expiry alone does not save you there.

### Two mint paths

The token request is form-encoded on both (RFC 6749 §4). Only the config differs.

| | Auth0-issued (`temper admin machine provision`) | Temper-issued (`temper admin machine issue`) |
|---|---|---|
| `client_id` | your Auth0 M2M app's client id | minted by temper, prefixed `tmpr_` |
| `client_secret` | from Auth0 | printed **once** at `issue` |
| `token_url` | your Auth0 tenant's `/oauth/token` | **your own instance's** `/oauth/token` |
| `audience` | **required** — must equal the API's `AUTH_AUDIENCE` | **omit it** |

Temper's own authorization server mints with its server-side `AS_AUDIENCE` and
ignores a request-supplied one, so the SDK leaves `audience` off the wire entirely
rather than sending an empty string.

## Correlation across the enqueue boundary

The Puma request writes as the signed-in user; the Sidekiq job it enqueues writes
as the **machine profile**. Two authors, honestly — the machine really did make the
second write. A correlation id stitches them together in the event ledger.

```ruby
correlation = SecureRandom.uuid
PostmortemJob.perform_async(correlation)
# ...and in the worker:
act = Temper::Act.new(confidence: :probable, correlation: correlation)
```

Correlation is provenance, never authorization. Nothing gates on it, and an act
that supplies none self-roots to its own event id — so you may always omit it.

## Errors

```
Temper::Error
├─ Temper::TransientError          # let it escape → Sidekiq retries
│  ├─ RateLimited      (429, #retry_after)
│  ├─ ServerError      (5xx)
│  └─ ConnectionError  (timeout / refused)
└─ Temper::PermanentError          # rescue it → dead-letter
   ├─ Unauthorized     (401)
   ├─ Forbidden        (403)
   │  └─ SystemAccessRequired
   ├─ NotFound         (404)
   ├─ Conflict         (409)
   └─ BadRequest       (400 / 422)
```

Every exception carries `#status`, `#code`, `#message`, and `#details`.

The split is load-bearing, not decorative: Sidekiq retries a job whose exception
escapes, so a 409 classified transient would spin forever and a 503 classified
permanent would be silently dropped.

Idempotent reads retry three times on 5xx and transport failures. **Writes are
never auto-retried.** The SDK classifies; it does not decide to re-submit.

## Going live

Authentication is not authorization. A valid M2M token does **not** get you in on
its own: temper keeps a registration allowlist, so your `client_id` must be
registered before your first call. An unregistered (or revoked) client
authenticates cleanly at its issuer and is then rejected with a terminal
`Unauthorized` — `client 'X' is not registered with this instance` — which the SDK
classifies as a `PermanentError` (a Sidekiq worker dead-letters it rather than
retrying). Registered but un-granted, you then get a `SystemAccessRequired` 403.
Those are the two walls, in that order.

Registration and reach happen in one command, on whichever mint path you chose:

```bash
# Auth0-issued: register the M2M application you already created
temper admin machine provision \
  --client-id "$TEMPER_M2M_CLIENT_ID" --label "acme-worker" \
  --owner-team acme-eng --team acme-eng:member --cogmap <map-ref>

# Temper-issued: temper mints the client id and secret; the secret prints once
temper admin machine issue \
  --label "acme-worker" \
  --owner-team acme-eng --team acme-eng:member --cogmap <map-ref>
```

Each `--team` gives **team membership** for read reach; each `--cogmap` applies a
**cogmap write grant**. Both are repeatable — reach is plural and explicit, and is
never inferred from `--owner-team` (which records the machine's owner only).

This need not be an operator ticket: minting is authorized for a system admin **or
the owner of the team that will own the machine**, so a team owner can register
their own team's machine. A teamless machine is admin-only.

**Rotation.** `temper admin machine rotate-secret` rolls a temper-issued **secret**
(same `client_id`), keeping the previous one valid for a grace window while you
redeploy the app with the new secret. `temper admin machine rebind` is a different
operation: a **new `client_id`** bound to the same agent profile, for rotating the
Auth0 application behind a machine — and it is system-admin only. Rotating an Auth0
*secret* needs no temper action at all.

See
[docs/guides/machine-credentials.md](https://github.com/tasker-systems/temper/blob/main/docs/guides/machine-credentials.md)
for the operator's side.

Assert it worked at boot rather than discovering it on the first write:

```ruby
Temper::Client.new(credentials: CREDENTIALS).whoami
```

An unregistered client surfaces as `Unauthorized` naming the client id; a
registered-but-under-granted one surfaces as `Forbidden` / `SystemAccessRequired`
naming the missing grant — both from the server's `error.details`, not a bare 4xx.

## Forking

The connection holds sockets. Call `Temper.reset_connection!` in a forked worker:

```ruby
# config/puma.rb
on_worker_boot { Temper.reset_connection! }

# config/initializers/sidekiq.rb
Sidekiq.configure_server { |_| Temper.reset_connection! }
```

Strictly speaking the sockets are already safe: `connection_pool` (>= 2.4, which
`net-http-persistent` pools through) drops them from a `Process._fork` hook, and
this gem's test suite proves a forked child opens its own socket. The hooks above
clear *this gem's* memoized client rather than relying on a transitive
dependency's default. Keep them.

## Addressing

Resources are addressed by **ref**: a bare UUID, or the decorated
`sluggify(title)-<uuid>` form. Resolution is trailing-UUID-only, so a stale slug
half is harmless.

```ruby
client.resources.show('p4-design-the-gem-019f4912-3f20-7fd3-814f-13a5ddbe3cd7')
client.resources.show('019f4912-3f20-7fd3-814f-13a5ddbe3cd7')  # identical
```

`Temper.parse_ref` is available directly, and never touches the network.

## Cognitive maps

The SDK authors into a map incrementally — `cognitive_maps.author` (ingest with
`home_cogmap_id`), `assert_relationship`, and `set_facet` — all paths where the
server chunks and embeds for you.

**Bulk reconcile is not exposed.** `PUT /api/cognitive-maps/{id}` takes a
pre-embedded desired-state manifest: its `chunks_packed` is a required,
client-computed 768-dimension BGE embedding, carried verbatim with no server-side
fallback. Ruby has no embedder, so that path is physically out of reach. It is a
CLI operator's job, not a Rails request's. If you truly need it, reach
`Temper::Generated::CognitiveMapsApi#reconcile` and pack the chunks yourself.

## Versioning

`Temper::VERSION` is the gem's own SemVer. `Temper::CONTRACT_VERSION` names the
`openapi.json` it was generated against. They answer different questions and are
not forced to agree.

## Development

```bash
bundle install
bundle exec rake          # rubocop + rspec
bundle exec rake generate # regenerate lib/temper/generated/** (needs Docker)
bundle exec rake drift    # fail if the committed core drifts from the contract
```

`rake generate` is the only writer of `lib/temper/generated/**`. Never hand-edit
that tree; the hand-written skin lives beside it, in `lib/temper/*.rb`.

## License

MIT.
