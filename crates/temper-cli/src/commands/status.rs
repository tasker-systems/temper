//! `temper status` — per-context projection staleness report.
//!
//! Reports, for each configured context:
//!   - Staleness outcome: Fresh / Stale / NotProjected / Skipped
//!   - Projected md-file count vs server-side resource count
//!
//! All cloud API calls happen inside a single `tokio::runtime::Runtime::new()
//! .block_on()` so the synchronous `run` signature is preserved.

use std::path::Path;

use crate::actions::runtime::build_config_store_and_client;
use crate::config::Config;
use crate::error::Result;
use crate::output;
use crate::projection::{check_context_staleness, read_cursor, StalenessOutcome};

/// Structured output shape for `temper status`.
#[derive(Debug, serde::Serialize)]
pub(crate) struct StatusReport {
    pub contexts: Vec<ContextStatus>,
}

/// Per-context entry in [`StatusReport`].
#[derive(Debug, serde::Serialize)]
pub(crate) struct ContextStatus {
    pub name: String,
    /// Kebab-case staleness wire string: `fresh`, `stale`, `not-projected`, `skipped`.
    pub staleness: String,
    pub projected: u64,
    pub server: Option<u64>,
}

/// Map a [`StalenessOutcome`] to its stable kebab-case wire string.
fn staleness_str(outcome: StalenessOutcome) -> &'static str {
    match outcome {
        StalenessOutcome::Fresh => "fresh",
        StalenessOutcome::Stale => "stale",
        StalenessOutcome::NotProjected => "not-projected",
        StalenessOutcome::Skipped => "skipped",
    }
}

pub fn run(config: &Config, _verbose: bool, fmt: crate::format::OutputFormat) -> Result<()> {
    if config.contexts.is_empty() {
        let report = StatusReport { contexts: vec![] };
        let rendered = crate::format::render(&report, fmt)?;
        println!("{rendered}");
        return Ok(());
    }

    // Attempt to build a client and fetch all contexts with server counts.
    let rt_result = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Api(format!("tokio runtime: {e}")));

    let rt = match rt_result {
        Ok(rt) => rt,
        Err(e) => {
            // Cannot build a runtime — fall back to degraded report.
            output::warning("cloud unreachable — showing local cursor info only");
            let report = StatusReport {
                contexts: degraded_items(config),
            };
            if let Ok(rendered) = crate::format::render(&report, fmt) {
                println!("{rendered}");
            }
            return Err(e);
        }
    };

    let client_result = build_config_store_and_client();

    let items: Vec<ContextStatus> = match client_result {
        Ok((_cfg, _store, client)) => {
            // Fetch all visible contexts from server (includes resource_count).
            let server_contexts = rt.block_on(client.contexts().list());
            match server_contexts {
                Ok(server_ctxs) => {
                    let mut names = config.contexts.clone();
                    names.sort();
                    names
                        .iter()
                        .map(|ctx_name| {
                            let staleness = rt.block_on(check_context_staleness(
                                &client,
                                &config.state_dir,
                                ctx_name,
                            ));
                            let owner = config.owner_for_context(ctx_name);
                            let projected =
                                count_projected_md_files(&config.vault_root, &owner, ctx_name);
                            let server_count = server_ctxs
                                .iter()
                                .find(|c| c.name == *ctx_name)
                                .map(|c| c.resource_count);
                            ContextStatus {
                                name: ctx_name.clone(),
                                staleness: staleness_str(staleness).to_string(),
                                projected: projected as u64,
                                server: server_count.map(|n| n as u64),
                            }
                        })
                        .collect()
                }
                Err(_) => {
                    // API call failed — degrade gracefully.
                    output::warning("cloud unreachable — showing local cursor info only");
                    degraded_items(config)
                }
            }
        }
        Err(_) => {
            // Client build failed (not authenticated, no config, etc.).
            output::warning("cloud unreachable — showing local cursor info only");
            degraded_items(config)
        }
    };

    let report = StatusReport { contexts: items };
    let rendered = crate::format::render(&report, fmt)?;
    println!("{rendered}");

    Ok(())
}

