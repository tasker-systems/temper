//! A handler is transport: no HTTP handler opens a transaction or holds a connection across writes.
//!
//! A transaction, and the order of the writes inside it, belong to a service command in
//! `temper-services`. The handler calls that command and renders what it answers. The guard reads
//! source at test time and fails if any `temper-api` source opens a transaction, commits one, names
//! a `Transaction` or a `PgConnection`, or acquires a pooled connection.
//!
//! What it does not cover:
//!
//! - **The MCP door.** `temper-mcp` holds no pool at all, which is stricter, and its own
//!   `source_gates.rs` (`no_module_in_the_tool_layer_holds_a_pool_or_server_state`) keeps it so.
//! - **Sequenced autocommits.** A handler that calls two service writes one after the other, each in
//!   its own transaction, opens nothing here and passes. The 2026-11-18 measurement found none; a
//!   per-function scan for two or more service write calls is how that was checked, and it is not a
//!   gate.
//! - **Indirection.** A helper outside these crates that opens a transaction on a handler's behalf
//!   is not seen. That helper is a service command, which is the point.
//! - **The matcher is line-grained.** A call split across lines (`pool\n.begin()`) is caught only
//!   if `.begin()` lands on one line, which rustfmt does. Comment lines are skipped.

use std::path::{Path, PathBuf};

/// The tokens a transaction-owning handler would need.
const TRANSACTION_TOKENS: &[&str] = &[
    ".begin()",
    ".begin_with(",
    ".commit()",
    "Transaction<",
    "PgConnection",
    ".acquire()",
];

/// The crates whose `src/` holds HTTP handlers that can reach the database. The MCP door has its
/// own, stricter gate (see the module doc).
const HANDLER_CRATES: &[&str] = &["temper-api"];

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

/// The code before any `//` on the line, or nothing for a comment line. Crude about `//` inside a
/// string literal, which can only hide a token, never invent one.
fn code_part(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return "";
    }
    line.split("//").next().unwrap_or("")
}

/// Every `file:line: text` in the handler crates that names a transaction token. Code inside a
/// `#[cfg(test)]` module is skipped from that line to the end of the file: a test may build the
/// shape it asserts is absent.
fn offenders() -> Vec<String> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has a parent");
    let mut files = Vec::new();
    for krate in HANDLER_CRATES {
        rust_files(&crates.join(krate).join("src"), &mut files);
    }
    files.sort();
    assert!(
        !files.is_empty(),
        "no handler sources found under {crates:?}"
    );

    let mut found = Vec::new();
    for file in files {
        let src = std::fs::read_to_string(&file).expect("read source");
        for (n, line) in src.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            let code = code_part(line);
            if TRANSACTION_TOKENS.iter().any(|t| code.contains(t)) {
                found.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn no_handler_opens_a_transaction() {
    let found = offenders();
    assert!(
        found.is_empty(),
        "a handler opens or holds a transaction; move the transaction into a temper-services \
         command and have the handler call it:\n{}",
        found.join("\n")
    );
}

#[test]
fn code_part_skips_comments() {
    assert_eq!(code_part("    // pool.begin()"), "");
    assert_eq!(
        code_part("let tx = pool.begin(); // note"),
        "let tx = pool.begin(); "
    );
}
