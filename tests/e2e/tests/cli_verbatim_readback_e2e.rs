#![cfg(all(feature = "test-db", feature = "test-embed"))]

//! E2E: a verbatim body piped to `temper resource create` reads back byte-for-byte through
//! `temper resource show --fields content`.
//!
//! `byte_exact_roundtrip_test.rs` proves the store: it drives the CLI create in-process and reads
//! the API `content` endpoint. This file proves the **surface a consumer actually touches**: the
//! real `temper` binary, the body on stdin (the non-TTY auto-detect branch no in-process call
//! reaches), and the body read back out of `show`'s JSON. A consumer reported exactly one `\n`
//! prepended on that path after a fresh create, so every body here is checked at three points —
//! the CLI's stdout right after create (while the local projection is fresh), the same read once
//! the projection has gone stale, and the stored bytes themselves — so a failure says which layer
//! added the byte.
//!
//! The bodies are the shapes that matter for a leading separator: prose-led, heading-led,
//! frontmatter-led (a body that opens with its own `---` fence), and bodies that genuinely begin
//! with one and two `\n`.
//!
//! Before trusting a local run: `cargo build -p temper-cli --bin temper` — `-p temper-e2e` does not
//! rebuild the binary these tests spawn (see `common::temper_bin_path`).

mod common;

use serde_json::Value;
use uuid::Uuid;

/// The bodies under test, each with a label for the failure message.
fn bodies() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "prose-led",
            "Plain prose opens this body.\n\nA second paragraph.\n",
        ),
        ("heading-led", "# Heading\n\nalpha\nbeta\n"),
        (
            "frontmatter-led",
            "---\nconsumer-key: value\n---\n\n# After a fence\n\nbody text\n",
        ),
        (
            "one-leading-newline",
            "\nA body that really starts with a newline.\n",
        ),
        ("two-leading-newlines", "\n\nTwo newlines lead this one.\n"),
    ]
}

/// Run the CLI with `input` on stdin and parse stdout as exactly one JSON document.
async fn cli_json_stdin(app: &common::E2eTestApp, input: &str, args: &[&str]) -> Value {
    let output = common::run_temper_cli_with_stdin(app, input, args)
        .await
        .expect("cli run");
    parse_one_json(&output, args)
}

async fn cli_json(app: &common::E2eTestApp, args: &[&str]) -> Value {
    let output = common::run_temper_cli(app, args).await.expect("cli run");
    parse_one_json(&output, args)
}

fn parse_one_json(output: &std::process::Output, args: &[&str]) -> Value {
    assert!(
        output.status.success(),
        "cli {args:?} failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "cli {args:?} did not emit exactly one JSON document ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// `show --fields content`'s body, as a string.
async fn show_content(app: &common::E2eTestApp, id: &str) -> String {
    let shown = cli_json(
        app,
        &[
            "resource", "show", id, "--fields", "content", "--format", "json",
        ],
    )
    .await;
    shown["content"]
        .as_str()
        .unwrap_or_else(|| panic!("show --fields content carries a string content: {shown}"))
        .to_string()
}

/// Age every file under `root` past the show debounce window, so no read can be served as fresh.
fn age_projection(root: &std::path::Path) {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(file) = std::fs::File::options().write(true).open(&path) {
                file.set_modified(old).expect("age projection file");
            }
        }
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn stdin_create_reads_back_byte_exact_through_show(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("verbatim-ctx", None)
        .await
        .expect("ctx create");

    for (label, body) in bodies() {
        let title = format!("verbatim readback {label}");
        let created = cli_json_stdin(
            &app,
            body,
            &[
                "resource",
                "create",
                "--type",
                "research",
                "--title",
                &title,
                "--context",
                "@me/verbatim-ctx",
                "--format",
                "json",
            ],
        )
        .await;
        let id = created["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: create carries an id: {created}"))
            .to_string();

        // The store: the server holds exactly the bytes that went in on stdin.
        let uuid = Uuid::parse_str(&id).expect("create id is a uuid");
        let stored = app
            .client
            .resources()
            .content(uuid)
            .await
            .expect("fetch content");
        assert_eq!(
            stored.markdown, body,
            "{label}: the server stored different bytes than stdin carried"
        );

        // Right after create: the create just wrote the local projection, so any read the CLI
        // could serve from it would be served now.
        let warm = show_content(&app, &id).await;
        assert_eq!(
            warm, body,
            "{label}: show right after create is not byte-exact"
        );

        // Once the projection is past the debounce window: the read must still be exact.
        age_projection(app.vault_dir.path());
        let cold = show_content(&app, &id).await;
        assert_eq!(
            cold, body,
            "{label}: show on a stale projection is not byte-exact"
        );
    }
}
