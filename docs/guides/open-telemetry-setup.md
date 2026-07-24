# OpenTelemetry Setup

How temper gets traces off a Vercel function and into a backend you can query.

> **Status, 2026-07-24.** Read this section before acting on anything below.
>
> | Part | State |
> |---|---|
> | Root spans on both Rust surfaces, act spans, the field convention + its gate | **shipped** (PR #528) |
> | Inbound `traceparent` extraction onto the root span | **shipped** (PR #529), measured in production |
> | One logging-init seam for all five binaries | **shipped** (this PR) |
> | **OTLP exporter — anything that sends a span anywhere** | **not shipped.** Task `019f943d`. |
> | Span links post-auth, outbound `traceparent` injection, metrics | **not shipped.** |
>
> So: temper produces well-formed, correlated spans and writes them to **stdout as JSON logs**.
> Nothing exports yet. The "Pointing temper at a backend" section below describes the operator
> steps for an exporter that does not exist — it is the design being built to, not a runbook you
> can follow today. It is written down now because the *shape* of the answer is what was hard, and
> it was hard for a reason worth recording.

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
authentication. Full rationale and the alternatives considered: decision
`019f95ff-e216-7dd1-b2aa-a49d20b1cd6c`, summarized in `crates/temper-telemetry/src/lib.rs`'s module
docs. The short version is that every trace worth joining is one temper sent itself, so refusing
everything else costs nothing.

**An inbound `sampled` flag is recorded, never obeyed.** Honoring it would let anyone set `-01` on
flood traffic and bill us for exporting every span of it. (Note that Vercel's *own* sampling does
consider the inbound decision — see the instrumentation docs' sampling table. That is the platform's
choice about the platform's spans, not ours.)

**Spans are flushed inside the invocation.** Vercel *freezes* the sandbox after a response rather
than exiting the process, so a batch-export timer may simply never fire — spans queued at freeze are
lost silently and non-deterministically. temper exports on an explicit flush at the response seam
instead of trusting a timer. The added latency is measured, not assumed: `apply_transport_layers`
already logs `latency_ms` on every response, so the cost is the before/after difference in a number
we already have.

## Pointing temper at a backend

> **Not yet shipped.** These variables are the design target for task `019f943d`. Setting them today
> does nothing.

Configuration is entirely [spec-standard OTel environment
variables](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/). No
temper-specific variable, and no vendor name, appears anywhere in our code.

| Variable | Purpose |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Where spans go. `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` overrides it for traces specifically. |
| `OTEL_EXPORTER_OTLP_HEADERS` | `key=value,key=value`. **This is where vendor auth lives** — which is what makes the setup vendor-agnostic. |
| `OTEL_SERVICE_NAME` | Which deployable this is. Set it per Vercel project; the surfaces are separate functions and want separate names. |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | Sampling, from our config only. |
| `OTEL_SDK_DISABLED` | Turn the exporter off without a deploy. |

Protocol is **HTTP/protobuf**, not gRPC and not OTLP/JSON. Not a style preference: JSON's
`TimeUnixNano{low, high}` encoding is mishandled by some collectors — `@vercel/otel`'s own source
carries a comment saying exactly that — and it surfaces as wrong timestamps rather than as an error.

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
An unreachable endpoint degrades to that same behavior with a warning — an exporter that cannot
reach its backend must never lengthen a request or fail a startup.

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
| Root span construction (HTTP) | `crates/temper-api/src/routes.rs`, `apply_transport_layers` |
| Root span construction (MCP) | `crates/temper-mcp/src/router.rs` |
| Act-grain span fields | `temper_services::backend::ACT_SPAN_FIELDS` |
| What is enforced, and why | [span-field-conventions.md](../development/span-field-conventions.md), gated by `tests/e2e/tests/logging_test.rs` |
| The trust decision | `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c` |
| Platform findings behind this guide | research `019f943a` §5 |
