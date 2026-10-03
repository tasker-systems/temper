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

/// The tools-module functions allowed to read `api_state` — and only its deployment config,
/// never the pool: the blob door's own posture (`api_state.config.blob`) and size ceiling. Named
/// by `(module, function)`; anything else naming `api_state` fails the witness.
const API_STATE_CONFIG_READERS: &[(&str, &str)] =
    &[("blobs", "read_ceiling"), ("blobs", "blob_door_open")];

// ── Parsing ──────────────────────────────────────────────────────────────────────────────────

fn manifest(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn parse(path: &Path) -> syn::File {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    syn::parse_file(&src).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// `#[cfg(test)]` (or any `cfg` naming `test`) — test code is no tool's path.
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
/// `relay_client`). Whitespace and line breaks cannot hide a name from a syntax tree.
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
/// directly. Fails if any production item under `src/tools/` names `temper_services`, `sqlx` or a
/// `pool`, or names `api_state` outside the named config readers ([`API_STATE_CONFIG_READERS`]).
/// Parsed, not grepped: a chain rustfmt splits across lines (`svc.api_state\n    .pool`) is the
/// same field access. After teardown `temper_services` remains in this crate only at the JWT edge
/// (`middleware.rs`), the router/transport (`router.rs`), config (`config.rs`) and boot
/// (`AppState`, in `service.rs`).
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
            for forbidden in ["temper_services", "sqlx", "pool"] {
                if named.contains(forbidden) {
                    offenders.push(format!("{module}.rs names `{forbidden}`"));
                }
            }
            if named.contains("api_state") {
                let reader = match item {
                    syn::Item::Fn(f) => API_STATE_CONFIG_READERS
                        .contains(&(module.as_str(), &*f.sig.ident.to_string())),
                    _ => false,
                };
                if !reader {
                    offenders.push(format!(
                        "{module}.rs names `api_state` outside the named config readers"
                    ));
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

    // Test-only items are not production code.
    let file: syn::File = syn::parse_str(
        "#[cfg(test)] use sqlx::PgPool; #[cfg(test)] mod tests { fn t() {} } fn real() {}",
    )
    .expect("file parses");
    assert_eq!(
        production_items(&file).len(),
        1,
        "only `real` is production"
    );
}
