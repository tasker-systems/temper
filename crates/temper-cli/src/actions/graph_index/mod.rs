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

use std::fs;
use std::path::Path;
use std::sync::Arc;

use temper_core::types::config::GraphIndexConfig;
use temper_llm::LlmProvider;
use uuid::Uuid;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Doc types whose presence marks a context as "active" for the graph-index
/// pipeline. Mirrors `seeds::ENTITY_DOC_TYPES` and `graph_build::ENTITY_DOC_TYPES`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

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
    /// Seed phrases in descending score order. Only populated when
    /// [`GraphIndexParams::verbose`] is true.
    pub seeds_preview: Vec<String>,
    /// Cluster summaries in formation order. Only populated when
    /// [`GraphIndexParams::verbose`] is true.
    pub clusters_preview: Vec<ClusterSummary>,
}

/// A compact, user-facing summary of a single cluster — just enough to inspect
/// what a seed picked up without echoing the full embedding vectors.
#[derive(Debug, Clone)]
pub struct ClusterSummary {
    pub seed: String,
    pub member_count: usize,
    pub top_members: Vec<String>,
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
/// When `params.context_filter` is `Some(ctx)`, a single pass runs against
/// that context. When `None`, the vault is scanned for every context under
/// `@me/` that contains at least one entity-doctype file, and each such
/// context is processed in its own isolated pass — seeds, clusters,
/// judgment, and materialization all stay within a single context. This is
/// the boundary that makes Concepts context-scoped: a member in
/// `@me/tasker/…` can never end up with a `relates_to` edge to a concept in
/// `@me/temper/concept/`.
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

    let contexts = match params.context_filter.as_deref() {
        Some(ctx) => {
            validate_context_is_active(vault_root, ctx)?;
            vec![ctx.to_string()]
        }
        None => discover_active_contexts(vault_root),
    };

    let mut aggregate = GraphIndexReport::default();

    for context in &contexts {
        let per_context = run_single_context(
            config,
            &params,
            provider.as_ref().cloned(),
            graph_index_config,
            &manifest,
            context,
        )?;
        merge_reports(&mut aggregate, per_context);
    }

    Ok(aggregate)
}

/// Run the full pipeline (seeds → clusters → judgment → materialize) for a
/// single context. Called once per context by [`run_with_provider`].
fn run_single_context(
    config: &Config,
    params: &GraphIndexParams,
    provider: Option<Arc<dyn LlmProvider>>,
    graph_index_config: &GraphIndexConfig,
    manifest: &cluster::IndexManifestView,
    context: &str,
) -> Result<GraphIndexReport> {
    let vault_root = &config.vault_root;

    let seeds = seeds::extract_seeds(vault_root, graph_index_config, Some(context));
    let context_manifest = cluster::filter_manifest_to_context(manifest, context);
    let clusters = cluster::form_clusters(&seeds, &context_manifest, graph_index_config);

    let mut report = GraphIndexReport {
        seeds_extracted: seeds.len(),
        clusters_formed: clusters.len(),
        ..Default::default()
    };

    if params.verbose {
        report.seeds_preview = seeds.iter().map(|s| s.phrase.clone()).collect();
        report.clusters_preview = clusters
            .iter()
            .map(|c| ClusterSummary {
                seed: c.seed.phrase.clone(),
                member_count: c.member_ids.len(),
                top_members: c.member_ids.iter().take(5).cloned().collect(),
            })
            .collect();
    }

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

    let mat = materialize::materialize_concepts(
        &proposals,
        config,
        graph_index_config,
        context,
        params.dry_run,
    );
    report.concepts_created = mat.concepts_created;
    report.concepts_skipped = mat.concepts_skipped;
    report.members_updated = mat.members_updated;
    report.errors = mat.errors;
    if params.verbose {
        report.failed = mat.failed;
    }

    Ok(report)
}

/// List the contexts under `{vault_root}/@me/` that have at least one file
/// in one of the entity doctype directories. Contexts with no content are
/// skipped — no point creating an empty concept directory for a stub context.
fn discover_active_contexts(vault_root: &Path) -> Vec<String> {
    let owner_root = vault_root.join("@me");
    let Ok(entries) = fs::read_dir(&owner_root) else {
        return Vec::new();
    };

    let mut contexts: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str())?.to_string();
            if !path.is_dir() || name.starts_with('.') {
                return None;
            }
            if context_has_entity_files(&path) {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    contexts.sort();
    contexts
}

/// True iff `{context_root}/{doc_type}/` exists for any entity doctype and
/// contains at least one `*.md` file.
fn context_has_entity_files(context_root: &Path) -> bool {
    for doc_type in ENTITY_DOC_TYPES {
        let type_dir = context_root.join(doc_type);
        let Ok(entries) = fs::read_dir(&type_dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                return true;
            }
        }
    }
    false
}

