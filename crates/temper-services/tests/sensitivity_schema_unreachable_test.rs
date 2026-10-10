//! No application code names the `sensitivity` schema (sensitivity-sweep spec witness 12, Q17).
//!
//! The findings store is a cross-tenant enumeration oracle (spec F5). On this deployment one role
//! migrates and serves, so it owns the schema and no grant can keep it out (Q17). This gate is
//! therefore the whole control, not a backstop. Reaching the store is a SQL function's job.
//!
//! The scanned set is every `src/` tree of every crate and every package, plus the root `api/` tree
//! of deployed entrypoints, derived by walking, never listed (the
//! `reblock_op_is_reachable_only_through_the_gated_write_paths` precedent): any of them can hold the
//! role's credentials, through a linked crate or its own client. A file a scanned source pulls in
//! with a literal `include_str!` is scanned with it. Test trees are outside the set, because these
//! witnesses read the store directly.
//!
//! The public doors that read the store for resources the caller names are held the same way: only
//! the survey services may name them (`the_pairwise_doors_are_named_only_by_the_surveys`).
//!
//! What a grep cannot see, and so is not claimed: a schema name assembled at run time
//! (`format!("{SCHEMA}.findings")`, `'sensitiv' || 'ity.findings'`), an `include_str!` whose path is
//! itself computed, and a `search_path` set by a migration rather than by application code.

use std::path::{Path, PathBuf};

const EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "mts", "cts", "js", "mjs", "cjs", "svelte", "sql",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

/// `<root>/crates/*/src` and `<root>/packages/*/src`, for every member that has one, and
/// `<root>/api`, where the deployed functions build their pools.
fn source_trees(root: &Path) -> Vec<PathBuf> {
    let mut trees = vec![root.join("api")];
    for parent in ["crates", "packages"] {
        let dir = root.join(parent);
        for member in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let src = member.expect("dir entry").path().join("src");
            if src.is_dir() {
                trees.push(src);
            }
        }
    }
    trees
}

fn source_files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "node_modules") {
                    stack.push(path);
                }
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTENSIONS.contains(&e))
            {
                out.push(path);
            }
        }
    }
    out
}

/// The byte offset just past any run of whitespace, quote characters and comments starting at `i`:
/// everything SQL lets sit between a schema name and its dot.
fn skip_separators(s: &[u8], mut i: usize) -> usize {
    loop {
        match s.get(i..) {
            Some([c, ..]) if c.is_ascii_whitespace() || matches!(c, b'"' | b'`') => i += 1,
            Some([b'/', b'*', ..]) => {
                i = match s[i + 2..].windows(2).position(|w| w == b"*/") {
                    Some(end) => i + 2 + end + 2,
                    None => return s.len(),
                }
            }
            Some([b'-', b'-', ..]) | Some([b'/', b'/', ..]) => {
                i = match s[i..].iter().position(|&c| c == b'\n') {
                    Some(end) => i + end,
                    None => return s.len(),
                }
            }
            _ => return i,
        }
    }
}

/// Every way a query can reach the schema by name: qualified, in any case, with any whitespace,
/// quoting or comment before the dot; unqualified after pointing `search_path` at it, however the
/// statement is split across lines; or behind a Unicode-escaped identifier, which hides the name
/// from any text search and so is refused outright.
fn names_the_schema(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let qualified = lower
        .match_indices("sensitivity")
        .any(|(at, word)| bytes.get(skip_separators(bytes, at + word.len())) == Some(&b'.'));
    let search_path = lower.match_indices("search_path").any(|(at, _)| {
        let end = lower.len().min(at + 200);
        lower
            .get(at..end)
            .is_some_and(|w| w.contains("sensitivity"))
    });
    let escaped = lower.contains("u&\"") || lower.contains("u&'");
    qualified || search_path || escaped
}

/// Files a source pulls in with a literal `include_str!("<path>")`, resolved beside it.
fn included_files(file: &Path, source: &str) -> Vec<PathBuf> {
    source
        .match_indices("include_str!(\"")
        .filter_map(|(at, open)| {
            let rest = &source[at + open.len()..];
            let path = &rest[..rest.find('"')?];
            let resolved = file.parent()?.join(path);
            resolved.is_file().then_some(resolved)
        })
        .collect()
}

