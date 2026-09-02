# Telemetry

**For operators** — anyone running a Temper deployment who needs to understand what
observability the system emits and how the export pipeline is shaped. Also relevant to
**integrators** who want to understand the trace structure their calls appear in.

## What Temper emits

Temper emits two kinds of telemetry:

1. **Distributed traces** — OpenTelemetry spans exported over OTLP/HTTP to a backend you
   configure. Every request to the API or MCP server produces a root span; work within that
   request produces child spans.
2. **Structured logs** — JSON on stdout (server functions) or human-readable text on stderr
   (the CLI). Logs and traces are independent layers: raising the log level does not widen
   what is exported, and silencing logs does not stop trace export.

Temper does not emit native OTLP metrics. What exists is **span metrics** — a derived
Prometheus view that a backend's metrics-generator produces from the spans Temper exports.
RED panels and alert rules read that derived view, not a native metrics stream.

## The OTLP export model

A Temper function does not export to a platform-hosted collector. It POSTs spans directly,
over the public internet, to whichever OTLP/HTTP-compatible backend you point it at — Grafana
Cloud, Honeycomb, Dash0, Axiom, and others all expose OTLP/HTTP ingest on the public internet.

Two properties follow, and they are why the setup is configuration rather than infrastructure:

- **No collector to provision.** No sidecar, no gateway, no co-resident process. A function
  POSTs directly to the vendor's ingest URL.
- **The vendor is an env change.** Switching backends is a matter of repointing the endpoint
  URL and auth header — both read from spec-standard OpenTelemetry environment variables. No
  vendor name appears in Temper's code.

The protocol is **HTTP/protobuf**, not gRPC and not OTLP/JSON. JSON's timestamp encoding is
mishandled by some collectors and surfaces as wrong timestamps rather than an error.

One constraint of the Rust-on-serverless choice: the platform's own OpenTelemetry product feeds
its in-dashboard trace view through a JavaScript hook a Rust process cannot reach. What that
costs is the platform's in-dashboard view of Temper's spans (and the platform paying the
egress). What it does not cost is the trace itself — a span POSTed from a Rust function to your
backend is an ordinary span in your backend.

## The trace structure

Every request — HTTP or MCP — produces a **root span** that roots its own trace. Three
properties of that structure are architectural decisions, not accidents.

**Roots, not children.** Temper never parents a span from an inbound `traceparent`. A trusted
caller's trace is joined with an OpenTelemetry **span link** recorded after authentication —
where *trusted* means the request passed an authentication gate (a verified JWT or an HMAC
signature over the body keyed on a secret only Temper's own services hold). So a linked trace
in your backend is always a caller that authenticated; an anonymous request carries the inbound
trace ids as inert log fields and joins nothing. Every trace worth joining is one Temper sent
itself, so refusing everything else costs nothing.

**The inbound `sampled` flag is recorded, never obeyed.** Honoring it would let anyone set the
sampled bit on flood traffic and bill you for exporting every span of it.

**Spans are flushed inside the invocation, on a budget.** Serverless platforms freeze the
sandbox after a response rather than exiting the process, so a batch-export timer may never
fire — spans queued at freeze are lost silently. Temper exports on an explicit flush at the
response seam instead of trusting a timer. That flush is a real network round trip, bounded at
**500ms**, run on a blocking thread so it cannot stall an async runtime, and single-flight so
concurrent flushes do not each pay the sum of those ahead. Past the budget the response goes out
and the span rides the next flush or is lost. Losing a span is the right trade against stalling
a request.

The flush cost is reported as **`flush_ms`**, a field separate from `latency_ms`. The flush can
only happen after the request span closes, so it is genuinely not part of the span it flushes.
A caller's observed latency is `latency_ms + flush_ms`.

The flush budget is deliberately not configurable. A knob nobody can yet evaluate is complexity
bought on credit, and a too-low setting is a silent kill switch — every flush times out,
nothing exports, and the logs still say export is on. The signal to watch is a `warn`: *"span
flush exceeded its budget; spans may be lost."* If that fires routinely in a healthy deployment,
the constant is wrong.

## Span attributes and navigation

A link is only navigable if the span it names was exported. The receiving side records a link
to `(trace_id, span_id)`, and something has to have sent those ids from a span that reached the
same backend. Temper injects `traceparent` on its own outbound calls, so a link in your backend
resolves to a real span rather than dangling. `tracestate` is omitted rather than sent empty —
W3C makes it optional, and a valueless header on every request is noise.

## What the telemetry contains — the inventory

What a deployment's telemetry actually carries, hop by hop. Audited 2026-09-02; each field set
has one definition in code, cited beside it. This section records the **post-audit** state: the
audit's findings (raw page paths in temper-ui spans, unbounded error detail, query strings in
proxy logs, model I/O one convention away from export) were fixed in the same pass that wrote
this.

