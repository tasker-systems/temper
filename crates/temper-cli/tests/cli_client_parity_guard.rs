//! The CLI↔client surface-parity drift guard — every API method on a temper-client
//! sub-client must have a CLI caller or a recorded reason it stays client-only.
//!
//! Task 01a0e2ed-a39d-74c0-925c-e5357eeb8254's third criterion: "a drift guard — the
//! cheapest check that a future client method cannot silently acquire no CLI caller."
//!
//! # Derivation, not a list
//!
//! Like `assert_every_compiled_in_doc_is_vetoed`'s grep-derived veto set, the client
//! surface here is READ FROM THE SOURCE at test time — each `pub async fn` on every
//! sub-client file under `crates/temper-client/src/` — never from a hand-maintained
//! inventory that goes stale the day a method lands. A method added to temper-client
//! without a CLI caller turns this red on its own and names itself in the failure.
//!
//! # The matcher is deliberately loose — read the guarantee precisely
//!
//! A method counts as covered only when it is called **on its own sub-client** somewhere in
//! `crates/temper-cli/src/`: through the chained accessor (`client.blobs().delete(`, whitespace
//! before a `.` collapsed so a chain split across lines matches), through a `let` binding of the
//! accessor (`let admin = client.admin(); admin.erase_resource(`), or through a parameter typed as
//! the sub-client (`admin: &AdminClient<'_>`). The accessor map is read from temper-client's
//! `lib.rs`. This is the receiver-aware upgrade the first version of this guard named as its
//! path: the loose `.<method>(` match let `BlobClient::delete` look covered by some other
//! receiver's `.delete(` (found 2026-10-05).
//!
//! What it still cannot see, stated rather than assumed away: a sub-client value passed through
//! an untyped binding (a closure parameter, a struct field) is not followed, which errs toward
//! red, and a call inside a comment or string counts, which errs toward green. Together with
//! temper-client's registry parity test (every operation in openapi.json has a client method),
//! this closes the chain from openapi.json to the CLI.
//!
//! # The allowlist is the record
//!
//! Every entry carries its reason inline. An entry without a reason is a lie the next
//! reader cannot audit; keep the reason current when the ruling changes.

use std::path::{Path, PathBuf};

/// Transport/infra files whose functions are not API methods a CLI command drives:
/// token storage, HTTP plumbing, config loading, and the OAuth login flow helpers.
/// Declared as a class with the reason, not discovered
/// by accident.
const INFRA_FILES: &[&str] = &[
    "auth.rs",       // token cache/refresh plumbing (`get_valid_token` etc.)
    "config.rs",     // cloud-config loading and client construction
    "http.rs",       // the HTTP seam itself (`send`, `send_json`, …)
    "endpoint.rs",   // endpoint validation
    "error.rs",      // the error type
    "lib.rs",        // the client struct's constructors and sub-client accessors
    "login.rs",      // the OAuth login flow (drives `temper auth login`)
    "login_page.rs", // the login flow's local success/failure pages
    "ops.rs",        // the endpoint registry: request constants, not API methods
];

