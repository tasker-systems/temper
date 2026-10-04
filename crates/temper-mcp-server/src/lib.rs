//! temper-mcp-server — the deployed MCP server around the `temper-mcp` tool layer.
//!
//! Holds what only a deployment needs: the JWT edge ([`middleware`]), OAuth discovery and the
//! static registration echo ([`discovery`], configured by [`discovery_config`]), the router with
//! its transport layers and CORS ([`router`]), the boot configuration it reads from the
//! environment ([`config`]), and the identity seam the tool layer relays as ([`seam`]).
//! Mounted by `api/mcp.rs` as a Vercel function beside the API.
//!
//! **It holds no database handle.** Every tool relays to the API across the network door, so
//! nothing here needs one: the boot reads only the auth identity, CORS origins and the blob
//! posture, and opens no connection. `tests/no_database_test.rs` keeps it that way.

pub mod config;
pub mod discovery;
pub mod discovery_config;
pub mod middleware;
pub mod router;
pub mod seam;

pub use config::McpServerConfig;
pub use discovery_config::DiscoveryConfig;
pub use router::{build_router, tool_service};
pub use seam::{BearerToken, DeployedDoorSeam};
