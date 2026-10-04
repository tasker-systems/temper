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
    "MIGRATOR",
    "temper_api",
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

/// A `cfg` that can only hold in a test build: `cfg(test)`, or `cfg(all(…))` with a bare `test`
/// among its arguments. Nothing else is exempt — `cfg(not(test))` is production code, and so is
/// `cfg(any(test, …))` or a feature whose NAME merely contains "test" (review, 2026-10-03: a
/// token-split match had exempted all three).
fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    fn requires_test(meta: &syn::Meta) -> bool {
        match meta {
            syn::Meta::Path(p) => p.is_ident("test"),
            syn::Meta::List(l) if l.path.is_ident("all") => l
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .map(|args| args.iter().any(requires_test))
                .unwrap_or(false),
            _ => false,
        }
    }
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Meta>()
                .map(|m| requires_test(&m))
                .unwrap_or(false)
    })
}

/// Every name a file's production items reach for — path segments (in `use` trees too), field
/// and method names, fields defined and bindings named — and every string literal they contain.
fn names_and_literals(file: &syn::File) -> (BTreeSet<String>, BTreeSet<String>) {
    let (names, literals, _) = read(file);
    (names, literals)
}

/// [`names_and_literals`], plus the source the walk cannot follow (`include!`, `#[path]`).
fn read(file: &syn::File) -> (BTreeSet<String>, BTreeSet<String>, Vec<String>) {
    #[derive(Default)]
    struct V {
        names: BTreeSet<String>,
        literals: BTreeSet<String>,
        unfollowed: Vec<String>,
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
            fn walk(tokens: proc_macro2::TokenStream, v: &mut V) {
                for tt in tokens {
                    match tt {
                        proc_macro2::TokenTree::Ident(i) => {
                            v.names.insert(i.to_string());
                        }
                        proc_macro2::TokenTree::Literal(lit) => {
                            if let Ok(s) = syn::parse_str::<syn::LitStr>(&lit.to_string()) {
                                v.literals.insert(s.value());
                            }
                        }
                        proc_macro2::TokenTree::Group(g) => walk(g.stream(), v),
                        _ => {}
                    }
                }
            }
            if m.path.is_ident("include") {
                self.unfollowed
                    .push("an `include!` the gate cannot follow".to_string());
            }
            walk(m.tokens.clone(), self);
            syn::visit::visit_macro(self, m);
        }
        fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
            if m.attrs.iter().any(|a| a.path().is_ident("path")) {
                self.unfollowed.push(format!(
                    "a `#[path]` module `{}` the gate cannot follow",
                    m.ident
                ));
            }
            syn::visit::visit_item_mod(self, m);
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
    (v.names, v.literals, v.unfollowed)
}

/// The server's production modules (every `.rs` under `src/`, recursively) plus its boot.
fn edge_sources() -> Vec<(String, syn::File)> {
    fn walk(dir: &Path, out: &mut Vec<(String, syn::File)>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path
                    .strip_prefix(manifest(""))
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                out.push((rel, parse(&path)));
            }
        }
    }
    let mut out = Vec::new();
    walk(&manifest("src"), &mut out);
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
        let (names, literals, unfollowed) = read(file);
        offenders.extend(unfollowed.into_iter().map(|w| format!("{rel}: {w}")));
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

/// Every dependency a manifest declares for a non-test build, by the PACKAGE it resolves to (a
/// renamed `db = { package = "sqlx" }` is `sqlx`): `[dependencies]`, `[build-dependencies]`, and
/// both under every `[target.'cfg(…)']`.
fn runtime_dependency_packages(cargo: &toml::Value) -> BTreeSet<String> {
    let mut tables: Vec<&toml::value::Table> = Vec::new();
    for key in ["dependencies", "build-dependencies"] {
        if let Some(t) = cargo.get(key).and_then(|d| d.as_table()) {
            tables.push(t);
        }
    }
    if let Some(targets) = cargo.get("target").and_then(|t| t.as_table()) {
        for target in targets.values() {
            for key in ["dependencies", "build-dependencies"] {
                if let Some(t) = target.get(key).and_then(|d| d.as_table()) {
                    tables.push(t);
                }
            }
        }
    }
    tables
        .into_iter()
        .flat_map(|t| t.iter())
        .map(|(key, spec)| {
            spec.get("package")
                .and_then(|p| p.as_str())
                .unwrap_or(key)
                .to_string()
        })
        .collect()
}

/// FAILS IF: the server's runtime dependencies (every non-dev table, by resolved package) gain a
/// database driver or the API crate — a pool constructible by a path the source walk does not
/// parse. The boot, `api/mcp.rs`, compiles in the root package beside the API's own bins, which
/// DO depend on sqlx; for it the source walk above is the barrier.
#[test]
fn the_edge_manifest_names_no_database_driver() {
    let raw = std::fs::read_to_string(manifest("Cargo.toml")).expect("read Cargo.toml");
    let cargo: toml::Value = toml::from_str(&raw).expect("Cargo.toml parses");
    let deps = runtime_dependency_packages(&cargo);
    assert!(
        deps.contains("temperkb-mcp"),
        "the walk missed the tool layer: {deps:?}"
    );
    let offenders: Vec<&str> = ["sqlx", "temper-api", "temper-substrate"]
        .into_iter()
        .filter(|name| deps.contains(*name))
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

    // A macro body's identifiers are names; source the walk cannot follow is reported.
    let file: syn::File = syn::parse_str(
        "fn f() { tokio::join!(sqlx::postgres::PgPoolOptions::new()); include!(\"x.rs\"); } \
         #[path = \"x.rs\"] mod m; #[cfg(not(test))] fn g() { let pool = 1; }",
    )
    .expect("parses");
    let (names, _, unfollowed) = read(&file);
    assert!(
        names.contains("sqlx") && names.contains("PgPoolOptions"),
        "{names:?}"
    );
    assert!(
        names.contains("pool"),
        "`cfg(not(test))` is production: {names:?}"
    );
    assert_eq!(unfollowed.len(), 2, "{unfollowed:?}");

    // Renamed and target-specific dependencies resolve to their package.
    let cargo: toml::Value = toml::from_str(
        "[dependencies]\ndb = { package = \"sqlx\", version = \"0.8\" }\n\
         [target.'cfg(unix)'.build-dependencies]\ntemper-api = { path = \"x\" }\n",
    )
    .expect("toml parses");
    let deps = runtime_dependency_packages(&cargo);
    assert!(
        deps.contains("sqlx") && deps.contains("temper-api"),
        "{deps:?}"
    );

    let comment_only: syn::File = syn::parse_str("// DATABASE_URL\nfn f() {}").expect("parses");
    assert!(names_and_literals(&comment_only).1.is_empty());
}
