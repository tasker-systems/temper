//! Operations layer — commands, actions, events, and the Backend trait.
//!
//! This module is the canonical home for command/action/event vocabulary
//! shared across all surfaces (CLI-local-vault, CLI-cloud, MCP, API-HTTP)
//! and both backends (DbBackend in temper-api, VaultBackend in temper-cli).
//!
//! See `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md`.

#[cfg(test)]
mod smoke {
    /// Smoke test: the module compiles and is reachable.
    #[test]
    fn module_exists() {
        // No-op; existence of this test passing means the module compiled.
    }
}