/// Degraded item list: no API available. Uses cursor presence to infer staleness.
fn degraded_items(config: &Config) -> Vec<ContextStatus> {
    let mut names = config.contexts.clone();
    names.sort();
    names
        .iter()
        .map(|ctx_name| {
            let cursor = read_cursor(&config.state_dir, ctx_name).ok().flatten();
            let staleness = if cursor.is_some() {
                "skipped"
            } else {
                "not-projected"
            };
            let owner = config.owner_for_context(ctx_name);
            let projected = count_projected_md_files(&config.vault_root, &owner, ctx_name);
            ContextStatus {
                name: ctx_name.clone(),
                staleness: staleness.to_string(),
                projected: projected as u64,
                server: None,
            }
        })
        .collect()
}

/// Count `.md` files under `<vault_root>/<owner>/<context>/<doc_type>/*.md`
/// across every doc-type subdirectory. Returns 0 if the context directory
/// does not exist (normal for a fresh user before `temper pull`).
fn count_projected_md_files(vault_root: &Path, owner: &str, context: &str) -> usize {
    // The vault layout is <vault_root>/<owner>/<context>/<doc_type>/<slug>.md.
    // Walk every subdirectory of <context_dir> as a doc-type bucket.
    let context_dir = vault_root.join(owner).join(context);

    if !context_dir.exists() {
        return 0;
    }
    let Ok(doctype_entries) = std::fs::read_dir(&context_dir) else {
        return 0;
    };
    let mut count = 0usize;
    for doctype_entry in doctype_entries.flatten() {
        if !doctype_entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(file_entries) = std::fs::read_dir(doctype_entry.path()) else {
            continue;
        };
        for file_entry in file_entries.flatten() {
            let path = file_entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_projected_md_files_empty_when_no_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(
            count_projected_md_files(&missing, "@me", "default"),
            0,
            "non-existent vault root returns 0"
        );
    }

    #[test]
    fn count_projected_md_files_walks_doctype_subdirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        // Create <root>/@me/default/{session,task,goal}/*.md
        for doctype in &["session", "task", "goal"] {
            let d = root.join("@me/default").join(doctype);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("file1.md"), "body").unwrap();
            std::fs::write(d.join("file2.md"), "body").unwrap();
        }

        assert_eq!(
            count_projected_md_files(root, "@me", "default"),
            6,
            "should count all .md files across all doc-type subdirs"
        );
    }

    #[test]
    fn count_projected_md_files_ignores_non_md() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let session_dir = root.join("@me/ctx/session");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(session_dir.join("file.md"), "body").unwrap();
        std::fs::write(session_dir.join("file.txt"), "skip").unwrap();
        std::fs::write(session_dir.join("file.json"), "skip").unwrap();

        assert_eq!(
            count_projected_md_files(root, "@me", "ctx"),
            1,
            "only .md files counted"
        );
    }

    #[test]
    fn render_status_report_json_shape() {
        let report = StatusReport {
            contexts: vec![ContextStatus {
                name: "temper".to_string(),
                staleness: "fresh".to_string(),
                projected: 42,
                server: Some(42),
            }],
        };
        let out =
            crate::format::render(&report, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.contains("\"contexts\""), "json: {out}");
        assert!(out.contains("\"staleness\": \"fresh\""), "json: {out}");
        assert!(out.contains("\"projected\": 42"), "json: {out}");
    }

    #[test]
    fn render_status_report_toon_includes_context_name() {
        let report = StatusReport {
            contexts: vec![ContextStatus {
                name: "temper".to_string(),
                staleness: "fresh".to_string(),
                projected: 42,
                server: Some(42),
            }],
        };
        let out =
            crate::format::render(&report, crate::format::OutputFormat::Toon).expect("toon render");
        assert!(out.contains("temper"), "toon: {out}");
        assert!(out.contains("fresh"), "toon: {out}");
    }
}
