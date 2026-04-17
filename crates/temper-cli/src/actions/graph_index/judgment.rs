//! LLM judgment — asks the agent to judge cluster quality with `max_turns=1`.
//!
//! Builds a prompt per cluster (seed phrase + member previews with title and
//! body excerpt), calls the LLM via `Agent::run`, and returns `ConceptProposal`
//! results. Failures are appended to `.temper/graph-index-errors-{run_id}.log`
//! (one JSON line per entry).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use temper_core::frontmatter::Frontmatter;
use temper_llm::types::{Cluster, ConceptProposal};
use temper_llm::{Agent, AgentOutcome, LlmProvider, Tool};

/// Max body excerpt characters per member preview. Keeps per-member tokens
/// bounded so even large clusters fit in the per-prompt budget.
const EXCERPT_MAX_CHARS: usize = 400;

/// Aggregate cap on member-preview excerpt bytes across a single prompt. When
/// exceeded, deeper members are dropped and a `... [N more members omitted]`
/// line is emitted.
const TOTAL_PREVIEW_CAP_CHARS: usize = 6000;

const SYSTEM_PROMPT: &str = "You are analyzing a cluster of documents to determine whether they \
represent a coherent Concept — a named idea, pattern, or domain term that recurs across multiple \
documents. Respond with valid JSON only, matching the schema shown in the user prompt.";

/// Run LLM judgment on `clusters` (single turn per cluster), returning the parsed
/// concept proposals. Failures (LLM errors, max-turns, parse failures) are logged
/// to `.temper/graph-index-errors-{run_id}.log` under `vault_root`.
pub async fn judge_clusters(
    clusters: &[Cluster],
    provider: Arc<dyn LlmProvider>,
    run_id: &str,
    vault_root: &Path,
) -> Vec<ConceptProposal> {
    let mut proposals: Vec<ConceptProposal> = Vec::new();
    let mut error_log_path: Option<PathBuf> = None;

    for cluster in clusters {
        let user = build_judgment_prompt(cluster, vault_root);
        let mut agent = Agent::new(Arc::clone(&provider), Vec::<Tool<()>>::new(), 1usize, ());

        match agent.run(SYSTEM_PROMPT, &user).await {
            Ok(AgentOutcome::Final { content }) => match parse_concept_proposal(&content) {
                Some(proposal) => proposals.push(proposal),
                None => log_error(
                    &mut error_log_path,
                    vault_root,
                    run_id,
                    &cluster.seed.phrase,
                    "parse_failed",
                    &content.to_string(),
                ),
            },
            Ok(AgentOutcome::MaxTurns) => log_error(
                &mut error_log_path,
                vault_root,
                run_id,
                &cluster.seed.phrase,
                "max_turns",
                "",
            ),
            Err(e) => log_error(
                &mut error_log_path,
                vault_root,
                run_id,
                &cluster.seed.phrase,
                "llm_error",
                &e.to_string(),
            ),
        }
    }

    proposals
}

