# Span and Field Conventions

What temper's spans are named, what fields they carry, and which of those are enforced.

This document is **not** the authority on its own — the gate in `tests/e2e/tests/logging_test.rs` is,
and the field set has a single definition in code (`temper_services::backend::ACT_SPAN_FIELDS`).
A convention that lives only in prose drifts from the code within a release; this one is written down
so the *reasoning* survives, while the *assertions* live where they can fail a build.

## The two clauses

**Clause 1 — every request produces a root span carrying the request-level fields.**
Unconditional. `method`, `path`, `version`, and `profile_id` once the request is authenticated.

**Clause 2 — when an act exists, its ids appear on a span of their own inside that request's tree.**
Conditional, and deliberately so.

The asymmetry is not an oversight. temper's C/U/D operations are **Acts** — they carry a
`correlation_id` (act grain) and optionally an `invocation_id` (run grain) into `kb_events`. A read
has no such mechanics: it is just a request. Asserting act ids on every request would encode a
fiction, so clause 2 fires only where an act genuinely exists.

## Why acts get their own span

`correlation_id` and `invocation_id` arrive in the **request body** (`ActInput` → `ActContext`), not
in headers or the URL. The `TraceLayer` root span is constructed before the body is parsed, so it
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
| `http_request` | `apply_transport_layers`, `crates/temper-api/src/routes.rs` | `method`, `path`, `version`, `profile_id` (deferred) |
| `mcp_request` | `build_router`, `crates/temper-mcp/src/router.rs` | same set; `profile_id` recorded in `service.rs` on profile resolution |
| act spans | `#[tracing::instrument]` on each write command in `crates/temper-services/src/backend/db_backend.rs` | `ACT_SPAN_FIELDS` — `correlation_id`, `invocation_id` (both deferred) |

Act spans take the **method name** as the span name (`update_resource`, `set_facet`, …) rather than a
uniform `act`, because the command is the most useful thing to see in a trace UI. The gate keys on
fields, not names, so adding a write command needs no gate edit.

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

1. Put `#[tracing::instrument(skip_all, fields(correlation_id = tracing::field::Empty,
   invocation_id = tracing::field::Empty))]` on the method.
   `skip_all` is not optional — commands carry bodies and secrets that must never reach a log.
2. Build the `EventContext` via `act_context(&cmd.act)`, which does the mapping *and* records the
   ids. Do not hand-roll the three-field struct; ten copies of it is what this helper replaced.
3. If the command fires under an invocation it opens itself rather than the caller's — as
   `reconcile_cognitive_map` does — build the `EventContext` explicitly and call `record_act_span`
   on **it**, so the span never reports an envelope the events do not carry.

## What this does not cover yet

- **No exporter.** These spans currently go to stdout as JSON via `tracing_subscriber::fmt().json()`.
  Turning them into an actual trace is the `temper-telemetry` task under goal
  `019f9404-2a4e-7530-8744-92ae4ab6d83e`.
- **No W3C trace context.** Nothing extracts or propagates `traceparent`, so spans do not yet join
  across deployables.
- **Reads are unspanned below the root.** Deliberate for now — see clause 2. If temper ever grows
  command-action mechanics for reads, this convention should grow with it rather than be worked
  around.
