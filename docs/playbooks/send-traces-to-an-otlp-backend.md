# Send traces to an OTLP backend

**For operators** — anyone running a Temper deployment who wants distributed traces to arrive
in an OTLP/HTTP-compatible backend (Grafana Cloud, Honeycomb, Dash0, Axiom, …).

## Outcome

By the end of this playbook your Temper deployment will export OpenTelemetry traces to a
backend of your choice, and you will have verified that spans appear there after a request. You
will know which environment variables to set, which to avoid, and how to turn export off without
a redeploy.

## Prerequisites

- **A running Temper deployment.** If you do not have one, see
  [self-hosting Temper](./self-host-temper.md).
- **An OTLP/HTTP-compatible backend** with an ingest URL and auth credentials ready. Any
  backend that accepts OTLP/HTTP on the public internet works.
- **The telemetry concept.** Read [telemetry](../concepts/telemetry.md) to understand the
  export model (direct POST, no collector) and the trace structure before configuring the
  destination.

## Configure the OTLP exporter

Configuration is entirely spec-standard OpenTelemetry environment variables. No
Temper-specific variable, and no vendor name, appears in the code.

| Variable | Purpose |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Where spans go — **the base**, to which the SDK appends `/v1/traces`. Use this one. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | The trace endpoint **verbatim** — nothing is appended. Copying a vendor's base URL into this variable POSTs to `/` and 404s. Avoid unless you need the verbatim form. |
| `OTEL_EXPORTER_OTLP_HEADERS` | `key=value,key=value`. **This is where vendor auth lives** — which is what makes the setup vendor-agnostic. |
| `OTEL_SERVICE_NAME` | Which deployable this is — for the **Node** hops. Rust functions name themselves in code (`temper-api` / `temper-mcp` / `temper-internal`), which the SDK ranks above this variable. On a project that also runs Node lambdas, set this to name the Node half. On a Rust-only project, leave it unset. |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | Sampling. |
| `OTEL_SDK_DISABLED` | Turn the exporter off without a deploy. Only the literal value `true` (case-insensitive) disables export; `1` and `yes` deliberately do not. |

Compression is supported via `OTEL_EXPORTER_OTLP_COMPRESSION=gzip`.

### Vendor examples

**Grafana Cloud**

```bash
OTEL_EXPORTER_OTLP_ENDPOINT="https://otlp-gateway-<region>.grafana.net/otlp"
OTEL_EXPORTER_OTLP_HEADERS="Authorization=Basic <base64 of instanceID:token>"
OTEL_SERVICE_NAME="temper-ui"   # names the Node half; Rust functions self-name in code
```

**Honeycomb**

```bash
OTEL_EXPORTER_OTLP_ENDPOINT="https://api.honeycomb.io"
OTEL_EXPORTER_OTLP_HEADERS="x-honeycomb-team=<ingest key>"
OTEL_SERVICE_NAME="temper-ui"   # names the Node half; Rust functions self-name in code
```

## Set the variables on Vercel

Each running site is an independent Vercel project, and each Rust surface is a separate
function within it. Environment variables are per project, so all functions in one project
share them.

```bash
vercel env add OTEL_EXPORTER_OTLP_ENDPOINT production
vercel env add OTEL_EXPORTER_OTLP_HEADERS production   # secret — the ingest key lives here
vercel env add OTEL_SERVICE_NAME production            # names the Node half only
```

Redeploy after adding the variables.

> **`service.name` for the Rust functions is set in code, not by `OTEL_SERVICE_NAME`.** A
> project that runs both Rust functions and Node lambdas cannot name both halves distinctly
> from one env var. Each Rust binary claims its own name in code, and the OTel SDK ranks that
> code-set value above `OTEL_SERVICE_NAME`. Set `OTEL_SERVICE_NAME` to whatever the **Node**
> half should be called; it does not touch the Rust spans.

### Tracing the CLI

The `temper` binary can export too, but it needs a second switch:

| Variable | Purpose |
|---|---|
| `TEMPER_CLI_TRACE` | `true` (case-insensitive; nothing else counts) lets the CLI export. Off by default. |

Both this **and** an OTLP endpoint are required. The extra switch exists because
`OTEL_EXPORTER_OTLP_ENDPOINT` is often already exported in a developer's shell for an unrelated
project, and the CLI should not start shipping vault activity to a collector configured for
something else.

## Verify

After the deployment finishes, confirm traces arrive.

### Send a request

Hit any endpoint:

```sh
curl https://<instance>/api/health
```

Or run a CLI command that talks to your instance (with `TEMPER_CLI_TRACE=true` and an OTLP
endpoint set):

```sh
temper resource list
```

### Check the backend

Open your vendor's trace explorer and filter by `service.name` — `temper-api`, `temper-mcp`, or
`temper-internal` for the Rust surfaces. A root span should appear within seconds.

If nothing arrives:

- **Confirm the endpoint is the base, not the verbatim traces URL.**
  `OTEL_EXPORTER_OTLP_ENDPOINT` gets `/v1/traces` appended; `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
  does not. A 404 in the vendor's ingest logs usually means the wrong variable was set.
- **Confirm `OTEL_SDK_DISABLED` is not `true`.** It is off by default; `1` and `yes` do not
  disable export.
- **Check deployment logs for the flush-budget warning.** A `warn` reading *"span flush
  exceeded its budget; spans may be lost"* means the endpoint is reachable but slow — spans are
  being dropped at the 500ms ceiling.
- **Remember `RUST_LOG=off` is not a kill switch.** It silences logs but still exports spans.
  If you intended to stop export, use `OTEL_SDK_DISABLED=true` or unset the endpoint.

### Confirm export is off when unset

With no `OTEL_*` variables set, Temper logs to stdout and exports nothing. This is the default
for local development, CI, and any unconfigured process — "unset" means "off," not "export to
localhost."

## Further reading

- **The export model and trace structure:**
  [telemetry](../concepts/telemetry.md).
- **Standing up a deployment:**
  [self-hosting Temper](./self-host-temper.md).
- **The observability and audit concept:**
  [temperkb.io/operating/observability-and-audit](https://temperkb.io/operating/observability-and-audit).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
