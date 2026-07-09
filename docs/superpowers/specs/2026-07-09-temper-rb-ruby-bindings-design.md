# temper-rb — a native Ruby client for the temper API

**Date:** 2026-07-09
**Status:** Design approved. Four preamble beats scoped; the gem itself is not yet specced.

## Purpose

Let Ruby on Rails and Sinatra applications use temper natively. `temper-rb` is generated from the
temper API's OpenAPI contract, with a thin hand-written ergonomic layer over it. It is a **sibling**
of `temper-client`, not a wrapper around it: both consume the same HTTP API.

This is the first of a possible family (`temper-py`, `temper-ts`), so the contract work below is
deliberately language-neutral — the later two inherit it for free.

## Target callers

Two, and only two, for v1:

1. **In-process request/response** — a Puma web process, pooled access, stateless API calls.
2. **Background worker** — Sidekiq / ActiveJob / equivalent.

The primary actions are authoring and modifying resources, content blocks, edges, and properties in
contexts and cognitive maps.

Local developer tooling is explicitly **not** a target: `temper-cli` is the fit-for-purpose tool there.

## Why codegen, not magnus/FFI

The original instinct was to bind `temper-client` through magnus, mirroring
`tasker-core/crates/tasker-rb`. That shape is right for tasker and wrong here, for a reason worth
recording: tasker's "client side" is a full worker state machine with a resident lifecycle — queue
polling, long-lived interactions — so a foundational Rust core with language bindings is the only
sane architecture. **temper's client side is an API client.** Nothing is resident. Nothing polls.

Once the facts were on the table, the case collapsed:

- **Embedding does not run client-side by necessity.** The CLI computes chunks via `temper-ingest`,
  but the server recomputes them as an unconditional fallback. Ruby never needs `ort`.
- **`temper-client`'s auth is the wrong auth.** It implements `authorization_code` (PKCE) and
  `refresh_token`. A server process needs `client_credentials` — which `temper-client` does not have
  (see G3) and which is a twenty-line POST in Ruby.
- **`parse_ref` is a pure string function.** Trailing-UUID-only; no DB.

Strip those away and the shared logic is: set two headers, hold a bearer token, refresh on 401, POST
JSON. Not worth a native extension. The cost of one is concrete — `tasker-rb`'s `extconf.rb` aborts
with "Rust toolchain not found!" absent cargo, so every deploy target grows a toolchain or you
maintain a precompiled-gem matrix; and its Cargo.toml records that magnus's `embed` feature had to be
dropped because "static linking with embed causes segfaults on Apple M4 Pro due to ARM64 FEAT_LSE2
atomic instruction incompatibility."

Codegen also honors a rule we already hold. CLAUDE.md: *"the wire type lives in temper-core with
ts-rs derives. Both sides share the generated type. Never define a zod schema that mirrors a Rust
struct manually."* A hand-written Ruby client mirroring Rust structs violates that spirit; a
generated one honors it, and `ts-rs` is the standing precedent.

## The finding that reorganized the design

The investigation began as "which of `temper-client`'s ~120 methods do we bind." Wrong question. The
question that mattered was **who the ledger says did the thing** — and answering it surfaced three
latent gaps in temper itself, none of them Ruby-specific, plus a fourth that only appeared once
codegen was on the table.

## How attribution works today

Temper's answer to "who" is a triple, not a field.

