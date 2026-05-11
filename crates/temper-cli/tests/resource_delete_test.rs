//! Pre-API behavior tests for `temper resource delete`.
//!
//! Tests in this file exercise the early-return guards: the invalid-doctype
//! check (fires before any state matters) and the non-TTY confirmation
//! guard (fires before `with_client` is invoked). Behaviors that require
//! a live API — the cloud-first delete and the local-tail manifest
//! cleanup — are covered in `tests/e2e/tests/resource_delete_e2e_test.rs`.

use tempfile::TempDir;

mod common;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    common::init_isolated_auth();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
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
fn rejects_invalid_doctype() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Force Local mode + force=true so the only thing that can fail before
    // a network call is `validate_doc_type`.
    let result = temp_env::with_vars([("TEMPER_VAULT_STATE", Some("local"))], || {
        temper_cli::commands::resource::delete(&config, "widget", "any-slug", Some("myapp"), true)
    });

    let err = result.expect_err("invalid doctype must error before the API call");
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown doctype") && msg.contains("widget"),
        "expected DocType::from_str rejection, got: {msg}"
    );
}

#[test]
fn rejects_non_tty_stdin_without_force_in_local_mode() {
    use std::io::IsTerminal;

    // The cargo-nextest test runner redirects stdin away from any terminal,
    // so `std::io::stdin().is_terminal()` returns false here. If a
    // future test harness changes that, skip rather than flake.
    if std::io::stdin().is_terminal() {
        eprintln!("skipping: this test requires a non-TTY stdin");
        return;
    }

    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let result = temp_env::with_vars([("TEMPER_VAULT_STATE", Some("local"))], || {
        temper_cli::commands::resource::delete(
            &config,
            "task",
            "some-slug",
            Some("myapp"),
            /* force */ false,
        )
    });

    let err = result.expect_err("non-TTY without --force must error fast in local mode");
    let msg = format!("{err}");
    assert!(
        msg.contains("non-interactive stdin"),
        "expected non-TTY guard error, got: {msg}"
    );
    assert!(
        msg.contains("--force"),
        "expected --force hint in error message, got: {msg}"
    );
}
