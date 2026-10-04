//! The network door's source gates — checks over this crate's own source, because the router's
//! `Tool` entries carry only a description, a schema and a function pointer: there is no body to
//! inspect at runtime.
//!
//! The tools modules are the unit the extracted tool crate will inherit, so the gates read them,
//! parsed with `syn`: real function bodies, real `#[cfg(test)]` attributes, real call and `.await`
//! chains. (Teardown first shipped a hand-written lexer here; review found it could be fooled by
//! a `;` inside an array type, a binding after a guard block, or an `.await` in a `let … else`,
//! so it was replaced before merge.)
//!
//! - [`every_tool_method_relays_or_is_allowlisted_pure`] — ruling (a), 2026-10-02.
//! - [`no_tool_module_binds_to_the_database_or_a_service`] — the `teardown-completes` witness.
//! - [`no_module_in_the_tool_layer_holds_a_pool_or_server_state`] and
//!   [`the_tool_layer_manifest_names_no_database_or_services_crate`] — least privilege: the crate
//!   holds no pool and no server state anywhere, not only in its tools (the follow-up to the
//!   teardown's security review, finding 3). The deployed edge's own no-database gate lives in
//!   `temper-mcp-server/tests/no_database_test.rs`.
//!
//! These are tripwires over the source, not controls: they catch the ordinary ways a tool could
//! regain a direct path (a pool read, a services call, a tool that never forwards), and their
//! limits are named where they bite.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::visit::Visit;

/// The tools that cross NO door: pure compute over compile-time data, holding no binding to
/// forward to. A NAMED list, not a pattern — an entry is a recorded decision (ruling (a),
/// 2026-10-02: `describe_schema` is static product vocabulary, never tenant data). An entry is
/// held to "pure": nothing it reaches builds a relay or names `api_state` / `temper_services`.
const PURE_COMPUTE_TOOLS: &[&str] = &["describe_schema"];

/// What the tool layer must never name, anywhere in its production source: the database
/// (`sqlx`, a `PgPool`, its builder, a `pool` field), the services crate, and the server state
/// that carried a pool (`AppState`, `ApiConfig`, the `api_state` field the service held until
/// the least-privilege follow-up). The host hands this crate plain values instead (`host.rs`).
const SERVER_STATE_NAMES: &[&str] = &[
    "sqlx",
    "PgPool",
    "PgPoolOptions",
    "pool",
    "MIGRATOR",
    "temper_services",
    "temper_api",
    "AppState",
    "ApiConfig",
    "api_state",
];

// ── Parsing ──────────────────────────────────────────────────────────────────────────────────

fn manifest(rel: &str) -> std::path::PathBuf {
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

/// The items of a file that are not test-only.
fn production_items(file: &syn::File) -> Vec<&syn::Item> {
    file.items
        .iter()
        .filter(|item| {
            let attrs = match item {
                syn::Item::Fn(i) => &i.attrs,
                syn::Item::Mod(i) => &i.attrs,
                syn::Item::Use(i) => &i.attrs,
                syn::Item::Impl(i) => &i.attrs,
                syn::Item::Const(i) => &i.attrs,
                syn::Item::Static(i) => &i.attrs,
                syn::Item::Struct(i) => &i.attrs,
                syn::Item::Enum(i) => &i.attrs,
                _ => return true,
            };
            !is_test_only(attrs)
        })
        .collect()
}

/// Every module under `src/tools/` → its production free functions, by name. A name defined
/// twice in one module is refused, so a body can never be shadowed out of the walk.
fn tool_modules() -> BTreeMap<String, BTreeMap<String, syn::ItemFn>> {
    let mut modules = BTreeMap::new();
    for entry in std::fs::read_dir(manifest("src/tools")).expect("read src/tools") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let module = path.file_stem().unwrap().to_string_lossy().to_string();
        let file = parse(&path);
        let mut fns = BTreeMap::new();
        for item in production_items(&file) {
            if let syn::Item::Fn(f) = item {
                let name = f.sig.ident.to_string();
                assert!(
                    fns.insert(name.clone(), f.clone()).is_none(),
                    "{module}.rs defines `{name}` twice — the gate's walk is by name"
                );
            }
        }
        modules.insert(module, fns);
    }
    assert!(
        modules.len() >= 10,
        "read only {} tools modules — the walk has broken",
        modules.len()
    );
    modules
}

/// The `#[tool]` methods of the `#[tool_router]` impl in `service.rs`: `(name, method)`.
fn tool_methods() -> Vec<(String, syn::ImplItemFn)> {
    let file = parse(&manifest("src/service.rs"));
    let router = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Impl(i) if i.attrs.iter().any(|a| a.path().is_ident("tool_router")) => {
                Some(i)
            }
            _ => None,
        })
        .expect("the #[tool_router] impl");
    let methods: Vec<_> = router
        .items
        .iter()
        .filter_map(|item| match item {
            syn::ImplItem::Fn(f) if f.attrs.iter().any(|a| a.path().is_ident("tool")) => {
                Some((f.sig.ident.to_string(), f.clone()))
            }
            _ => None,
        })
        .collect();
    assert!(
        methods.len() >= 20,
        "found only {} #[tool] methods — the gate would be checking almost nothing",
        methods.len()
    );
    methods
}

