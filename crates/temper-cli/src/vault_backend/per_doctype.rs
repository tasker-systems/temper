//! Per-doctype dispatch table for `create_resource`.
//!
//! # Audit gate (Task 7, Wave 1 Phase 4a)
//!
//! The four delegated doctypes — task, goal, session, research — **all fail the
//! audit gate**. Their existing creators each call
//! `crate::actions::runtime::publish_local_write_best_effort`, which internally
//! calls `resolve_token_store` → `VaultState::from_env()`. Dispatching from
//! `VaultBackend::create_resource` into those creators would re-enter the
//! `VaultState` branching that Phase 4b is tasked with removing.
//!
//! Task 7 therefore implements concept/decision inline and returns
//! `BadRequest` for task/goal/session/research. Follow-up task:
//! `complete-per-doctype-write-dispatch-for-task-goal-session-research`.

use std::path::{Path, PathBuf};

use askama::Template;
use chrono::Local;
use temper_core::error::TemperError;
use temper_core::frontmatter::Frontmatter;
use temper_core::types::ids::ResourceId;
use temper_core::vault::Vault;
use uuid::Uuid;

use crate::config::Config;
use crate::templates::{ConceptTemplate, DecisionTemplate};

/// Arguments for the per-doctype write dispatch.
///
/// Exceeds 5 fields — params struct required by project convention.
pub(crate) struct WriteArgs<'a> {
    pub doctype: &'a str,
    pub title: &'a str,
    pub slug: &'a str,
    pub context: &'a str,
    pub body: &'a str,
    pub open_meta: Option<&'a serde_json::Value>,
    pub vault_root: &'a Path,
    pub owner: &'a str,
    /// Held for Phase 4b when task/goal/session/research dispatch is added;
    /// those existing creators need `&Config` for path construction.
    #[expect(
        dead_code,
        reason = "needed by task/goal/session/research dispatch in follow-up task \
                  complete-per-doctype-write-dispatch-for-task-goal-session-research"
    )]
    pub config: &'a Config,
}

/// Outcome of a per-doctype file write.
#[derive(Debug)]
pub(crate) struct WriteResult {
    /// Stable UUID for this resource (from the `temper-id` field written to disk).
    pub resource_id: ResourceId,
    /// Absolute filesystem path to the written file.
    pub abs_path: PathBuf,
    /// Vault-relative path (relative to `vault_root`), e.g. `@me/temper/concept/my-concept.md`.
    pub rel_path: String,
}

/// Dispatch a file write to the correct per-doctype implementation.
///
/// # Scoping note (Phase 4a)
///
/// Only `concept` and `decision` are implemented inline. Calls for
/// `task`, `goal`, `session`, or `research` return `BadRequest` until
/// the follow-up task `complete-per-doctype-write-dispatch-for-task-goal-session-research`
/// rewires those creators to avoid `VaultState::from_env()` re-entry.
pub(crate) fn write_for(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    match args.doctype {
        "concept" | "decision" => write_concept_or_decision(args),
        "task" | "goal" | "session" | "research" => Err(TemperError::BadRequest(format!(
            "doctype '{}' not yet supported by VaultBackend; use commands/{}.rs directly \
             until the follow-up task \
             `complete-per-doctype-write-dispatch-for-task-goal-session-research` lands the \
             rewire (Phase 4b). Audit gate reason: existing creator calls \
             `publish_local_write_best_effort` → `VaultState::from_env()` internally.",
            args.doctype, args.doctype,
        ))),
        other => Err(TemperError::BadRequest(format!(
            "unsupported doctype for create: '{other}'"
        ))),
    }
}