| Column | Meaning |
| --- | --- |
| `kb_events.emitter_entity_id` (NOT NULL) | principal × surface, resolved as the natural key `<handle>@<surface>` |
| `kb_events.metadata` | `AgentAuthorship { reasoning, confidence, rationale, persona, model }` |
| `kb_events.invocation_id` | run-grain correlator: the agentic invocation this act ran under |
| `kb_events.correlation_id` | act-grain correlator: groups a multi-event act (e.g. a block's stream) |

`writes::resolve_emitter` resolves the emitter by joining `kb_entities` to `kb_profiles`:

```sql
SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id
WHERE e.profile_id = $1 AND e.name = p.handle || '@' || $2
```

It is a `fetch_one`. There is no lazy creation — `profile_service` provisions `<handle>@web`,
`<handle>@cli`, and `<handle>@mcp` for every new profile in a hardcoded loop, and without them
"every write would 500 on a missing emitter."

**On live write paths, the agent is never the entity.** The entity stays the principal whose
authority was borrowed, tagged with the surface it was borrowed through; the agent lives in
`metadata`. `temper-substrate`'s scenario loader *can* emit as an arbitrary agent-instance entity
(`emitter: "charter-agent#1"`), but that path is unreachable from the API.

The three real-world shapes, all already present in temper:

| Case | emitter | authorship | invocation |
| --- | --- | --- | --- |
| user acts, app transports (temperkb.io) | `dana@web` | `None` | `None` |
| agent acts, delegated (MCP session) | `dana@mcp` | `Some{…}` | `Some(run)` |
| app acts (Eve / steward, M2M) | `eve@mcp` | `Some{…}` | `Some(run)` |

## Gaps discovered

### G0 — the OpenAPI spec is not authoritative

70 routes are registered on the Axum router. 76 `#[utoipa::path]` annotations exist across the
handlers. Only **55** are registered in `ApiDoc`'s `paths(...)`. No `openapi.json` is emitted
anywhere; `components(schemas(...))` is hand-maintained; nothing fails when the router and the spec
disagree.

The cost is concrete. `resources::delete` annotates `params(("id" = Uuid, Path, …))`, but its handler
signature takes `Query<ActInput>` — the authorship and invocation query params appear **nowhere** in
the spec. A client generated from today's contract would emit `delete_resource(id)`, physically
incapable of sending authorship. `ActInput` is exactly the type G2 extends.

### G1 — `Surface` is dropped at the HTTP boundary

`Surface` has three variants (`CliCloud → "cli"`, `Mcp → "mcp"`, `ApiHttp → "web"`) and is stamped
**server-side** from the command's origin. `temper-cli`'s cloud backend constructs
`Surface::CliCloud`, threads it through the command, and discards it at the wire, as its own comment
concedes:

> `origin` is unused here: this backend forwards over HTTP, and the server attributes the event to
> the surface it actually received (`Surface::ApiHttp`). Carrying the CLI's origin across the wire
> would need a header or payload field.

Consequently **every cloud-mode CLI write is attributed `<handle>@web`**, and the `<handle>@cli`
entity provisioned for every profile is never resolved. `@mcp` is real only because `temper-mcp`
calls `DbBackend` in-process. A Rails process is likewise indistinguishable from a browser.

### G2 — `correlation_id` is not client-settable

The SQL is ready: `emit_event` takes `p_correlation uuid DEFAULT NULL` and applies a root-event
convention, `COALESCE(p_correlation, v_ev)`. The wire type is not. `ActContext` — "the single
canonical home… the shared wire carrier" — carries only `invocation` and `authorship`.

The two things currently *named* correlation ids never reach the ledger:
`SegmentedBeginResponse.correlation_id` is minted server-side and documented as "not yet threaded onto
the server's event ledger"; the steward's `x-steward-correlation-id` header is passed to
`tracing::info!` and nothing else.

### G3 — `temper-client` cannot obtain a machine token

`temper-services::auth::normalize_machine` validates Auth0 `client_credentials` tokens
(`gty == "client-credentials"`, client id from `azp`). But `temper-client` implements only
`authorization_code` (PKCE) and `refresh_token`. Under the codegen design this stops being a
`temper-client` problem — the Ruby client implements the grant itself — but it is recorded here
because it explains why no existing client could serve the background-worker caller.

## Design decisions

**D1 — A fourth surface, named `sdk`.** Not `rb`. The existing markers name *how the principal's
authority was exercised* (`web` = browser, `cli` = terminal, `mcp` = agent tooling), not the client's
implementation language. A second Rust client built on `temper-client` is the same kind of surface.
`temper-py` and `temper-ts` inherit it.

**D2 — Surface travels in an `X-Temper-Surface` header, with an allowlist.** `{sdk, cli}` are
trusted. Everything else — absent, unparseable, and **`mcp`** — is logged and degraded to `web`.
`mcp` is excluded deliberately: `temper-mcp` reaches `DbBackend` in-process, so a remote caller
claiming `mcp` is lying by construction. Surface is provenance, never authorization; a bad value
degrades, it never rejects.

**D3 — Attribution changes honestly at the enqueue boundary.** The Puma request carries the end
user's token and writes as `dana@sdk`. The Sidekiq job authenticates as the machine profile and
writes as `acme-app@sdk`. The app really is the one writing, four hours later, on retry #3 — a user's
access token is long dead by then, so forwarding it is not an option.

**D4 — `correlation_id` stitches the two together.** The request mints a correlation id, serializes
it into the job arguments as a bare UUID (which outlives any credential), and the worker's writes
stamp the same value. Act-grain, which is what a "publish this postmortem" act spanning a request and
a deferred job actually is.

**D5 — `authorship` stays `None` in v1.** `AgentAuthorship` is agent-shaped: `confidence` is
mandatory precisely because it is an agent's subjective self-assessment, alongside `persona` and
`model`. A request handler and a background job have none of those. Both are keyboard-holder-shaped
acts. Authorship becomes necessary only when a Ruby *agent* authors into a cogmap.

**D6 — Invocations are not the carrier.** `kb_invocations` is run-grain and agent-shaped
(`trigger_kind`, `originating_cogmap_id`, `delegated_launch`, `scoped_entity_id`). It models an agent
working-session envelope — procedural discipline and findability for a deployed agent. A Rails request
is not an agent run.

**D7 — Generated core, hand-written skin.** Raw `openapi-generator` Ruby output is boilerplate-heavy.
The likely seam: generate the wire types and low-level operations; hand-write a thin ergonomic layer
over them — keyword args, sensible defaults, typed responses. This is the role
`lib/tasker_core/client.rb` plays over `TaskerCore::FFI`, except what sits underneath is generated
Ruby rather than a compiled `.bundle`. Choosing the generator and drawing that seam is P4's job.

## Preamble — four beats, none of them Ruby

**P0. Make the OpenAPI spec authoritative.** Register the ~15 orphaned annotated handlers, document
`ActInput` as query params where handlers take it, emit `openapi.json` as a build artifact, and add a
CI gate that fails when the router and the spec disagree. Precondition for any generation. Valuable
independently: it is the public contract.

**P1. Provision the `sdk` emitter entity.** `profile_service`'s loop becomes
`["web","cli","mcp","sdk"]`, plus an additive migration backfilling `<handle>@sdk` for every existing
profile. Guard with `NOT EXISTS`, not `ON CONFLICT`: `kb_entities` has no unique constraint on
`(profile_id, name)`.

*Must land before P2 deploys.* `resolve_emitter` is a `fetch_one`; a client sending `sdk` before the
entity exists 500s every write — the migrate-ahead-of-deploy skew shape.

**P2. Make `Surface` travel over the wire.** Add `Surface::Sdk`; `HttpClient` sends
`X-Temper-Surface` alongside the `X-Temper-Device-Id` it already sends; the Axum handlers read it
instead of hardcoding `ApiHttp`, applying the D2 allowlist.

Bundles the G1 CLI mis-attribution fix — the new header is exactly what makes `@cli` reachable, and
repo convention says a fix surfaced by a PR's own new code path rides along in it. **This visibly
changes ledger attribution for CLI writes from `@web` to `@cli`.**

**P3. `ActContext` grows `correlation: Option<Uuid>`.** Threaded through the commands into
`EventContext` (documented as mapping 1:1) and out to `emit_event(p_correlation)`. ts-rs types
regenerate and ride along. Lands after P0 so the extended `ActInput` is documented once.

**P4 (plan, large). Design the gem.** Generator choice; the generated/hand-written seam (D7);
`client_credentials` in Ruby; connection pooling under Puma; where the gem lives in this monorepo
(`packages/` is a Node workspace — undecided); packaging and release.

Ordering: **P1 → P2** is a hard constraint. **P0 → P3** is preferred. P0 and P1 are independent.

## Scope

### Rejected

- **A magnus/FFI binding over `temper-client`.** See "Why codegen, not magnus/FFI." *Condition for
  revisiting:* if Ruby ever needs client-side embedding via `ort` to spare server CPU, or to reuse
  genuinely heavy Rust computation. That would be a narrow optional native gem bolted on beside the
  client — never the client itself.
- **Binding by method count.** Surface size is not a cost driver, and is not a reason to shrink scope.
- **Cogmaps as a peripheral tier.** Cogmaps are what temper is *for*. Shipping a client that reads a
  map but cannot author into one is not a smaller scope; it is a broken one.
- **A surface marker named per language (`rb`/`py`/`ts`).** See D1.
- **Trusting `mcp` over the wire.** See D2.
- **Forwarding the end user's bearer token into background jobs.** Expired by retry #3. See D3.

### Deferred

- **Admin and SAML surfaces.** Operator surfaces, not library surfaces.
- **The `steward` sub-client** (`delta` / `advance-watermark`). A different role.
- **Invocation envelopes and `authorship`.** Needed when a Ruby agent authors into a cogmap; not
  before. See D5, D6.
- **`client_credentials` in `temper-client` (Rust).** Only if a Rust SDK consumer wants it.
- **`temper-py` and `temper-ts`.** Inherit P0–P3 for free.
