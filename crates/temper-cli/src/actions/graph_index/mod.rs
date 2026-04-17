//! Graph-index module — LLM-assisted concept discovery pipeline.
//!
//! Phases:
//! 1. Seed extraction (TF-IDF) — [`seeds`]
//! 2. Cluster formation (cosine NN over the index manifest) — [`cluster`]
//! 3. LLM judgment — [`judgment`]
//! 4. Materialization — [`materialize`]
//!
//! The orchestrator [`run`] wires these phases together and is a synchronous
//! entry point; it spins up a tokio runtime internally to drive [`judgment`].

pub mod cluster;
pub mod judgment;
pub mod materialize;
pub mod seeds;

use std::sync::Arc;

use temper_core::types::config::GraphIndexConfig;
use temper_llm::LlmProvider;
use uuid::Uuid;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Parameters for a graph-index run.
#[derive(Debug, Clone)]
pub struct GraphIndexParams {
    /// Optional single-context filter. `None` means all configured contexts.
    pub context_filter: Option<String>,
    /// If true, extract seeds and form clusters but skip LLM judgment and writes.
    pub dry_run: bool,
    /// If true, include per-concept detail in the report.
    pub verbose: bool,
}

/// Final report from a graph-index run.
#[derive(Debug, Default, Clone)]
pub struct GraphIndexReport {
    pub seeds_extracted: usize,
    pub clusters_formed: usize,
    pub proposals_returned: usize,
    pub concepts_created: usize,
    pub concepts_skipped: usize,
    pub members_updated: usize,
    pub errors: usize,
    /// Only populated when [`GraphIndexParams::verbose`] is true.
    pub failed: Vec<String>,
}

/// Orchestrate the full pipeline: seeds → clusters → judgment → materialization.
///
/// Loads global config from disk, builds an LLM provider from `global.llm`, and
/// delegates to [`run_with_provider`]. Tests that need to inject a mock provider
/// (or avoid a disk read for graph-index config) should call [`run_with_provider`]
/// directly.
pub fn run(config: &Config, params: GraphIndexParams) -> Result<GraphIndexReport> {
    let global = crate::config::load_global_config()?;
    let graph_index_config = global.graph_index.clone();

    let provider = if params.dry_run {
        // No LLM calls in dry-run, so we don't need to build a real provider.
        None
    } else {
        let llm_config = temper_core::types::config::LlmConfig::load(&global.llm);
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
        let provider = rt
            .block_on(crate::llm::build_provider(&llm_config))
            .map_err(|e| TemperError::Api(format!("build llm provider: {e}")))?;
        Some(provider)
    };

    run_with_provider(config, params, provider, &graph_index_config)
}

/// Orchestrate the pipeline with an explicitly-provided LLM provider and
/// graph-index config. This is the dependency-injection seam used by
/// integration tests to substitute a [`temper_llm::MockLlmProvider`] and avoid
/// reading global config from disk.
///
/// `provider` is required for non-dry-run calls and ignored for dry-run.
pub fn run_with_provider(
    config: &Config,
    params: GraphIndexParams,
    provider: Option<Arc<dyn LlmProvider>>,
    graph_index_config: &GraphIndexConfig,
) -> Result<GraphIndexReport> {
    let vault_root = &config.vault_root;
    let temper_dir = vault_root.join(".temper");

    let manifest = cluster::load_manifest(&temper_dir).ok_or_else(|| {
        TemperError::Project(
            "no index manifest found at .temper/index.json — run `temper index` first".to_string(),
        )
    })?;

    let seeds = seeds::extract_seeds(
        vault_root,
        graph_index_config,
        params.context_filter.as_deref(),
    );
    let clusters = cluster::form_clusters(&seeds, &manifest, graph_index_config);

    let mut report = GraphIndexReport {
        seeds_extracted: seeds.len(),
        clusters_formed: clusters.len(),
        ..Default::default()
    };

    if params.dry_run {
        return Ok(report);
    }

    let provider = provider.ok_or_else(|| {
        TemperError::Api("run_with_provider: provider required for non-dry-run".to_string())
    })?;

    let run_id = Uuid::now_v7().to_string();
    let proposals = {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
        rt.block_on(judgment::judge_clusters(
            &clusters, provider, &run_id, vault_root,
        ))
    };
    report.proposals_returned = proposals.len();

    let mat =
        materialize::materialize_concepts(&proposals, config, graph_index_config, params.dry_run);
    report.concepts_created = mat.concepts_created;
    report.concepts_skipped = mat.concepts_skipped;
    report.members_updated = mat.members_updated;
    report.errors = mat.errors;
    if params.verbose {
        report.failed = mat.failed;
    }

    Ok(report)
}
