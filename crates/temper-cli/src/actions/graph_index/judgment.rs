//! LLM judgment — asks the agent to judge cluster quality with `max_turns=1`.
//!
//! Builds a prompt per cluster (seed phrase + members), calls the LLM via
//! `Agent::run`, and returns `ConceptProposal` results. Failures are appended
//! to `.temper/graph-index-errors-{run_id}.log` (one JSON line per entry).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use temper_llm::types::{Cluster, ConceptProposal};
use temper_llm::{Agent, AgentOutcome, LlmProvider, Tool};

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
        let user = build_judgment_prompt(cluster);
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
fn build_judgment_prompt(cluster: &Cluster) -> String {
    let member_list = cluster
        .member_ids
        .iter()
        .enumerate()
        .map(|(i, id)| format!("- {id} (cluster member #{})", i + 1))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Seed phrase: \"{}\"\n\n\
         Cluster members:\n{member_list}\n\n\
         Respond with JSON matching this shape:\n\
         {{\n\
           \"is_concept\": true | false,\n\
           \"slug\": \"proposed-kebab-slug-if-true\",\n\
           \"title\": \"Human-readable title if true\",\n\
           \"body_markdown\": \"## Members\\n\\n- ...\",\n\
           \"member_edges\": [\n\
             {{ \"target_slug\": \"...\", \"edge_type\": \"relates-to\" }}\n\
           ]\n\
         }}",
        cluster.seed.phrase,
    )
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