/// (file, method) pairs ruled client-only, each with the reason a future reader can
/// audit. Sorted; a hit here is a RULING, not an omission.
const CLIENT_ONLY: &[(&str, &str)] = &[
    (
        "data_artifacts.rs",
        // The home-type-keyed dispatch conveniences — MCP's `home_type` is a
        // vocabulary, not a route. The CLI calls the typed arms
        // (list_shapes/list_cogmap_shapes/declare_shape/declare_cogmap_shape)
        // directly; a `--context`/`--cogmap` flag pair picking the arm IS the CLI's
        // spelling of `list_for`.
        "declare_for",
    ),
    ("data_artifacts.rs", "list_for"),
    (
        "profile.rs",
        // UI-intended: the settings page's linked-identities read. The CLI's auth
        // surface is the login/logout/status flow, not the identity inventory.
        "auth_links",
    ),
    (
        "resources.rs",
        // The finding-addressed audit arm — a routing address whose only job is to
        // refuse a path/body mismatch. The CLI surfaces the block-addressed door
        // (`resource audit-citation`), which cannot name a finding to mismatch.
        "record_citation_audit",
    ),
    (
        "search.rs",
        // `query` and `search` are conveniences over `search_with_params`, which `temper search`
        // calls: the same `POST /api/search` operation, so the CLI reaches it either way. The
        // receiver-aware matcher (2026-10-05) showed the earlier "drives `search`" was a homonym
        // match — the CLI has only ever called `search_with_params`.
        "query",
    ),
    ("search.rs", "search"),
    (
        "search.rs",
        // The exact-arm convenience over the same operation.
        "text_query",
    ),
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        panic!("read_dir {}: {e}", dir.display());
    });
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_client_api_method_has_a_cli_caller_or_a_recorded_reason() {
    // CARGO_MANIFEST_DIR is …/temper/crates/temper-cli; two ancestors up is the repo root.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf();

    // 1. Derive the client surface from the source, per sub-client file.
    let client_src = repo.join("crates/temper-client/src");
    let mut client_files = Vec::new();
    collect_rs_files(&client_src, &mut client_files);
    client_files.sort();

    // 2. Concatenate the CLI tree once — the caller search space.
    let cli_src = repo.join("crates/temper-cli/src");
    let mut cli_files = Vec::new();
    collect_rs_files(&cli_src, &mut cli_files);
    let cli_blob: String = cli_files
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())))
        .collect();

    let cli_blob = collapse_chains(&cli_blob);
    let lib_src = std::fs::read_to_string(client_src.join("lib.rs")).expect("read client lib.rs");
    let accessors = sub_client_accessors(&lib_src);

    // Every (file, method) API method on a sub-client, and the subset with no CLI caller.
    let mut all_methods: Vec<(String, String)> = Vec::new();
    let mut uncalled: Vec<(String, String)> = Vec::new();

    for path in &client_files {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("file name");
        if INFRA_FILES.contains(&file_name) {
            continue;
        }
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let module = file_name.trim_end_matches(".rs");
        let receivers = accessors
            .iter()
            .find(|a| a.module == module)
            .map(|a| receivers_of(&cli_blob, a))
            .unwrap_or_else(|| {
                panic!(
                    "{file_name} is not INFRA and TemperClient has no accessor returning its \
                     sub-client — add it to INFRA_FILES with its reason, or give it an accessor"
                )
            });
        for method in api_method_names(&src) {
            all_methods.push((file_name.to_string(), method.clone()));
            if !receivers
                .iter()
                .any(|r| cli_blob.contains(&format!("{r}.{method}(")))
            {
                uncalled.push((file_name.to_string(), method));
            }
        }
    }

    // The allowlist is not a dumping ground: an entry is LIVE only while its method
    // still exists on the client and still has no CLI caller. A method that gained a
    // caller (or was renamed away) makes its entry stale — retire it by hand.
    let stale: Vec<String> = CLIENT_ONLY
        .iter()
        .filter(|(f, m)| !uncalled.iter().any(|(uf, um)| uf == f && um == m))
        .map(|(f, m)| format!("{f}:{m}"))
        .collect();
    assert!(
        stale.is_empty(),
        "stale CLIENT_ONLY entries (their methods now have callers or no longer exist — \
         retire them): {stale:?}"
    );

    let messages: Vec<String> = uncalled
        .iter()
        .filter(|(f, m)| !CLIENT_ONLY.iter().any(|(lf, lm)| lf == f && lm == m))
        .map(|(f, m)| {
            format!(
                "{f}:{m} — no call to `{m}` on its sub-client under crates/temper-cli/src/. \
                 Add the CLI command, or record it client-only in CLIENT_ONLY with its reason."
            )
        })
        .collect();
    assert!(
        messages.is_empty(),
        "temper-client methods with no CLI caller and no recorded reason:\n  {}",
        messages.join("\n  ")
    );
}

/// One `TemperClient` accessor: `pub fn <accessor>(&self) -> <module>::<client_type><'_>`.
#[derive(Debug, PartialEq)]
struct Accessor {
    accessor: String,
    module: String,
    client_type: String,
}

/// Read every sub-client accessor out of temper-client's `lib.rs`.
fn sub_client_accessors(lib_src: &str) -> Vec<Accessor> {
    let mut out = Vec::new();
    for line in lib_src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub fn ") else {
            continue;
        };
        let Some((accessor, rest)) = rest.split_once("(&self) -> ") else {
            continue;
        };
        let Some((module, rest)) = rest.split_once("::") else {
            continue;
        };
        let Some((client_type, _)) = rest.split_once("<'_>") else {
            continue;
        };
        out.push(Accessor {
            accessor: accessor.to_string(),
            module: module.to_string(),
            client_type: client_type.to_string(),
        });
    }
    out
}

