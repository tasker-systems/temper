//! Pre-API behavior tests for `temper resource delete`.
//!
//! Tests in this file exercise the early-return guard: an unparseable ref is
//! rejected by `parse_ref` before any state matters or network call is made.
//! (Under decorated-ref addressing, delete no longer validates a doctype
//! string — doctype is read from the resolved server row.)
//!
//! Full cloud delete behavior (server-side soft-delete + projection-file
//! removal) is covered end-to-end in:
//! - `tests/e2e/tests/cloud_writes_test.rs` — `delete_removes_the_projection_file`
//!
//! The old local-mode non-TTY confirmation gate has been removed: cloud is
//! the only mode and cloud delete is non-interactive.

use tempfile::TempDir;

mod common;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    common::init_isolated_auth();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        profile_slug: None,
    }
}

#[test]
fn rejects_unparseable_ref() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // `force=true` so the only thing that can fail before a network call is
    // `parse_ref` rejecting a string with no trailing UUID.
    let result = temper_cli::commands::resource::delete(
        &config,
        "not-a-ref",
        true,
        temper_core::types::ActInput::default(),
        temper_cli::format::OutputFormat::Json,
    );

    let err = result.expect_err("an unparseable ref must error before the API call");
    let msg = format!("{err}");
    assert!(
        msg.contains("not a resource ref"),
        "expected parse_ref rejection, got: {msg}"
    );
}
