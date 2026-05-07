//! Trait-impl integration tests for `DbBackend`.
//!
//! Each test uses `#[sqlx::test(migrator = "crate::MIGRATOR")]` for an
//! isolated per-test database. Happy path + one error path per trait method.
//! Object-safety is verified by promoting Phase 1's smoke test to a real
//! `Box::new(DbBackend) as Box<dyn Backend>`.

#![cfg(test)]
