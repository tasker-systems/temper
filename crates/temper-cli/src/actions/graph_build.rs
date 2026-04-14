//! `temper graph build` pipeline implementation.
//!
//! Three-pass additive seeder that walks the vault, scans markdown
//! bodies for explicit references (markdown links, wikilinks, bare
//! UUIDs), resolves them within-owner, and writes the resolved set
//! back into each file's `open_meta.references`.
//!
//! Owner boundaries are enforced by map partitioning: every resolution
//! map is keyed by owner, and a scanning file can only look inside
//! the map for its own owner. Cross-owner references are structurally
//! impossible.

use std::collections::HashMap;
use std::path::PathBuf;

use uuid::Uuid;

use crate::config::Config;
use crate::error::Result;

/// Doc types that live at `{vault}/{owner}/{context}/{doc_type}/`.
/// Matches `actions::doctor::ENTITY_DOC_TYPES`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

/// Parameters for a graph build run.
#[derive(Debug, Clone)]
pub struct GraphBuildParams {
    /// Optional single-context filter. None means all configured contexts.
    pub context_filter: Option<String>,
    /// If true, do not write any files; report what would change.
    pub dry_run: bool,
    /// If true, emit per-file detail in the report.
    pub verbose: bool,
}

/// Final report from a graph build run.
#[derive(Debug, Default, Clone)]
pub struct GraphBuildReport {
    pub files_walked: usize,
    pub references_found: usize,
    pub files_modified: usize,
    pub references_added: usize,
    pub already_present: usize,
    pub modified_files: Vec<ModifiedFile>,
}

/// Per-file change record for the report.
#[derive(Debug, Clone)]
pub struct ModifiedFile {
    pub rel_path: String,
    pub added: usize,
    /// Only populated when verbose = true
    pub added_refs: Vec<String>,
}

/// Owner sigil-prefixed identifier, e.g. "@me" or "+platform-eng".
pub type Owner = String;
/// Context name, e.g. "temper", "tasker".
pub type Context = String;

/// Slug resolution maps, partitioned by owner AND context for
/// same-context-first resolution. A slug lookup walks "same context
/// first, then cross-context if unique" — never crossing the owner
/// boundary.
#[derive(Debug, Default)]
pub(crate) struct SlugMap {
    inner: HashMap<Owner, HashMap<Context, HashMap<String, PathBuf>>>,
}

/// UUID resolution map, partitioned by owner only (UUIDs are globally
/// unique within the vault and do not need context partitioning).
#[derive(Debug, Default)]
pub(crate) struct UuidMap {
    inner: HashMap<Owner, HashMap<Uuid, PathBuf>>,
}

/// Top-level entry point. Walks the vault, scans bodies, merges
/// references into open_meta, writes files back.
pub fn run(config: &Config, params: GraphBuildParams) -> Result<GraphBuildReport> {
    let _ = (config, params);
    Err(crate::error::TemperError::Project(
        "graph_build::run: not yet implemented".into(),
    ))
}
