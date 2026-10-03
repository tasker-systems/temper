//! **The deployed MCP edge holds no database handle — boot included.**
//!
//! The least-privilege follow-up to the network door's teardown (its security review, finding 3):
//! every tool relays to the API, so a compromise of the MCP function must not hand over a database
//! handle the surface no longer uses. temper-mcp (the tool layer) carries its own crate-wide gate
//! (`src/source_gates.rs`); this one covers what the tool layer cannot see — the server's modules
//! AND its boot, `api/mcp.rs`, which is where a pool would actually be opened.
//!
//! Parsed with `syn`, the tool layer's idiom: a name split across lines by rustfmt is still the
//! same path or field, and a string literal is read as a literal (so `"DATABASE_URL"` is caught
//! where a comment naming it is not).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::visit::Visit;

/// Never named by the edge or its boot: the database driver and its pool, the API's state and
/// configuration (which carry the database URL), and the field that held them.
const FORBIDDEN_NAMES: &[&str] = &[
    "sqlx",
    "PgPool",
    "PgPoolOptions",
    "pool",
    "AppState",
    "ApiConfig",
    "api_state",
    "database_url",
];

/// Never read as a literal: the variable the API's database URL arrives in.
const FORBIDDEN_LITERALS: &[&str] = &["DATABASE_URL"];

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn parse(path: &Path) -> syn::File {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    syn::parse_file(&src).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// `#[cfg(test)]` (or any `cfg` naming `test`).
fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.meta
                .require_list()
                .map(|l| {
                    l.tokens
                        .to_string()
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|t| t == "test")
                })
                .unwrap_or(false)
    })
}

/// Every name a file's production items reach for — path segments (in `use` trees too), field
/// and method names, fields defined and bindings named — and every string literal they contain.
fn names_and_literals(file: &syn::File) -> (BTreeSet<String>, BTreeSet<String>) {
    #[derive(Default)]
    struct V {
        names: BTreeSet<String>,
        literals: BTreeSet<String>,
    }
    impl<'a> Visit<'a> for V {
        fn visit_path_segment(&mut self, s: &'a syn::PathSegment) {
            self.names.insert(s.ident.to_string());
            syn::visit::visit_path_segment(self, s);
        }
        fn visit_use_name(&mut self, n: &'a syn::UseName) {
            self.names.insert(n.ident.to_string());
        }
        fn visit_use_path(&mut self, p: &'a syn::UsePath) {
            self.names.insert(p.ident.to_string());
            syn::visit::visit_use_path(self, p);
        }
        fn visit_member(&mut self, m: &'a syn::Member) {
            if let syn::Member::Named(i) = m {
                self.names.insert(i.to_string());
            }
        }
        fn visit_expr_method_call(&mut self, c: &'a syn::ExprMethodCall) {
            self.names.insert(c.method.to_string());
            syn::visit::visit_expr_method_call(self, c);
        }
        fn visit_field(&mut self, f: &'a syn::Field) {
            if let Some(i) = &f.ident {
                self.names.insert(i.to_string());
            }
            syn::visit::visit_field(self, f);
        }
        fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
            self.names.insert(p.ident.to_string());
            syn::visit::visit_pat_ident(self, p);
        }
        fn visit_lit_str(&mut self, l: &'a syn::LitStr) {
            self.literals.insert(l.value());
        }
        // Macro bodies (`tracing::info!(…)`, `format!(…)`) are token streams, not syntax trees:
        // read their string literals so a `"DATABASE_URL"` inside one is still seen.
        fn visit_macro(&mut self, m: &'a syn::Macro) {
            fn literals_in(tokens: proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
                for tt in tokens {
                    match tt {
                        proc_macro2::TokenTree::Literal(lit) => {
                            if let Ok(s) = syn::parse_str::<syn::LitStr>(&lit.to_string()) {
                                out.insert(s.value());
                            }
                        }
                        proc_macro2::TokenTree::Group(g) => literals_in(g.stream(), out),
                        _ => {}
                    }
                }
            }
            literals_in(m.tokens.clone(), &mut self.literals);
            syn::visit::visit_macro(self, m);
        }
    }
    let mut v = V::default();
    for item in &file.items {
        let attrs: &[syn::Attribute] = match item {
            syn::Item::Fn(i) => &i.attrs,
            syn::Item::Mod(i) => &i.attrs,
            syn::Item::Use(i) => &i.attrs,
            syn::Item::Impl(i) => &i.attrs,
            syn::Item::Const(i) => &i.attrs,
            syn::Item::Static(i) => &i.attrs,
            syn::Item::Struct(i) => &i.attrs,
            syn::Item::Enum(i) => &i.attrs,
            _ => &[],
        };
        if !is_test_only(attrs) {
            v.visit_item(item);
        }
    }
    (v.names, v.literals)
}