// ── What a body does ────────────────────────────────────────────────────────────────────────

/// Walk a receiver chain back toward its root — through method calls, `?`, `.await`, field
/// access and parentheses — asking `hit` of each link. The chain is the expression an `.await`
/// (or a `let`'s initializer) is actually applied to, so an `.await` elsewhere in the statement
/// never counts.
fn chain_any(mut expr: &syn::Expr, hit: &dyn Fn(&syn::Expr) -> bool) -> bool {
    loop {
        if hit(expr) {
            return true;
        }
        expr = match expr {
            syn::Expr::MethodCall(m) => &m.receiver,
            syn::Expr::Try(t) => &t.expr,
            syn::Expr::Await(a) => &a.base,
            syn::Expr::Field(f) => &f.base,
            syn::Expr::Paren(p) => &p.expr,
            syn::Expr::Reference(r) => &r.expr,
            _ => return false,
        };
    }
}

fn is_relay_client_call(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::MethodCall(m) if m.method == "relay_client")
}

fn root_ident(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()),
        _ => None,
    }
}

/// Does `body` SEND a relay — not merely build one? A send is an `.await` whose receiver chain
/// reaches a `relay_client(..)` call (the fluent `svc.relay_client(parts)?.x().y(..).await`),
/// reaches a local bound by `let <name> = <a chain containing relay_client(..)>` (a client built
/// once, awaited later), or is a call handed that local as an argument (`helper(&client).await`). A client built and dropped, a request built and never awaited, and a
/// pattern binding (`let Ok(c) = … else { … }`) are not sends.
fn sends_relay(body: &syn::Block) -> bool {
    #[derive(Default)]
    struct V {
        clients: BTreeSet<String>,
        awaited: Vec<syn::Expr>,
    }
    impl<'a> Visit<'a> for V {
        fn visit_local(&mut self, local: &'a syn::Local) {
            if let (syn::Pat::Ident(p), Some(init)) = (&local.pat, &local.init) {
                if init.diverge.is_none() && chain_any(&init.expr, &is_relay_client_call) {
                    self.clients.insert(p.ident.to_string());
                }
            }
            syn::visit::visit_local(self, local);
        }
        fn visit_expr_await(&mut self, a: &'a syn::ExprAwait) {
            self.awaited.push((*a.base).clone());
            syn::visit::visit_expr_await(self, a);
        }
    }
    let mut v = V::default();
    v.visit_block(body);
    let is_client = |e: &syn::Expr| {
        let e = match e {
            syn::Expr::Reference(r) => &*r.expr,
            other => other,
        };
        root_ident(e).is_some_and(|i| v.clients.contains(&i))
    };
    v.awaited.iter().any(|base| {
        chain_any(base, &|e| {
            is_relay_client_call(e)
                || is_client(e)
                // A helper handed the client and awaited (`enriched_view(&client, id).await`):
                // the awaited call is where the client's requests are sent.
                || matches!(e, syn::Expr::Call(c) if c.args.iter().any(&is_client))
        })
    })
}

/// The functions a body calls by path — `name(..)` or `module::name(..)` — with the path segment
/// before the name when there is one. Method calls are not followed (no tools module defines one
/// a tool relies on to forward).
fn callees(body: &syn::Block) -> Vec<(Option<String>, String)> {
    struct V(Vec<(Option<String>, String)>);
    impl<'a> Visit<'a> for V {
        fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
            if let syn::Expr::Path(p) = &*c.func {
                let segs: Vec<String> = p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                if let Some(name) = segs.last() {
                    let qualifier = segs.len().checked_sub(2).map(|i| segs[i].clone());
                    self.0.push((qualifier, name.clone()));
                }
            }
            syn::visit::visit_expr_call(self, c);
        }
    }
    let mut v = V(Vec::new());
    v.visit_block(body);
    v.0
}

/// Every function reachable from `module::name` within the tools modules: `m::name(` in module
/// `m`, else the same module, else any module defining it (an over-approximation, named: a name
/// shared across modules is followed into each). Returned as `(module, name, fn)`.
fn reachable<'m>(
    modules: &'m BTreeMap<String, BTreeMap<String, syn::ItemFn>>,
    module: &str,
    name: &str,
) -> Vec<(String, String, &'m syn::ItemFn)> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![(module.to_string(), name.to_string())];
    let mut out = Vec::new();
    while let Some((m, f)) = stack.pop() {
        if !seen.insert((m.clone(), f.clone())) {
            continue;
        }
        let Some(item) = modules.get(&m).and_then(|fns| fns.get(&f)) else {
            continue;
        };
        for (qualifier, callee) in callees(&item.block) {
            let targets: Vec<String> = match qualifier {
                Some(q) if modules.contains_key(&q) => vec![q],
                _ if modules[&m].contains_key(&callee) => vec![m.clone()],
                _ => modules
                    .iter()
                    .filter(|(_, fns)| fns.contains_key(&callee))
                    .map(|(k, _)| k.clone())
                    .collect(),
            };
            stack.extend(targets.into_iter().map(|t| (t, callee.clone())));
        }
        out.push((m, f, item));
    }
    out
}