/// Build the user prompt for a single cluster.
///
/// Renders a per-member block containing `path`, `title`, and a body excerpt
/// (up to `EXCERPT_MAX_CHARS`). Members beyond `TOTAL_PREVIEW_CAP_CHARS` of
/// aggregate body-excerpt content are dropped and replaced with a single
/// `... [N more members omitted]` line — this keeps the prompt bounded even
/// for unusually large clusters.
///
/// Unreadable files (missing, malformed frontmatter) log a `warn!` and are
/// rendered with empty title/excerpt — the path is still present so the LLM
/// can still reason about membership.
fn build_judgment_prompt(cluster: &Cluster, vault_root: &Path) -> String {
    let total = cluster.member_ids.len();

    let mut member_blocks: Vec<String> = Vec::new();
    let mut budget_used: usize = 0;
    let mut omitted: usize = 0;

    for (i, rel_path) in cluster.member_ids.iter().enumerate() {
        let preview = load_member_preview(vault_root, rel_path);

        // Stop adding full previews once the budget would be exceeded. We
        // always include at least one member's preview regardless of size —
        // a single oversized preview is better than an empty prompt body.
        if budget_used + preview.excerpt.len() > TOTAL_PREVIEW_CAP_CHARS
            && !member_blocks.is_empty()
        {
            omitted = total - i;
            break;
        }
        budget_used += preview.excerpt.len();

        member_blocks.push(format!(
            "--- member #{num} ---\n\
             path: {path}\n\
             title: \"{title}\"\n\
             excerpt: \"{excerpt}\"",
            num = i + 1,
            path = rel_path,
            title = preview.title,
            excerpt = preview.excerpt,
        ));
    }

    if omitted > 0 {
        member_blocks.push(format!("... [{omitted} more members omitted]"));
    }

    let members_section = member_blocks.join("\n\n");

    format!(
        "Seed phrase: \"{seed}\"\n\n\
         Cluster members ({total} total):\n\n\
         {members_section}\n\n\
         Respond with JSON matching this shape. Echo member paths verbatim in \
         `member_edges[].target_path` — do not rewrite or re-slug them:\n\
         {{\n\
           \"is_concept\": true | false,\n\
           \"slug\": \"proposed-kebab-slug-if-true\",\n\
           \"title\": \"Human-readable title if true\",\n\
           \"body_markdown\": \"## Members\\n\\n- ...\",\n\
           \"member_edges\": [\n\
             {{ \"target_path\": \"<exact path from above>\", \"edge_type\": \"relates-to\" }}\n\
           ]\n\
         }}",
        seed = cluster.seed.phrase,
    )
}

/// Per-member preview rendered into the judgment prompt.
struct MemberPreview {
    title: String,
    excerpt: String,
}

/// Load `{vault_root}/{rel_path}`, parse its frontmatter via [`Frontmatter`],
/// and return the title plus a truncated body excerpt. Unreadable files return
/// empty fields — the prompt still names the member by path.
fn load_member_preview(vault_root: &Path, rel_path: &str) -> MemberPreview {
    let path = vault_root.join(rel_path);
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(rel_path, error = %e, "judgment preview: file unreadable");
            return MemberPreview {
                title: String::new(),
                excerpt: String::new(),
            };
        }
    };

    let (title, body) = match Frontmatter::try_from(raw.as_str()) {
        Ok(fm) => {
            let title = fm
                .value()
                .as_mapping()
                .and_then(|m| m.get(serde_yaml::Value::String("title".to_string())))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (title, fm.body().to_string())
        }
        Err(e) => {
            tracing::warn!(rel_path, error = %e, "judgment preview: frontmatter unparseable");
            (String::new(), raw)
        }
    };

    // Truncate and sanitize — newlines/quotes would break the simple
    // `title: "…"` / `excerpt: "…"` lines used in the prompt.
    let excerpt = escape_preview_text(&truncate_excerpt(body.trim(), EXCERPT_MAX_CHARS));
    let title = escape_preview_text(&title);

    MemberPreview { title, excerpt }
}

/// Truncate `text` to at most `max` characters, preferring to cut at a word
/// boundary within the final 10% of the budget.
fn truncate_excerpt(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut end = 0usize;
    for (i, (byte_idx, _)) in text.char_indices().enumerate() {
        if i == max {
            end = byte_idx;
            break;
        }
    }
    if end == 0 {
        end = text.len();
    }
    let slice = &text[..end];
    let fallback = end.saturating_sub(max / 10);
    if let Some(space_idx) = slice[fallback..].rfind(' ') {
        format!("{}...", &slice[..fallback + space_idx])
    } else {
        format!("{slice}...")
    }
}

