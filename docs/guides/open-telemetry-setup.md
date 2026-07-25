# OpenTelemetry Setup

How temper gets traces off a Vercel function and into a backend you can query.

> **Status, 2026-07-24.** Read this section before acting on anything below.
>
> | Part | State |
> |---|---|
> | Root spans on both Rust surfaces, act spans, the field convention + its gate | **shipped** (PR #528) |
> | Inbound `traceparent` extraction onto the root span | **shipped** (PR #529), measured in production |
> | One logging-init seam for all five binaries | **shipped** (PR #533) |
> | **OTLP exporter — traces to any OTLP/HTTP backend** | **shipped** (PR #535), verified against an in-memory exporter through the real router. Not yet run against a live vendor. |
> | Span links post-auth | **shipped** (this PR) — every authentication gate on both Rust surfaces. |
> | Outbound `traceparent` injection, metrics taxonomy | **not shipped.** Goal steps 5 and 6. |
>
> So the operator steps below are real: set the variables and traces will arrive. What has *not*
> happened yet is anyone pointing it at a live account, so the first person to do so should expect
> to shake out an auth-header or endpoint-suffix detail — and is the one who can finally answer the
> open question further down about Vercel's own infra spans.

## The thing that confuses everyone first: Vercel does not host a collector

The natural assumption when deploying to Vercel is that the platform offers an OTLP endpoint you
export to, the way it offers a Postgres integration. It does not, and the docs do not say so
plainly, so it is worth stating outright.

**Vercel's OTel product is a *drain*.** From
[the trace drain reference](https://vercel.com/docs/drains/reference/traces):

> Vercel forwards distributed tracing data to a configured OTLP/HTTP-compatible endpoint. The
> destination must be an HTTPS endpoint capable of receiving OTLP/HTTP requests.

The arrow points **outward**. You register your vendor's ingest URL in the Vercel dashboard, and
Vercel POSTs to your vendor. There is nothing on Vercel's side to export *to*.

Two things follow, and they are the whole reason this guide is short:

1. **There is no collector to provision** — not on Vercel, and not by us. Every OTLP backend worth
   using (Grafana Cloud, Honeycomb, Dash0, Axiom, …) exposes OTLP/HTTP ingest on the public
   internet. A temper function POSTs directly to it. No sidecar, no gateway, no infrastructure.
2. **The vendor is configuration, not architecture.** It is an endpoint URL and an auth header, both
   read from spec-standard environment variables. Switching backends is an env change.

### The one thing being Rust costs us

From [the instrumentation docs' Limitations section](https://vercel.com/docs/tracing/instrumentation):

> If your app uses manual OpenTelemetry SDK configuration without the usage of `@vercel/otel`, you
> will not be able to use [Session Tracing] or [Trace Drains].

That limitation is real and it is narrow. `@vercel/otel` feeds Vercel's drain through
`globalThis[Symbol.for("@vercel/request-context")].telemetry.reportSpans(...)` — **a JavaScript
function call**, which a Rust process cannot reach. That single sentence in the docs is describing
that one mechanism.

What it costs us: Vercel's in-dashboard Session Tracing view of our spans, and Vercel paying the
egress. What it does **not** cost us: the trace. A span POSTed from a Rust function to Honeycomb is
an ordinary span in Honeycomb.

**Not yet established:** whether Vercel's *own* infra spans (the `service.name: vercel-function`,
`scope.name: vercel` shape in their drain reference) still reach a configured drain for a **Rust**
function. If they do, we get platform request spans alongside ours for free. That is a ten-minute
check once an account exists — configure a drain, hit `api/axum.rs`, look — and it is not something
to design around beforehand.

### And a path that looks open but isn't

`@vercel/otel` 2.1.3's source contains a second mechanism: when `VERCEL_OTEL_ENDPOINTS` is set it
exports to an OTLP collector at `http://localhost:4318/v1/traces` — runtime-agnostic, reachable
from Rust. It is tempting and it is a ghost. Its documentation has been withdrawn (the announcing
changelog's link 308s to a page that never mentions a collector), nothing documents what would be
listening inside the sandbox, and a co-resident collector recurses the freeze problem rather than
solving it. Evidence is in research `019f943a` §5e.

We log whether the variable is present, because that is one line in a config dump. We do not design
around it.

## The shape of the design

```
                          ┌─────────────────────────────┐
  request ──traceparent──▶│ Vercel edge (forwards it)   │
                          └──────────────┬──────────────┘
                                         ▼
                          ┌─────────────────────────────┐
                          │ Rust function               │
                          │  root span (roots its own   │──── OTLP/HTTP+protobuf ────▶ your backend
                          │  trace; links the caller's) │       direct, over the
                          │  flush before freeze        │       public internet
                          └─────────────────────────────┘
```

Three properties of that picture are decisions, not accidents:

**Every root span roots its own trace.** temper never parents from an inbound `traceparent`, on any
surface. A trusted caller's trace is joined with an OTel **span link** recorded after
authentication — where *trusted* means the request passed an authentication gate (a JWT verified
against our JWKS, or an HMAC signature over the body keyed on a secret only temper's own services
hold), not a list of surfaces someone has to keep correct. So a linked trace in your backend is
always a caller that authenticated; an anonymous request carries the inbound ids as inert log fields
and joins nothing. Full rationale and the alternatives considered: decision
`019f95ff-e216-7dd1-b2aa-a49d20b1cd6c`, summarized in `crates/temper-telemetry/src/lib.rs`'s module
docs. The short version is that every trace worth joining is one temper sent itself, so refusing
everything else costs nothing.

**An inbound `sampled` flag is recorded, never obeyed.** Honoring it would let anyone set `-01` on
flood traffic and bill us for exporting every span of it. (Note that Vercel's *own* sampling does
consider the inbound decision — see the instrumentation docs' sampling table. That is the platform's
choice about the platform's spans, not ours.)

**Spans are flushed inside the invocation, on a budget.** Vercel *freezes* the sandbox after a
response rather than exiting the process, so a batch-export timer may simply never fire — spans queued
at freeze are lost silently and non-deterministically. temper exports on an explicit flush at the
response seam instead of trusting a timer.

That flush is a real network round trip, and it is bounded on purpose. It runs on `spawn_blocking` (so
it cannot stall a Tokio worker — measured: a blocking flush froze a one-worker runtime for its whole
duration, zero timer ticks), under a **500ms budget**, and single-flight (one `BatchSpanProcessor`
thread serves everyone, so concurrent flushes would otherwise each pay the sum of those ahead —
measured at 16 concurrent requests against a 300ms endpoint: median 1.226s for one round trip's work).
Past the budget the response goes out and the span rides the next flush or is lost. **Losing a span is
the right trade against stalling a request**, and the SDK's own ceiling — 5 seconds, hardcoded and not
configurable — is what that budget exists to cap.

**The budget is deliberately not configurable.** 500ms is a best-bet default we intend to *watch*
rather than a knob to turn — a knob nobody can yet evaluate is complexity bought on credit, and its
too-low setting is a silent kill switch (every flush times out, nothing exports, the logs still say
`span export on`). The signal to watch is a **`warn`**: *"span flush exceeded its budget; spans may be
lost."* If that fires routinely in a healthy deployment, the constant is wrong and by then there is a
number to replace it with. For the finer distribution, `RUST_LOG=debug` samples `flush_ms` per request
— safe on a live deployment, because both stacks filter per-layer, so raising the log level no longer
widens what is exported or billed.

The cost is reported as **`flush_ms`**, a field of its own. It is deliberately *not* folded into
`latency_ms`: the flush can only happen after the request span closes (until then there is nothing
queued to flush), so it is genuinely not part of the span it flushes. A caller's observed latency is
`latency_ms + flush_ms`. An earlier version of this guide named `latency_ms` alone as the meter for the
exporter's cost — that was structurally impossible, since it is taken before the flush runs.

**A link is only navigable if the span it names was exported.** The two halves have to ship together:
the receiving side records a link to `(trace_id, span_id)`, and something has to have *sent* those ids
from a span that reached the same backend. temper injects `traceparent` on its own outbound calls
(`crates/temper-telemetry/src/propagate.rs`), so a link in your backend resolves to a real span rather
than dangling. `tracestate` is omitted rather than sent empty, since W3C makes it optional and a valueless header on
every request is noise. Note that temper never *forwards* a caller's `tracestate` today: nothing reads
it inbound, and because temper never parents from inbound context, every `SpanContext` it builds carries
an empty one. Using the standard propagator is still right — it owns the wire format and the sampled
bit — but forwarding vendor state is a property this setup does not yet have.

## Pointing temper at a backend

Configuration is entirely [spec-standard OTel environment
variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/). No
temper-specific variable, and no vendor name, appears anywhere in our code.

| Variable | Purpose |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Where spans go — **the base**, to which the SDK appends `/v1/traces`. Use this one; the vendor examples below assume it. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | The trace endpoint **verbatim** — nothing is appended. Copying a vendor's base URL into this variable POSTs to `/` and 404s. |
| `OTEL_EXPORTER_OTLP_HEADERS` | `key=value,key=value`. **This is where vendor auth lives** — which is what makes the setup vendor-agnostic. |
| `OTEL_SERVICE_NAME` | Which deployable this is. Set it per Vercel project; the surfaces are separate functions and want separate names. |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | Sampling, from our config only. |
| `OTEL_SDK_DISABLED` | Turn the exporter off without a deploy. |

Two behaviours are temper's rather than the SDK's, and both are operator-visible:

- **No endpoint means no export — not `localhost:4318`.** The OTLP spec defaults the endpoint to a
  local collector, which would make every unconfigured process (your laptop, CI, a self-hosted
  install) export at something that is not there. temper treats "unset" as "off".
- **`OTEL_SDK_DISABLED` works because temper implements it.** `opentelemetry_sdk` 0.32 contains zero
  occurrences of that variable — it is in the spec and not in the crate. Only the literal value
  `true` (case-insensitive) disables export; `1` and `yes` deliberately do **not**, so a typo cannot
  silently blind you.

Protocol is **HTTP/protobuf**, not gRPC and not OTLP/JSON. Not a style preference: JSON's
`TimeUnixNano{low, high}` encoding is mishandled by some collectors — `@vercel/otel`'s own source
carries a comment saying exactly that — and it surfaces as wrong timestamps rather than as an error.

The HTTP client is reqwest's **blocking** one, and that is a correctness requirement rather than a
preference. `BatchSpanProcessor` exports from a dedicated OS thread with no Tokio reactor, so the async
client panics there with *"there is no reactor running"* and every span is silently dropped — the only
symptom being a `warn` from temper's own flush path. Being inside a runtime at the *call site* does not
help, which is why the mistake is easy to make in an otherwise fully-async codebase. Held in place by
`crates/temper-telemetry/tests/live_export_client.rs`, which posts to a real local socket; it fails on
the async client and passes on the blocking one.

### Tracing the CLI

The `temper` binary can export too, but it needs a **second** switch:

| Variable | Purpose |
|---|---|
| `TEMPER_CLI_TRACE` | `true` (case-insensitive; nothing else counts) lets the CLI export. Off by default. |

Compression is supported (`OTEL_EXPORTER_OTLP_COMPRESSION=gzip`). Worth stating because it was *not*
until recently: without the `gzip-http` feature the exporter does not ignore that variable, it fails to
build — and export goes silently off. Grafana Cloud's own OTLP examples commonly set it.

Both this *and* an OTLP endpoint are required. The extra switch exists because
`OTEL_EXPORTER_OTLP_ENDPOINT` is often already exported in a developer's shell for an unrelated
project, and `temper` should not start shipping your vault activity to a collector you configured for
something else. The servers need no equivalent: a deployment's environment is set deliberately, per
project.

Two CLI-specific behaviours follow from a CLI being a process that actually exits:

- **Spans are drained on the way out of `main`, on the success *and* failure paths.** The failure arm
  ends in `std::process::exit`, which runs no destructors, so a flush placed after a successful run
  would lose exactly the traces worth having.
- **Turning on tracing does not make the CLI chatty.** The fmt layer keeps its own `warn` default while
  the export layer filters at `info` independently, so stdout stays clean for `temper … | jq` and
  stderr stays quiet. `RUST_LOG=info temper …` still opts into verbose logging without changing what is
  exported, and vice versa.

### Grafana Cloud

```bash
OTEL_EXPORTER_OTLP_ENDPOINT="https://otlp-gateway-<region>.grafana.net/otlp"
OTEL_EXPORTER_OTLP_HEADERS="Authorization=Basic <base64 of instanceID:token>"
OTEL_SERVICE_NAME="temper-api"
```

Worth preferring if the metrics-taxonomy work (task `019f943d-f2f0`) lands next: traces, metrics,
and logs all arrive through one OTLP endpoint, so metrics do not reopen the destination question.

### Honeycomb

```bash
OTEL_EXPORTER_OTLP_ENDPOINT="https://api.honeycomb.io"
OTEL_EXPORTER_OTLP_HEADERS="x-honeycomb-team=<ingest key>"
OTEL_SERVICE_NAME="temper-api"
```

Worth preferring for trace query ergonomics. Traces-first, so metrics would likely need a second
destination.

### Setting them on Vercel

Each running site is an independent Vercel project (see [DEPLOYING.md](../../DEPLOYING.md)), and each
Rust surface is a separate **function** within it — `api/axum.rs`, `api/mcp.rs`, `api/internal.rs`.
Environment variables are per project, so all three functions in one project share them; give each
*project* its own `OTEL_SERVICE_NAME`.

```bash
vercel env add OTEL_EXPORTER_OTLP_ENDPOINT production
vercel env add OTEL_EXPORTER_OTLP_HEADERS production   # secret — the ingest key lives here
vercel env add OTEL_SERVICE_NAME production
```

## Local development

Nothing about the export path is Vercel-aware, so local is the same binary reading the same
variables. Two options:

- **Point straight at the vendor.** Set the same three variables with a separate
  `OTEL_SERVICE_NAME` (e.g. `temper-api-local`) so local spans are filterable. Simplest, and it
  exercises the exact production path.
- **Run a collector locally.** The sibling `tasker-core` repo has working Grafana/Tempo compose
  files to borrow. Better when iterating on span shape, since you are not filling a vendor account
  with debug traces.

With no `OTEL_*` variables set at all, temper logs to stdout as it does today and exports nothing.
An unreachable endpoint costs each request **at most the 500ms flush budget** and then degrades to the
same behaviour with a warning — an exporter that cannot reach its backend must never fail a startup, and
must never lengthen a request without bound. (Before that budget existed, an unreachable endpoint added
the SDK's full 5s to *every* request. `crates/temper-telemetry/tests/flush_budget.rs` is what keeps the
bound honest — it drives a listener that accepts and never answers.)

## Logging, which is what exists today

`temper_telemetry::init` owns how every temper process logs, in two variants:

- **`init_server_logging()`** — JSON on stdout, default `info`. Used by `api/axum.rs`, `api/mcp.rs`,
  `api/internal.rs`, and `temper-api`'s `main`. Their stdout *is* the log stream.
- **`init_cli_logging()`** — human-readable on **stderr**, default `warn`. Used by the `temper`
  binary. Its stdout is reserved for machine-readable JSON/TOON so `temper … | jq` stays clean;
  `ort`'s INFO chatter on embed paths would otherwise interleave with command output and break
  parsing.

`RUST_LOG` overrides either default. An unparseable `RUST_LOG` falls back to the default rather than
refusing to start.

The two variants differ deliberately, and `crates/temper-telemetry/src/init.rs`'s tests hold that
difference in place — including a differential test that renders the same event through the
`tracing_subscriber::fmt()` builder each `main` used to call and through the stack that replaced it,
asserting they match.

## Where the pieces live

| Concern | Location |
|---|---|
| Logging init, both variants | `crates/temper-telemetry/src/init.rs` |
| Inbound trace-context extraction, `ROOT_TRACE_FIELDS` | `crates/temper-telemetry/src/lib.rs` |
| Outbound trace-context injection (the mirror of the link) | `crates/temper-telemetry/src/propagate.rs`; called from `temper-client`'s outbound span |
| Keeping credentials out of span attributes | `crates/temper-telemetry/src/redact.rs` — a stopgap for one route family; goal `019f99dd-dc9c-79f1-947c-e61bde2148a9` owns the real registry |
| The flush budget | `export::flush_within_budget`, gated by `tests/flush_budget.rs` |
| Root span construction (HTTP) | `crates/temper-api/src/routes.rs`, `apply_transport_layers` |
| Root span construction (MCP) | `crates/temper-mcp/src/router.rs` |
| Act-grain span fields | `temper_services::backend::ACT_SPAN_FIELDS`, declared by `#[act_span]` (`crates/temper-macros`) |
| Joining a trusted caller's trace (the link) | `crates/temper-telemetry/src/link.rs`; called from each auth gate |
| What is enforced, and why | [span-field-conventions.md](../development/span-field-conventions.md), gated by `tests/e2e/tests/logging_test.rs` |
| The trust decision | `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c` |
| Platform findings behind this guide | research `019f943a` §5 |