/// The binding names a piece of code reaches for — every path segment (`temper_services::…`,
/// `sqlx::…`, in `use` trees too), every field or method named (`api_state`, `pool`,
/// `relay_client`), every field defined and every binding named. Whitespace and line breaks
/// cannot hide a name from a syntax tree.
fn names_in<'a>(visit: impl FnOnce(&mut dyn Visit<'a>)) -> BTreeSet<String> {
    #[derive(Default)]
    struct V(BTreeSet<String>);
    impl<'a> Visit<'a> for V {
        fn visit_path_segment(&mut self, s: &'a syn::PathSegment) {
            self.0.insert(s.ident.to_string());
            syn::visit::visit_path_segment(self, s);
        }
        fn visit_use_name(&mut self, n: &'a syn::UseName) {
            self.0.insert(n.ident.to_string());
        }
        fn visit_use_path(&mut self, p: &'a syn::UsePath) {
            self.0.insert(p.ident.to_string());
            syn::visit::visit_use_path(self, p);
        }
        fn visit_member(&mut self, m: &'a syn::Member) {
            if let syn::Member::Named(i) = m {
                self.0.insert(i.to_string());
            }
        }
        fn visit_expr_method_call(&mut self, c: &'a syn::ExprMethodCall) {
            self.0.insert(c.method.to_string());
            syn::visit::visit_expr_method_call(self, c);
        }
        // A field DEFINED (`struct S { pool: P }`) and a binding NAMED (`fn f(api_state: S)`,
        // `let pool = …`) are holding the thing as surely as a field read is.
        fn visit_field(&mut self, f: &'a syn::Field) {
            if let Some(i) = &f.ident {
                self.0.insert(i.to_string());
            }
            syn::visit::visit_field(self, f);
        }
        fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
            self.0.insert(p.ident.to_string());
            syn::visit::visit_pat_ident(self, p);
        }
        // A macro body (`tokio::join!(…)`, `vec![…]`) is a token stream, not a syntax tree: every
        // identifier in it counts, so `sqlx::…` inside one is still named.
        fn visit_macro(&mut self, m: &'a syn::Macro) {
            fn idents(tokens: proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
                for tt in tokens {
                    match tt {
                        proc_macro2::TokenTree::Ident(i) => {
                            out.insert(i.to_string());
                        }
                        proc_macro2::TokenTree::Group(g) => idents(g.stream(), out),
                        _ => {}
                    }
                }
            }
            idents(m.tokens.clone(), &mut self.0);
            syn::visit::visit_macro(self, m);
        }
    }
    let mut v = V::default();
    visit(&mut v);
    v.0
}

/// The `tools::<module>::<fn>(self, &parts, …)` a `#[tool]` method dispatches to.
fn dispatch_of(method: &syn::ImplItemFn) -> Option<(String, String)> {
    struct V(Option<(String, String)>);
    impl<'a> Visit<'a> for V {
        fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
            if let syn::Expr::Path(p) = &*c.func {
                let segs: Vec<String> = p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                let args: Vec<&syn::Expr> = c.args.iter().collect();
                let self_first =
                    matches!(args.first(), Some(syn::Expr::Path(a)) if a.path.is_ident("self"));
                let parts_second = matches!(
                    args.get(1),
                    Some(syn::Expr::Reference(r)) if matches!(&*r.expr, syn::Expr::Path(a) if a.path.is_ident("parts"))
                );
                if segs.len() == 3 && segs[0] == "tools" && self_first && parts_second {
                    self.0.get_or_insert((segs[1].clone(), segs[2].clone()));
                }
            }
            syn::visit::visit_expr_call(self, c);
        }
    }
    let mut v = V(None);
    v.visit_block(&method.block);
    v.0
}

// ── The gates ───────────────────────────────────────────────────────────────────────────────

