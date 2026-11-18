//! Every CLI command honours `--format`: nothing prints prose to stdout outside a human-only arm.
//!
//! Under `--format json`, stdout carries exactly one parseable document, or nothing. Guidance for
//! a person goes to stderr (`output::hint`, `output::warning`, the `*_err` twins). The guard reads
//! `temper-cli`'s own source at test time and fails on any stdout prose writer it finds outside
//! [`ALLOWED`]:
//!
//! - `output::success` / `item` / `header` / `label` / `dim` / `status_icon` / `blank`, which only
//!   ever write prose;
//! - `output::plain` called with a string literal or `format!`, not a rendered document;
//! - `println!` / `print!` whose format string has text besides its `{}` placeholders.
//!
//! [`ALLOWED`] names each function that may still do so, and why: today only human-only arms
//! (prose under `OutputFormat::Toon`, never under `Json`). An entry whose function no longer
//! offends fails the guard too, so the list only shrinks.
//!
//! **Raw artifacts are not prose.** A few commands exist to emit an artifact for a pipe or a file,
//! and print it verbatim in either format: `skill generate` (the skill markdown), `resource create
//! --show-template`, `auth export-token`, `blob get` to stdout, and `admin saml provision`'s env
//! bundle and SQL and `--from-seen`'s rows. Their `println!("{x}")` has no text of its own, so the
//! guard passes them by construction. Their status lines go to stderr.
//!
//! What it does not cover:
//!
//! - **Attribution is by the nearest preceding `fn` line.** A closure or nested fn is charged to
//!   its enclosing function. An allowlisted function is allowed every prose line it holds.
//! - **Payload shape.** A rendered document is trusted to be one document. Two `render` calls in
//!   one command, which would print two documents, are not caught.
//! - **Writers outside these forms.** A raw `std::io::stdout()` write, or a prose `String` built
//!   elsewhere and passed to `output::plain`, is not seen.
//! - **The matcher is line-grained.** A call split across lines is followed one line ahead only.
//!   Lines from a `#[cfg(test)]` line to the end of the file are skipped.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Functions allowed to write prose to stdout: `(path under src/, fn name, why)`.
const ALLOWED: &[(&str, &str, &str)] = &[
    // Human-only arms: the prose sits under `OutputFormat::Toon`, and `Json` renders the ack.
    (
        "commands/edge.rs",
        "print_ack",
        "Toon-only arm; Json renders the ack",
    ),
    (
        "commands/facet.rs",
        "print_ack",
        "Toon-only arm; Json renders the ack",
    ),
];

/// The `output::` writers that only ever print prose to stdout.
const PROSE_WRITERS: &[&str] = &[
    "output::success(",
    "output::item(",
    "output::header(",
    "output::label(",
    "output::dim(",
    "output::status_icon(",
    "output::blank(",
];

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

/// The code part of a line: blank for a comment line, otherwise the line itself. A `//` inside a
/// string literal is common in prose (URLs), so a trailing comment is not stripped; a writer only
/// named inside a trailing comment would be a false alarm, and none exists.
fn code_part(line: &str) -> &str {
    if line.trim_start().starts_with("//") {
        ""
    } else {
        line
    }
}

/// The name of the function a line declares, if it declares one.
fn fn_name(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let t = t.strip_prefix("pub(crate) ").unwrap_or(t);
    let t = t.strip_prefix("pub ").unwrap_or(t);
    let t = t.strip_prefix("async ").unwrap_or(t);
    let rest = t.strip_prefix("fn ")?;
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Whether a format string literal (the text after the opening quote) holds text besides `{}`
/// placeholders.
fn literal_has_text(after_quote: &str) -> bool {
    let mut depth = 0usize;
    let mut chars = after_quote.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // An escape: `\n` and friends are formatting, not prose.
                chars.next();
            }
            '"' if depth == 0 => return false,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            c if depth == 0 && !c.is_whitespace() => return true,
            _ => {}
        }
    }
    false
}

/// Whether the argument that starts `arg` (a call's text after its open paren, possibly empty
/// when the argument is on the next line) is prose.
fn plain_arg_is_prose(arg: &str) -> bool {
    let a = arg.trim_start();
    a.starts_with('"') || a.starts_with("format!(") || a.starts_with("r#\"")
}

/// Whether a `println!(`/`print!(` argument list is a format string with prose in it.
fn print_arg_is_prose(arg: &str) -> bool {
    match arg.trim_start().strip_prefix('"') {
        Some(rest) => literal_has_text(rest),
        None => false,
    }
}

