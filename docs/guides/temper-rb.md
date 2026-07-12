# Integrating Ruby with temper-rb

How to call Temper from a Rails application: which credential a Puma request uses, which one a
Sidekiq job uses, why the two write under different names, and what happens between minting a
machine credential and your first successful write.

**Audience:** a Ruby developer wiring `temper-rb` into a Rails app — a web tier serving signed-in
users and a background tier doing unattended work.

**Scope:** the SDK's own behaviour — credentials, the token lifecycle, attribution, the error
taxonomy — plus the onboarding cliff you hit the first time a machine principal calls the API. The
operator's side of a machine credential (who may mint one, reach containment, revocation) is
[machine-credentials.md](machine-credentials.md); this guide points at it rather than restating it.

Every pattern below is pinned by a passing spec in `clients/temper-rb/spec/`. Where a claim rests on
server behaviour rather than the gem's, this guide says so.

## Install and configure

```ruby
# Gemfile
gem 'temper-rb'
```

Ruby >= 3.1. There is no native extension — one source gem, no compiler on the install box.

```ruby
# config/initializers/temper.rb — process-wide settings, set once at boot
Temper.configure do |c|
  c.base_url  = ENV.fetch('TEMPER_API_URL')
  c.device_id = ENV['TEMPER_DEVICE_ID']   # optional; sets X-Temper-Device-Id
end
```

The **connection** is process-global and memoized: one `ApiClient`, one Faraday connection, one
`net-http-persistent` pool. The **token** is per call, resolved from fiber-local storage at request
time. That split is the whole design — a fresh client per request would mean a fresh TLS handshake
per request, and a shared client with a shared token would leak one user's token into another
user's call.

> **A token never escapes its call.** `Temper.with_token` restores the previous value on the way
> out — including when the block raises — and a **thread spawned inside a token scope sees no
> token at all**. Do not hand a `Temper::Client` to a thread and expect its credential to travel.

## The two callers

Which credential you construct is determined by **who is acting**, and it is your explicit choice —
the gem never sniffs the environment to decide.

| | Puma request | Sidekiq / ActiveJob worker |
|---|---|---|
| **Who is acting** | a signed-in human | the application itself |
| **Class** | `Temper::Credentials::BearerToken` | `Temper::Credentials::ClientCredentials` |
| **Where the token comes from** | the user's session — you already hold it | the SDK mints it, `client_credentials` grant |
| **I/O at construction** | none | none; the first mint is lazy, on first use |
| **Can it refresh?** | **no** — `refresh!` raises `Unauthorized` | yes — mints, caches, re-mints |
| **Writes land as** | the user's profile | the machine's agent profile |

### A Puma request — a token the caller already holds

```ruby
# In a controller, with the signed-in user's access token
client = Temper::Client.new(
  credentials: Temper::Credentials::BearerToken.new(session_token))

client.resources.create(
  title: 'Postmortem', context_ref: '@dana/incidents',
  doc_type_name: 'note', content: markdown,
  act: Temper::Act.new(confidence: :probable, reasoning: 'summarised the incident'))
```

A `BearerToken` does no I/O and **cannot refresh** — `refresh!` raises `Unauthorized` rather than
attempting a mint. That is deliberate, not an omission: a user's token is not the application's to
re-mint. On a 401 the exception propagates on the first request, with no retry.

### A Sidekiq worker — a machine principal

```ruby
# config/initializers/temper.rb — one credential per process; it caches its own token
TEMPER_M2M = Temper::Credentials::ClientCredentials.new(
  token_url:     ENV.fetch('TEMPER_M2M_TOKEN_URL'),
  client_id:     ENV.fetch('TEMPER_M2M_CLIENT_ID'),
  client_secret: ENV.fetch('TEMPER_M2M_CLIENT_SECRET'),
  audience:      ENV['TEMPER_M2M_AUDIENCE'])   # Auth0 only — see below
```

Every field is required to be a non-empty String, and a missing one raises `ArgumentError` at
construction. The credential **throws rather than defaulting** — a machine that boots with a blank
secret and discovers it on its first write is strictly worse than one that refuses to boot.

`audience` is the one exception: it is optional, because it is Auth0's, not the protocol's. Absent
is a supported configuration; **present-and-empty is still a caller bug** and raises.

*Backed by:* `spec/temper/credentials_spec.rb` — "returns the token it was constructed with, with no
I/O", "cannot refresh and says so", "requires every mandatory M2M field, throwing rather than
defaulting", "accepts an absent audience but rejects an empty one"; `spec/temper/client_spec.rb` —
"raises Unauthorized immediately for a BearerToken on 401 -- it cannot refresh".