/// Combine a per-context report into the aggregate that crosses all contexts.
fn merge_reports(acc: &mut GraphIndexReport, next: GraphIndexReport) {
    acc.seeds_extracted += next.seeds_extracted;
    acc.clusters_formed += next.clusters_formed;
    acc.proposals_returned += next.proposals_returned;
    acc.concepts_created += next.concepts_created;
    acc.concepts_skipped += next.concepts_skipped;
    acc.members_updated += next.members_updated;
    acc.errors += next.errors;
    acc.failed.extend(next.failed);
    acc.seeds_preview.extend(next.seeds_preview);
    acc.clusters_preview.extend(next.clusters_preview);
}

/// Reject a `--context X` invocation when `X` is not an active context — i.e.,
/// has no files under `@me/X/{entity doctype}/`. Prevents the pipeline from
/// silently producing zero output, and surfaces the list of active contexts in
/// the error so the user can self-correct.
fn validate_context_is_active(vault_root: &Path, ctx: &str) -> Result<()> {
    let active = discover_active_contexts(vault_root);
    if active.iter().any(|c| c == ctx) {
        return Ok(());
    }
    let msg = if active.is_empty() {
        format!(
            "context '{ctx}' has no entity files under {}/@me/{ctx}/ — no active contexts found in the vault",
            vault_root.display()
        )
    } else {
        format!(
            "context '{ctx}' has no entity files under @me/{ctx}/ — active contexts: {}",
            active.join(", ")
        )
    };
    Err(TemperError::Config(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_entity_file(vault_root: &Path, context: &str, doc_type: &str, name: &str) {
        let dir = vault_root.join("@me").join(context).join(doc_type);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), "---\n---\n").unwrap();
    }

    #[test]
    fn test_validate_context_is_active_accepts_active_context() {
        let tmp = tempfile::tempdir().unwrap();
        write_entity_file(tmp.path(), "temper", "task", "t1.md");

        validate_context_is_active(tmp.path(), "temper").expect("active context should validate");
    }

    #[test]
    fn test_validate_context_is_active_rejects_unknown_with_active_list() {
        let tmp = tempfile::tempdir().unwrap();
        write_entity_file(tmp.path(), "temper", "task", "t1.md");
        write_entity_file(tmp.path(), "tasker", "goal", "g1.md");

        let err = validate_context_is_active(tmp.path(), "storyteller")
            .expect_err("unknown context must error");
        let msg = err.to_string();
        assert!(
            msg.contains("storyteller"),
            "error names the bad context: {msg}"
        );
        assert!(msg.contains("tasker"), "error lists active contexts: {msg}");
        assert!(msg.contains("temper"), "error lists active contexts: {msg}");
    }

    #[test]
    fn test_validate_context_is_active_rejects_context_without_entity_files() {
        let tmp = tempfile::tempdir().unwrap();
        // Context directory exists but is empty — no entity files.
        fs::create_dir_all(tmp.path().join("@me").join("stub")).unwrap();
        write_entity_file(tmp.path(), "temper", "task", "t1.md");

        let err = validate_context_is_active(tmp.path(), "stub")
            .expect_err("context without entity files must error");
        assert!(err.to_string().contains("stub"));
    }

    #[test]
    fn test_validate_context_is_active_error_when_no_contexts() {
        let tmp = tempfile::tempdir().unwrap();
        // No @me directory at all.
        let err =
            validate_context_is_active(tmp.path(), "temper").expect_err("missing @me should error");
        let msg = err.to_string();
        assert!(
            msg.contains("no active contexts"),
            "empty-vault message: {msg}"
        );
    }

    /// Wiring check: `run_with_provider` must fail fast when `--context` names
    /// an inactive context, before it spends any work on seeds/clusters.
    #[test]
    fn test_run_with_provider_rejects_unknown_context_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_root = tmp.path();
        write_entity_file(vault_root, "temper", "task", "t1.md");

        // Minimal manifest so we pass the manifest-load gate and reach the
        // context validation.
        let temper_dir = vault_root.join(".temper");
        fs::create_dir_all(&temper_dir).unwrap();
        fs::write(temper_dir.join("index.json"), r#"{"files":[]}"#).unwrap();

        let config = Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: temper_dir,
            contexts: vec!["temper".to_string()],
            subscriptions: Vec::new(),
            skill_output: vault_root.join(".skill"),
        };
        let graph_config = GraphIndexConfig::default();

        let err = run_with_provider(
            &config,
            GraphIndexParams {
                context_filter: Some("typo-ctx".to_string()),
                dry_run: true,
                verbose: false,
            },
            None,
            &graph_config,
        )
        .expect_err("invalid context must propagate as an error");
        let msg = err.to_string();
        assert!(msg.contains("typo-ctx"), "error names bad context: {msg}");
        assert!(
            msg.contains("temper"),
            "error lists the real context: {msg}"
        );
    }
}