/// **Every `#[tool]` relays — or sits, by name, on the pure-compute allowlist.**
///
/// The network door's teardown gate (ruling (a), 2026-10-02; it replaced the `(self, &parts`
/// shape heuristic beat 5 left). A `#[tool]` method passes only if:
///
/// 1. its own body names no `api_state`, `temper_services` or `sqlx` — no work runs in the method
///    before (or beside) the dispatch, where the API's Level 1 + 2 cannot see it;
/// 2. it dispatches `tools::<module>::<fn>(self, &parts, …)` — the parts exist on the dispatch
///    path to be forwarded; and
/// 3. from that function, following calls through the tools modules, some reachable body SENDS
///    a relay ([`sends_relay`]). Building a client is not forwarding.
///
/// — unless it is on [`PURE_COMPUTE_TOOLS`], and then it must be pure: nothing it reaches names
/// `relay_client`, `api_state` or `temper_services`.
///
/// "Actually forwards" is judged per TOOL, not per path: a tool whose requirement arm refuses
/// pre-wire (`get requires \`id\``) still relays on its other arms, and the parity suites pin the
/// pre-wire arms byte-exact. What this rules out is a tool with no forwarding path at all.
#[test]
fn every_tool_method_relays_or_is_allowlisted_pure() {
    let modules = tool_modules();
    let mut offenders: Vec<String> = Vec::new();

    for (method, item) in tool_methods() {
        let named = names_in(|v| v.visit_block(&item.block));
        for forbidden in ["api_state", "temper_services", "sqlx"] {
            if named.contains(forbidden) {
                offenders.push(format!(
                    "{method} (its own body names `{forbidden}` — work in the method runs beside \
                     the dispatch, unseen by the API's gate)"
                ));
            }
        }

        let Some((module, function)) = dispatch_of(&item) else {
            offenders.push(format!(
                "{method} (no network-door dispatch — a `tools::<module>::<fn>(self, &parts, …)` call)"
            ));
            continue;
        };
        if !modules
            .get(&module)
            .is_some_and(|fns| fns.contains_key(&function))
        {
            offenders.push(format!(
                "{method} (dispatches to `tools::{module}::{function}`, which the gate cannot find \
                 in src/tools/{module}.rs)"
            ));
            continue;
        }
        let reached = reachable(&modules, &module, &function);

        if PURE_COMPUTE_TOOLS.contains(&method.as_str()) {
            for (m, f, body) in &reached {
                let named = names_in(|v| v.visit_item_fn(body));
                for forbidden in ["relay_client", "api_state", "temper_services"] {
                    if named.contains(forbidden) {
                        offenders.push(format!(
                            "{method} (on the pure-compute allowlist, but `{m}::{f}` names \
                             `{forbidden}` — an allowlisted tool must not bind to anything)"
                        ));
                    }
                }
            }
        } else if !reached.iter().any(|(_, _, f)| sends_relay(&f.block)) {
            offenders.push(format!(
                "{method} (`tools::{module}::{function}` never sends a relay — no `.await` on a \
                 `relay_client(…)` chain among the {} function(s) it reaches; relay it, or put it \
                 on PURE_COMPUTE_TOOLS by a recorded decision)",
                reached.len()
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "these #[tool] methods do not cross the network door — every tool must relay through \
         `relay_client` (its gate runs at the API), or sit on the named pure-compute allowlist:\n  {}",
        offenders.join("\n  ")
    );
}

/// **`teardown-completes`, witnessed:** no tool module reads the database or calls a service
/// directly. Fails if any production item under `src/tools/` names `temper_services`, `sqlx`, a
/// `pool` or `api_state`. Parsed, not grepped: a chain rustfmt splits across lines
/// (`svc.api_state\n    .pool`) is the same field access. The blob door's two config readers that
/// once named `api_state` read the host's plain `BlobDoor` now, so there is no exception left.
#[test]
fn no_tool_module_binds_to_the_database_or_a_service() {
    let mut offenders = Vec::new();
    let mut read = 0;
    for entry in std::fs::read_dir(manifest("src/tools")).expect("read src/tools") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        read += 1;
        let module = path.file_stem().unwrap().to_string_lossy().to_string();
        let file = parse(&path);
        for item in production_items(&file) {
            let named = names_in(|v| v.visit_item(item));
            for forbidden in ["temper_services", "sqlx", "pool", "api_state"] {
                if named.contains(forbidden) {
                    offenders.push(format!("{module}.rs names `{forbidden}`"));
                }
            }
        }
    }
    assert!(
        read >= 10,
        "read only {read} tools modules — the walk has broken"
    );
    assert!(
        offenders.is_empty(),
        "a tool module binds past the network door — every tool relays to the API, which owns \
         the database and the services:\n  {}",
        offenders.join("\n  ")
    );
}

/// Every production `.rs` file under `src/`, recursively, except this gate's own file (whose
/// detector tests spell the forbidden names as inputs). Returned as (path relative to `src/`,
/// parsed file).
fn crate_sources() -> Vec<(String, syn::File)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, syn::File)>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                if rel != "source_gates.rs" {
                    out.push((rel, parse(&path)));
                }
            }
        }
    }
    let root = manifest("src");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out
}

/// **Least privilege, witnessed:** no module of the tool layer — the service, the tools, the
/// resources, the config, the host values — names a database pool, the services crate, or the
/// server state that once carried a pool ([`SERVER_STATE_NAMES`]). The crate has no boot of its
/// own (the deployed edge, `temper-mcp-server`, boots it), so the rule has no exception: a pool
/// reappearing anywhere here fails by file and name. Test-only items are exempt (the crate has
/// no services dependency even for tests — see the manifest gate below).
#[test]
fn no_module_in_the_tool_layer_holds_a_pool_or_server_state() {
    let sources = crate_sources();
    assert!(
        sources.len() >= 20,
        "read only {} source files — the walk has broken",
        sources.len()
    );
    assert!(
        sources.iter().any(|(rel, _)| rel == "service.rs")
            && sources.iter().any(|(rel, _)| rel == "tools/blobs.rs"),
        "the walk missed the service or the tools"
    );
    let mut offenders = Vec::new();
    for (rel, file) in &sources {
        offenders.extend(
            unfollowed_sources(file)
                .into_iter()
                .map(|w| format!("{rel}: {w}")),
        );
        for item in production_items(file) {
            let named = names_in(|v| v.visit_item(item));
            for forbidden in SERVER_STATE_NAMES {
                if named.contains(*forbidden) {
                    offenders.push(format!("{rel} names `{forbidden}`"));
                }
            }
        }
    }
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "the tool layer holds server state — it relays every act to the API, which owns the \
         database; a host hands it plain values (`host.rs`), never a pool:\n  {}",
        offenders.join("\n  ")
    );
}

