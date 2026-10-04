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
//! A method counts as covered when `<method>(` appears anywhere in
//! `crates/temper-cli/src/` as a call. The guard's guaranteed direction: **a method
//! with no CLI caller and no allowlist row turns this red, naming itself.** The
//! converse is NOT guaranteed — the matcher cannot tell which sub-client a `.get(`
//! belongs to, and the CLI tree is full of non-client `.get(`/`.list(` receivers
//! (`serde_json::Value`, reqwest builders). So a homonymous method losing its one
//! genuine caller can stay green on a homonym's match. That residue is adjudicated by
//! the allowlist (its stale-entry arm reddens entries whose method no longer appears
//! uncovered — the inverse signal) and by the reviewer, not by this matcher; a tighter
//! receiver-aware matcher is the known upgrade path if that residue ever bites.
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
        // Intentional: `temper search` drives the richer `search`/`search_with_params`
        // (full semantics); `text_query` is the exact-arm convenience over it.
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
        for method in api_method_names(&src) {
            all_methods.push((file_name.to_string(), method.clone()));
            if !cli_blob.contains(&format!(".{method}(")) {
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
                "{f}:{m} — no `.{m}(` caller under crates/temper-cli/src/. Add the CLI \
                 command, or record it client-only in CLIENT_ONLY with its reason."
            )
        })
        .collect();
    assert!(
        messages.is_empty(),
        "temper-client methods with no CLI caller and no recorded reason:\n  {}",
        messages.join("\n  ")
    );
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