## The two mint paths

Temper can be fronted by **two different issuers**, and `ClientCredentials` works against both. The
only thing that changes is your config.

| | Auth0-issued (`temper admin machine provision`) | Temper-issued (`temper admin machine issue`) |
|---|---|---|
| **Who holds the secret** | your Auth0 tenant | Temper — it *is* the authorization server |
| **`client_id`** | the Auth0 M2M application's client id | minted by Temper, prefixed **`tmpr_`** |
| **`client_secret`** | from Auth0 | printed **once** at `issue`; stored only as a hash |
| **`token_url`** | your Auth0 tenant's `/oauth/token` | **your own instance's** `/oauth/token` |
| **`audience`** | **required** — must equal the API's `AUTH_AUDIENCE` | **omit it** |
| **Rotate the secret with** | Auth0 (no Temper action needed) | `temper admin machine rotate-secret` |

The request itself is identical on both paths, and that is the point: the SDK sends a
**form-encoded** body (`application/x-www-form-urlencoded`), which RFC 6749 §4 mandates at the token
endpoint. Auth0 also tolerates JSON; Temper's own authorization server reads the body with
`req.formData()` and does not. Form-encoding is the shape both accept.

> **Omit `audience` for a temper-issued credential — do not pass an empty string.** Temper's AS mints
> with its server-side `AS_AUDIENCE` and ignores a request-supplied audience entirely, so the SDK
> leaves the parameter **off the wire** rather than sending a lie. Passing `audience: ''` raises at
> construction.

```ruby
# Temper-issued: token_url is your own instance, and there is no audience
TEMPER_M2M = Temper::Credentials::ClientCredentials.new(
  token_url:     'https://temper.acme.internal/oauth/token',
  client_id:     ENV.fetch('TEMPER_M2M_CLIENT_ID'),      # tmpr_...
  client_secret: ENV.fetch('TEMPER_M2M_CLIENT_SECRET'))
```

The wire shape is pinned as a **cross-language contract** at `tests/contracts/m2m-token-request.json`,
asserted from both ends — the gem's spec proves it *emits* that shape, and temper-cloud's integration
test proves the AS *accepts* it. Neither test alone would catch a mismatch, which is exactly how the
gem once shipped a JSON mint against a form parser with both suites green.

*Backed by:* `spec/temper/credentials_spec.rb` — "sends the content type the token endpoint actually
parses", "emits exactly the params the shared wire contract requires", "omits audience entirely for a
temper-issued credential"; server side, `packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts`
— "accepts a request built from the contract, exactly as a client emits it".

## The token lifecycle you get for free

`ClientCredentials` is not a thin wrapper over a POST. Three behaviours come with it, each of them a
bug that a hand-rolled minter tends to ship.

| Behaviour | What it does | Why |
|---|---|---|
| **Absolute-expiry cache** | caches until 60s before the token's absolute expiry | a *duration* cannot survive being cached; only an absolute `expires_at` can |
| **Mutex-guarded mint** | many threads racing a cold cache mint **once** | under Puma every in-flight thread hits expiry together |
| **Re-mint on 401** | re-mints once and retries the call, then gives up | a job holding a token across a long unit of work outlives it |

Refresh-ahead-of-expiry alone is **not sufficient**, and this is the subtle one. A worker resolves its
token at the top of a long unit of work; the token expires mid-job; the next call takes a 401 that no
amount of expiry-checking recovers. So the client repairs a 401 by re-minting **once** and replaying
the request — for reads and writes alike, because re-authenticating is not re-submitting. A second
401 raises.

```ruby
# Nothing to write. The credential is a constant; the client re-mints under you.
class PostmortemJob
  include Sidekiq::Job

  def perform(correlation)
    client = Temper::Client.new(credentials: TEMPER_M2M)
    client.resources.create(
      title: 'Postmortem (enriched)', context_ref: '@acme/incidents',
      doc_type_name: 'note', content: enrich,
      act: Temper::Act.new(confidence: :probable, correlation: correlation))
  end
end
```

A `BearerToken` takes no part in any of this: `refresh!` raises, so the repair path terminates
immediately and the `Unauthorized` propagates out of `refresh!` itself. One request, one exception.