/// What a production item reads from the process or says about a host: every `env::…` read
/// (`std::env::var`, `var_os`, `vars`, `vars_os`), every `option_env!`, every `env!` other than
/// cargo's compile-time `CARGO_*`, every `from_env`, every string literal naming a `TEMPER_`
/// variable, and every name of the deployed host's credential or attribution carrier — whether
/// spelled as its constant, its header text, or the surface it pins.
fn host_configuration_reads(item: &syn::Item) -> BTreeSet<String> {
    #[derive(Default)]
    struct V(BTreeSet<String>);
    fn literal(v: &mut BTreeSet<String>, text: &str) {
        let lower = text.to_ascii_lowercase();
        if text.contains("TEMPER_") {
            v.insert(format!("the variable literal {text:?}"));
        }
        for header in ["service-credential", "relayed-surface"] {
            if lower.contains(header) {
                v.insert(format!("the header literal {text:?}"));
            }
        }
    }
    fn tokens(v: &mut BTreeSet<String>, stream: proc_macro2::TokenStream) {
        let flat: Vec<proc_macro2::TokenTree> = stream.into_iter().collect();
        for (i, tt) in flat.iter().enumerate() {
            match tt {
                proc_macro2::TokenTree::Literal(l) => literal(v, &l.to_string()),
                proc_macro2::TokenTree::Group(g) => tokens(v, g.stream()),
                proc_macro2::TokenTree::Ident(id) => {
                    let id = id.to_string();
                    if HOST_NAMES.contains(&id.as_str()) {
                        v.insert(format!("`{id}`"));
                    }
                    // `env :: var` / `Surface :: Mcp` spelled inside a macro body.
                    let next = |n: usize| match flat.get(i + n) {
                        Some(proc_macro2::TokenTree::Ident(x)) => Some(x.to_string()),
                        _ => None,
                    };
                    if let Some(after) = next(3) {
                        if id == "env" && ENV_READS.contains(&after.as_str()) {
                            v.insert(format!("`env::{after}`"));
                        }
                        if id == "Surface" && after == "Mcp" {
                            v.insert("`Surface::Mcp`".to_string());
                        }
                    }
                }
                proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }
    const ENV_READS: &[&str] = &["var", "var_os", "vars", "vars_os"];
    const HOST_NAMES: &[&str] = &[
        "SERVICE_CREDENTIAL_HEADER",
        "RELAYED_SURFACE_HEADER",
        "from_env",
        "option_env",
    ];
    impl<'a> Visit<'a> for V {
        fn visit_path(&mut self, p: &'a syn::Path) {
            let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
            for w in segs.windows(2) {
                if w[0] == "env" && ENV_READS.contains(&w[1].as_str()) {
                    self.0.insert(format!("`env::{}`", w[1]));
                }
                if w[0] == "Surface" && w[1] == "Mcp" {
                    self.0.insert("`Surface::Mcp`".to_string());
                }
            }
            for s in &segs {
                if HOST_NAMES.contains(&s.as_str()) {
                    self.0.insert(format!("`{s}`"));
                }
            }
            syn::visit::visit_path(self, p);
        }
        fn visit_use_name(&mut self, n: &'a syn::UseName) {
            let id = n.ident.to_string();
            if HOST_NAMES.contains(&id.as_str()) || ENV_READS.contains(&id.as_str()) {
                self.0.insert(format!("`{id}` (imported)"));
            }
        }
        // `use …::SERVICE_CREDENTIAL_HEADER as H;` / `use std::env::var as getenv;` — the
        // rename hides every later use, so the import itself is the offence.
        fn visit_use_rename(&mut self, r: &'a syn::UseRename) {
            let id = r.ident.to_string();
            if HOST_NAMES.contains(&id.as_str()) || ENV_READS.contains(&id.as_str()) {
                self.0
                    .insert(format!("`{id}` (imported as `{}`)", r.rename));
            }
        }
        // `use …::Surface::Mcp;` / `use …::Surface::*;` / `use std::env::*;` — importing the
        // variant (or everything beside it) lets a bare `Mcp` or `var` through.
        fn visit_use_path(&mut self, p: &'a syn::UsePath) {
            let id = p.ident.to_string();
            let tree = &*p.tree;
            let names_it = |want: &str| match tree {
                syn::UseTree::Name(n) => n.ident == want,
                syn::UseTree::Rename(r) => r.ident == want,
                syn::UseTree::Glob(_) => true,
                syn::UseTree::Group(g) => g.items.iter().any(|i| match i {
                    syn::UseTree::Name(n) => n.ident == want,
                    syn::UseTree::Rename(r) => r.ident == want,
                    syn::UseTree::Glob(_) => true,
                    _ => false,
                }),
                syn::UseTree::Path(_) => false,
            };
            if id == "Surface" && names_it("Mcp") {
                self.0.insert("`Surface::Mcp` (imported)".to_string());
            }
            if id == "env" && matches!(tree, syn::UseTree::Glob(_)) {
                self.0.insert("`env::*` (imported)".to_string());
            }
            syn::visit::visit_use_path(self, p);
        }
        fn visit_ident(&mut self, i: &'a proc_macro2::Ident) {
            if i == "from_env" {
                self.0.insert("`from_env`".to_string());
            }
        }
        fn visit_lit_str(&mut self, l: &'a syn::LitStr) {
            literal(&mut self.0, &l.value());
        }
        fn visit_macro(&mut self, m: &'a syn::Macro) {
            let name = m.path.segments.last().map(|s| s.ident.to_string());
            match name.as_deref() {
                Some("option_env") => {
                    self.0.insert("`option_env!`".to_string());
                }
                Some("env") => {
                    let arg = m.tokens.to_string();
                    if !arg.trim_start_matches('"').starts_with("CARGO_") {
                        self.0.insert(format!("`env!({arg})`"));
                    }
                }
                _ => tokens(&mut self.0, m.tokens.clone()),
            }
            syn::visit::visit_macro(self, m);
        }
    }
    let mut v = V::default();
    v.visit_item(item);
    v.0
}

/// **The crate accepts configuration and never reads it; the host's credential is opaque.**
/// No production item of the tool layer reads the process environment, defines or calls a
/// `from_env`, names a `TEMPER_` variable, or names the deployed host's service credential or
/// `mcp` attribution carrier (by constant, header text, or `Surface::Mcp`). Those belong to the
/// host's seam impl (`temper-mcp-server`), which hands this crate an opaque identity per request
/// and a `RelayConfig` per process. Fails by file and name.
#[test]
fn the_tool_layer_reads_no_configuration_and_names_no_host_credential() {
    let sources = crate_sources();
    assert!(
        sources.iter().any(|(rel, _)| rel == "seam.rs")
            && sources.iter().any(|(rel, _)| rel == "service.rs"),
        "the walk missed the seam or the service"
    );
    let mut offenders = Vec::new();
    for (rel, file) in &sources {
        for item in production_items(file) {
            offenders.extend(
                host_configuration_reads(item)
                    .into_iter()
                    .map(|what| format!("{rel} names {what}")),
            );
        }
    }
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "the tool layer reads configuration or names a host's credential — a host supplies \
         both through `RelayConfig` and its `IdentitySeam`:\n  {}",
        offenders.join("\n  ")
    );
}

/// The detector itself, against each shape it must catch and the shapes it must pass — so a
/// refactor of the reader cannot quietly turn the gate into one that passes everything.
#[test]
fn the_host_configuration_detector_catches_each_shape() {
    let item = |src: &str| -> syn::Item { syn::parse_str(src).expect("item parses") };
    let caught = [
        "fn f() { let _ = std::env::var(\"X\"); }",
        "fn f() { let _ = env::var_os(\"X\"); }",
        "use std::env::vars;",
        "fn f() { let _ = option_env!(\"X\"); }",
        "fn f() { let _ = env!(\"HOME\"); }",
        "impl C { fn from_env() -> Self { todo!() } }",
        "fn f() { C::from_env(); }",
        "const K: &str = \"TEMPER_API_BASE_URL\";",
        "fn f() { tracing::info!(\"read TEMPER_MCP_SERVICE_SECRET\"); }",
        "use temper_workflow::operations::SERVICE_CREDENTIAL_HEADER;",
        "fn f() { h.insert(RELAYED_SURFACE_HEADER, v); }",
        "fn f() { h.insert(\"X-Temper-Service-Credential\", v); }",
        "fn f() { let s = Surface::Mcp; }",
        "fn f() { vec![Surface::Mcp]; }",
        "fn f() { tokio::join!(std::env::var(\"X\")); }",
        "use temper_workflow::operations::SERVICE_CREDENTIAL_HEADER as H;",
        "use std::env::var as getenv;",
        "use std::env::{var_os as v, args};",
        "use std::env::*;",
        "use temper_workflow::operations::Surface::Mcp;",
        "use temper_workflow::operations::Surface::*;",
        "use temper_workflow::operations::Surface::{CliCloud, Mcp};",
    ];
    for src in caught {
        assert!(
            !host_configuration_reads(&item(src)).is_empty(),
            "the detector missed: {src}"
        );
    }
    let passed = [
        "fn f() { let _ = env!(\"CARGO_PKG_VERSION\"); }",
        "fn f() { let s = Surface::CliCloud; let v = vars_of(x); }",
        "const K: &str = \"temper\";",
        "use temper_workflow::operations::{Surface, RELAYED_ATTRS as R};",
        "use temper_workflow::operations::Surface::CliCloud;",
    ];
    for src in passed {
        assert!(
            host_configuration_reads(&item(src)).is_empty(),
            "the detector flagged an innocent shape: {src} → {:?}",
            host_configuration_reads(&item(src))
        );
    }
}

/// Source the walk cannot follow: an `include!` (code spliced from another file) or a
/// `#[path]` module (a file outside the directory walk). Refused rather than skipped, so the
/// gate never passes code it did not read.
fn unfollowed_sources(file: &syn::File) -> Vec<String> {
    #[derive(Default)]
    struct V(Vec<String>);
    impl<'a> Visit<'a> for V {
        fn visit_macro(&mut self, m: &'a syn::Macro) {
            if m.path.is_ident("include") {
                self.0
                    .push("an `include!` the gate cannot follow".to_string());
            }
            syn::visit::visit_macro(self, m);
        }
        fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
            if m.attrs.iter().any(|a| a.path().is_ident("path")) {
                self.0.push(format!(
                    "a `#[path]` module `{}` the gate cannot follow",
                    m.ident
                ));
            }
            syn::visit::visit_item_mod(self, m);
        }
    }
    let mut v = V::default();
    v.visit_file(file);
    v.0
}

