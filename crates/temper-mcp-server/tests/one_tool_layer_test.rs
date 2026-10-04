//! **`one-tool-layer`, from the host's side: the deployed shell declares no tool of its own.**
//!
//! Every temper MCP tool is declared once, in the published tool layer (temperkb-mcp), and every
//! host serves that declaration. A `#[tool]` written here would be a tool the deployed door
//! serves and no other host can, which is exactly the drift the clause forbids. So would a second
//! `ServerHandler`: an rmcp tool router is tied to its service type, so a shell-side handler is
//! the only way a shell-side tool could reach the wire.
//!
//! The scan covers the shell's modules and its boot (`api/mcp.rs`), parsed with `syn`, test-only
//! items included: a tool is a declaration wherever it is written. Its limit: a `#[tool]` inside
//! a macro body is tokens, not an attribute, and is not seen. The positive half is that the
//! router serves the tool layer's service, which the declaration witness
//! (`tool_declaration_witness_test.rs`) holds to the shipped declarations.

use std::path::{Path, PathBuf};

use syn::visit::Visit;

/// rmcp's declaration attributes, by last path segment, so `#[tool]`, `#[rmcp::tool]` and a
/// renamed import's spelled path are all caught.
const DECLARATION_ATTRIBUTES: &[&str] = &[
    "tool",
    "tool_router",
    "tool_handler",
    "prompt",
    "prompt_router",
    "prompt_handler",
];

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Each declaration the source makes: an rmcp declaration attribute, or an `impl … ServerHandler`.
fn declarations(source: &str) -> Vec<String> {
    #[derive(Default)]
    struct V(Vec<String>);
    impl<'a> Visit<'a> for V {
        fn visit_attribute(&mut self, attr: &'a syn::Attribute) {
            if let Some(last) = attr.path().segments.last() {
                let name = last.ident.to_string();
                if DECLARATION_ATTRIBUTES.contains(&name.as_str()) {
                    self.0.push(format!("#[{name}]"));
                }
            }
            syn::visit::visit_attribute(self, attr);
        }
        fn visit_item_impl(&mut self, item: &'a syn::ItemImpl) {
            if let Some((_, path, _)) = &item.trait_ {
                if path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "ServerHandler")
                {
                    self.0.push("impl ServerHandler".to_string());
                }
            }
            syn::visit::visit_item_impl(self, item);
        }
    }
    let file = syn::parse_file(source).unwrap_or_else(|e| panic!("parse: {e}"));
    let mut v = V::default();
    v.visit_file(&file);
    v.0
}

/// The shell's modules (every `.rs` under `src/`) and its boot.
fn shell_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut dirs = vec![manifest("src")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&path).expect("read source");
                files.push((path.display().to_string(), src));
            }
        }
    }
    let boot = manifest("../../api/mcp.rs");
    files.push((
        "api/mcp.rs (the boot)".to_string(),
        std::fs::read_to_string(&boot).unwrap_or_else(|e| panic!("read {boot:?}: {e}")),
    ));
    files
}

/// FAILS IF: the shell or its boot declares a tool, a prompt, a tool router or an MCP handler of
/// its own.
#[test]
fn the_deployed_shell_declares_no_tool_of_its_own() {
    let sources = shell_sources();
    assert!(
        sources.len() >= 5,
        "read only {} files — the walk missed the shell",
        sources.len()
    );
    let offenders: Vec<String> = sources
        .iter()
        .flat_map(|(path, src)| {
            declarations(src)
                .into_iter()
                .map(move |d| format!("{path}: {d}"))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "the deployed shell declares MCP surface of its own; every tool is declared once, in \
         temperkb-mcp, and served from there:\n  {}",
        offenders.join("\n  ")
    );
}

/// The detector sees each shape it must, and not a comment or a string.
#[test]
fn the_detector_sees_each_declaration_shape() {
    for (source, expected) in [
        ("impl S { #[tool] fn t(&self) {} }", "#[tool]"),
        (
            "impl S { #[rmcp::tool(description = \"x\")] fn t(&self) {} }",
            "#[tool]",
        ),
        ("#[tool_router] impl S {}", "#[tool_router]"),
        ("#[rmcp::tool_handler] impl X for S {}", "#[tool_handler]"),
        ("#[prompt_router] impl S {}", "#[prompt_router]"),
        ("impl rmcp::ServerHandler for S {}", "impl ServerHandler"),
        ("#[cfg(test)] mod t { #[tool] fn t() {} }", "#[tool]"),
    ] {
        assert!(
            declarations(source).iter().any(|d| d == expected),
            "missed {expected} in {source}"
        );
    }
    for clean in [
        "// #[tool] in a comment\nfn f() {}",
        "fn f() -> &'static str { \"#[tool]\" }",
        "#[test] fn tool() {}",
        "impl Handler for S {}",
    ] {
        assert!(
            declarations(clean).is_empty(),
            "a false positive on {clean}: {:?}",
            declarations(clean)
        );
    }
}
