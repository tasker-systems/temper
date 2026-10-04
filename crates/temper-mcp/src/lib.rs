//! temper-mcp — the MCP tool layer for agent workflows.
//!
//! Declares temper's MCP tools and relays every act across the network door to the API.
//! It holds no database handle, reads no configuration and verifies no credential. A host
//! supplies plain values: per process, the [`RelayConfig`] and the [`BlobDoor`] ([`host`]);
//! per request, the identity the relay acts as, through its [`IdentitySeam`] ([`seam`]). The
//! deployed host is `temper-mcp-server`; any other host serves the same tools the same way.

pub(crate) mod cache_policy;
pub mod host;
pub mod resources;
pub mod seam;
pub mod service;
pub mod tools;

#[cfg(test)]
mod source_gates;

pub use host::BlobDoor;
pub use seam::{IdentitySeam, OutgoingIdentity, RelayConfig};
pub use service::TemperMcpService;
