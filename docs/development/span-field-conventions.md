# Span and Field Conventions

What temper's spans are named, what fields they carry, and which of those are enforced.

This document is **not** the authority on its own — the gate in `tests/e2e/tests/logging_test.rs` is,
and each field set has a single definition in code (`temper_services::backend::ACT_SPAN_FIELDS` for
the act grain, `temper_telemetry::ROOT_TRACE_FIELDS` for inbound trace context).
A convention that lives only in prose drifts from the code within a release; this one is written down
so the *reasoning* survives, while the *assertions* live where they can fail a build.

## The two clauses

**Clause 1 — every request produces a root span carrying the request-level fields.**
Unconditional. `method`, `path`, `version`, and `profile_id` once the request is authenticated.
Plus, **when the caller sent them**, the inbound trace-context fields (`ROOT_TRACE_FIELDS`).

That last qualifier is the one exception to clause 1's unconditional shape, and it is not a
weakening. `traceparent` is request-level and header-borne, so it belongs to clause 1's grain rather
than earning a clause of its own — but a request genuinely may arrive with no upstream trace, and
the honest record of that is an empty field. The tempting alternative, minting a trace id locally
when none arrives, would give **every hop its own trace** instead of one trace per user action:
each deployable would report a confident-looking id that no other deployable shares. Minting root
ids is the tracer provider's job, and the gate asserts the absence specifically so this cannot be
"fixed" into existence later.

**Clause 2 — when an act exists, its ids appear on a span of their own inside that request's tree.**
Conditional, and deliberately so.

The asymmetry is not an oversight. temper's C/U/D operations are **Acts** — they carry a
`correlation_id` (act grain) and optionally an `invocation_id` (run grain) into `kb_events`. A read
has no such mechanics: it is just a request. Asserting act ids on every request would encode a
fiction, so clause 2 fires only where an act genuinely exists.

## Why acts get their own span

`correlation_id` and `invocation_id` arrive in the **request body** (`ActInput` → `ActContext`), not
in headers or the URL. The root span is constructed before the body is parsed, so it
*cannot* carry them. It would also be the wrong owner: an act is a domain concept and the root span
is a transport one.

There is a tempting shortcut here, and it is a trap. With no other spans in the process,
`Span::current()` inside a handler resolves to the root span, so recording the ids there "works" —
until the first nested span appears, at which point the ids silently attach to whatever happens to
be current. The gate's clause-2 assertion is written specifically to reject that arrangement: it
requires the carrying span **not** to be the root, identified by the absence of `path`.

## Where the spans come from

| Span | Created by | Fields |
|---|---|---|
| `http_request` | `apply_transport_layers`, `crates/temper-api/src/routes.rs` | `method`, `path`, `version`, `profile_id` (deferred), plus `ROOT_TRACE_FIELDS` (deferred) |
| `mcp_request` | `build_router`, `crates/temper-mcp/src/router.rs` | same set; `profile_id` recorded in `service.rs` on profile resolution |
| act spans | `#[act_span]` (`temper-macros`) on each write command in `crates/temper-services/src/backend/db_backend.rs` | `ACT_SPAN_FIELDS` — `correlation_id`, `invocation_id` (both deferred) |

Act spans take the **method name** as the span name (`update_resource`, `set_facet`, …) rather than a
uniform `act`, because the command is the most useful thing to see in a trace UI. The gate keys on
fields, not names, so adding a write command needs no gate edit.

### Inbound trace context

`temper_telemetry::record_inbound_trace_context` reads the request's headers and records five
fields, all deferred, all conditional on the header being present and well-formed:

| Field | Source | Notes |
|---|---|---|
| `trace_id` | W3C `traceparent` | The grouping key. The same value on every hop of one user action. |
| `parent_span_id` | W3C `traceparent` | The upstream span ours would be a child of, once a provider exists. |
| `trace_sampled` | W3C `traceparent` flags, bit 0 | Upstream's sampling decision; re-deciding downstream produces broken traces. |
| `vercel_id` | `x-vercel-id` | The bridge into Vercel's own per-request view. Generalizes the hand-rolled logging at the steward hop. |
| `vercel_invocation_id` | `x-vercel-internal-invocation-id` | Vercel's **function-invocation** grain. |

The header is *parsed*, not copied: `TraceParent::parse` enforces the spec's shape (hex lengths, the
forbidden all-zero sentinels, version `ff`, version `00`'s exact field count) and normalizes hex to
lowercase, because two spellings of one trace id split one user action into two rows in any query
grouped by it. A malformed header logs at debug and records nothing — a `trace_id` holding
`"garbage"` is worse than an empty one, since it looks like a real key until someone groups by it.

**`vercel_invocation_id` is not `invocation_id`, and the name is load-bearing.** temper's
`invocation_id` is the agent-run envelope in `ACT_SPAN_FIELDS`; Vercel's is a serverless
function-invocation id. They are unrelated grains that would merge silently under one field name, so
the two field sets are asserted disjoint in `temper-telemetry`'s own tests.

### Deferred fields are the house pattern

Declare `tracing::field::Empty` at span creation, `record` the value at the point it becomes known.
Established by `profile_id` in `crates/temper-api/src/middleware/auth.rs`, and now used by every
field in the table above. It is what lets a span carry a value that does not exist yet when the span
opens — which is true of every identifier worth correlating on.

### Naming: `http_request` is already overloaded

Both temper-api's root span and temper-client's outbound request span are named `http_request`. In a
single process's logs that is survivable; in an exported trace it is two different things under one
name, and in the e2e suite — which runs client and server in one process — you can watch both appear
side by side. temper-mcp's root span is therefore `mcp_request`, not a third `http_request`. Prefer a
name that says which side of the wire you are on.