/// Collapse newlines/whitespace and escape `"` so preview text fits on a
/// single quoted line in the prompt.
fn escape_preview_text(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
        .replace('"', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a `ConceptProposal` from the agent's final content value.
///
/// Providers return either a structured `Value::Object` (preferred) or a
/// `Value::String` wrapping JSON when the model emits raw text. Try both
/// before giving up.
fn parse_concept_proposal(content: &Value) -> Option<ConceptProposal> {
    if let Ok(p) = serde_json::from_value::<ConceptProposal>(content.clone()) {
        return Some(p);
    }
    if let Some(text) = content.as_str() {
        if let Ok(p) = serde_json::from_str::<ConceptProposal>(text) {
            return Some(p);
        }
    }
    None
}

#[derive(Debug, Serialize)]
struct ErrorEntry<'a> {
    run_id: &'a str,
    phase: &'a str,
    seed: &'a str,
    error: &'a str,
    detail: &'a str,
}

fn log_error(
    error_log_path: &mut Option<PathBuf>,
    vault_root: &Path,
    run_id: &str,
    seed: &str,
    error_type: &str,
    detail: &str,
) {
    let path = error_log_path.get_or_insert_with(|| {
        vault_root
            .join(".temper")
            .join(format!("graph-index-errors-{run_id}.log"))
    });

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let entry = ErrorEntry {
        run_id,
        phase: "llm_judgment",
        seed,
        error: error_type,
        detail,
    };
    let line = match serde_json::to_string(&entry) {
        Ok(s) => s,
        Err(_) => return,
    };

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use temper_llm::types::SeedPhrase;

    fn write_vault_file(vault_root: &Path, rel_path: &str, title: &str, body: &str) {
        let full = vault_root.join(rel_path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        let doc = format!(
            "---\n\
temper-id: \"01900000-0000-7000-8000-000000000001\"\n\
temper-type: task\n\
temper-context: temper\n\
temper-created: \"2026-01-01T00:00:00Z\"\n\
temper-owner: \"@me\"\n\
title: \"{title}\"\n\
temper-stage: backlog\n\
slug: foo\n\
---\n\
\n\
{body}\n"
        );
        fs::write(&full, doc).unwrap();
    }

    #[test]
    fn test_build_judgment_prompt_includes_title_and_excerpt_per_member() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_root = tmp.path();

        write_vault_file(
            vault_root,
            "@me/temper/task/foo.md",
            "Resource Update Accepts Stdin",
            "The temper resource update command currently only accepts content via command-line \
             arguments, making it awkward to pipe multiline markdown bodies into commands.",
        );
        write_vault_file(
            vault_root,
            "@me/temper/task/bar.md",
            "MCP Server for Agent Workflows",
            "The MCP server exposes a set of tools that let agent clients discover, search, and \
             manipulate the temper knowledge base remotely via Streamable HTTP.",
        );

        let seed = SeedPhrase::new("resource update".to_string(), 2, Vec::new());
        let cluster = Cluster::new(
            seed,
            vec![
                "@me/temper/task/foo.md".to_string(),
                "@me/temper/task/bar.md".to_string(),
            ],
            vec![Vec::new(), Vec::new()],
        );

        let prompt = build_judgment_prompt(&cluster, vault_root);

        assert!(prompt.contains("resource update"), "seed phrase: {prompt}");
        assert!(
            prompt.contains("@me/temper/task/foo.md"),
            "member #1 path: {prompt}"
        );
        assert!(
            prompt.contains("Resource Update Accepts Stdin"),
            "member #1 title: {prompt}"
        );
        assert!(
            prompt.contains("temper resource update command"),
            "member #1 body excerpt: {prompt}"
        );
        assert!(
            prompt.contains("@me/temper/task/bar.md"),
            "member #2 path: {prompt}"
        );
        assert!(
            prompt.contains("MCP Server for Agent Workflows"),
            "member #2 title: {prompt}"
        );
        assert!(
            prompt.contains("MCP server exposes"),
            "member #2 body excerpt: {prompt}"
        );
        assert!(
            prompt.contains("target_path"),
            "schema target_path: {prompt}"
        );
        assert!(
            !prompt.contains("target_slug"),
            "schema should not have target_slug: {prompt}"
        );
    }

    #[test]
    fn test_build_judgment_prompt_truncates_when_total_preview_exceeds_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_root = tmp.path();

        let long_body = "alpha beta gamma delta epsilon ".repeat(60);
        let mut member_ids = Vec::new();
        for i in 0..25 {
            let rel = format!("@me/temper/task/doc-{i:02}.md");
            write_vault_file(vault_root, &rel, &format!("Doc {i:02}"), &long_body);
            member_ids.push(rel);
        }

        let seed = SeedPhrase::new("bigcluster".to_string(), 25, Vec::new());
        let cluster = Cluster::new(seed, member_ids, vec![Vec::new(); 25]);

        let prompt = build_judgment_prompt(&cluster, vault_root);
        assert!(
            prompt.contains("more members omitted"),
            "truncation marker: {prompt}"
        );
    }
}