/// Write a concept or decision resource using Askama templates.
///
/// Mirrors the body of `commands/resource.rs::create_simple_resource` but:
/// - Takes the pre-resolved `owner` string (no `config.owner_for_context` call here;
///   the caller has already resolved this).
/// - Returns a `WriteResult` instead of printing output.
/// - Does NOT call `publish_local_write_best_effort` — that is the backend's
///   responsibility via the push-as-tail-action path.
fn write_concept_or_decision(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    let WriteArgs {
        doctype,
        title,
        slug,
        context,
        body,
        open_meta,
        vault_root,
        owner,
        config: _,
    } = args;

    let today = Local::now().format("%Y-%m-%d").to_string();
    let id_str = crate::ids::generate_id();

    let content = match doctype {
        "concept" => {
            let tmpl = ConceptTemplate {
                id: &id_str,
                title,
                date: &today,
                project: context,
                slug,
            };
            tmpl.render()
                .map_err(|e| TemperError::Vault(format!("template error: {e}")))?
        }
        "decision" => {
            let tmpl = DecisionTemplate {
                id: &id_str,
                title,
                date: &today,
                project: context,
                slug,
            };
            tmpl.render()
                .map_err(|e| TemperError::Vault(format!("template error: {e}")))?
        }
        _ => unreachable!("only called for concept/decision"),
    };

    // Parse the rendered template, then overlay additional fields.
    let mut fm = Frontmatter::try_from(content.as_str())?;

    // Apply open-tier metadata if provided.
    if let Some(open) = open_meta {
        if let Some(obj) = open.as_object() {
            for (key, value) in obj {
                fm.set_open_field(key, value.clone());
            }
        }
    }

    // Set body if provided.
    if !body.is_empty() {
        fm.set_body(body.to_string());
    }

    let vault_layout = Vault::new(vault_root);
    let dir = vault_layout.doc_type_dir(owner, context, doctype);
    let abs_path = vault_layout.doc_file(owner, context, doctype, slug);
    let rel_path = vault_layout.rel_path(owner, context, doctype, slug);

    if abs_path.exists() {
        return Err(TemperError::Vault(format!(
            "{doctype} already exists: {slug}"
        )));
    }

    std::fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    fm.write_to(&abs_path)?;

    // Parse the resource_id back from the written file's temper-id field.
    // The template injects `temper-id: "<id_str>"` — we parse from the string
    // we already have rather than re-reading from disk.
    let resource_id = Uuid::parse_str(&id_str)
        .map(ResourceId::from)
        .map_err(|e| {
            TemperError::Vault(format!("generated id is not a valid UUID: {id_str}: {e}"))
        })?;

    Ok(WriteResult {
        resource_id,
        abs_path,
        rel_path,
    })
}

/// Derive the vault-relative path from the written result for doctypes that
/// delegate to an existing creator (for use once the audit-gate follow-up lands).
///
/// Not used in Phase 4a (task/goal/session/research are scoped down), but
/// placed here so Phase 4b can build on it without duplication.
#[expect(
    dead_code,
    reason = "used when task/goal/session/research dispatch is added in the \
              follow-up task complete-per-doctype-write-dispatch-for-task-goal-session-research"
)]
pub(crate) fn derive_rel_path(vault_root: &Path, abs_path: &Path) -> Result<String, TemperError> {
    abs_path
        .strip_prefix(vault_root)
        .map(|rel| rel.to_string_lossy().into_owned())
        .map_err(|_| {
            TemperError::Vault(format!(
                "written path {} is not inside vault root {}",
                abs_path.display(),
                vault_root.display()
            ))
        })
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use std::fs;

    use super::*;
    use crate::config::Config;

    fn make_config(vault_root: &Path) -> Config {
        Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        }
    }

    #[test]
    fn write_for_concept_creates_file_at_expected_path() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let slug = crate::vault::slugify("My Concept");
        let args = WriteArgs {
            doctype: "concept",
            title: "My Concept",
            slug: &slug,
            context: "temper",
            body: "",
            open_meta: None,
            vault_root: tmp.path(),
            owner: "@me",
            config: &config,
        };
        let result = write_for(args).expect("write ok");
        assert!(result.abs_path.exists(), "file must exist at abs_path");
        assert!(
            result.rel_path.ends_with(".md"),
            "rel_path must end with .md"
        );
        let content = fs::read_to_string(&result.abs_path).unwrap();
        assert!(content.contains("My Concept"), "title in frontmatter");
        // Concept/decision templates write `temper-provisional-id` (not `temper-id`).
        // The write path does not upgrade provisional→permanent — that is sync's job.
        assert!(
            content.contains("temper-provisional-id") || content.contains("temper-id"),
            "id field (provisional or permanent) must be present; got: {content}"
        );
    }

    #[test]
    fn write_for_decision_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = WriteArgs {
            doctype: "decision",
            title: "Choose Postgres",
            slug: "choose-postgres",
            context: "temper",
            body: "",
            open_meta: None,
            vault_root: tmp.path(),
            owner: "@me",
            config: &config,
        };
        let result = write_for(args).expect("write ok");
        assert!(result.abs_path.exists());
        let content = fs::read_to_string(&result.abs_path).unwrap();
        assert!(content.contains("Choose Postgres"));
    }

    #[test]
    fn write_for_task_returns_bad_request() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = WriteArgs {
            doctype: "task",
            title: "My Task",
            slug: "my-task",
            context: "temper",
            body: "",
            open_meta: None,
            vault_root: tmp.path(),
            owner: "@me",
            config: &config,
        };
        let err = write_for(args).expect_err("task not supported");
        assert!(
            matches!(err, TemperError::BadRequest(_)),
            "expected BadRequest for task, got: {err:?}"
        );
    }

    #[test]
    fn write_for_unsupported_doctype_returns_bad_request() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = WriteArgs {
            doctype: "widget",
            title: "W",
            slug: "w",
            context: "temper",
            body: "",
            open_meta: None,
            vault_root: tmp.path(),
            owner: "@me",
            config: &config,
        };
        let err = write_for(args).expect_err("widget not supported");
        assert!(matches!(err, TemperError::BadRequest(_)));
    }
}