## Adding a write command

1. Put `#[act_span]` on the method. It expands to the `#[tracing::instrument]` with the field set —
   including `skip_all`, which is not optional, because commands carry bodies and secrets that must
   never reach a log.
2. Build the `EventContext` via `act_context(&cmd.act)`, which does the mapping *and* records the
   ids. Do not hand-roll the three-field struct; ten copies of it is what this helper replaced.
3. If the command fires under an invocation it opens itself rather than the caller's — as
   `reconcile_cognitive_map` does — build the `EventContext` explicitly and call `record_act_span`
   on **it**, so the span never reports an envelope the events do not carry.

Both of the first two steps used to be spelled out by hand at every site, and both drifted. The
attribute was copy-pasted eleven times minus one: `record_citation_audit` shipped with neither the
attribute nor the `act_context` call, hand-rolling the `EventContext` instead — so a citation
audit's ids reached no span at all, and no gate noticed, because the constant that named the field
set was asserted only against a copy of itself. The rule worth carrying: **a constant that exists
"so things cannot drift" prevents nothing unless something asserts its consumers against it.**
`act_span_declares_every_act_field` (in `db_backend.rs`'s own tests) is that assertion — it opens a
real `#[act_span]` span and checks it declares every name in `ACT_SPAN_FIELDS`.

## What this does not cover yet

- **The exporter has landed; `trace_id` did not become a parent.** Spans go to stdout as JSON and,
  when `OTEL_EXPORTER_OTLP_ENDPOINT` is configured, over OTLP to whatever vendor it names
  ([../guides/open-telemetry-setup.md](../guides/open-telemetry-setup.md)). Init is one seam —
  `temper_telemetry::init_server_logging()`, with `init_cli_logging()` as the CLI's deliberately
  different variant — built on `Registry` + layers precisely so the exporter attached as one more
  layer. The CLI exports too, behind its own `TEMPER_CLI_TRACE` switch, and drains on the way out of
  `main` on both the success and failure paths.

  **The exporter did not actually reach a real endpoint until later, and the test suite could not
  see it.** As shipped, `BatchSpanProcessor`'s dedicated export thread ran an *async* HTTP client and
  panicked with *"there is no reactor running"*, dropping every span. The flush test passed throughout
  because an `InMemorySpanExporter` needs neither HTTP nor a runtime. Two lessons worth keeping: a
  telemetry test whose exporter has no transport cannot observe a transport bug, and *"we are inside a
  Tokio runtime"* was true at the call site and irrelevant to the thread doing the work.
  `crates/temper-telemetry/tests/live_export_client.rs` now posts to a real socket, which is the
  version of the assertion that can fail.

  `trace_id` keeps exactly the meaning it had before export: a **field, not a parent**. Decision
  `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c` settles that temper roots every trace locally on every
  surface and joins a *trusted* caller by span **link**, recorded after authentication
  (`temper_telemetry::link_trusted_caller`, called from each auth gate). "Trusted" means the request
  passed an authentication gate — a JWT verified against our JWKS, or an HMAC signature over the
  body keyed on a secret only our own services hold — not an allowlist of surfaces, which would be
  an enumeration that has to stay correct as routes move. An unauthenticated request keeps the inert
  fields and gains no link; `crates/temper-api/tests/telemetry_link_test.rs` drives both halves
  through the real router.
- **Propagation has landed on the Rust side, and it is what makes the link above worth having.**
  `temper_telemetry::propagate` injects `traceparent` onto outbound calls from the span representing
  that call (`temper-client`'s `http_request`), so a receiving surface's link names a span that was
  actually exported instead of one no exporter ever emitted. `tracestate` is forwarded when present and
  omitted when empty — W3C makes it optional, and a valueless header on every request is noise.
  `tests/e2e/tests/client_injection_link_test.rs` drives the whole loop at the production caller:
  temper-client sends, the server links, and the two ids match.

  Still missing is the **TypeScript** side. The mention flow's hops are all TS → Rust, so a trace still
  stops at the first hop an *eve agent* originates (task
  `019f943b-b22f-7602-a417-6df251102aa0`).

## What production actually does (measured 2026-07-24)

Whether Vercel forwards a client's `traceparent` into a Rust function could not be settled from the
crate source or the docs, so it was settled by deploying these fields and reading the logs
(`dpl_4jneaF2DPYrX92zMo5L6KiGBbkHP`, `iad1`):

| Request | `trace_id` / `parent_span_id` / `trace_sampled` | `vercel_id` / `vercel_invocation_id` |
|---|---|---|
| with a client `traceparent` | populated, intact, on **both** `http_request` and `mcp_request` | populated |
| with none | absent | populated |

**Vercel forwards a caller's `traceparent` and does not synthesize one.** The negative row is
trustworthy rather than a silent no-op precisely because the Vercel fields are populated beside it:
the extractor provably ran and found nothing. That disambiguation is why those two fields belong in
this set rather than in a later increment.

This **contradicted** the prior the work carried in. The spike had reasoned from `@vercel/otel` that
Vercel "does not use a header" — its `VercelRuntimePropagator.extract()` reads `rootSpanContext` off
a JS request-context object, `inject()` is a no-op, `fields()` returns `[]` — and inferred that
nothing would arrive. The inference confused two different things: a *library's propagator
implementation* is not the *platform's edge behaviour*. Worth remembering the next time a Node
package's internals are used to predict what a Rust function receives.

The consequence for the goal is that one trace per user action is reachable end to end, and the
remaining work is ours: every hop of the mention flow is our own code, and the header survives the
platform between them.
- **Reads are unspanned below the root.** Deliberate for now — see clause 2. If temper ever grows
  command-action mechanics for reads, this convention should grow with it rather than be worked
  around.
