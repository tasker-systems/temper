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

pub fn run(config: &Config, _verbose: bool) -> Result<()> {
    output::header("Temper Status");
    output::label("Config", crate::config::global_config_path().display());
    output::label("Vault", config.vault_root.display());
    output::blank();

    if config.contexts.is_empty() {
        output::hint("  (no contexts configured)");
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
            render_degraded(config);
            return Err(e);
        }
    };

    let client_result = build_config_store_and_client();

    match client_result {
        Ok((_cfg, _store, client)) => {
            // Fetch all visible contexts from server (includes resource_count).
            let server_contexts = rt.block_on(client.contexts().list());
            match server_contexts {
                Ok(server_ctxs) => {
                    output::header("Contexts");
                    let mut names = config.contexts.clone();
                    names.sort();
                    for ctx_name in &names {
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

                        render_context_line(ctx_name, staleness, projected, server_count);
                    }
                }
                Err(_) => {
                    // API call failed — degrade gracefully.
                    output::warning(
                        "Contexts (cloud unreachable — showing local cursor info only)",
                    );
                    render_degraded(config);
                }
            }
        }
        Err(_) => {
            // Client build failed (not authenticated, no config, etc.).
            output::warning("Contexts (cloud unreachable — showing local cursor info only)");
            render_degraded(config);
        }
    }

    Ok(())
}

/// Render one context status line.
///
/// Format (tty):
///
/// ```text
///   <ctx-name>    Fresh    [12 projected / 12 server]
///   <ctx-name>    Stale    [47 projected / 51 server]  → run `temper pull <ctx-name>`
///   <ctx-name>    —        (not projected — run `temper pull <ctx-name>`)
///   <ctx-name>    Skipped  [47 projected / ? server]
/// ```
fn render_context_line(
    name: &str,
    outcome: StalenessOutcome,
    projected: usize,
    server_count: Option<i64>,
) {
    let indicator = match outcome {
        StalenessOutcome::Fresh => "Fresh",
        StalenessOutcome::Stale => "Stale",
        StalenessOutcome::NotProjected => "—",
        StalenessOutcome::Skipped => "Skipped",
    };

    match outcome {
        StalenessOutcome::NotProjected => {
            output::plain(format!(
                "  {name:<16} {indicator:<8}  (not projected — run `temper pull {name}`)"
            ));
        }
        StalenessOutcome::Stale => {
            let counts = format_counts(projected, server_count);
            output::plain(format!(
                "  {name:<16} {indicator:<8}  {counts}  → run `temper pull {name}`"
            ));
        }
        StalenessOutcome::Fresh | StalenessOutcome::Skipped => {
            let counts = format_counts(projected, server_count);
            output::plain(format!("  {name:<16} {indicator:<8}  {counts}"));
        }
    }
}

/// Format the projected-vs-server counts bracket.
fn format_counts(projected: usize, server_count: Option<i64>) -> String {
    match server_count {
        Some(sc) => format!("[{projected} projected / {sc} server]"),
        None => format!("[{projected} projected / ? server]"),
    }
}

/// Degraded render path: no API available. Shows cursor presence only.
fn render_degraded(config: &Config) {
    let mut names = config.contexts.clone();
    names.sort();
    for ctx_name in &names {
        let cursor = read_cursor(&config.state_dir, ctx_name).ok().flatten();
        let indicator = if cursor.is_some() { "cached" } else { "—" };
        let pulled = cursor
            .map(|c| format!("  (last pulled: {})", c.pulled_at.format("%Y-%m-%d")))
            .unwrap_or_else(|| "  (not projected)".to_string());
        output::plain(format!("  {ctx_name:<16} {indicator:<8}{pulled}"));
    }
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
}
