//! Integration tests for the graph-index pipeline (D3h).
//!
//! Exercises `temper index` → `temper graph index` against fixture vaults.
//! Gated on `test-embed` because the pipeline's cluster phase needs
//! `temper_ingest::embed::embed_text`, which requires ONNX Runtime at
//! runtime (installed on CI's dedicated "Embed" job).

#![cfg(feature = "test-embed")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use temper_cli::actions::graph_index::{self, GraphIndexParams};
use temper_cli::actions::index::{self as index_action, IndexParams};
use temper_cli::config::Config;
use temper_core::types::config::GraphIndexConfig;
use temper_llm::{LlmProvider, MockLlmProvider, MockScenario};

// ---------------------------------------------------------------------------
// Fixture helpers — shared across all three test scenarios.
// ---------------------------------------------------------------------------

fn fixture_config(tmp: &TempDir) -> Config {
    Config {
        vault_root: tmp.path().to_path_buf(),
        state_dir: tmp.path().join(".temper"),
        contexts: vec!["temper".to_string()],
        subscriptions: Vec::new(),
        skill_output: tmp.path().join(".skill"),
    }
}

/// Build a markdown file body with the minimum valid frontmatter for the doctype.
///
/// Shape mirrors `doctor_test.rs::VALID_TASK_FM` etc. — required base fields:
/// `temper-id`, `temper-type`, `temper-context`, `temper-created`, `temper-owner`,
/// `title`, `slug`; plus doctype-specific fields.
fn task_file(id_suffix: &str, slug: &str, title: &str, body: &str) -> String {
    format!(
        "---\n\
temper-id: \"01900000-0000-7000-8000-{id_suffix}\"\n\
temper-type: task\n\
temper-context: temper\n\
temper-created: \"2026-01-01T00:00:00Z\"\n\
temper-owner: \"@me\"\n\
title: \"{title}\"\n\
temper-stage: backlog\n\
slug: {slug}\n\
---\n\
\n\
{body}\n"
    )
}

fn goal_file(id_suffix: &str, slug: &str, title: &str, body: &str) -> String {
    format!(
        "---\n\
temper-id: \"01900000-0000-7000-8000-{id_suffix}\"\n\
temper-type: goal\n\
temper-context: temper\n\
temper-created: \"2026-01-01T00:00:00Z\"\n\
temper-owner: \"@me\"\n\
title: \"{title}\"\n\
temper-status: active\n\
slug: {slug}\n\
---\n\
\n\
{body}\n"
    )
}

fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

/// Build a vault with 4 markdown files across 2 doctypes, all sharing the
/// "graph indexing pipeline" concept so TF-IDF + cosine clustering surface
/// it as a coherent concept.
fn build_fixture_vault(tmp: &TempDir) {
    let task_dir = tmp.path().join("@me").join("temper").join("task");
    let goal_dir = tmp.path().join("@me").join("temper").join("goal");

    // Repeating core phrase "graph indexing pipeline" across 4 docs —
    // each doc adds its own distinct prose so chunk embeddings differ
    // but the seed phrase still clusters them together.
    write_file(
        &task_dir,
        "design-graph-indexing.md",
        &task_file(
            "000000000101",
            "design-graph-indexing",
            "Design graph indexing",
            "# Design graph indexing pipeline\n\nWe need to design the graph indexing pipeline. \
             The graph indexing pipeline takes markdown files, extracts TF-IDF seed phrases, \
             and forms clusters. Graph indexing pipeline quality depends on embedding fidelity.",
        ),
    );
    write_file(
        &task_dir,
        "implement-graph-indexing.md",
        &task_file(
            "000000000102",
            "implement-graph-indexing",
            "Implement graph indexing",
            "# Implement graph indexing pipeline\n\nImplement the graph indexing pipeline in \
             Rust. The graph indexing pipeline wires seeds, clusters, judgment, and \
             materialization phases. Graph indexing pipeline integration tests live in the \
             temper-cli crate.",
        ),
    );
    write_file(
        &task_dir,
        "test-graph-indexing.md",
        &task_file(
            "000000000103",
            "test-graph-indexing",
            "Test graph indexing",
            "# Test graph indexing pipeline\n\nWrite integration tests for the graph indexing \
             pipeline. The graph indexing pipeline must produce deterministic clusters given \
             a fixed corpus. Graph indexing pipeline failures surface in the error log.",
        ),
    );
    write_file(
        &goal_dir,
        "ship-graph-indexing.md",
        &goal_file(
            "000000000104",
            "ship-graph-indexing",
            "Ship graph indexing",
            "# Ship graph indexing pipeline\n\nShip the graph indexing pipeline to production. \
             The graph indexing pipeline enables LLM-assisted concept discovery across the \
             vault. Graph indexing pipeline rollout covers all temper users.",
        ),
    );
}

