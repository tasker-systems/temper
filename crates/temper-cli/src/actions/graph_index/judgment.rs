//! LLM judgment — calls Agent::run with max_turns=1 to judge cluster quality.
//!
//! Builds a prompt per cluster (seed phrase + member summaries), calls the LLM via
//! Agent, and returns ConceptProposal results. Logs failures to `.temper/graph-index-errors-{run_id}.log`.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

use temper_core::types::config::GraphIndexConfig;
use temper_llm::types::{Cluster, ConceptProposal};
use temper_llm::{Agent, LlmProvider};

use crate::config::Config;

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Run LLM judgment on clusters, returning concept proposals.
pub fn judge_clusters<P: LlmProvider>(
    clusters: &[Cluster],
    provider: &P,
    config: &Config,
    graph_config: &GraphIndexConfig,
) -> Vec<ConceptProposal> {
    let run_id = Uuid::new_v4().to_string();
    let mut proposals = Vec::new();
    let mut error_log_path: Option<PathBuf> = None;

    for cluster in clusters {
        let prompt = build_judgment_prompt(cluster);

        let messages = vec![temper_llm::Message {
            role: temper_llm::provider::MessageRole::User,
            content: prompt,
        }];

        let agent = Agent::new(provider.clone(), vec![], Some(1));
        let result = agent.run(messages, None::<String>);

        match result {
            Ok(outcome) => {
                if let temper_llm::AgentOutcome::Final(response) = outcome {
                    // Parse ConceptProposal from response
                    if let Some(proposal) = parse_concept_proposal(&response.content, &cluster.seed.phrase) {
                        proposals.push(proposal);
                    } else {
                        // Log parse failure
                        let log_path = get_error_log_path(&run_id);
                        append_error(&log_path, &cluster.seed.phrase, "parse_failed", &response.content);
                    }
                }
            }
            Err(e) => {
                // Log LLM error
                let log_path = get_error_log_path(&run_id);
                append_error(&log_path, &cluster.seed.phrase, "llm_error", &e.to_string());
            }
        }
    }

    proposals
}

/// Build a judgment prompt for a cluster.
fn build_judgment_prompt(cluster: &Cluster) -> String {
    let member_list = cluster
        .member_ids
        .iter()
        .enumerate()
        .map(|(i, id)| format!("- {}: cluster member #{}", id, i + 1))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"You are analyzing a cluster of documents to determine if they represent a coherent Concept.
A Concept is a named idea, pattern, or domain term that recurs across multiple documents.

Seed phrase: "{}"

Cluster members:
{}

Existing concepts in this context (do not duplicate these):
- (none currently exist)

Respond with JSON:
{{
  "is_concept": true/false,
  "slug": "proposed-slug-if-true",
  "title": "Human-readable title if true",
  "body_markdown": "## Members\n\n- ...",
  "member_edges": [
    {{"target_slug": "...", "edge_type": "relates-to"}}
  ]
}}"#,
        cluster.seed.phrase,
        member_list
    )
}

/// Parse a ConceptProposal from LLM response content.
fn parse_concept_proposal(content: &str, _seed: &str) -> Option<ConceptProposal> {
    // Try to extract JSON from the content
    let json_str = content
        .lines()
        .filter(|l| !l.trim().starts_with("```"))
        .collect::<Vec<_>>()
        .join(" ");

    // Find JSON object boundaries
    let start = json_str.find('{')?;
    let end = json_str.rfind('}').map(|p| p + 1).unwrap_or(json_str.len());
    let json = json_str[start..end].trim();

    serde_json::from_str(json).ok()
}

fn get_error_log_path(run_id: &str) -> PathBuf {
    let vault = crate::config::load(None).ok()
        .map(|c| c.vault_root)
        .unwrap_or_else(|| std::path::PathBuf::from("~/.temper".to_string()));
    vault.join(".temper").join(format!("graph-index-errors-{}.log", run_id))
}

fn append_error(log_path: &PathBuf, seed: &str, error_type: &str, detail: &str) {
    let entry = serde_json::json!({
        "run_id": log_path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown"),
        "phase": "llm_judgment",
        "seed": seed,
        "error": error_type,
        "detail": detail,
    });
    let line = serde_json::to_string(&entry).unwrap_or_default();
    let _ = fs::write(log_path, format!("{}\n", line));
}