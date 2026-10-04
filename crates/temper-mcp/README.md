# temperkb-mcp

Temper's MCP tool layer: every tool an agent uses to reach a [temper](https://github.com/tasker-systems/temper)
knowledge base, declared once and served by any host.

The crate declares the tools, validates their inputs, relays every act to temper's API, maps the
API's refusals to MCP errors and stamps the cache policy. It holds no credential, verifies
nothing and reads no configuration. The API authenticates, authorizes, attributes and refuses
every act. A host supplies plain values: the relay's configuration once, and, per request, the
identity the relay acts as.

The library is `temper_mcp`. The package is `temperkb-mcp` because temper's published crates
share the `temperkb-` prefix.

## Hosting the tools

A host does three things.

1. **Implements `IdentitySeam`.** For each request, the seam returns the `OutgoingIdentity` the
   relay acts as: the person's bearer and the `Surface` the client declares, plus optional opaque
   extra headers and an optional `correlation_id`. It returns `None` when it has no identity. Then
   every tool answers the crate's not-connected refusal, never an answer.
2. **Builds a `RelayConfig`** from its own sources: the API base URL and a request timeout that
   fits its runtime.
3. **Mounts `TemperMcpService`** with its own transport. Construct it once and clone it per
   connection or request; the clone shares one connection pool.

```rust
use std::sync::Arc;
use std::time::Duration;

use temper_mcp::{BlobDoor, IdentitySeam, OutgoingIdentity, RelayConfig, TemperMcpService};
use temper_workflow::operations::Surface; // temperkb-workflow

struct PersonSeam {
    bearer: String, // however the host obtained the person's credential
}

impl IdentitySeam for PersonSeam {
    fn outgoing_identity(&self, _parts: &http::request::Parts) -> Option<OutgoingIdentity> {
        Some(OutgoingIdentity::new(self.bearer.clone(), Surface::CliCloud))
    }
}

let service = TemperMcpService::new(
    BlobDoor::Closed { refusal: "blob tools are not served by this host".into() },
    RelayConfig::new("https://temperkb.io", Duration::from_secs(120)),
    Arc::new(PersonSeam { bearer }),
);
// Serve it with rmcp, through the re-export so the versions match:
// temper_mcp::rmcp::serve_server(service, transport).await?;
```

The seam reads the request's `http::request::Parts`. A transport that carries no HTTP parts (stdio,
an in-memory duplex) gets empty ones, so a seam that does not look at the request works on every
transport.

`TemperMcpService::unavailable(blob_door, refusal)` builds a service with no relay. Its relayed
tools answer the host's own refusal sentence, which suits a host that is not configured yet.

## Declarations are fixtures

The crate ships its `tools/list` answer as a fixture, `temper_mcp::declarations::TOOLS_LIST`. Each
host runs one test that sends `tools/list` through its own transport and checks the answer:

```rust
// the raw JSON-RPC response bytes your transport answered…
temper_mcp::declarations::assert_tools_list_response(&bytes);
// …or the `result` value, if your client already parsed it
temper_mcp::declarations::assert_tools_list(&result);
```

If the host advertises anything else, its test goes red. That covers a changed declaration, a
different rmcp resolved into the host's graph, or middleware rewriting the answer. A host with a
closed blob door is checked against the set without the two blob tools.

## Features

- `telemetry` (default): forwarded to `temperkb-client`. With `default-features = false` the host
  inherits no OpenTelemetry export stack, and the relay sends no `traceparent`.

## Versioning

`rmcp` is part of this crate's public API and is re-exported as `temper_mcp::rmcp`; an rmcp major
bump is a breaking release of this crate. A change to any tool's name, inputs, answers or refusals
is a wire-contract change, and it is versioned as one. temper's crates release in lockstep at one
version.

## License

MIT
