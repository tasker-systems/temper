//! The CLI's write path is `temper-client`, and no reachable method exists only to refuse.
//!
//! Two guards, both read from source at test time:
//!
//! - **No stub answers.** No string literal in any `crates/*/src` file answers "not wired" or
//!   "not implemented". Such a literal is the answer a method gives when it exists to satisfy a
//!   trait rather than to do its act. Comments are not scanned: they describe, they do not answer.
//!   Test trees (`tests/`) are not scanned, because a test may name the phrase while asserting
//!   that it is absent.
//! - **One CLI write path.** No `temper-cli` source names the `Backend` trait from
//!   `temper_workflow::operations` or implements a `Backend`. The CLI reaches the server through
//!   `temper-client`'s operations, like every other CLI act. A second write abstraction would
//!   bring back the per-method stubs the first guard catches.
//!
//! - **No shared trait.** No crate declares `trait Backend` or implements one. `DbBackend` runs
//!   the server's write commands as its own methods.
//! - **No server command in the CLI.** No `temper-cli` source names the server's write commands
//!   (`CreateResource`, `UpdateResource`, `DeleteResource`, `AnnotateResource`) or their
//!   pre-flight validators. The CLI expresses a write as a `temper_workflow::shaping` input; the
//!   command vocabulary is the server's.
//! - **One shaping per act.** No door's source (`temper-cli`, `temper-mcp`) builds a resource-write
//!   wire request (`IngestPayload`, `ResourceUpdateRequest`, `ResourceAnnotateRequest`) by struct
//!   literal. Every door gets its request from `temper_workflow::shaping`, so the same inputs send
//!   the same request through every door.
//!
//! The matcher is line-grained. A phrase split across a string continuation (`"not \` then
//! `wired"`) is not caught. That gap favours green; the phrase list is short so it can be read.

use std::path::{Path, PathBuf};

/// The phrases a stub answers with.
const STUB_PHRASES: &[&str] = &["not wired", "not implemented"];

fn workspace_crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has a parent")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `.rs` file under each crate's `src/`.
fn src_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates = std::fs::read_dir(workspace_crates()).expect("read crates/");
    for krate in crates.flatten() {
        rust_files(&krate.path().join("src"), &mut out);
    }
    out.sort();
    out
}

/// The code part of a line: blank for a comment line, otherwise the text before a trailing `//`
/// that sits outside a string literal.
fn code_part(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return "";
    }
    let mut in_str = false;
    let mut escaped = false;
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_str = false,
                _ => {}
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return &line[..i];
        }
    }
    line
}

/// The text inside string literals on a line's code part. A literal still open at the end of
/// the line runs to the end of the line.
fn string_contents(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    let mut escaped = false;
    for (i, b) in code.bytes().enumerate() {
        match start {
            Some(s) => match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => {
                    out.push(&code[s..i]);
                    start = None;
                }
                _ => {}
            },
            None if b == b'"' => start = Some(i + 1),
            None => {}
        }
    }
    if let Some(s) = start {
        out.push(&code[s..]);
    }
    out
}

