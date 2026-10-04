//! temper-mcp — the MCP tool layer for agent workflows.
//!
//! Declares temper's MCP tools and relays every act across the network door to the API.
//! It holds no database handle and no server configuration: the deployed edge (the JWT
//! check, OAuth discovery/registration, the router and its boot) is `temper-mcp-server`,
//! which hands this crate plain values ([`host`]) and serves the [`service`] it builds.

pub(crate) mod cache_policy;
pub mod config;
pub mod host;
pub mod resources;
pub mod service;
pub mod tools;

#[cfg(test)]
mod source_gates;

pub use config::McpConfig;
pub use host::{BearerToken, BlobDoor};
pub use service::TemperMcpService;