**Rust request roots** (`http_request`, `mcp_request` — one constructor,
`temper_telemetry::root_span!`): `method`; `path` — the real request path, run through the
deny-by-default redaction (`temper_telemetry::redact::redact_path`) that rewrites
credential-shaped segments under `/api/invitations/`; `http.route` and the exported name — the
matched route *template* (`/api/resources/{id}`); `version`; `profile_id` — temper's internal
profile UUID, recorded only once the request is authenticated, never the OAuth `sub` (PR #613's
2a decision); when the caller sent them, the inbound trace fields (`trace_id`,
`parent_span_id`, `trace_sampled`); `vercel_id` / `vercel_invocation_id`; and at the end,
`status` + `latency_ms`. A panicking handler records an ERROR span and a `panic` event with
`latency_ms` — the panic payload itself is never recorded.

**Act spans** (`#[act_span]`, one per write command): `correlation_id` and `invocation_id` —
caller-minted UUIDs, the attribution grain. Nothing else.

**temper-client** (`http_client_request`, the CLI/MCP outbound edge): method, redacted path,
`has_auth` (a boolean — never the token), `status`, `latency_ms`.

**Events riding exported spans** (the export filter is `info`, so WARN and above go to the
backend too): the structured error events carry `error_code` plus a message **bounded at 2 KiB**
(`temper_services::error::bounded`); a 5xx body carries the fixed generic string while only the
*event* keeps the bounded detail. The `unmatched route` event carries the path redacted and
capped at 512 bytes. sqlx slow statements (WARN, >1s) carry the statement's full SQL text —
documented under Logging. Slack lifecycle events carry the principal id
(`slack:<team>:<user>`, length-validated) and bounded IdP error text; the mint outcome's
hand-written `Debug` redacts the access token it wraps.

