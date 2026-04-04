//! temper-mcp — MCP (Model Context Protocol) server for agent workflows.
//!
//! Exposes the temper knowledge base to LLM agents via Claude Desktop,
//! Claude Code, and other MCP-compatible clients. Deployed as a Vercel
//! serverless function alongside the main temper-api.

pub mod config;
pub mod discovery;
pub mod middleware;
pub mod resources;
pub mod router;
pub mod service;
pub mod tools;

pub use config::McpConfig;
pub use router::build_router;