/// A `GraphIndexConfig` loose enough that small fixture vaults still form
/// clusters. Defaults (e.g. `concept_min_members: 3`) are tuned for
/// real-sized vaults; tests need lower thresholds to exercise the pipeline.
fn loose_graph_config() -> GraphIndexConfig {
    GraphIndexConfig {
        seed_min_doc_frequency: 2,
        seed_top_n: 20,
        cluster_similarity_threshold: 0.30,
        cluster_max_members: 12,
        concept_min_members: 2,
        concept_default_edge_type: "relates-to".to_string(),
        // Fixture vault has only 4 docs and repeats "graph indexing pipeline"
        // in every one; the production 0.5 max-df default would drop those
        // stems as gravity wells. Disable both the max-df gravity-well filter
        // and the cluster-overlap dedup for these mechanics tests.
        seed_max_doc_frequency_ratio: 1.1,
        cluster_overlap_threshold: 1.1,
        ..GraphIndexConfig::default()
    }
}

// ---------------------------------------------------------------------------
// Scenario 1 — `temper index` writes sidecar + HNSW binary; both load clean.
// ---------------------------------------------------------------------------

#[test]
fn test_graph_index_temper_index_writes_sidecar_and_hnsw_binary() {
    use hnsw_rs::prelude::{DistCosine, HnswIo};

    let tmp = TempDir::new().unwrap();
    build_fixture_vault(&tmp);
    let config = fixture_config(&tmp);

    let report = index_action::run(
        &config,
        IndexParams {
            context_filter: None,
            full: true,
        },
    )
    .unwrap();
    assert_eq!(report.files_indexed, 4, "all 4 fixture files indexed");

    let temper_dir = tmp.path().join(".temper");
    assert!(
        temper_dir.join("index.json").exists(),
        "sidecar manifest written"
    );
    assert!(
        temper_dir.join("index.hnsw.data").exists(),
        "HNSW data file written"
    );
    assert!(
        temper_dir.join("index.hnsw.graph").exists(),
        "HNSW graph file written"
    );

    // Reload the HNSW dump via HnswIo to confirm the binary is well-formed.
    let mut io = HnswIo::new(&temper_dir, "index");
    let _hnsw = io
        .load_hnsw::<f32, DistCosine>()
        .expect("HnswIo loads the binary dump");
}

// ---------------------------------------------------------------------------
// Scenario 2 — `graph index --dry-run` returns a report, writes no Concepts.
// ---------------------------------------------------------------------------

