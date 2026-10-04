//! temper-mcp-server — the deployed MCP server around the `temper-mcp` tool layer.
//!
//! Holds what only a deployment needs: the JWT edge ([`middleware`]), OAuth discovery and the
//! static registration echo ([`discovery`]), the router with its transport layers and CORS
//! ([`router`]), and the boot configuration it reads from the environment ([`config`]).
//! Mounted by `api/mcp.rs` as a Vercel function beside the API.
//!
//! **It holds no database handle.** Every tool relays to the API across the network door, so
//! nothing here needs one: the boot reads only the auth identity, CORS origins and the blob
//! posture, and opens no connection. `tests/no_database_test.rs` keeps it that way.

pub mod config;
pub mod discovery;
pub mod middleware;
pub mod router;

pub use config::McpServerConfig;
pub use router::build_router;