/// Remove whitespace that precedes a `.`, so a method chain split across lines reads as one
/// (`client\n    .blobs()\n    .delete(` becomes `client.blobs().delete(`).
fn collapse_chains(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '.' {
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The receiver spellings a call on this sub-client can take in the CLI source: the chained
/// accessor (`.blobs()`), a `let` binding of it (`let admin = client.admin();` → `admin`), and a
/// parameter typed as the sub-client (`admin: &AdminClient<'_>` → `admin`). A method counts as
/// called only through one of these, so a homonym on another receiver (`serde_json::Value::get`,
/// another sub-client's `delete`) no longer keeps it green.
fn receivers_of(cli: &str, a: &Accessor) -> Vec<String> {
    let mut receivers = vec![format!(".{}()", a.accessor)];
    let call = format!(".{}();", a.accessor);
    for line in cli.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("let ") {
            if line.ends_with(&call) {
                let name: String = rest
                    .trim_start_matches("mut ")
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    receivers.push(name);
                }
            }
        }
    }
    let typed = format!("{}<", a.client_type);
    let mut rest = cli;
    while let Some(idx) = rest.find(&typed) {
        let before = &rest[..idx];
        // `name: &AdminClient<` or `name: &temper_client::admin::AdminClient<`: anchor on the
        // parameter's `: &`, never the last `:`, which a qualified path's `::` would supply; and
        // what lies between it and the type must be a path, nothing else.
        if let Some(colon) = before.rfind(": &") {
            let head = before[..colon].trim_end();
            let tail_ok = before[colon + 3..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':');
            let name: String = head
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if tail_ok && !name.is_empty() && !name.chars().all(|c| c.is_ascii_digit()) {
                receivers.push(name);
            }
        }
        rest = &rest[idx + typed.len()..];
    }
    receivers.sort();
    receivers.dedup();
    receivers
}

/// Extract the `pub async fn <name>(` method names from one client source file, via
/// string scanning (no regex dependency). Multi-line signatures are fine — only the
/// name is read.
fn api_method_names(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = src;
    while let Some(idx) = rest.find("pub async fn ") {
        rest = &rest[idx + "pub async fn ".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            names.push(name);
        }
    }
    names
}

#[test]
fn the_scanner_reads_multi_line_signatures_and_skips_non_matches() {
    let src = r#"
        impl Blobs {
            pub async fn progress(
                &self,
                upload_id: Uuid,
            ) -> Result<BlobUploadProgress> {
            }
            fn helper(&self) {}
            pub async fn relations(&self, blob_id: Uuid) -> Result<Vec<BlobRelationRow>> {}
        }
    "#;
    assert_eq!(api_method_names(src), vec!["progress", "relations"]);
}

fn blobs_accessor() -> Accessor {
    Accessor {
        accessor: "blobs".to_string(),
        module: "blobs".to_string(),
        client_type: "BlobClient".to_string(),
    }
}

fn is_called(cli: &str, method: &str) -> bool {
    let cli = collapse_chains(cli);
    receivers_of(&cli, &blobs_accessor())
        .iter()
        .any(|r| cli.contains(&format!("{r}.{method}(")))
}

#[test]
fn the_accessor_map_reads_lib_rs() {
    let lib = "    pub fn blobs(&self) -> blobs::BlobClient<'_> {\n    pub fn new(x: u8) -> Self {";
    assert_eq!(sub_client_accessors(lib), vec![blobs_accessor()]);
}

/// The defect this matcher exists for: `BlobClient::delete` looked covered by another
/// receiver's `.delete(`. FAILS IF the matcher goes back to a bare `.<method>(` search.
#[test]
fn a_homonym_on_another_receiver_does_not_count() {
    assert!(!is_called("client.contexts().delete(id).await", "delete"));
    assert!(!is_called("map.delete(&key);", "delete"));
}

#[test]
fn a_chain_split_across_lines_counts() {
    assert!(is_called(
        "client\n    .blobs()\n    .delete(blob, &act)",
        "delete"
    ));
}

#[test]
fn a_let_bound_sub_client_counts() {
    assert!(is_called(
        "let b = client.blobs();\nb.delete(blob, &act)",
        "delete"
    ));
}

#[test]
fn a_parameter_typed_as_the_sub_client_counts() {
    assert!(is_called(
        "fn go(b: &temper_client::blobs::BlobClient<'_>) { b.delete(x, &a) }",
        "delete"
    ));
}
