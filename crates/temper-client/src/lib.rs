//! temper-client — Auth-aware HTTP client wrapping the temper cloud API.
//!
//! Shared by temper-cli, temper-mcp, and any future client. Handles JWT
//! lifecycle (login, refresh, logout), device identity, and typed methods
//! for every R5 API endpoint.

pub mod auth;
pub mod error;
pub mod http;