/// Every dependency a manifest declares for a non-test build, by the PACKAGE it resolves to (a
/// renamed `db = { package = "sqlx" }` is `sqlx`): `[dependencies]`, `[build-dependencies]`, and
/// both under every `[target.'cfg(…)']`.
pub(crate) fn runtime_dependency_packages(cargo: &toml::Value) -> BTreeSet<String> {
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

/// **Least privilege, structurally:** the tool layer's runtime dependencies (every non-dev table,
/// target-specific ones included, by resolved package name) name no database driver and no
/// server crate, so a pool cannot be constructed here even by a path the source gate does
/// not parse (a macro, a build script).
#[test]
fn the_tool_layer_manifest_names_no_database_or_services_crate() {
    let raw = std::fs::read_to_string(manifest("Cargo.toml")).expect("read Cargo.toml");
    let cargo: toml::Value = toml::from_str(&raw).expect("Cargo.toml parses");
    let deps = runtime_dependency_packages(&cargo);
    assert!(deps.len() >= 10, "read only {} dependencies", deps.len());
    let forbidden = [
        "sqlx",
        "temper-services",
        "temper-api",
        "temper-substrate",
        "temper-mcp-server",
    ];
    let offenders: Vec<&str> = forbidden
        .into_iter()
        .filter(|name| deps.contains(*name))
        .collect();
    assert!(
        offenders.is_empty(),
        "the tool layer depends on a database or server crate at runtime: {offenders:?}"
    );
}

/// **No temper-services dependency of any kind** — not even for a test. The `AuthzError`
/// witness, the one test that named the services' error types, lives in the deployed host
/// (`temper-mcp-server`), which may see both sides. Every dependency table is read, dev and
/// target-specific included, by resolved package name.
#[test]
fn the_tool_layer_has_no_services_dependency_even_for_tests() {
    let raw = std::fs::read_to_string(manifest("Cargo.toml")).expect("read Cargo.toml");
    let cargo: toml::Value = toml::from_str(&raw).expect("Cargo.toml parses");
    let deps = every_dependency_package(&cargo);
    assert!(
        deps.contains("syn") && deps.contains("rmcp"),
        "the walk missed a table: {deps:?}"
    );
    for forbidden in ["temper-services", "temper-api", "temper-mcp-server"] {
        assert!(
            !deps.contains(forbidden),
            "the tool layer depends on `{forbidden}` (in some table)"
        );
    }
}

/// **No utoipa in the published graph.** The MCP server neither exposes nor expresses utoipa, so
/// the two temper crates that gate it behind `web-api` are depended on without it (as
/// temperkb-client depends on them), and no runtime table names utoipa directly. Paired with
/// `cargo tree -p temper-mcp -e normal`, which the PR records showing none.
#[test]
fn the_tool_layer_pulls_no_web_api_surface() {
    let raw = std::fs::read_to_string(manifest("Cargo.toml")).expect("read Cargo.toml");
    let cargo: toml::Value = toml::from_str(&raw).expect("Cargo.toml parses");
    let deps = cargo["dependencies"].as_table().expect("[dependencies]");
    for name in ["temperkb-core", "temperkb-workflow"] {
        let features: Vec<&str> = deps[name]
            .get("features")
            .and_then(|f| f.as_array())
            .map(|f| f.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(
            !features.contains(&"web-api"),
            "`{name}` is depended on with `web-api`, which pulls utoipa into the tool layer"
        );
    }
    assert!(
        !runtime_dependency_packages(&cargo).contains("utoipa"),
        "the tool layer names utoipa directly"
    );
}

/// [`runtime_dependency_packages`] plus `[dev-dependencies]`, top-level and per target.
fn every_dependency_package(cargo: &toml::Value) -> BTreeSet<String> {
    let mut out = runtime_dependency_packages(cargo);
    let mut tables: Vec<&toml::value::Table> = Vec::new();
    if let Some(t) = cargo.get("dev-dependencies").and_then(|d| d.as_table()) {
        tables.push(t);
    }
    if let Some(targets) = cargo.get("target").and_then(|t| t.as_table()) {
        for target in targets.values() {
            if let Some(t) = target.get("dev-dependencies").and_then(|d| d.as_table()) {
                tables.push(t);
            }
        }
    }
    out.extend(
        tables
            .into_iter()
            .flat_map(|t| t.iter())
            .map(|(key, spec)| {
                spec.get("package")
                    .and_then(|p| p.as_str())
                    .unwrap_or(key)
                    .to_string()
            }),
    );
    out
}

/// The gates' own detectors, against the shapes they must accept and the shapes review showed
/// a lexer gets wrong — so a refactor of the reader cannot quietly turn the gate into one that
/// passes everything.
#[test]
fn the_relay_send_detector_tells_a_send_from_a_build() {
    let block = |src: &str| -> syn::Block { syn::parse_str(src).expect("block parses") };
    let sends = [
        // the fluent chain
        "{ let x = svc.relay_client(parts)?.contexts().shape(id, None).await.across_auth(f)?; Ok(x) }",
        // a bound client, awaited later
        "{ let client = svc.relay_client(parts)?; let r = client.blobs().read(id).await; r }",
        // a bound client after a guard block (a `}` before the `let`)
        "{ if bad { return Err(e); } let client = svc.relay_client(parts)?; client.x().await }",
        // a bound client handed to an awaited helper
        "{ let client = svc.relay_client(parts)?; let v = enriched_view(&client, id).await?; Ok(v) }",
    ];
    let builds = [
        // built and dropped
        "{ let client = svc.relay_client(parts)?; Ok(done()) }",
        // a request built, never awaited
        "{ let client = svc.relay_client(parts)?; let f = client.blobs().read(id); Ok(f) }",
        // an unrelated `.await` beside a built client
        "{ let _c = svc.relay_client(parts)?; other().await; Ok(()) }",
        // a `let … else` whose `.await` is the fallback, not the relay
        "{ let Ok(c) = svc.relay_client(parts) else { return fallback().await; }; Ok(local(c).await) }",
        // a binding NAME that merely contains the client's name
        "{ let client = svc.relay_client(parts)?; let client_count = 3; count(client_count).await }",
        // a helper awaited WITHOUT the client
        "{ let client = svc.relay_client(parts)?; let v = enriched_view(&other, id).await?; Ok(v) }",
    ];
    for src in sends {
        assert!(sends_relay(&block(src)), "a send: {src}");
    }
    for src in builds {
        assert!(!sends_relay(&block(src)), "not a send: {src}");
    }

    // A `;` inside an array type is no declaration: the body is read.
    let file: syn::File =
        syn::parse_str("fn f() -> [u8; 4] { svc.api_state.pool }").expect("file parses");
    let syn::Item::Fn(f) = &file.items[0] else {
        panic!("a fn")
    };
    let named = names_in(|v| v.visit_item_fn(f));
    assert!(
        named.contains("pool") && named.contains("api_state"),
        "{named:?}"
    );

    // A split chain is the same field access.
    let split: syn::Expr =
        syn::parse_str("svc\n    .api_state\n    .pool\n    .acquire()").expect("expr parses");
    let named = names_in(|v| v.visit_expr(&split));
    assert!(named.contains("pool"), "{named:?}");

    // A field defined and a binding named are holds too.
    let file: syn::File =
        syn::parse_str("struct S { pool: P } fn f(api_state: S) { let sqlx = 1; }")
            .expect("file parses");
    let named = names_in(|v| v.visit_file(&file));
    for n in ["pool", "api_state", "sqlx"] {
        assert!(named.contains(n), "{n} unseen: {named:?}");
    }

    // Test-only items are not production code — and only `cfg(test)` / `cfg(all(test, …))` is
    // test-only: `not(test)`, `any(test, …)` and a feature named like a test are production.
    let file: syn::File = syn::parse_str(
        "#[cfg(test)] use sqlx::PgPool; #[cfg(test)] mod tests { fn t() {} } \
         #[cfg(all(test, feature = \"x\"))] fn t2() {} fn real() {} \
         #[cfg(not(test))] fn prod_a() {} #[cfg(any(test, feature = \"x\"))] fn prod_b() {} \
         #[cfg(feature = \"test-harness\")] fn prod_c() {}",
    )
    .expect("file parses");
    assert_eq!(
        production_items(&file).len(),
        4,
        "`real`, `prod_a`, `prod_b` and `prod_c` are production"
    );

    // A macro body's identifiers are names.
    let file: syn::File =
        syn::parse_str("fn f() { tokio::join!(sqlx::postgres::PgPoolOptions::new()); }")
            .expect("file parses");
    let named = names_in(|v| v.visit_file(&file));
    assert!(
        named.contains("sqlx") && named.contains("PgPoolOptions"),
        "{named:?}"
    );

    // Source the walk cannot follow is refused, not skipped.
    let file: syn::File =
        syn::parse_str("#[path = \"elsewhere.rs\"] mod m; fn f() { include!(\"x.rs\"); }")
            .expect("file parses");
    assert_eq!(unfollowed_sources(&file).len(), 2);

    // Every runtime dependency table counts, by resolved package.
    let cargo: toml::Value = toml::from_str(
        "[dependencies]\ndb = { package = \"sqlx\", version = \"0.8\" }\n\
         [build-dependencies]\nb = \"1\"\n\
         [target.'cfg(unix)'.dependencies]\ntemper-services = { path = \"x\" }\n\
         [dev-dependencies]\nd = \"1\"\n",
    )
    .expect("toml parses");
    let deps = runtime_dependency_packages(&cargo);
    assert!(deps.contains("sqlx") && deps.contains("temper-services") && deps.contains("b"));
    assert!(!deps.contains("db") && !deps.contains("d"), "{deps:?}");
}