#[test]
fn test_graph_index_dry_run_reports_clusters_without_writing_concepts() {
    let tmp = TempDir::new().unwrap();
    build_fixture_vault(&tmp);
    let config = fixture_config(&tmp);
    let graph_config = loose_graph_config();

    index_action::run(
        &config,
        IndexParams {
            context_filter: None,
            full: true,
        },
    )
    .unwrap();

    let report = graph_index::run_with_provider(
        &config,
        GraphIndexParams {
            context_filter: None,
            dry_run: true,
            verbose: false,
        },
        None,
        &graph_config,
    )
    .unwrap();

    assert!(
        report.seeds_extracted > 0,
        "seeds_extracted non-zero (got {})",
        report.seeds_extracted
    );
    assert!(
        report.clusters_formed > 0,
        "clusters_formed non-zero (got {})",
        report.clusters_formed
    );
    assert_eq!(report.concepts_created, 0, "dry-run writes no concepts");

    let concept_dir = tmp.path().join("@me").join("temper").join("concept");
    assert!(
        !concept_dir.exists() || fs::read_dir(&concept_dir).unwrap().next().is_none(),
        "no Concept files written in dry-run"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3 — full run with a MockLlmProvider creates Concept files, adds
// relates-to edges to members, and is idempotent on a second run.
// ---------------------------------------------------------------------------

#[test]
fn test_graph_index_creates_concepts_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    build_fixture_vault(&tmp);
    let config = fixture_config(&tmp);
    let graph_config = loose_graph_config();

    index_action::run(
        &config,
        IndexParams {
            context_filter: None,
            full: true,
        },
    )
    .unwrap();

    // MockLlmProvider returns a single-turn ConceptProposal JSON payload,
    // shaped to match `judgment::parse_concept_proposal`. Every cluster the
    // pipeline generates gets the same proposal — but the materialization
    // phase skips duplicates by slug on the second run.
    let proposal = json!({
        "is_concept": true,
        "slug": "graph-indexing-pipeline",
        "title": "Graph Indexing Pipeline",
        "body_markdown": "## Members\n\n- design-graph-indexing\n- implement-graph-indexing",
        "member_edges": [
            { "target_slug": "design-graph-indexing", "edge_type": "relates-to" },
            { "target_slug": "implement-graph-indexing", "edge_type": "relates-to" },
            { "target_slug": "test-graph-indexing", "edge_type": "relates-to" },
            { "target_slug": "ship-graph-indexing", "edge_type": "relates-to" },
        ]
    });
    let mock = MockLlmProvider::new("mock", "mock-model")
        .scenario(MockScenario::SingleTurn(proposal.clone()));
    let provider: Arc<dyn LlmProvider> = Arc::new(mock);

    let report = graph_index::run_with_provider(
        &config,
        GraphIndexParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
        Some(Arc::clone(&provider)),
        &graph_config,
    )
    .unwrap();

    assert!(report.clusters_formed > 0, "clusters formed");
    assert!(report.proposals_returned > 0, "proposals returned");
    assert!(
        report.concepts_created >= 1,
        "at least one concept created (got {})",
        report.concepts_created
    );
    assert!(report.members_updated >= 1, "members updated");

    // Concept file exists at the canonical path.
    let concept_path = tmp
        .path()
        .join("@me")
        .join("temper")
        .join("concept")
        .join("graph-indexing-pipeline.md");
    assert!(
        concept_path.exists(),
        "concept file written at {concept_path:?}"
    );

    // A member's frontmatter picked up the relates-to edge.
    let member_path = tmp
        .path()
        .join("@me")
        .join("temper")
        .join("task")
        .join("design-graph-indexing.md");
    let fm = temper_core::frontmatter::Frontmatter::parse_file(&member_path).unwrap();
    let relates: Vec<String> = fm
        .value()
        .get("relates_to")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        relates.iter().any(|s| s == "graph-indexing-pipeline"),
        "member has relates_to edge to concept slug, got {relates:?}"
    );

    // Second run: idempotent — vault observables don't change.
    let mock2 =
        MockLlmProvider::new("mock", "mock-model").scenario(MockScenario::SingleTurn(proposal));
    let provider2: Arc<dyn LlmProvider> = Arc::new(mock2);

    graph_index::run_with_provider(
        &config,
        GraphIndexParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
        Some(provider2),
        &graph_config,
    )
    .unwrap();

    // Idempotency observable: exactly one Concept file still exists with
    // the pipeline's slug, and member frontmatter edges are deduplicated
    // (the relates_to list still contains the concept slug exactly once).
    let concept_dir = tmp.path().join("@me").join("temper").join("concept");
    let concept_files: Vec<PathBuf> = fs::read_dir(&concept_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    assert_eq!(
        concept_files.len(),
        1,
        "still exactly one concept file after second run, got {concept_files:?}"
    );

    let fm2 = temper_core::frontmatter::Frontmatter::parse_file(&member_path).unwrap();
    let relates2: Vec<String> = fm2
        .value()
        .get("relates_to")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let count = relates2
        .iter()
        .filter(|s| *s == "graph-indexing-pipeline")
        .count();
    assert_eq!(
        count, 1,
        "member relates_to contains concept slug exactly once (idempotent), got {relates2:?}"
    );
}
