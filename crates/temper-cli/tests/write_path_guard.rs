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

#[test]
fn the_matchers_see_what_they_claim() {
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