/// The server's production modules plus its boot.
fn edge_sources() -> Vec<(String, syn::File)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(manifest("src")).expect("read src") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let rel = format!("src/{}", path.file_name().unwrap().to_string_lossy());
            out.push((rel, parse(&path)));
        }
    }
    out.push((
        "api/mcp.rs (the boot)".to_string(),
        parse(&manifest("../../api/mcp.rs")),
    ));
    out
}

/// FAILS IF: any server module or the boot names a pool, the database driver, the API's state or
/// configuration, or reads `DATABASE_URL` — i.e. if the MCP function could open, or be handed, a
/// database connection again.
#[test]
fn neither_the_edge_nor_its_boot_names_a_database_handle() {
    let sources = edge_sources();
    let rels: Vec<&str> = sources.iter().map(|(r, _)| r.as_str()).collect();
    for required in [
        "src/router.rs",
        "src/middleware.rs",
        "src/config.rs",
        "api/mcp.rs (the boot)",
    ] {
        assert!(
            rels.contains(&required),
            "the walk missed {required}: {rels:?}"
        );
    }

    let mut offenders = Vec::new();
    for (rel, file) in &sources {
        let (names, literals) = names_and_literals(file);
        for forbidden in FORBIDDEN_NAMES {
            if names.contains(*forbidden) {
                offenders.push(format!("{rel} names `{forbidden}`"));
            }
        }
        for forbidden in FORBIDDEN_LITERALS {
            if literals.contains(*forbidden) {
                offenders.push(format!("{rel} reads \"{forbidden}\""));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the MCP edge reaches for a database handle — every tool relays to the API, which owns the \
         database; the edge boots from `McpServerConfig` and opens no connection:\n  {}",
        offenders.join("\n  ")
    );
}

/// FAILS IF: the server's `[dependencies]` gain a database driver or the API crate — a pool
/// constructible by a path the source walk does not parse.
#[test]
fn the_edge_manifest_names_no_database_driver() {
    let raw = std::fs::read_to_string(manifest("Cargo.toml")).expect("read Cargo.toml");
    let cargo: toml::Value = toml::from_str(&raw).expect("Cargo.toml parses");
    let deps = cargo
        .get("dependencies")
        .and_then(|d| d.as_table())
        .expect("a [dependencies] table");
    let offenders: Vec<&str> = ["sqlx", "temper-api", "temper-substrate"]
        .into_iter()
        .filter(|name| deps.contains_key(*name))
        .collect();
    assert!(
        offenders.is_empty(),
        "the MCP edge depends on a database crate: {offenders:?}"
    );
}

/// The detector reads what it must: a split field chain, a `use` tree, a literal inside a macro,
/// and not a comment.
#[test]
fn the_detector_sees_split_chains_use_trees_and_macro_literals() {
    let file: syn::File = syn::parse_str(
        r#"
        use sqlx::postgres::PgPoolOptions;
        // DATABASE_URL in a comment is not a read.
        struct Held { kept: u8 }
        fn boot(s: S, database_url: String) {
            let p = s
                .api_state
                .pool;
            let url = std::env::var("DATABASE_URL");
            tracing::info!(target: "boot", "{}", ("DATABASE_URL"));
        }
        #[cfg(test)]
        mod tests { use temper_services::state::AppState; }
        "#,
    )
    .expect("parses");
    let (names, literals) = names_and_literals(&file);
    for n in [
        "sqlx",
        "PgPoolOptions",
        "api_state",
        "pool",
        "kept",
        "database_url",
    ] {
        assert!(names.contains(n), "{n} unseen: {names:?}");
    }
    assert!(!names.contains("AppState"), "a test-only item was read");
    assert!(literals.contains("DATABASE_URL"), "{literals:?}");

    let comment_only: syn::File = syn::parse_str("// DATABASE_URL\nfn f() {}").expect("parses");
    assert!(names_and_literals(&comment_only).1.is_empty());
}
