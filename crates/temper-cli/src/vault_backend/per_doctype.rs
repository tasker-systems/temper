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
use crate::templates::{
    ConceptTemplate, DecisionTemplate, GoalTemplate, SessionTemplate, TaskTemplate,
};

/// Doctype-specific frontmatter fields not covered by the doctype-agnostic
/// `WriteArgs` fields. Each variant carries the values required to render
/// that doctype's template. Concept and decision do not need extra fields
/// and pass `None`.
///
/// Extended across Phase A as task/goal/session/research dispatch lands:
/// A1 introduces `Task`, A2 adds `Goal`; A3-A4 add `Session`/`Research`.
pub(crate) enum DoctypeFields<'a> {
    /// Task-specific fields: goal slug, mode, effort, and sequence number.
    /// `mode`/`effort` are pre-validated by the caller; `seq` is pre-computed
    /// via `actions::task::next_seq`.
    Task {
        goal: &'a str,
        mode: &'a str,
        effort: &'a str,
        seq: u32,
    },
    /// Goal-specific fields: sequence number.
    ///
    /// `seq` is pre-computed by the wrapper via `actions::goal::next_seq`. `id`
    /// and `date` are generated inside `write_goal` to match the `write_task`
    /// shape — they are not caller inputs. `title`, `slug`, and `context` ride
    /// on the doctype-agnostic `WriteArgs` fields.
    Goal { seq: u32 },
    /// Session-specific fields: none beyond the doctype-agnostic `WriteArgs`.
    ///
    /// The session template's frontmatter is populated from `title`, `slug`,
    /// `context` (already on `WriteArgs`), plus an `id` and `date` generated
    /// inside `write_session`. The empty variant exists so the dispatch is
    /// explicit and unambiguous — passing `None` (or a wrong variant) hard-errors
    /// with `BadRequest`, preventing silent fall-through from a misrouted call.
    Session,
}

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
    /// Held for state-dir / context-aware path construction in writers that
    /// need it (currently unused by task/concept/decision/goal/session writers;
    /// research dispatch in A4 may need it).
    #[expect(dead_code, reason = "needed by research dispatch in follow-up task A4")]
    pub config: &'a Config,
    /// Doctype-specific frontmatter fields. `None` for doctypes whose template
    /// only consumes the doctype-agnostic fields (concept, decision).
    pub doctype_fields: Option<DoctypeFields<'a>>,
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
/// # Scoping note (Phase 4a → Phase 4 completion)
///
/// Concept and decision were implemented in Phase 4a. Task is wired in
/// A1, goal in A2; session/research land in A3-A4. The audit-gate fallback for
/// the still-pending doctypes is removed wholesale in A5 once all four
/// per-doctype writers are in place.
pub(crate) fn write_for(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    match args.doctype {
        "concept" | "decision" => write_concept_or_decision(args),
        "task" => write_task(args),
        "goal" => write_goal(args),
        "session" => write_session(args),
        "research" => Err(TemperError::BadRequest(format!(
            "doctype '{}' not yet supported by VaultBackend; use commands/{}.rs directly \
             until follow-up task A4 lands the per-doctype writer.",
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
        doctype_fields: _,
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

/// Write a task resource using the Askama `TaskTemplate`.
///
/// Mirrors the template + frontmatter + write half of `actions::task::create`,
/// minus the validation (goal-exists, mode/effort) and tail actions (publish,
/// discovery event, output) which remain at the wrapper. The wrapper computes
/// `seq` via `actions::task::next_seq` and passes it through `DoctypeFields::Task`.
///
/// Hard-errors when the target slug already exists on disk. The pre-pull
/// `actions::task::create` would silently overwrite an existing slug — A1
/// tightens that to error-on-exists (matches concept/decision behavior).
///
/// # Byte-preservation
///
/// When no `open_meta` is supplied, the rendered template is written raw with
/// `body` string-appended (matching the pre-pull `vault::write_note` path). This
/// preserves YAML serialization details (e.g. quoted string values) that the
/// canonical `Frontmatter::serialize` path drops. When `open_meta` is supplied,
/// we parse + overlay + serialize via `Frontmatter` — caller opts into the
/// canonical form.
fn write_task(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    let WriteArgs {
        doctype: _,
        title,
        slug,
        context,
        body,
        open_meta,
        vault_root,
        owner,
        config: _,
        doctype_fields,
    } = args;

    let (goal, mode, effort, seq) = match doctype_fields {
        Some(DoctypeFields::Task {
            goal,
            mode,
            effort,
            seq,
        }) => (goal, mode, effort, seq),
        _ => {
            return Err(TemperError::BadRequest(
                "task write requires DoctypeFields::Task".to_string(),
            ));
        }
    };

    let id_str = crate::ids::generate_id();
    let datetime = Local::now().to_rfc3339();
    let seq_str = seq.to_string();
    let tmpl = TaskTemplate {
        id: &id_str,
        title,
        slug,
        context,
        goal,
        mode,
        effort,
        seq: &seq_str,
        datetime: &datetime,
    };
    let rendered = tmpl
        .render()
        .map_err(|e| TemperError::Vault(format!("template error: {e}")))?;

    let vault_layout = Vault::new(vault_root);
    let dir = vault_layout.doc_type_dir(owner, context, "task");
    let abs_path = vault_layout.doc_file(owner, context, "task", slug);
    let rel_path = vault_layout.rel_path(owner, context, "task", slug);

    if abs_path.exists() {
        return Err(TemperError::Vault(format!("task already exists: {slug}")));
    }

    std::fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;

    if open_meta.is_some() {
        // Open-meta overlay path: parse rendered template, apply open-tier
        // fields, optionally set body, write via Frontmatter::write_to.
        let mut fm = Frontmatter::try_from(rendered.as_str())?;
        if let Some(open) = open_meta {
            if let Some(obj) = open.as_object() {
                for (key, value) in obj {
                    fm.set_open_field(key, value.clone());
                }
            }
        }
        if !body.is_empty() {
            fm.set_body(body.to_string());
        }
        fm.write_to(&abs_path)?;
    } else {
        // No open-meta overlay: byte-preserve the rendered template and
        // string-append the body (mirrors the pre-pull
        // `vault::write_note(&content)` path with `content.push_str(body)`
        // semantics). Avoids re-serializing through Frontmatter, which would
        // canonicalize away quoting that callers rely on.
        let mut content = rendered;
        if !body.is_empty() {
            content.push_str(body);
            content.push('\n');
        }
        crate::vault::write_note(&abs_path, &content)?;
    }

    // The task template injects `temper-provisional-id: "<id_str>"` — we parse
    // from the locally generated string rather than re-reading from disk,
    // matching `write_concept_or_decision`.
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

/// Write a goal resource using the Askama `GoalTemplate`.
///
/// Mirrors the template + frontmatter + write half of `actions::goal::create`,
/// minus the tail actions (publish, discovery event, output) which remain at
/// the wrapper. The wrapper computes `seq` via `actions::goal::next_seq` and
/// passes it through `DoctypeFields::Goal`.
///
/// Hard-errors when the target slug already exists on disk. The pre-pull
/// `actions::goal::create` already had this check (matches concept/decision/task).
///
/// Note: `ensure_maintenance` does NOT route through `write_goal` — it has an
/// idempotent get-or-create semantic that doesn't fit the hard-error-on-exists
/// contract. Its write path stays inline at `actions::goal::ensure_maintenance`.
///
/// # Byte-preservation
///
/// Goals are written via raw `vault::write_note` (matching pre-pull behavior).
/// Goals have no `open_meta` write path today, but the overlay branch is
/// retained for symmetry with `write_task` should a backend caller pass open
/// fields in the future.
fn write_goal(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    let WriteArgs {
        doctype: _,
        title,
        slug,
        context,
        body,
        open_meta,
        vault_root,
        owner,
        config: _,
        doctype_fields,
    } = args;

    let seq = match doctype_fields {
        Some(DoctypeFields::Goal { seq }) => seq,
        _ => {
            return Err(TemperError::BadRequest(
                "goal write requires DoctypeFields::Goal".to_string(),
            ));
        }
    };

    let id_str = crate::ids::generate_id();
    let date = Local::now().format("%Y-%m-%d").to_string();
    let seq_str = seq.to_string();
    let tmpl = GoalTemplate {
        id: &id_str,
        title,
        slug,
        context,
        seq: &seq_str,
        date: &date,
    };
    let rendered = tmpl
        .render()
        .map_err(|e| TemperError::Vault(format!("template error: {e}")))?;

    let vault_layout = Vault::new(vault_root);
    let dir = vault_layout.doc_type_dir(owner, context, "goal");
    let abs_path = vault_layout.doc_file(owner, context, "goal", slug);
    let rel_path = vault_layout.rel_path(owner, context, "goal", slug);

    if abs_path.exists() {
        return Err(TemperError::Vault(format!("goal already exists: {slug}")));
    }

    std::fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;

    if open_meta.is_some() {
        // Open-meta overlay path: parse rendered template, apply open-tier
        // fields, optionally set body, write via Frontmatter::write_to.
        let mut fm = Frontmatter::try_from(rendered.as_str())?;
        if let Some(open) = open_meta {
            if let Some(obj) = open.as_object() {
                for (key, value) in obj {
                    fm.set_open_field(key, value.clone());
                }
            }
        }
        if !body.is_empty() {
            fm.set_body(body.to_string());
        }
        fm.write_to(&abs_path)?;
    } else {
        // No open-meta overlay: byte-preserve the rendered template via
        // `vault::write_note`. This matches the pre-pull
        // `actions::goal::create` write path. Body is empty in today's call
        // sites (goal templates have no body input), but we string-append for
        // future-proofing should a caller pass one.
        let mut content = rendered;
        if !body.is_empty() {
            content.push_str(body);
            content.push('\n');
        }
        crate::vault::write_note(&abs_path, &content)?;
    }

    // The goal template injects `temper-provisional-id: "<id_str>"` — we parse
    // from the locally generated string rather than re-reading from disk,
    // matching `write_concept_or_decision` and `write_task`.
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

/// Write a session resource using the Askama `SessionTemplate`.
///
/// Mirrors the new-file-create branch (lines 84-120) of
/// `commands::session::save`, minus the tail actions (publish, discovery event,
/// output) which remain at the wrapper. The wrapper resolves context/title/slug
/// and decides whether to delegate here or take the save-or-update overload path
/// (which stays inline at the surface — see `commands::session::save`).
///
/// Hard-errors when the target slug already exists on disk. The pre-pull
/// `commands::session::save` reached this branch only after checking
/// `note_path.exists()` was false, so this error path is only hit if the file
/// appears between the wrapper's check and the writer's write (race) or if a
/// future caller (e.g. `VaultBackend.create_resource` via B5) routes here
/// without a prior exists-check.
///
/// # Managed-meta handling
///
/// The session template renders `temper-context: ""` and only the provisional
/// id + type + date. The original creator overlaid managed-meta via
/// `build_managed_meta_for_create` + `set_managed_meta` to populate
/// `temper-context` (and `temper-type`/`temper-title` for symmetry). Phase A3
/// migrates that call here as a small early Phase C migration — the helper's
/// other callers (research, cloud-mode resource.rs) stay until A4/Phase C.
///
/// # Byte-preservation
///
/// Unlike task/goal, session always serializes through `Frontmatter::write_to`
/// because the managed-meta overlay step requires it. The existing creator did
/// the same, so this preserves the wire format.
fn write_session(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    let WriteArgs {
        doctype: _,
        title,
        slug,
        context,
        body,
        open_meta,
        vault_root,
        owner,
        config: _,
        doctype_fields,
    } = args;

    match doctype_fields {
        Some(DoctypeFields::Session) => {}
        _ => {
            return Err(TemperError::BadRequest(
                "session write requires DoctypeFields::Session".to_string(),
            ));
        }
    }

    let id_str = crate::ids::generate_id();
    let date = Local::now().format("%Y-%m-%d").to_string();
    let tmpl = SessionTemplate {
        id: &id_str,
        title,
        date: &date,
    };
    let rendered = tmpl
        .render()
        .map_err(|e| TemperError::Vault(format!("template error: {e}")))?;

    let vault_layout = Vault::new(vault_root);
    let abs_path = vault_layout.doc_file(owner, context, "session", slug);
    let rel_path = vault_layout.rel_path(owner, context, "session", slug);

    if abs_path.exists() {
        return Err(TemperError::Vault(format!(
            "session already exists: {slug}"
        )));
    }

    // Parse rendered template, then overlay managed-meta (replaces the
    // pre-pull `build_managed_meta_for_create` + `set_managed_meta` calls at
    // commands::session::save:96-110). This fixes the template's empty
    // `temper-context: ""` and ensures `temper-type`/`temper-title` are present
    // for downstream consumers.
    let mut fm = Frontmatter::try_from(rendered.as_str())?;
    let meta = crate::actions::frontmatter::build_managed_meta_for_create(
        crate::actions::frontmatter::NewResourceArgs {
            doc_type: "session",
            context,
            title,
            mode: None,
            effort: None,
            goal: None,
            stage: None,
            seq: None,
            status: None,
            provenance: None,
            llm_model: None,
            llm_run: None,
        },
    );
    fm.set_managed_meta(&meta);

    // Apply open-tier metadata if provided (no callers do this today for
    // session, but retained for symmetry with task/goal/concept/decision).
    if let Some(open) = open_meta {
        if let Some(obj) = open.as_object() {
            for (key, value) in obj {
                fm.set_open_field(key, value.clone());
            }
        }
    }

    if !body.is_empty() {
        fm.set_body(body.to_string());
    }

    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TemperError::Vault(e.to_string()))?;
    }
    fm.write_to(&abs_path)?;

    // The session template injects `temper-provisional-id: "<id_str>"` — we
    // parse from the locally generated string rather than re-reading from disk,
    // matching `write_concept_or_decision` / `write_task` / `write_goal`.
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
            doctype_fields: None,
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
            doctype_fields: None,
        };
        let result = write_for(args).expect("write ok");
        assert!(result.abs_path.exists());
        let content = fs::read_to_string(&result.abs_path).unwrap();
        assert!(content.contains("Choose Postgres"));
    }

    fn task_args<'a>(
        config: &'a Config,
        vault_root: &'a Path,
        title: &'a str,
        slug: &'a str,
        body: &'a str,
    ) -> WriteArgs<'a> {
        WriteArgs {
            doctype: "task",
            title,
            slug,
            context: "temper",
            body,
            open_meta: None,
            vault_root,
            owner: "@me",
            config,
            doctype_fields: Some(DoctypeFields::Task {
                goal: "my-goal",
                mode: "build",
                effort: "small",
                seq: 10,
            }),
        }
    }

    #[test]
    fn write_for_task_creates_file_with_correct_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = task_args(&config, tmp.path(), "My Task", "2026-05-13-my-task", "");
        let result = write_for(args).expect("write ok");
        assert!(result.abs_path.exists(), "file must exist at abs_path");
        let content = fs::read_to_string(&result.abs_path).unwrap();
        // Identity + classification fields populated by the template.
        // serde_yaml may re-serialize string values without surrounding quotes,
        // so we assert key presence + value content rather than exact rendering.
        let fm = Frontmatter::try_from(content.as_str()).expect("frontmatter must parse");
        let mapping = fm
            .value()
            .as_mapping()
            .expect("frontmatter must be mapping");
        let get = |key: &str| {
            mapping
                .get(serde_yaml::Value::String(key.to_string()))
                .cloned()
        };
        assert!(
            get("temper-provisional-id").is_some(),
            "temper-provisional-id must be present; got: {content}"
        );
        assert_eq!(
            get("temper-title"),
            Some(serde_yaml::Value::String("My Task".to_string())),
            "temper-title must equal 'My Task'; got: {content}"
        );
        assert_eq!(
            get("temper-type"),
            Some(serde_yaml::Value::String("task".to_string())),
            "temper-type must equal 'task'; got: {content}"
        );
        // Task-specific fields populated from DoctypeFields::Task.
        assert_eq!(
            get("temper-goal"),
            Some(serde_yaml::Value::String("my-goal".to_string())),
            "temper-goal must equal 'my-goal'; got: {content}"
        );
        assert_eq!(
            get("temper-mode"),
            Some(serde_yaml::Value::String("build".to_string())),
            "temper-mode must equal 'build'; got: {content}"
        );
        assert_eq!(
            get("temper-effort"),
            Some(serde_yaml::Value::String("small".to_string())),
            "temper-effort must equal 'small'; got: {content}"
        );
        assert_eq!(
            get("temper-seq"),
            Some(serde_yaml::Value::Number(10.into())),
            "temper-seq must equal 10; got: {content}"
        );
    }

    #[test]
    fn write_for_task_errors_on_existing_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args1 = task_args(&config, tmp.path(), "First", "2026-05-13-dup", "");
        write_for(args1).expect("first write ok");

        let args2 = task_args(&config, tmp.path(), "Second", "2026-05-13-dup", "");
        let err = write_for(args2).expect_err("second write must error");
        assert!(
            matches!(err, TemperError::Vault(ref m) if m.contains("already exists")),
            "expected Vault(already exists) error; got: {err:?}"
        );
    }

    #[test]
    fn write_for_task_writes_body_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let body = "## Plan\n\nDo the thing.\n";
        let args = task_args(&config, tmp.path(), "Bodied", "2026-05-13-bodied", body);
        let result = write_for(args).expect("write ok");
        let content = fs::read_to_string(&result.abs_path).unwrap();
        assert!(
            content.contains("Do the thing."),
            "body must be present in written file; got: {content}"
        );
    }

    #[test]
    fn write_for_task_empty_body_does_not_corrupt_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = task_args(&config, tmp.path(), "Empty", "2026-05-13-empty", "");
        let result = write_for(args).expect("write ok");
        let content = fs::read_to_string(&result.abs_path).unwrap();
        // Frontmatter still parseable.
        let fm = Frontmatter::try_from(content.as_str())
            .expect("frontmatter must parse after empty-body write");
        assert!(
            fm.value()
                .as_mapping()
                .and_then(|m| m.get(serde_yaml::Value::String("temper-type".to_string())))
                .is_some(),
            "temper-type must be a top-level key after empty-body write"
        );
    }

    fn goal_args<'a>(
        config: &'a Config,
        vault_root: &'a Path,
        title: &'a str,
        slug: &'a str,
        seq: u32,
    ) -> WriteArgs<'a> {
        WriteArgs {
            doctype: "goal",
            title,
            slug,
            context: "temper",
            body: "",
            open_meta: None,
            vault_root,
            owner: "@me",
            config,
            doctype_fields: Some(DoctypeFields::Goal { seq }),
        }
    }

    #[test]
    fn write_for_goal_creates_file_with_correct_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = goal_args(&config, tmp.path(), "Ship It", "ship-it", 10);
        let result = write_for(args).expect("write ok");
        assert!(result.abs_path.exists(), "file must exist at abs_path");
        let content = fs::read_to_string(&result.abs_path).unwrap();
        let fm = Frontmatter::try_from(content.as_str()).expect("frontmatter must parse");
        let mapping = fm
            .value()
            .as_mapping()
            .expect("frontmatter must be mapping");
        let get = |key: &str| {
            mapping
                .get(serde_yaml::Value::String(key.to_string()))
                .cloned()
        };
        assert!(
            get("temper-provisional-id").is_some(),
            "temper-provisional-id must be present; got: {content}"
        );
        assert_eq!(
            get("temper-title"),
            Some(serde_yaml::Value::String("Ship It".to_string())),
            "temper-title must equal 'Ship It'; got: {content}"
        );
        assert_eq!(
            get("temper-type"),
            Some(serde_yaml::Value::String("goal".to_string())),
            "temper-type must equal 'goal'; got: {content}"
        );
        assert_eq!(
            get("temper-slug"),
            Some(serde_yaml::Value::String("ship-it".to_string())),
            "temper-slug must equal 'ship-it'; got: {content}"
        );
        assert_eq!(
            get("temper-context"),
            Some(serde_yaml::Value::String("temper".to_string())),
            "temper-context must equal 'temper'; got: {content}"
        );
        assert_eq!(
            get("temper-seq"),
            Some(serde_yaml::Value::Number(10.into())),
            "temper-seq must equal 10; got: {content}"
        );
        assert_eq!(
            get("temper-status"),
            Some(serde_yaml::Value::String("active".to_string())),
            "temper-status must equal 'active'; got: {content}"
        );
    }

    #[test]
    fn write_for_goal_errors_on_existing_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args1 = goal_args(&config, tmp.path(), "First", "dup-goal", 10);
        write_for(args1).expect("first write ok");

        let args2 = goal_args(&config, tmp.path(), "Second", "dup-goal", 20);
        let err = write_for(args2).expect_err("second write must error");
        assert!(
            matches!(err, TemperError::Vault(ref m) if m.contains("already exists")),
            "expected Vault(already exists) error; got: {err:?}"
        );
    }

    #[test]
    fn write_for_goal_returns_bad_request_when_doctype_fields_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = WriteArgs {
            doctype: "goal",
            title: "Missing Fields",
            slug: "missing-fields",
            context: "temper",
            body: "",
            open_meta: None,
            vault_root: tmp.path(),
            owner: "@me",
            config: &config,
            doctype_fields: None,
        };
        let err = write_for(args).expect_err("missing DoctypeFields::Goal must error");
        assert!(
            matches!(err, TemperError::BadRequest(ref m) if m.contains("DoctypeFields::Goal")),
            "expected BadRequest mentioning DoctypeFields::Goal; got: {err:?}"
        );
    }

    fn session_args<'a>(
        config: &'a Config,
        vault_root: &'a Path,
        title: &'a str,
        slug: &'a str,
        body: &'a str,
    ) -> WriteArgs<'a> {
        WriteArgs {
            doctype: "session",
            title,
            slug,
            context: "temper",
            body,
            open_meta: None,
            vault_root,
            owner: "@me",
            config,
            doctype_fields: Some(DoctypeFields::Session),
        }
    }

    #[test]
    fn write_for_session_creates_file_with_correct_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = session_args(
            &config,
            tmp.path(),
            "My Session",
            "2026-05-13-my-session",
            "",
        );
        let result = write_for(args).expect("write ok");
        assert!(result.abs_path.exists(), "file must exist at abs_path");
        let content = fs::read_to_string(&result.abs_path).unwrap();
        let fm = Frontmatter::try_from(content.as_str()).expect("frontmatter must parse");
        let mapping = fm
            .value()
            .as_mapping()
            .expect("frontmatter must be mapping");
        let get = |key: &str| {
            mapping
                .get(serde_yaml::Value::String(key.to_string()))
                .cloned()
        };
        assert!(
            get("temper-provisional-id").is_some(),
            "temper-provisional-id must be present; got: {content}"
        );
        assert_eq!(
            get("temper-type"),
            Some(serde_yaml::Value::String("session".to_string())),
            "temper-type must equal 'session'; got: {content}"
        );
        assert_eq!(
            get("temper-title"),
            Some(serde_yaml::Value::String("My Session".to_string())),
            "temper-title must equal 'My Session'; got: {content}"
        );
        // Session template renders `temper-context: ""`; managed-meta overlay
        // must replace it with the real context value.
        assert_eq!(
            get("temper-context"),
            Some(serde_yaml::Value::String("temper".to_string())),
            "temper-context must equal 'temper' after managed-meta overlay; got: {content}"
        );
    }

    #[test]
    fn write_for_session_errors_on_existing_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args1 = session_args(&config, tmp.path(), "First", "2026-05-13-dup-session", "");
        write_for(args1).expect("first write ok");

        let args2 = session_args(&config, tmp.path(), "Second", "2026-05-13-dup-session", "");
        let err = write_for(args2).expect_err("second write must error");
        assert!(
            matches!(err, TemperError::Vault(ref m) if m.contains("already exists")),
            "expected Vault(already exists) error; got: {err:?}"
        );
    }

    #[test]
    fn write_for_session_writes_body_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let body = "## Goal\n\nDo the work.\n";
        let args = session_args(
            &config,
            tmp.path(),
            "Bodied",
            "2026-05-13-bodied-session",
            body,
        );
        let result = write_for(args).expect("write ok");
        let content = fs::read_to_string(&result.abs_path).unwrap();
        assert!(
            content.contains("Do the work."),
            "body must be present in written file; got: {content}"
        );
    }

    #[test]
    fn write_for_session_returns_bad_request_when_doctype_fields_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let args = WriteArgs {
            doctype: "session",
            title: "Missing Fields",
            slug: "2026-05-13-missing-fields-session",
            context: "temper",
            body: "",
            open_meta: None,
            vault_root: tmp.path(),
            owner: "@me",
            config: &config,
            doctype_fields: None,
        };
        let err = write_for(args).expect_err("missing DoctypeFields::Session must error");
        assert!(
            matches!(err, TemperError::BadRequest(ref m) if m.contains("DoctypeFields::Session")),
            "expected BadRequest mentioning DoctypeFields::Session; got: {err:?}"
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
            doctype_fields: None,
        };
        let err = write_for(args).expect_err("widget not supported");
        assert!(matches!(err, TemperError::BadRequest(_)));
    }
}
