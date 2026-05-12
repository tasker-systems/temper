//! Integration test for `temper graph build` end-to-end pipeline.
//!
//! Builds a fixture vault in a temp dir, runs graph_build::run,
//! asserts file contents, then runs again to verify idempotency.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use temper_cli::actions::graph_build::{self, GraphBuildParams};
use temper_cli::config::Config;

fn write_file(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

fn fixture_config(tmp: &TempDir, contexts: &[&str]) -> Config {
    Config {
        vault_root: tmp.path().to_path_buf(),
        state_dir: tmp.path().join(".temper"),
        contexts: contexts.iter().map(|s| s.to_string()).collect(),
        subscriptions: Vec::new(),
        skill_output: tmp.path().join(".skill"),
        profile_slug: None,
    }
}

fn file_content(temper_ctx: &str, slug: &str, body: &str) -> String {
    format!(
        "---\n\
temper-context: {temper_ctx}\n\
temper-type: task\n\
temper-owner: '@me'\n\
temper-title: {slug}\n\
temper-slug: {slug}\n\
---\n\
{body}\n"
    )
}

#[test]
fn graph_build_resolves_mixed_references() {
    let tmp = TempDir::new().unwrap();

    let temper_task_dir = tmp.path().join("@me").join("temper").join("task");

    // Targets
    write_file(
        &temper_task_dir,
        "alpha.md",
        &file_content("temper", "alpha", "alpha body"),
    );
    write_file(
        &temper_task_dir,
        "beta.md",
        &file_content("temper", "beta", "beta body"),
    );

    // Source with wikilink + markdown link + code block that should be ignored
    let source_body = "\
# Source

See [[alpha]] and [beta](beta.md).

```
This is a code block with [[fake-ref]] that must be ignored.
```

Back to prose, another mention: [[alpha]].
";
    let source = write_file(
        &temper_task_dir,
        "source.md",
        &file_content("temper", "source", source_body),
    );

    let config = fixture_config(&tmp, &["temper"]);

    // First run: should write
    let report = graph_build::run(
        &config,
        GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
    )
    .unwrap();

    assert_eq!(report.files_walked, 3);
    assert_eq!(report.files_modified, 1);
    assert_eq!(report.references_added, 2);

    // Verify source.md has references: [alpha, beta]
    let fm = temper_core::frontmatter::Frontmatter::parse_file(&source).unwrap();
    let refs: Vec<String> = fm
        .value()
        .get("references")
        .and_then(|v| v.as_sequence())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert_eq!(refs, vec!["alpha", "beta"]);

    // Fake-ref from the code block must NOT appear
    assert!(!refs.iter().any(|r| r == "fake-ref"));

    // Second run: idempotent
    let second = graph_build::run(
        &config,
        GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
    )
    .unwrap();
    assert_eq!(second.files_modified, 0);
    assert_eq!(second.references_added, 0);
}

#[test]
fn graph_build_respects_owner_boundary() {
    use temper_core::types::vault_config::Subscription;

    let tmp = TempDir::new().unwrap();

    // @me/temper/task/shared.md
    let me_dir = tmp.path().join("@me").join("temper").join("task");
    write_file(
        &me_dir,
        "shared.md",
        &file_content("temper", "shared", "@me shared body"),
    );

    // +team-x/team-ctx/task/shared.md (same slug, different owner)
    let team_dir = tmp.path().join("+team-x").join("team-ctx").join("task");
    write_file(
        &team_dir,
        "shared.md",
        &file_content("team-ctx", "shared", "+team-x shared body"),
    );

    // @me source file tries to link to "shared"
    let source = write_file(
        &me_dir,
        "source.md",
        &file_content("temper", "source", "See [[shared]]."),
    );

    // Config: two contexts, one per owner. The "team-ctx" subscription
    // has explicit owner "+team-x" so that context maps to that owner.
    let mut config = fixture_config(&tmp, &["temper", "team-ctx"]);
    config.subscriptions = vec![Subscription {
        context: "team-ctx".to_string(),
        owner: Some("+team-x".to_string()),
        team: None,
        doc_types: None,
        auto_sync: false,
        merge_policy: temper_core::types::config::MergePolicy::Manual,
        local_paths: Vec::new(),
        repos: Vec::new(),
    }];

    let report = graph_build::run(
        &config,
        GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
    )
    .unwrap();

    // Source resolves shared → @me/shared (same owner), NOT +team-x/shared
    let fm = temper_core::frontmatter::Frontmatter::parse_file(&source).unwrap();
    let refs: Vec<String> = fm
        .value()
        .get("references")
        .and_then(|v| v.as_sequence())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert_eq!(refs, vec!["shared"]);
    assert_eq!(report.references_added, 1);
}
