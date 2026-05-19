//! `CloudBackend` — cloud-mode impl of [`temper_core::operations::Backend`].
//!
//! Struct and constructor land here in Task 1. The `Backend` trait impl
//! lands in Task 5.

use std::sync::Arc;

use temper_client::TemperClient;
use temper_core::operations::Surface;

use crate::config::Config;

use super::ctx::CloudBackendCtx;

/// Cloud-mode backend for CLI dispatch.
///
/// Holds the per-request fields needed to translate `Backend` trait commands
/// into `temper_client` API calls. `impl Backend for CloudBackend` lands in
/// Task 5.
pub struct CloudBackend {
    #[expect(dead_code, reason = "wired in Task 5")]
    pub(crate) client: Arc<TemperClient>,
    #[expect(dead_code, reason = "wired in Task 5")]
    pub(crate) owner: String,
    #[expect(dead_code, reason = "wired in Task 5")]
    pub(crate) config: Arc<Config>,
    #[expect(dead_code, reason = "wired in Task 5")]
    pub(crate) surface: Surface,
}

impl CloudBackend {
    pub fn new(ctx: CloudBackendCtx) -> Self {
        Self {
            client: ctx.client,
            owner: ctx.owner,
            config: ctx.config,
            surface: ctx.surface,
        }
    }
}