/// Whether `code` (with `next` as the following line, for a call whose argument wraps) writes
/// prose to stdout.
fn writes_prose(code: &str, next: &str) -> bool {
    if PROSE_WRITERS.iter().any(|w| code.contains(w)) {
        return true;
    }
    let arg_after = |marker: &str| -> Option<String> {
        let i = code.find(marker)?;
        let rest = &code[i + marker.len()..];
        Some(if rest.trim().is_empty() {
            next.to_string()
        } else {
            rest.to_string()
        })
    };
    if let Some(arg) = arg_after("output::plain(") {
        if plain_arg_is_prose(&arg) {
            return true;
        }
    }
    for marker in ["println!(", "print!("] {
        // `eprintln!(` / `eprint!(` contain these; they write to stderr.
        if let Some(i) = code.find(marker) {
            if code[..i].ends_with('e') {
                continue;
            }
            if let Some(arg) = arg_after(marker) {
                if print_arg_is_prose(&arg) {
                    return true;
                }
            }
        }
    }
    false
}

/// Every `(path, fn)` in `temper-cli/src` that writes prose to stdout, with its first line.
fn offenders() -> Vec<(String, String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no sources under {src:?}");

    let mut found: Vec<(String, String, String)> = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&src)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        // The output module is where the writers live.
        if rel.starts_with("output/") {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("read source");
        let lines: Vec<&str> = text.lines().collect();
        let mut current = String::from("<file>");
        for (n, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            if let Some(name) = fn_name(line) {
                current = name.to_string();
            }
            let code = code_part(line);
            let next = lines.get(n + 1).copied().map(code_part).unwrap_or("");
            if writes_prose(code, next) && !found.iter().any(|(p, f, _)| *p == rel && *f == current)
            {
                found.push((
                    rel.clone(),
                    current.clone(),
                    format!("{}: {}", n + 1, line.trim()),
                ));
            }
        }
    }
    found
}

#[test]
fn no_command_prints_prose_to_stdout_outside_the_allowlist() {
    let allowed: BTreeSet<(&str, &str)> = ALLOWED.iter().map(|(p, f, _)| (*p, *f)).collect();
    let unexpected: Vec<String> = offenders()
        .into_iter()
        .filter(|(p, f, _)| !allowed.contains(&(p.as_str(), f.as_str())))
        .map(|(p, f, l)| format!("{p}::{f} line {l}"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "a command prints prose to stdout, which breaks `--format json`. Render a document \
         through `format::render` (build a small receipt from what the act knows if the server \
         answers nothing), or send guidance to stderr:\n{}",
        unexpected.join("\n")
    );
}

#[test]
fn every_allowlisted_function_still_offends() {
    let found: BTreeSet<(String, String)> =
        offenders().into_iter().map(|(p, f, _)| (p, f)).collect();
    let stale: Vec<String> = ALLOWED
        .iter()
        .filter(|(p, f, _)| !found.contains(&(p.to_string(), f.to_string())))
        .map(|(p, f, why)| format!("{p}::{f} ({why})"))
        .collect();
    assert!(
        stale.is_empty(),
        "these allowlist entries no longer print prose to stdout; delete them:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_matcher_tells_prose_from_a_document() {
    assert!(writes_prose(r#"    output::success("Team deleted.");"#, ""));
    assert!(writes_prose(
        r#"    output::plain("Nothing to withdraw.");"#,
        ""
    ));
    assert!(writes_prose(
        "    output::plain(",
        r#"        "wrapped prose","#
    ));
    assert!(writes_prose(r#"    println!("Pulled {n} files");"#, ""));
    assert!(!writes_prose("    output::plain(rendered);", ""));
    assert!(!writes_prose(
        "    output::plain(crate::format::render(&x, fmt)?);",
        ""
    ));
    assert!(!writes_prose(r#"    println!("{rendered}");"#, ""));
    assert!(!writes_prose(
        r#"    println!("{}", render(&x, fmt)?);"#,
        ""
    ));
    assert!(!writes_prose(r#"    eprintln!("a warning");"#, ""));
    assert!(!writes_prose(r#"    output::success_err("ok");"#, ""));
    assert_eq!(fn_name("pub async fn leave_remote("), Some("leave_remote"));
    assert_eq!(fn_name("    let x = 1;"), None);
}
