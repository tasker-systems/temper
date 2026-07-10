# temper-rb

A pure-Ruby SDK for the [Temper](https://temperkb.io) knowledge-base API.

There is no native extension: one source gem, no compiler on the install box, no
platform matrix. The client core is generated from the API's `openapi.json` and
committed, so building and testing this gem needs neither Docker nor a JVM.

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

`Temper::Act` refuses to be built with `reasoning`, `rationale`, `persona`, or
`model` unless you also give `confidence` — the server would reject it with a
400, so the gem rejects it locally instead.

## Sidekiq — a machine principal

```ruby
CREDENTIALS = Temper::Credentials::ClientCredentials.new(
  token_url:     ENV.fetch('TEMPER_M2M_TOKEN_URL'),
  client_id:     ENV.fetch('TEMPER_M2M_CLIENT_ID'),
  client_secret: ENV.fetch('TEMPER_M2M_CLIENT_SECRET'),
  audience:      ENV.fetch('TEMPER_M2M_AUDIENCE'))
```

It caches the token until 60 seconds before its absolute expiry, refreshes under
a mutex, and re-mints once on a 401 — because a job that holds a token across a
long unit of work can outlive it, and refresh-ahead-of-expiry alone does not save
you there.

`audience` must equal the API's configured `AUTH_AUDIENCE`, or the minted token
fails validation before it ever reaches the machine-profile resolver.

## Correlation across the enqueue boundary

A correlation id is a bare UUID that outlives any credential. Mint one in the web
request, serialize it into the job arguments, and stamp it again in the worker —
the two writes then join in the event ledger even though they were made by
different principals.

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

Authentication is not authorization. A minted M2M token gets you a
JIT-provisioned agent profile and *nothing else* — every call will authenticate
cleanly and then 403. This is invisible from the contract, and it is the first
wall a new caller hits.

1. Provision an Auth0 M2M application and a client grant for the API's audience.
2. Set `TEMPER_M2M_TOKEN_URL`, `TEMPER_M2M_CLIENT_ID`, `TEMPER_M2M_CLIENT_SECRET`,
   and `TEMPER_M2M_AUDIENCE`.
3. Grant the agent profile a **cogmap write grant** on the map it will author into.
4. Give the agent profile **team membership** in the team whose contexts it must read.

Assert it worked at boot rather than discovering it on the first write:

```ruby
Temper::Client.new(credentials: CREDENTIALS).whoami
```

`Forbidden` and `SystemAccessRequired` name the missing grant, using the server's
`error.details`, instead of surfacing a bare 403.

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