**temper-ui request spans**: `method`; the **door** — the route pattern
(`/vault/[owner]/[context]`) for matched pages, the proxy group (`/api/*`, `/mcp/*`) for
proxied paths, `unmatched` otherwise; scheme; `server.address` (the deployment's own host);
response status. Page titles, slugs, handles, and resource ids do not export: the raw pathname
appears nowhere in the span. A handler exception is recorded as an OTel exception (message +
stack of temper's own code).

**eve agents**: undici auto-instrumentation emits HTTP client spans whose URL is the configured
MCP endpoint (configuration, not user data). AI-SDK spans carry model name, tool names, and
token usage — and **no model inputs or outputs**: `NEVER_RECORD_MODEL_IO`
(`temper-telemetry-ts`) pins `recordInputs`/`recordOutputs` to `false`, the AI SDK's own
default is `true`, and each agent's test suite fails if the pin is dropped.

**What never exports**: OAuth subjects; message history; page titles, slugs, and handles;
query strings; access or refresh tokens; vault keys or ciphertext. Widening `RUST_LOG` never
widens export — it widens the *log stream*, which is a separate surface with its own retention
(see Logging).

## Logging

Every Temper process logs through one of two variants:

- **Server logging** — JSON on stdout, default `info`. Used by the API, MCP, and internal
  functions. Their stdout is the log stream.
- **CLI logging** — human-readable on stderr, default `warn`. The CLI's stdout is reserved for
  machine-readable output so `temper … | jq` stays clean.

`RUST_LOG` overrides either default. An unparseable `RUST_LOG` falls back to the default rather
than refusing to start.

**sqlx slow-statement logging comes for free, and is worth keeping.** sqlx logs any statement
whose execution reaches **1 second** at `WARN` on the target `sqlx::query` — *"slow statement:
execution time exceeded alert threshold"*, with the statement summary, the full SQL text, row
counts, and `elapsed_secs` (a JSON-friendly field to alert on). The default `info` filter
inherits it; no configuration is involved, and silencing it means narrowing `RUST_LOG` against
`sqlx::query` deliberately. Two consequences to know: the `WARN` event is **exported** (the
export layer's fixed filter is `info`), so a trace backend receives the SQL text alongside the
log stream; and `RUST_LOG=debug` widens sqlx from *slow statements* to **every** statement at
`DEBUG` — see below for why that keeps `debug` out of production.

## What the architecture provides vs. what a deployment configures

The trace structure, the export-on-flush model, the per-layer log filtering, the "unset means
off" default, and the protocol choice are fixed by the architecture. A deployment configures
the destination:

- **The OTLP endpoint** — which backend receives spans.
- **The auth headers** — vendor credentials, read from `OTEL_EXPORTER_OTLP_HEADERS`.
- **The service name** — for the Node hops; Rust functions self-name in code.
- **Sampling** — from `OTEL_TRACES_SAMPLER` / `OTEL_TRACES_SAMPLER_ARG`.
- **Whether export is on at all** — `OTEL_SDK_DISABLED=true`, or simply unsetting the endpoint.

Two defaults are Temper's rather than the SDK's, and both are operator-visible:

- **No endpoint means no export** — not `localhost:4318`. The OTLP spec defaults to a local
  collector, which would make every unconfigured process (your laptop, CI, a self-hosted
  install) export at something that is not there. Temper treats "unset" as "off."
- **`RUST_LOG` does not control export in either direction.** Both stacks filter per layer: the
  fmt layer follows `RUST_LOG`, the export layer carries its own fixed filter. Widening
  `RUST_LOG` changes what the log stream carries, never what is exported and billed. The
  surprising half: `RUST_LOG=off` still exports spans. The switches that stop export are
  `OTEL_SDK_DISABLED=true` and unsetting the endpoint.

- **`RUST_LOG=debug` does not belong in production.** The billing is unchanged — what widens is
  the *log stream's* leak surface, and the log stream is a third-party platform's with its own
  retention and access list. At `debug`, dependency targets log data-bearing detail: sqlx emits
  the **full SQL text of every statement** (not just the slow ones), HTTP clients log URLs —
  query strings included — and any `debug!` site's payload (mint outcomes, verification
  details) becomes log material. Diagnose a live deployment with `debug` only when the
  alternative is worse, and narrow it again afterward; the default `info` carries the
  slow-statement and request spans that answer most questions.

## Reading the exported spans

Spans reach a backend two ways, and they are not equivalent. The trace store (e.g. Tempo) holds
every span. **Span metrics** are a derived Prometheus view the backend's metrics-generator
produces, and everything Prometheus-backed — RED panels, alert rules — reads only that view.

The generator emits only for `CLIENT`, `SERVER`, `CONSUMER`, and `PRODUCER` span kinds. No
`INTERNAL` span appears in any RED panel or Prometheus alert — which includes agent tool-call
spans and the drain spans. The trace store answers directly over the spans that already exist,
with no generator change: a TraceQL metrics query over tool-call spans with `status=error`
returns per-tool error rates and is accepted as an alert query.

An unreachable endpoint costs each request at most the 500ms flush budget and then degrades to
no-export with a warning — an exporter that cannot reach its backend never fails a startup and
never lengthens a request without bound.

## Further reading

- **Sending traces to a backend (operator steps):**
  [Send traces to an OTLP backend](../playbooks/send-traces-to-an-otlp-backend.md).
- **Standing up a deployment:**
  [self-hosting Temper](../playbooks/self-host-temper.md).
- **The observability and audit concept:**
  [temperkb.io/operating/observability-and-audit](https://temperkb.io/operating/observability-and-audit).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