#[test]
fn no_application_code_names_the_sensitivity_schema() {
    let root = workspace_root();
    let trees = source_trees(&root);
    for required in [
        "api",
        "crates/temper-api/src",
        "crates/temper-substrate/src",
        "packages/temper-ui/src",
    ] {
        assert!(
            trees.contains(&root.join(required)) && root.join(required).is_dir(),
            "the walk must reach {required}"
        );
    }
    let mut scanned = 0;
    let mut offenders = Vec::new();
    for tree in &trees {
        for file in source_files_under(tree) {
            scanned += 1;
            let source = std::fs::read_to_string(&file).expect("read source");
            if names_the_schema(&source) {
                offenders.push(file.display().to_string());
            }
            for included in included_files(&file, &source) {
                let text = std::fs::read_to_string(&included).unwrap_or_default();
                if names_the_schema(&text) {
                    offenders.push(format!("{} (via {})", included.display(), file.display()));
                }
            }
        }
    }
    // An empty walk would pass vacuously.
    assert!(scanned > 200, "walked only {scanned} files");
    assert!(
        offenders.is_empty(),
        "these files name the sensitivity schema; reach it through a SQL function instead: {offenders:#?}"
    );
}

/// The public doors into the sweep that take the resources to read from their caller: each is a
/// confirmation oracle over resources the caller names (resource erasure spec D10, build order 3c
/// security review; field-grain scrub S1's family flags, a per-resource oracle), so naming one is
/// reaching the store.
const PAIRWISE_DOORS: &[&str] = &[
    "resource_erasure_deriver_fingerprints",
    "block_history_scrub_flagged_blocks",
    "resource_field_scrub_family_flags",
];

/// The only callers allowed, each behind the system-admin gate, each passing its own resource and
/// plan ids.
const PAIRWISE_CALLERS: &[&str] = &[
    "crates/temper-services/src/services/resource_erasure_service.rs",
    "crates/temper-services/src/services/block_history_scrub_service.rs",
    "crates/temper-services/src/services/field_scrub_service.rs",
];

/// FAILS IF a door is named anywhere but the three survey services, or if the walk stops
/// reaching any of them (an empty match would pass vacuously).
#[test]
fn the_pairwise_doors_are_named_only_by_the_surveys() {
    let root = workspace_root();
    let mut callers = Vec::new();
    for tree in source_trees(&root) {
        for file in source_files_under(&tree) {
            let source = std::fs::read_to_string(&file).expect("read source");
            if PAIRWISE_DOORS.iter().any(|door| source.contains(door)) {
                let relative = file.strip_prefix(&root).expect("under the root");
                callers.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    callers.sort();
    let mut allowed: Vec<String> = PAIRWISE_CALLERS.iter().map(|c| c.to_string()).collect();
    allowed.sort();
    assert_eq!(
        callers, allowed,
        "only the three survey services may name a door into the sensitivity sweep"
    );
}

/// The detector bites on each spelling it claims to catch, and not on the bare word, which names
/// the persona and the dispatch type legitimately.
#[test]
fn the_detector_catches_every_spelling() {
    for hit in [
        "SELECT * FROM sensitivity.findings",
        "select 1 from SENSITIVITY.findings",
        r#"FROM "sensitivity"."findings""#,
        r#"FROM "sensitivity" ."findings""#,
        "FROM sensitivity . findings",
        "FROM sensitivity\n    .findings",
        "FROM sensitivity/**/.findings",
        "FROM sensitivity /* the store */ .findings",
        "FROM sensitivity -- the store\n .findings",
        r#"FROM U&"sensitivit\0079".findings"#,
        "SET search_path = sensitivity, public",
        "sql`set search_path to 'sensitivity'`",
        "SET search_path\n    TO public,\n       sensitivity",
    ] {
        assert!(names_the_schema(hit), "{hit:?}");
    }
    for miss in [
        "Persona::Sensitivity",
        "persona = 'sensitivity'",
        "sensitivity-sweep",
        "SET search_path = public",
        "the sensitivity persona",
    ] {
        assert!(!names_the_schema(miss), "{miss:?}");
    }
}

/// A source that keeps its SQL in another file is scanned through it.
#[test]
fn an_included_file_is_scanned_with_its_includer() {
    let dir = std::env::temp_dir().join(format!("sensitivity-gate-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sql")).unwrap();
    std::fs::write(dir.join("sql/q.sql"), "SELECT 1 FROM sensitivity.findings").unwrap();
    let includer = dir.join("lib.rs");
    let found = included_files(&includer, r#"const Q: &str = include_str!("sql/q.sql");"#);
    assert_eq!(found, vec![dir.join("sql/q.sql")]);
    assert!(names_the_schema(
        &std::fs::read_to_string(&found[0]).unwrap()
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}
