//! Resource lookup primitives for CLI commands.
//!
//! `FindableResource` formalizes the inputs to a vault-file lookup:
//! owner (defaulting to `@me` canonical), context (optional — defaults to
//! every configured context), typed doc_type, and a raw slug-or-suffix
//! string. `find_resource` walks the on-disk vault using the same
//! match-by-stem / match-by-slug-portion / suffix-match rules as
//! `actions::task::find_task`, with no `slugify` normalization (which
//! would silently collapse `--` and break double-hyphen slugs — see C.1
//! in the 2026-05-09 audit sweep).
//!
//! When `manifest` is provided and a match is found, the resolved record
//! also carries `temper-id` (or `temper-provisional-id` for unsynced
//! files) so callers don't need a second frontmatter parse.

use std::path::PathBuf;

use temper_core::frontmatter::DocType;
use temper_core::types::ids::ResourceId;
use temper_core::types::Manifest;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Lookup request for a single resource by slug-or-suffix.
///
/// `owner: None` defaults to the canonical `@me` directory. Pass
/// `Some("@<other-slug>")` to look up a team-shared or other-user
/// resource explicitly.
///
/// `context: None` scans every configured context in `config.contexts`.
///
/// `manifest`, when provided, is consulted for `slug → ResourceId`
/// resolution if the file's frontmatter doesn't carry a parsed `temper-id`.
pub struct FindableResource<'a> {
    pub config: &'a Config,
    pub manifest: Option<&'a Manifest>,
    pub owner: Option<String>,
    pub context: Option<String>,
    pub doc_type: DocType,
    pub slug_or_suffix: String,
}

/// Result of a successful `find_resource` call.
#[derive(Debug, Clone)]
pub struct ResolvedResource {
    pub path: PathBuf,
    pub context: String,
    pub owner: String,
    pub doc_type: DocType,
    pub resource_id: Option<ResourceId>,
    pub provisional_id: Option<String>,
}

/// Locate a resource on disk. See module-level docs for the matching
/// algorithm.
///
/// Errors:
/// - `TemperError::Vault("<doctype> not found: <slug>")` when no file matches.
/// - `TemperError::Vault("ambiguous slug suffix '<input>', matches: ...")`
///   when more than one file matches by suffix-only (mirroring `find_task`).
pub fn find_resource(_req: FindableResource<'_>) -> Result<ResolvedResource> {
    Err(TemperError::Vault(
        "find_resource: not yet implemented".into(),
    ))
}