*Backed by:* `spec/temper/credentials_spec.rb` — "caches the token across calls", "refreshes 60s
before the absolute expiry, not at it", "mints once when many threads race a cold cache" (8 threads),
"refresh! mints unconditionally, even on a warm cache", "raises Unauthorized when the mint is
rejected"; `spec/temper/client_spec.rb` — "re-mints once and retries when ClientCredentials takes a
401 mid-job", "gives up after a single re-mint, rather than looping".

## Attribution across the enqueue boundary

A Puma request writes as the signed-in user. The Sidekiq job it enqueues writes as the **machine
principal** — a different profile, with different reach. So a single logical operation that spans the
enqueue boundary lands in the ledger under **two different authors**:

| Where the write happens | Credential | Author |
|---|---|---|
| the Puma request | `BearerToken` (the user's token) | `dana@sdk` |
| the Sidekiq job it enqueued | `ClientCredentials` (the machine) | `acme-app@sdk` |

**That is honest, not a bug.** The machine really did make the second write; the user was not in the
room. Attributing it to the user would be a lie the ledger cannot later distinguish from the truth.

What stitches the two together is a caller-supplied **correlation id** — a bare UUID that outlives any
credential. Mint it in the request, serialize it into the job arguments, stamp it again in the worker.

```ruby
# In the Puma request: mint the correlation id and carry it across the boundary
correlation = SecureRandom.uuid
client.resources.create(
  title: 'Postmortem', context_ref: '@dana/incidents', doc_type_name: 'note',
  content: markdown,
  act: Temper::Act.new(confidence: :probable, correlation: correlation))
PostmortemJob.perform_async(correlation)
```

`Temper::Act` renames `correlation:` to the wire key `correlation_id` (and `invocation:` to
`invocation_id`) for you. The act's keys **flatten into the write body** — and onto the *query string*
for a delete, which takes its act context as query parameters rather than a body.

Correlation is **provenance, never authorization**. Nothing gates on it, and an act that supplies none
self-roots to its own event id — so you may always omit it. That is why `Act` accepts `correlation:`
with no `confidence:`, while it *refuses* `reasoning:`, `rationale:`, `persona:`, or `model:` without
one: those are **authorship**, the server's `AgentAuthorship.confidence` is non-optional, and the call
would earn a 400. The gem rejects it locally instead of paying the round trip.

The gem also stamps `X-Temper-Surface: sdk` on every request for you — once, on the client, not per
call. That is where the `@sdk` suffix on both author names comes from.

*Backed by:* `spec/temper/act_spec.rb` — "renames correlation and invocation to their wire keys",
"permits correlation with no confidence", "rejects <field> without confidence, locally, rather than
earning a 400"; `spec/temper/resources_spec.rb` — "flattens the act keys into the ingest body",
"routes the act keys onto the query string for delete"; `spec/temper/connection_spec.rb` — "stamps
X-Temper-Surface: sdk once, on the client"; `spec/temper/client_spec.rb` — "scopes the credential
token around the call and stamps the surface header".

*Not spec-backed:* the **author names themselves** (`dana@sdk` vs `acme-app@sdk`) are the server's
attribution of the credential you authenticated with. The gem cannot assert them; they follow from
the machine principal being an ordinary agent profile — see [machine-credentials.md](machine-credentials.md).

## Errors, and what retries

Every exception is a `Temper::Error` carrying `#status`, `#code`, `#message`, and `#details`.

```
Temper::Error
├─ Temper::TransientError          # let it escape → Sidekiq retries
│  ├─ RateLimited      (429, #retry_after)
│  ├─ ServerError      (5xx)
│  └─ ConnectionError  (timeout / refused)
└─ Temper::PermanentError          # rescue it → dead-letter
   ├─ Unauthorized     (401)
   ├─ Forbidden        (403)
   │  └─ SystemAccessRequired      (403 with code SYSTEM_ACCESS_REQUIRED)
   ├─ NotFound         (404)
   ├─ Conflict         (409)
   └─ BadRequest       (400 / 422)
```

The split is **load-bearing, not decorative**. Sidekiq retries a job whose exception escapes, so a 409
classified transient would spin forever and a 503 classified permanent would be silently dropped.

| Call | On a `TransientError` | On a `PermanentError` |
|---|---|---|
| **Idempotent read** (`show`, `list`, `search`, `whoami`) | retried, **3 attempts**, backoff 200ms then 400ms | raised on the first attempt |
| **Write** (`create`, `update`, `delete`, `assert_relationship`, `set_facet`) | **never auto-retried** — raised | raised |

> **Writes are never auto-retried, even on a 503.** The SDK classifies; it does not decide to
> re-submit. Retrying a write it cannot prove failed is how you get two postmortems. A 401 is the one
> exception, and it is not a re-submission — re-minting a token and replaying the call is repairing
> *authentication*, not retrying the operation.

Two classifications are not derivable from the HTTP status alone:

- **`SystemAccessRequired`** discriminates off `error.code == "SYSTEM_ACCESS_REQUIRED"`, not off the
  403. A plain 403 maps to `Forbidden`. It means the caller **authenticated but has no reach** — the
  machine profile exists and is registered, and is missing a grant.
- **`ConnectionError`** is inferred from an `ApiError` with a **nil** status. The generated client
  rescues Faraday timeouts and connection failures and re-raises them code-less; without that branch
  every timeout would fall through to a bare `Temper::Error` and Sidekiq would dead-letter it.

A response that is not the server's `{"error":{code,message,details}}` envelope — an HTML 502 from a
proxy, say — degrades to a raw body on `#details` rather than raising inside the error path.

*Backed by:* `spec/temper/error_mapper_spec.rb` (every mapping above, individually);
`spec/temper/client_spec.rb` — "retries an idempotent read on 5xx, three attempts", "backs off between
read retries", "never auto-retries a write, even on 503", "does not retry a permanent error on a
read", "translates a 403 SYSTEM_ACCESS_REQUIRED into the named exception".

## Going live

**Authentication is not authorization.** This is the wall every new machine caller hits, and it hits
it in two stages.

A valid M2M token — one your issuer happily minted, correctly signed, correctly audienced — gets you a
clean **401** until someone registers your `client_id` with the instance:

```
machine client 'tmpr_abc123' is not registered with this instance.
An administrator must run: temper admin machine provision --client-id tmpr_abc123 --label <label>
```

Temper keeps a registration allowlist and the gate is **lookup-or-reject** — there is no first-call
auto-provisioning. The SDK classifies that 401 as a `PermanentError`, so a Sidekiq worker
dead-letters it rather than retrying forever against a wall.

Once registered, you clear authentication and meet the **second** wall: a **403**
(`SystemAccessRequired`) on anything you have no reach for. Registration creates the agent profile;
it does not, by itself, grant it anything.

### Register the client, and grant its reach, in one command

Both mint paths register in the same step that mints. Reach is **plural and explicit** — repeat
`--team` and `--cogmap` — and is **never inferred** from `--owner-team`, which records who *owns* the
machine and is never consulted for authorization.

```bash
# Auth0-issued: you already created the M2M application; register its client id
temper admin machine provision \
  --client-id "$TEMPER_M2M_CLIENT_ID" --label "acme-worker" \
  --owner-team acme-eng \
  --team acme-eng:member \
  --cogmap acme-roadmap
```

```bash
# Temper-issued: temper mints the client id and secret; the SECRET PRINTS ONCE
temper admin machine issue \
  --label "acme-worker" \
  --owner-team acme-eng \
  --team acme-eng:member \
  --cogmap acme-roadmap
```

Each `--team <ref>[:role]` gives the machine **team membership** (role defaults to `member`) for read
reach. Each `--cogmap <ref>[:ro]` applies a **cogmap write grant** on that map (`:ro` for read-only).

> **This is not necessarily an operator ticket.** Minting is authorized by `is_system_admin` **or**
> ownership of the team that will own the machine — so a **team owner can register their own team's
> machine** with no admin in the loop, bounded to reach they could already confer on a human. A
> *teamless* machine (no `--owner-team`) is admin-only, because the empty owning team fails closed.
> The full rule, and what a team owner may and may not confer, is
> [machine-credentials.md](machine-credentials.md).

### Assert it at boot, not on the first write

```ruby
# Fail loudly at boot rather than discovering the wall on the first job
Temper::Client.new(credentials: TEMPER_M2M).whoami
```

An **unregistered** client surfaces as `Unauthorized` naming the client id. A
**registered-but-under-granted** one surfaces as `Forbidden` / `SystemAccessRequired` naming the
missing grant. Both come from the server's `error.details` — you get the diagnosis, not a bare 4xx.

*Backed by:* `spec/temper/client_spec.rb` — `whoami` reads `GET /api/profile` as an idempotent call;
"translates a 403 SYSTEM_ACCESS_REQUIRED into the named exception"; `spec/temper/error_mapper_spec.rb`
— "maps 401 to Unauthorized and 404 to NotFound" (as a `PermanentError`).

*Not spec-backed:* the wording of the server's 401 (from `crates/temper-services/src/services/profile_service.rs`)
and the CLI flags (from `crates/temper-cli/src/cli.rs`) are server- and CLI-side facts, verified
against that source, not against a gem spec.

## Rotation — two different operations

These are **not** the same thing and conflating them will cost you your authorship history.

| | `rotate-secret` | `rebind` |
|---|---|---|
| **What changes** | the **secret** — same `client_id` | a **new `client_id`**, same agent profile |
| **For** | a temper-issued (`tmpr_`) credential | rotating the external **IdP application** behind an Auth0 machine |
| **Who may run it** | admin, or owner of the machine's team | **system admin only** |
| **Downtime** | none — a **grace window** (default 24h) keeps the previous secret valid | none if you pass `--no-revoke-old` for an overlap |
| **What you redeploy with** | the new `TEMPER_M2M_CLIENT_SECRET` | a new `client_id` *and* secret |

```bash
# Roll a temper-issued secret; the old one stays valid for an hour while you redeploy
temper admin machine rotate-secret <machine-id> --grace 3600
```

The grace window is what makes this a non-event for a running fleet: mint the new secret, redeploy the
app with it in the environment, and the old secret expires under you with nothing dropped. At most two
secrets are ever live at once.

Rotating an **Auth0 secret** needs no Temper action at all — the `client_id` is unchanged, so
authorship history is continuous and Temper never saw the secret. Only rotating the Auth0
**application** (a genuinely new `client_id`) needs `rebind`, which transplants the existing agent
profile's identity — and its full accumulated reach — onto the new id. That inherited reach can exceed
a team owner's own authority, which is why `rebind` alone keeps the system-admin bar.

## Fork safety

The connection holds sockets. Clear the memo in every forked worker:

```ruby
# config/puma.rb
on_worker_boot { Temper.reset_connection! }

# config/initializers/sidekiq.rb
Sidekiq.configure_server { |_| Temper.reset_connection! }
```

Strictly speaking the sockets are already safe: `connection_pool` (>= 2.4, which
`net-http-persistent` pools through) drops pooled connections from a `Process._fork` hook, and the
gem's suite **measures** it — a forked child that skips the reset still causes a fresh TCP accept on a
keep-alive server. The hooks above clear *this gem's* memoized client rather than relying on a
transitive dependency's default. Keep them.

*Backed by:* `spec/temper/fork_safety_spec.rb` — "a forked child opens its own socket, even without
reset_connection!" (asserted on the server's accept count), "a forked child completes a real request
after reset_connection!".

## Addressing, and what the SDK will not do

Resources are addressed by **ref**: a bare UUID, or the decorated `sluggify(title)-<uuid>` form.
Resolution is **trailing-UUID-only**, so a stale slug half is harmless and there is no by-slug lookup.

```ruby
# Identical — the slug half is parsed off and ignored
client.resources.show('p4-design-the-gem-019f4912-3f20-7fd3-814f-13a5ddbe3cd7')
client.resources.show('019f4912-3f20-7fd3-814f-13a5ddbe3cd7')
```

`Temper.parse_ref` is available directly and never touches the network. Unparseable input raises
before a request is made — no fuzzy matching, never a guess.

**Bulk cogmap reconcile is deliberately absent.** `PUT /api/cognitive-maps/{id}` takes a pre-embedded
desired-state manifest whose `chunks_packed` is a required, client-computed 768-dimension BGE
embedding carried verbatim, with no server-side fallback. Ruby has no embedder, so that path is
physically out of reach — it is a CLI operator's job, not a Rails request's. Incremental authoring
(`cognitive_maps.author`, `assert_relationship`, `set_facet`) is fully supported, and on every one of
those paths **the server chunks and embeds for you**.

*Backed by:* `spec/temper/refs_spec.rb`; `spec/temper/resources_spec.rb` — "resolves a decorated ref to
its trailing UUID before addressing", "rejects a ref with no trailing UUID before making a request",
"never sends chunks_packed or content_hash -- the server computes them";
`spec/temper/cognitive_maps_spec.rb` — "authors into a map via ingest with home_cogmap_id -- the
server embeds".

## See also

- [Machine credentials](machine-credentials.md) — the operator's side: who may mint a credential, how reach is contained to the minter's own authority, revocation.
- [Working with Teams](teams.md) — the roles and ownership that a machine's reach is expressed in.
- [Building a cognitive map](building-a-cognitive-map.md) — what you are authoring into, and why bulk reconcile is a CLI act.
- [Self-hosting](self-hosting.md) — standing up the instance whose `/oauth/token` a temper-issued credential mints against.