#[test]
fn no_source_string_answers_not_wired() {
    let mut hits = Vec::new();
    for path in src_files() {
        let text = std::fs::read_to_string(&path).expect("read source");
        for (n, line) in text.lines().enumerate() {
            for lit in string_contents(code_part(line)) {
                let lower = lit.to_lowercase();
                if STUB_PHRASES.iter().any(|p| lower.contains(p)) {
                    hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "a reachable method answers with a stub refusal; do the act or delete the method:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_cli_names_no_backend_trait() {
    let mut src = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut src);
    let mut hits = Vec::new();
    for path in src {
        let text = std::fs::read_to_string(&path).expect("read source");
        for (n, line) in text.lines().enumerate() {
            // Any whole-word `Backend` in code: an import (one-line or inside a multi-line
            // `use … ::{ … }` group), a bound, a `dyn`, an `impl … for`.
            let names_trait = code_part(line)
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == "Backend");
            if names_trait {
                hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "the CLI writes through temper-client, not a Backend trait:\n{}",
        hits.join("\n")
    );
}

/// No crate declares or implements a shared `Backend` trait. The server's command layer is
/// `DbBackend`'s own methods; a trait over it, with one implementation per process, is the seam a
/// second write path would hang from.
#[test]
fn no_crate_declares_a_shared_backend_trait() {
    let mut hits = Vec::new();
    for path in src_files() {
        let text = std::fs::read_to_string(&path).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let words: Vec<&str> = code_part(line)
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|w| !w.is_empty())
                .collect();
            let declares = words.windows(2).any(|w| w == ["trait", "Backend"]);
            let implements = words.windows(3).any(|w| w == ["impl", "Backend", "for"]);
            if declares || implements {
                hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "a shared Backend trait is back; the server's commands are DbBackend's own methods:\n{}",
        hits.join("\n")
    );
}

/// Whole-word identifiers in a line's code part.
fn code_words(line: &str) -> Vec<&str> {
    code_part(line)
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// The server's resource-write commands and their pre-flight validators.
const SERVER_COMMANDS: &[&str] = &[
    "CreateResource",
    "UpdateResource",
    "DeleteResource",
    "AnnotateResource",
    "validate_create",
    "validate_update",
];

#[test]
fn the_cli_builds_no_server_command() {
    let mut src = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut src);
    let mut hits = Vec::new();
    for path in src {
        let text = std::fs::read_to_string(&path).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if code_words(line).iter().any(|w| SERVER_COMMANDS.contains(w)) {
                hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "the CLI builds a server write command; express the write as a \
         `temper_workflow::shaping` input instead:\n{}",
        hits.join("\n")
    );
}

/// The resource-write wire requests a door must not build for itself.
const WRITE_REQUESTS: &[&str] = &[
    "IngestPayload",
    "ResourceUpdateRequest",
    "ResourceAnnotateRequest",
];

/// Whether a line opens a struct literal of a write request: the type's last path segment
/// followed by `{`, as in `IngestPayload {` or `resource::ResourceUpdateRequest {`. A
/// signature (`-> Result<IngestPayload, …> {`) is not one: the brace there follows `>`.
fn builds_write_request(line: &str) -> bool {
    let code = code_part(line);
    WRITE_REQUESTS.iter().any(|ty| {
        code.match_indices(ty).any(|(i, _)| {
            let before_ok = code[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            let after = code[i + ty.len()..].trim_start();
            before_ok && after.starts_with('{')
        })
    })
}

#[test]
fn no_door_builds_a_write_request() {
    let mut src = Vec::new();
    for door in ["temper-cli", "temper-mcp"] {
        rust_files(&workspace_crates().join(door).join("src"), &mut src);
    }
    src.sort();
    let mut hits = Vec::new();
    for path in src {
        let text = std::fs::read_to_string(&path).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if builds_write_request(line) {
                hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "a door builds its own write request; shape it with `temper_workflow::shaping`:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_matchers_see_what_they_claim() {
    // A write-request literal is seen; a signature, a use and a comment are not.
    assert!(builds_write_request("    let p = IngestPayload {"));
    assert!(builds_write_request(
        "    let r = temper_workflow::types::resource::ResourceUpdateRequest {"
    ));
    assert!(!builds_write_request(
        "fn wire_begin(input: X) -> Result<IngestPayload, rmcp::ErrorData> {"
    ));
    assert!(!builds_write_request(
        "use temper_core::types::ingest::IngestPayload;"
    ));
    assert!(!builds_write_request(
        "    // IngestPayload { … } is built by shaping"
    ));
    assert!(!builds_write_request("    let p = MyIngestPayload {"));
    // A command named in code is seen; one named in a comment is not.
    assert!(code_words("    let cmd = CreateResource {").contains(&"CreateResource"));
    assert!(!code_words("    // no CreateResource here").contains(&"CreateResource"));

    // A stub literal is seen; a comment and a doc comment are not.
    let stub = r#"        Err(TemperError::Project("X::y not wired until cutover".to_string()))"#;
    assert!(string_contents(code_part(stub))
        .iter()
        .any(|l| l.contains("not wired")));
    assert!(string_contents(code_part("    // \"not wired\" in a comment")).is_empty());
    assert!(string_contents(code_part("    /// is not wired up")).is_empty());
    let trailing = r#"    let a = "ok"; // "not wired""#;
    assert_eq!(string_contents(code_part(trailing)), vec!["ok"]);
    // An escaped quote does not end the literal.
    assert_eq!(
        string_contents(r#"x("a \" not implemented")"#),
        vec![r#"a \" not implemented"#]
    );
}
