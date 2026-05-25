use chrono::Local;
use serde::Serialize;
use temper_core::schema;
use temper_core::vault::Vault;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use crate::output;
use crate::output::columns as col_registry;
use crate::output::table::TableRenderer;
use crate::vault;

/// Require a context, returning an error if none specified.
///
/// In cloud mode (no local vault) we skip the vault-filesystem fallback
/// and trust the provided name directly — there are no context directories
/// on disk to check.
fn require_context(config: &Config, context: Option<&str>) -> Result<String> {
    use temper_core::types::config::VaultState;
    match context {
        Some(ctx) => {
            if matches!(VaultState::from_env(), VaultState::Cloud) {
                Ok(ctx.to_string())
            } else {
                Ok(super::resolve_context_with_fallback(config, ctx).into_owned())
            }
        }
        None => Err(TemperError::Project(
            "no context specified — use --context <name>".into(),
        )),
    }
}

/// Map current `VaultState` to the appropriate `Surface` origin for cmd construction.
pub(crate) fn surface_for_state() -> temper_core::operations::Surface {
    use temper_core::types::config::VaultState;
    match VaultState::from_env() {
        VaultState::Local => temper_core::operations::Surface::CliLocalVault,
        VaultState::Cloud => temper_core::operations::Surface::CliCloud,
    }
}

/// True iff `events` contains a `VaultFileWritten` event (local-mode indicator).
pub(crate) fn has_vault_file_event(events: &[temper_core::operations::DomainEvent]) -> bool {
    use temper_core::operations::DomainEvent;
    events
        .iter()
        .any(|e| matches!(e, DomainEvent::VaultFileWritten { .. }))
}

/// Extract the rel_path string from a `VaultFileWritten` event in the slice,
/// if any. Used by surfaces that emit discovery events with the vault path.
pub(crate) fn vault_file_path_from_events(
    events: &[temper_core::operations::DomainEvent],
) -> Option<String> {
    use temper_core::operations::DomainEvent;
    events.iter().find_map(|e| match e {
        DomainEvent::VaultFileWritten { path } => Some(path.clone()),
        _ => None,
    })
}

/// Render the result of `VaultBackend::create_resource` to stdout in the
/// shape that each doctype's pre-B5b dispatch path emitted.
///
/// Doctype-aware switch preserves backward-compatible JSON output. The
/// dispatch itself is now uniform; only output shape varies by doctype.
fn render_create_output(
    output: &temper_core::operations::CommandOutput<temper_core::types::resource::ResourceRow>,
    doc_type: &str,
    format: &str,
) -> Result<()> {
    let rendered = render_create_output_to_string(output, doc_type, format)?;
    if !rendered.is_empty() {
        println!("{rendered}");
    }
    Ok(())
}

/// Test-friendly core of `render_create_output` — returns the string that
/// would be printed (empty string for the non-JSON success path).
///
/// Audited per-doctype JSON shapes (preserved verbatim from pre-B5b dispatch,
/// confirmed 2026-05-14):
///
/// - Task: `{ "type": "task", "temper-slug", "temper-title", "temper-context" }`
///   Source: `commands::resource::create` match arm "task" lines 158–167.
///
/// - Goal: serialized `GoalInfo` → `{ "temper-title", "temper-slug",
///   "temper-context", "temper-seq" (Option<u32>), "temper-status" }`
///   Shape: `crate::actions::types::GoalInfo` (seq from ResourceRow.seq).
///
/// - Session: `{ "title", "context", "path", "date" }` where `path` is the
///   vault-relative file path (`{owner}/{context}/session/{slug}.md`) and
///   `date` is today's date (`%Y-%m-%d`).
///   Source: `commands::session::save`, inline `SessionCreated` struct.
///
/// - Research: `{ "title", "project", "path", "date", "id", "slug" }` where
///   `project` is the context name, `path` is vault-relative, `id` is the
///   resource UUID (string), `slug` is the date-prefixed slug.
///   Source: `commands::research::save`, `serde_json::json!` at line 84.
///
/// - Concept / Decision: serialized `ResourceCreated` →
///   `{ "doc_type", "title", "slug", "context", "path", "date", "id" }`
///   where `path` is vault-relative and `id` is the UUID (string).
///   Source: `create_simple_resource`, inline `ResourceCreated` struct.
fn render_create_output_to_string(
    output: &temper_core::operations::CommandOutput<temper_core::types::resource::ResourceRow>,
    doc_type: &str,
    format: &str,
) -> Result<String> {
    use temper_core::frontmatter::DocType;

    let row = &output.value;
    let doctype = DocType::from_str(doc_type)
        .map_err(|e| TemperError::Vault(format!("invalid doctype: {e}")))?;

    if format != "json" {
        // Non-JSON path: emit a "Created: <slug>" success line.
        let slug_display = row.slug.as_deref().unwrap_or("(no slug)");
        output::success(format!("Created: {slug_display}"));
        return Ok(String::new());
    }

    let today = Local::now().format("%Y-%m-%d").to_string();

    // Mode-implicit `path` field: emit only when a VaultFileWritten event is
    // present (local mode, real on-disk file). In cloud mode the synthesized
    // path would point at a nonexistent file — agents chaining
    // `temper resource create ... --format json | jq -r .path` and cat-ing
    // the result would hit ENOENT. Mirrors the surface-rendering pattern
    // used by update (commands::resource::update) and session::save.
    let vault_path = vault_file_path_from_events(&output.events);

    let json = match doctype {
        DocType::Task => serde_json::json!({
            "type": "task",
            "temper-slug": row.slug,
            "temper-title": row.title,
            "temper-context": row.context_name,
        }),
        DocType::Goal => {
            // Goal JSON is the GoalInfo serialization shape. `status` is always
            // "active" at create time. `seq` comes from ResourceRow.seq (i64 →
            // u32 lossy-cast; matches the i64 stored in managed_meta.seq).
            serde_json::json!({
                "temper-title": row.title,
                "temper-slug": row.slug,
                "temper-context": row.context_name,
                "temper-seq": row.seq.and_then(|s| u32::try_from(s).ok()),
                "temper-status": "active",
            })
        }
        DocType::Session => {
            // Session JSON: title, context, vault-relative path (local only),
            // today's date.
            serde_json::json!({
                "title": row.title,
                "context": row.context_name,
                "path": vault_path.as_deref().unwrap_or(""),
                "date": today,
            })
        }
        DocType::Research => {
            // Research JSON: title, project (=context), vault-relative path
            // (local only), today's date, id (UUID string), slug.
            let slug = row.slug.as_deref().unwrap_or("");
            serde_json::json!({
                "title": row.title,
                "project": row.context_name,
                "path": vault_path.as_deref().unwrap_or(""),
                "date": today,
                "id": row.id.to_string(),
                "slug": slug,
            })
        }
        DocType::Concept | DocType::Decision => {
            // Concept/Decision JSON: doc_type, title, slug, context, vault-relative
            // path (local only), today's date, id (UUID string).
            let slug = row.slug.as_deref().unwrap_or("");
            serde_json::json!({
                "doc_type": row.doc_type_name,
                "title": row.title,
                "slug": slug,
                "context": row.context_name,
                "path": vault_path.as_deref().unwrap_or(""),
                "date": today,
                "id": row.id.to_string(),
            })
        }
    };

    let s = serde_json::to_string_pretty(&json)
        .map_err(|e| TemperError::Vault(format!("json render failed: {e}")))?;
    Ok(s)
}

/// Create a new resource.
#[expect(
    clippy::too_many_arguments,
    reason = "per-doctype creators have different required args; params struct deferred to Task 12+"
)]
pub fn create(
    config: &Config,
    doc_type: &str,
    title: &str,
    context: Option<&str>,
    goal: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    slug: Option<&str>,
    body_flag: Option<String>,
    format: &str,
) -> Result<()> {
    use std::io::IsTerminal;

    use temper_core::types::ManagedMeta;

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let ctx = require_context(config, context)?;

    // Body resolution — both modes use --body flag + stdin pipe.
    let stdin_is_tty = std::io::stdin().is_terminal();
    let body_opt = crate::actions::body_source::resolve_body_source(
        body_flag.as_deref(),
        stdin_is_tty,
        std::io::stdin(),
    )?;

    // Slug derivation (mode-independent — Concept and Goal skip date prefix).
    let doctype_enum = temper_core::frontmatter::DocType::from_str(doc_type)?;
    let slug_resolved = slug.map(String::from).unwrap_or_else(|| {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let base_slug = vault::slugify(title);
        match doctype_enum {
            // Concept and Goal are identified by name — no date prefix.
            temper_core::frontmatter::DocType::Concept
            | temper_core::frontmatter::DocType::Goal => base_slug,
            // All other doctypes get a date prefix.
            _ => format!("{today}-{base_slug}"),
        }
    });

    // Build the CreateResource cmd. Body-None when no body input; both backends
    // know how to handle the empty case (VaultBackend writes the doctype template,
    // CloudBackend synthesizes `# {title}\n` in its translator).
    let cmd = temper_core::operations::CreateResource {
        slug: slug_resolved,
        doctype: doc_type.to_string(),
        context: ctx.to_string(),
        title: title.to_string(),
        body: body_opt.filter(|b| !b.is_empty()).map(|content| {
            temper_core::operations::BodyUpdate {
                content,
                content_hash: None,
                chunks_packed: None,
            }
        }),
        managed_meta: ManagedMeta {
            mode: mode.map(String::from),
            effort: effort.map(String::from),
            goal: goal.map(String::from),
            ..ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: surface_for_state(),
    };

    // Surface-side pre-flight validation — mirrors the hoist of
    // `validate_update_args` for update. Without this, cloud-mode create would
    // skip `validate_create` entirely (CloudBackend has no equivalent), and
    // bad inputs (e.g., --mode plan-or-build whitelist violations) would ship
    // a doomed request to the server. VaultBackend runs `validate_create` as
    // its first step (vault_backend.rs:484) — hoisting here makes both modes
    // symmetric and lets local-mode fail-fast benefit cloud-mode too.
    temper_core::operations::validate_create(&cmd)
        .map_err(|e| TemperError::BadRequest(e.to_string()))?;

    // Acquire backend (mode picked via VaultState::from_env) and dispatch.
    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.create_resource(cmd))?;

    // Discovery event (local mode only — gated on VaultFileWritten presence).
    // Concept and Decision were never emitted pre-Phase 5; preserve that parity.
    if !matches!(
        doctype_enum,
        temper_core::frontmatter::DocType::Concept | temper_core::frontmatter::DocType::Decision
    ) && has_vault_file_event(&output.events)
    {
        let rel_path = vault_file_path_from_events(&output.events).unwrap_or_default();
        let event = Event::ResourceCreate {
            ts: Local::now().to_rfc3339(),
            doc_type: doc_type.to_string(),
            title: title.to_string(),
            path: rel_path,
            context: ctx.to_string(),
        };
        if let Err(e) = discovery::append_event(&config.state_dir, &event) {
            tracing::warn!("Failed to append discovery event: {e}");
        }
    }

    render_create_output(&output, doc_type, format)
}

// ---------------------------------------------------------------------------
// Unified resource list pipeline
// ---------------------------------------------------------------------------

/// A single resource row used by the unified list pipeline.
///
/// Carries the full frontmatter (for JSON output and column extraction) plus
/// the vault-relative path for debugging / future display needs.
#[derive(Debug, Clone)]
pub struct ResourceRow {
    /// Full parsed frontmatter as a JSON `Value`.
    pub frontmatter: serde_json::Value,
    /// Vault-relative path to the source markdown file.
    pub path: String,
}

impl ResourceRow {
    fn updated_at(&self) -> &str {
        self.frontmatter
            .get("temper-updated")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    #[cfg(test)]
    pub fn slug_for_tests(&self) -> String {
        self.frontmatter
            .get("temper-slug")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
}

/// Filters applied after scanning and parsing rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListFilters<'a> {
    pub stage: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub status: Option<&'a str>,
}

/// Parameters for the public `list` command, bundled to keep the CLI entry
/// signature compact (and clippy happy).
#[derive(Debug, Clone, Copy)]
pub struct ListParams<'a> {
    pub doc_type: &'a str,
    pub context: Option<&'a str>,
    pub limit: Option<usize>,
    pub stage: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub status: Option<&'a str>,
    pub format: &'a str,
}

/// Parameters for `render_list`, the testable core of the unified list
/// pipeline. Bundling the inputs keeps the function under the project's
/// positional-argument limit and makes test call sites read naturally.
#[derive(Debug, Clone, Copy)]
pub struct RenderListParams<'a> {
    pub doc_type: &'a str,
    pub config: &'a Config,
    pub context: Option<&'a str>,
    pub limit: Option<usize>,
    pub filters: ListFilters<'a>,
    pub format: OutputFormat,
}

/// Scan disk for all resources of `doc_type`, optionally restricted to one
/// context. Returns one `ResourceRow` per `.md` file that has valid
/// frontmatter.
pub fn scan_rows(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
) -> Result<Vec<ResourceRow>> {
    let contexts_to_scan: Vec<String> = match context {
        Some(c) => vec![c.to_string()],
        None => config.contexts.clone(),
    };

    let mut rows = Vec::new();
    let vault_layout = Vault::new(&config.vault_root);
    for ctx in &contexts_to_scan {
        let owner = config.owner_for_context(ctx);
        let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| TemperError::Vault(e.to_string()))? {
            let entry = entry.map_err(|e| TemperError::Vault(e.to_string()))?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            if let Some(row) = parse_row(&path, &config.vault_root)? {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

/// Read one markdown file and convert its YAML frontmatter into a
/// `ResourceRow`. Files without valid frontmatter are skipped (`Ok(None)`).
fn parse_row(path: &std::path::Path, vault_root: &std::path::Path) -> Result<Option<ResourceRow>> {
    let content = std::fs::read_to_string(path).map_err(|e| TemperError::Vault(e.to_string()))?;
    let Ok(fm) = temper_core::frontmatter::Frontmatter::try_from(content.as_str()) else {
        return Ok(None);
    };
    // Convert YAML value -> JSON value so downstream code can use
    // the existing `serde_json::Value`-based column registry.
    let frontmatter = serde_json::to_value(fm.value())
        .map_err(|e| TemperError::Vault(format!("frontmatter YAML→JSON: {e}")))?;
    let relative = path.strip_prefix(vault_root).unwrap_or(path);
    Ok(Some(ResourceRow {
        frontmatter,
        path: relative.to_string_lossy().to_string(),
    }))
}

/// Sort rows by `temper-updated` descending (most recent first). Rows missing
/// the field sort to the end.
pub fn sort_rows(rows: &mut [ResourceRow]) {
    rows.sort_by(|a, b| b.updated_at().cmp(a.updated_at()));
}

/// Apply stage/goal/status filters, dropping rows that don't match.
pub fn filter_rows(rows: Vec<ResourceRow>, filters: ListFilters<'_>) -> Vec<ResourceRow> {
    rows.into_iter()
        .filter(|row| match_filters(row, &filters))
        .collect()
}

fn match_filters(row: &ResourceRow, filters: &ListFilters<'_>) -> bool {
    if let Some(stage) = filters.stage {
        if row.frontmatter.get("temper-stage").and_then(|v| v.as_str()) != Some(stage) {
            return false;
        }
    }
    if let Some(goal) = filters.goal {
        if row.frontmatter.get("temper-goal").and_then(|v| v.as_str()) != Some(goal) {
            return false;
        }
    }
    if let Some(status) = filters.status {
        if row
            .frontmatter
            .get("temper-status")
            .and_then(|v| v.as_str())
            != Some(status)
        {
            return false;
        }
    }
    true
}

/// Cloud-first list: call the server, return rows sorted server-side
/// (`ORDER BY updated DESC`).
async fn fetch_list_rows(
    client: &temper_client::TemperClient,
    doc_type: &str,
    context: Option<&str>,
    limit: usize,
) -> Result<Vec<temper_core::types::resource::ResourceRow>> {
    use temper_core::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

    let params = ResourceListParams {
        doc_type_name: Some(doc_type.to_string()),
        context_name: context.map(ToString::to_string),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        ..Default::default()
    };
    let resp = client
        .resources()
        .list(&params)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    Ok(resp.rows)
}

/// Map a server `ResourceRow` to the frontmatter-shaped `serde_json::Value`
/// that `col_registry::extract_row` expects. The registry was built for
/// local scan_rows output; we adapt the server row shape to the same
/// keys so rendering is unchanged.
fn row_to_frontmatter_value(row: &temper_core::types::resource::ResourceRow) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "temper-title".into(),
        serde_json::Value::String(row.title.clone()),
    );
    if let Some(slug) = &row.slug {
        map.insert(
            "temper-slug".into(),
            serde_json::Value::String(slug.clone()),
        );
    }
    map.insert(
        "temper-updated".into(),
        serde_json::Value::String(row.updated.to_rfc3339()),
    );
    map.insert(
        "temper-context".into(),
        serde_json::Value::String(row.context_name.clone()),
    );
    map.insert(
        "temper-type".into(),
        serde_json::Value::String(row.doc_type_name.clone()),
    );
    if let Some(stage) = &row.stage {
        map.insert(
            "temper-stage".into(),
            serde_json::Value::String(stage.clone()),
        );
    }
    if let Some(mode) = &row.mode {
        map.insert(
            "temper-mode".into(),
            serde_json::Value::String(mode.clone()),
        );
    }
    if let Some(effort) = &row.effort {
        map.insert(
            "temper-effort".into(),
            serde_json::Value::String(effort.clone()),
        );
    }
    if let Some(seq) = row.seq {
        map.insert("temper-seq".into(), serde_json::Value::Number(seq.into()));
    }
    serde_json::Value::Object(map)
}

/// Render server rows using the same per-doctype column registry used
/// by the local-mode `render_list`. This keeps table output shape
/// stable between the two modes.
fn render_server_rows(
    doc_type: &str,
    rows: &[temper_core::types::resource::ResourceRow],
    format: OutputFormat,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(
            &rows
                .iter()
                .map(row_to_frontmatter_value)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default()),
        OutputFormat::Pretty | OutputFormat::NoTty => {
            let columns = col_registry::display_columns(doc_type);
            if columns.is_empty() || rows.is_empty() {
                return Ok(String::new());
            }
            let mut renderer = TableRenderer::new(columns.clone());
            for row in rows {
                let fm_value = row_to_frontmatter_value(row);
                renderer.push_row(col_registry::extract_row(&fm_value, &columns));
            }
            Ok(if format == OutputFormat::Pretty {
                renderer.render_pretty()
            } else {
                renderer.render_no_tty()
            })
        }
    }
}

/// Render the unified list pipeline to a `String` for the given format.
///
/// This is the testable core used by both `list()` (CLI entry) and the
/// integration tests below.
///
/// Empty-result handling: JSON always emits `[]` (machine-friendly),
/// Pretty/NoTty return an empty string so the caller (`list()`) can surface
/// a user-friendly "No X resources found" hint instead of a bare header row.
pub fn render_list(params: &RenderListParams<'_>) -> Result<String> {
    let _ = temper_core::frontmatter::DocType::from_str(params.doc_type)?;
    // Filter first, then sort — sorting unfiltered rows wastes work on rows
    // we're about to discard.
    let rows = scan_rows(params.config, params.doc_type, params.context)?;
    let mut rows = filter_rows(rows, params.filters);
    sort_rows(&mut rows);
    rows.truncate(params.limit.unwrap_or(20));

    match params.format {
        OutputFormat::Json => {
            let frontmatters: Vec<&serde_json::Value> =
                rows.iter().map(|r| &r.frontmatter).collect();
            Ok(serde_json::to_string_pretty(&frontmatters).unwrap_or_default())
        }
        OutputFormat::Pretty | OutputFormat::NoTty => {
            // Empty result → return empty string so the CLI entry can render
            // a "No X resources found" hint instead of a header-only table.
            if rows.is_empty() {
                return Ok(String::new());
            }
            let columns = col_registry::display_columns(params.doc_type);
            if columns.is_empty() {
                return Ok(String::new());
            }
            let mut renderer = TableRenderer::new(columns.clone());
            for row in &rows {
                renderer.push_row(col_registry::extract_row(&row.frontmatter, &columns));
            }
            if params.format == OutputFormat::Pretty {
                Ok(renderer.render_pretty())
            } else {
                Ok(renderer.render_no_tty())
            }
        }
    }
}

/// List resources of a given type (unified pipeline for all doc types).
pub fn list(config: &Config, params: ListParams<'_>) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::config::VaultState;

    // Hints for filters that only apply to certain types (unchanged).
    if params.stage.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--stage filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.goal.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--goal filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.status.is_some() && params.doc_type != "goal" {
        output::hint(format!(
            "--status filter is only meaningful for goals; ignored for {}.",
            params.doc_type
        ));
    }

    if let Some(s) = params.stage {
        if params.doc_type == "task" {
            vault::validate_stage(s)?;
        }
    }

    let format = OutputFormat::parse(params.format);
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let limit = params.limit.unwrap_or(20);

    let vault_state = VaultState::from_env();

    let state_dir = config.state_dir.clone();

    // Attempt server-first. Fall back to local scan on network error
    // in Local mode only; Cloud mode surfaces the error.
    let rows_result = runtime::with_client(move |client| {
        Box::pin(async move {
            if let Some(ctx) = context.as_deref() {
                crate::projection::warn_if_context_stale(client, &state_dir, ctx).await;
            }
            fetch_list_rows(client, &doc_type, context.as_deref(), limit).await
        })
    });

    let server_rows = match (rows_result, vault_state) {
        (Ok(rows), _) => Some(rows),
        // Local mode: fall back to the local vault scan only when the server
        // is unreachable. Server-originated errors (4xx/5xx, auth) surface
        // as-is — silently masking them with stale local data would hide
        // real problems.
        (Err(e @ TemperError::Network(_)), VaultState::Local) => {
            output::hint(format!(
                "cloud unreachable: {e}. Falling back to local scan."
            ));
            None
        }
        (Err(e), _) => return Err(e),
    };

    let body = match server_rows {
        Some(rows) => render_server_rows(params.doc_type, &rows, format)?,
        None => render_list(&RenderListParams {
            doc_type: params.doc_type,
            config,
            context: params.context,
            limit: params.limit,
            filters: ListFilters {
                stage: if params.doc_type == "task" {
                    params.stage
                } else {
                    None
                },
                goal: if params.doc_type == "task" {
                    params.goal
                } else {
                    None
                },
                status: if params.doc_type == "goal" {
                    params.status
                } else {
                    None
                },
            },
            format,
        })?,
    };

    if body.trim().is_empty() {
        output::hint(format!("No {} resources found.", params.doc_type));
        return Ok(());
    }

    output::plain(body.trim_end());
    Ok(())
}

/// Delete a resource.
///
/// `--force` skips the interactive confirmation prompt for the local-file
/// removal. In non-TTY contexts (agents, CI), `--force` is required in
/// local mode because we won't read confirmation from a non-terminal stdin.
/// Cloud mode is non-interactive — no prompt regardless of `--force`.
///
/// Cloud-first ordering: API failure means no local mutation in either mode
/// (enforced inside `VaultBackend::delete_resource`; CloudBackend has only the
/// API step).
pub fn delete(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    force: bool,
) -> Result<()> {
    use std::io::IsTerminal;

    use temper_core::operations::{DeleteResource, DomainEvent, ResourceRef};
    use temper_core::types::config::VaultState;

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let ctx = require_context(config, context)?;

    // Local-mode UX gate: non-TTY guard + [y/N] prompt. Cloud mode skips
    // this — non-interactive by design (no local file to remove).
    let in_local_mode = matches!(VaultState::from_env(), VaultState::Local);
    if in_local_mode {
        if !force && !std::io::stdin().is_terminal() {
            return Err(TemperError::Vault(
                "non-interactive stdin detected; pass --force to skip the local-file confirmation"
                    .to_string(),
            ));
        }

        if !force {
            output::progress(format!("Delete {doc_type}/{slug}? [y/N] "));
            use std::io::Write as _;
            std::io::stderr().flush().ok();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            if !input.trim().eq_ignore_ascii_case("y") {
                return Ok(());
            }
        }
    }

    let owner = config.owner_for_context(&ctx);
    let cmd = DeleteResource {
        resource: ResourceRef::scoped(owner, &ctx, doc_type, slug),
        force,
        origin: surface_for_state(),
    };

    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.delete_resource(cmd))?;

    // Translate events into surface output. Cloud-first ordering inside the
    // backend guarantees `RemoteSynced` precedes any vault events when both
    // are emitted, so a single linear scan is order-correct.
    //
    // Local mode emits RemoteSynced + VaultFileRemoved + VaultManifestUpdated;
    // cloud mode emits RemoteSynced only. Same loop, both shapes.
    //
    // The "(cloud)" suffix is mode-implicit via event presence: append only
    // when no VaultFileRemoved is also emitted, otherwise the local-mode
    // output ("Deleted X (cloud)\nRemoved vault file: ...") would
    // contradict itself.
    let has_vault_removed = output
        .events
        .iter()
        .any(|e| matches!(e, DomainEvent::VaultFileRemoved { .. }));
    for event in &output.events {
        match event {
            DomainEvent::RemoteSynced { .. } => {
                let suffix = if has_vault_removed { "" } else { " (cloud)" };
                self::output::success(format!("Deleted {doc_type}/{slug}{suffix}"));
            }
            DomainEvent::VaultFileRemoved { path } => {
                self::output::dim(format!("Removed vault file: {path}"));
            }
            DomainEvent::VaultManifestUpdated { .. } => {
                // Internal bookkeeping — not surfaced.
            }
            _ => {}
        }
    }

    Ok(())
}

/// Show a resource's content.
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    edges: bool,
) -> Result<()> {
    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format),
        "session" => crate::commands::session::show(config, slug, context, format),
        _ => show_generic(config, doc_type, slug, context, format),
    }?;

    if edges {
        show_edges(slug, format)?;
    }

    Ok(())
}

/// API fallback for `temper resource show` when the local vault file is
/// missing in local mode. Resolves the resource id via
/// `GET /api/resources/by-uri`, fetches body via
/// `GET /api/resources/{id}/content`, and prints it. Does not write to the
/// vault — recovery to disk is `temper sync run`'s job.
///
/// Distinguishes `TemperError::Network(_)` (couldn't reach server) from
/// `TemperError::Api(_)` (server confirms not-found) so the caller sees
/// a clearer error than the prior local-only `"<doctype> not found"`.
pub(crate) fn show_via_api_fallback(
    config: &Config,
    doc_type: &str,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    let ctx = context.map(str::to_string);
    let slug = slug_or_suffix.to_string();
    let dt = doc_type.to_string();
    let config_clone = config.clone();
    let format_owned = format.to_string();

    let body = runtime::with_client(|client| {
        Box::pin(async move {
            // Local-mode: try fast-path via local file frontmatter / manifest first,
            // then fall back to API resolution.
            let id = resolve_id_local_first(&config_clone, client, ctx.as_deref(), &dt, &slug)
                .await
                .map_err(|e| match e {
                    TemperError::Network(msg) => TemperError::Vault(format!(
                        "couldn't reach server to verify resource exists; \
                         offline lookup failed for {slug}: {msg}"
                    )),
                    _ => TemperError::Vault(format!("{dt} not found locally or on server: {slug}")),
                })?;
            let content = client
                .resources()
                .content(*id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
                .map_err(|e| match e {
                    TemperError::Network(msg) => TemperError::Vault(format!(
                        "couldn't reach server to fetch body for {slug}: {msg}"
                    )),
                    _ => TemperError::Vault(format!("{dt} not found locally or on server: {slug}")),
                })?;

            if format_owned == "json" {
                Ok(serde_json::to_string_pretty(&content)
                    .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?)
            } else {
                Ok(content.markdown)
            }
        })
    })?;

    print!("{body}");
    Ok(())
}

/// Resolve a resource to its [`temper_core::types::ids::ResourceId`] by trying the local manifest/
/// frontmatter first, then falling back to a cloud `GET /api/resources/by-uri`
/// call.
///
/// The local fast-path is best-effort: if `doc_type` fails to parse, or if
/// `find_resource` finds no match, the call falls through to the API without
/// returning an error. An error is only returned when both paths fail.
///
/// `context` must be `Some` for the API fallback; the call returns
/// `TemperError::Project("no context specified …")` when it is `None` and the
/// local path also found nothing.
///
/// Error mapping uses [`crate::actions::runtime::client_err_to_temper`] so
/// network failures surface as `TemperError::Network` and server-side
/// not-found responses surface as `TemperError::Api`. Call sites that need a
/// different error shape (e.g. `TemperError::Vault`) should `map_err` on the
/// returned `Result`.
pub(crate) async fn resolve_id_local_first(
    config: &Config,
    client: &temper_client::TemperClient,
    context: Option<&str>,
    doc_type: &str,
    slug: &str,
) -> crate::error::Result<temper_core::types::ids::ResourceId> {
    let local_id = temper_core::frontmatter::DocType::from_str(doc_type)
        .ok()
        .and_then(|dt| {
            crate::lookup::find_resource(crate::lookup::FindableResource {
                config,
                manifest: None,
                owner: None,
                context: context.map(str::to_string),
                doc_type: dt,
                slug_or_suffix: slug.to_string(),
            })
            .ok()
            .and_then(|r| r.resource_id)
        });

    if let Some(id) = local_id {
        return Ok(id);
    }

    let ctx = context.ok_or_else(|| {
        TemperError::Project("no context specified — use --context <name>".into())
    })?;
    let owner = config.owner_for_context(ctx);
    client
        .resources()
        .resolve_by_uri(&owner, ctx, doc_type, slug)
        .await
        .map(|row| row.id)
        .map_err(crate::actions::runtime::client_err_to_temper)
}

/// Return the existing local path for a resource if found, or compute where
/// it would live based on `Vault::doc_file`.
fn find_or_compute_local_path(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
) -> Result<(std::path::PathBuf, String)> {
    if let Ok(dt) = temper_core::frontmatter::DocType::from_str(doc_type) {
        if let Ok(resolved) = crate::lookup::find_resource(crate::lookup::FindableResource {
            config,
            manifest: None,
            owner: None,
            context: context.map(str::to_string),
            doc_type: dt,
            slug_or_suffix: slug.to_string(),
        }) {
            return Ok((resolved.path, resolved.context));
        }
    }
    let ctx = require_context(config, context)?;
    let owner = config.owner_for_context(&ctx);
    let vault_layout = Vault::new(&config.vault_root);
    let path = vault_layout.doc_file(&owner, &ctx, doc_type, slug);
    Ok((path, ctx))
}

/// Render generic resource output in the requested format.
///
/// `local_path` is `None` in Cloud mode (no file on disk).
fn render_generic_output(
    doc_type: &str,
    slug: &str,
    context: &str,
    config: &Config,
    local_path: Option<&std::path::Path>,
    body: String,
    format: &str,
) -> Result<()> {
    if format == "json" {
        let fm = temper_core::frontmatter::Frontmatter::try_from(body.as_str()).ok();
        let title = fm
            .as_ref()
            .and_then(|f| f.value().get("temper-title"))
            .and_then(|v| v.as_str())
            .unwrap_or(slug);
        let path_str = local_path
            .and_then(|p| p.strip_prefix(&config.vault_root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        #[derive(Serialize)]
        struct ResourceShow<'a> {
            doc_type: &'a str,
            slug: &'a str,
            title: &'a str,
            context: &'a str,
            path: String,
            content: String,
        }
        let info = ResourceShow {
            doc_type,
            slug,
            title,
            context,
            path: path_str,
            content: body,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    print!("{body}");
    Ok(())
}

/// Show a generic resource (goal, research, concept, decision).
///
/// In Local mode: resolves an id from frontmatter or by-uri, then uses the
/// three-tier freshness ladder (`show_cache::fetch`) before rendering.
/// In Cloud mode: fetches content directly from the API with no disk write.
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::{runtime, show_cache};
    use std::time::Duration;
    use temper_core::types::VaultState;

    let vault_state = VaultState::from_env();
    let doc_type_s = doc_type.to_string();
    let slug_s = slug.to_string();
    let context_owned = context.map(str::to_string);
    let format_s = format.to_string();

    match vault_state {
        VaultState::Cloud => {
            let config_clone = config.clone();
            let doc_type_inner = doc_type_s.clone();
            let slug_inner = slug_s.clone();
            let ctx_inner = context_owned.clone();

            let body = runtime::with_client(|client| {
                Box::pin(async move {
                    let ctx = ctx_inner
                        .as_deref()
                        .ok_or_else(|| {
                            TemperError::Project(
                                "no context specified — use --context <name>".into(),
                            )
                        })?
                        .to_string();
                    let owner = config_clone.owner_for_context(&ctx);
                    let row = client
                        .resources()
                        .resolve_by_uri(&owner, &ctx, &doc_type_inner, &slug_inner)
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;
                    let resp = client
                        .resources()
                        .content(*row.id.as_uuid())
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;

                    // Per-resource projection refresh: write the fetched
                    // resource to its canonical projection path. Best-effort
                    // — a write failure must not stop `show` from displaying.
                    if let Err(e) = crate::projection::write_resource_file_from_parts(
                        &config_clone.vault_root,
                        &row,
                        &resp,
                    ) {
                        crate::output::warning(format!(
                            "could not refresh projection file for '{slug_inner}': {e}"
                        ));
                    }

                    Ok(resp.markdown)
                })
            })?;

            let ctx = context_owned.unwrap_or_default();
            render_generic_output(&doc_type_s, &slug_s, &ctx, config, None, body, &format_s)
        }
        VaultState::Local => {
            let config_clone = config.clone();
            let doc_type_inner = doc_type_s.clone();
            let slug_inner = slug_s.clone();
            let ctx_inner = context_owned.clone();

            // If the local file isn't resolvable on disk at all, route to
            // the API fallback so the vault stays untouched. The
            // show_cache path below would write the rebuilt file as
            // tier-3 — that's correct when the file *was* present but
            // stale, but cloud-only resources should not be materialized
            // implicitly. Recovery to disk is `temper sync run`'s job.
            if temper_core::frontmatter::DocType::from_str(&doc_type_s).is_ok()
                && crate::lookup::find_resource(crate::lookup::FindableResource {
                    config,
                    manifest: None,
                    owner: None,
                    context: context.map(str::to_string),
                    doc_type: temper_core::frontmatter::DocType::from_str(&doc_type_s)?,
                    slug_or_suffix: slug_s.clone(),
                })
                .is_err()
            {
                return show_via_api_fallback(config, &doc_type_s, &slug_s, context, &format_s);
            }

            let (path, ctx) = find_or_compute_local_path(config, &doc_type_s, &slug_s, context)?;

            // Tier 0: debounce check before spinning up the runtime.
            // If the file is fresh we serve it immediately; no network, no
            // resource id resolution needed.
            if let Some(body) = show_cache::read_if_fresh(
                &path,
                std::time::Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
            )? {
                return render_generic_output(
                    &doc_type_s,
                    &slug_s,
                    &ctx,
                    config,
                    Some(&path),
                    body,
                    &format_s,
                );
            }

            let (body, local_path_for_render) = runtime::with_client(|client| {
                Box::pin(async move {
                    // Local-mode: try fast-path via local file frontmatter / manifest first,
                    // then fall back to API resolution.
                    let id = resolve_id_local_first(
                        &config_clone,
                        client,
                        ctx_inner.as_deref(),
                        &doc_type_inner,
                        &slug_inner,
                    )
                    .await?;
                    let result = show_cache::fetch(show_cache::ShowCacheParams {
                        client,
                        resource_id: id,
                        local_path: &path,
                        debounce: Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
                    })
                    .await?;
                    Ok((result.content, path))
                })
            })?;

            render_generic_output(
                &doc_type_s,
                &slug_s,
                &ctx,
                config,
                Some(&local_path_for_render),
                body,
                &format_s,
            )
        }
    }
}

/// Fetch and display edges for a resource via the API.
fn show_edges(slug: &str, format: &str) -> Result<()> {
    use crate::actions::runtime;

    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let resource_id = manifest
        .entries
        .iter()
        .find(|(_, entry)| {
            entry
                .path
                .strip_suffix(".md")
                .and_then(|p| p.rsplit('/').next())
                == Some(slug)
        })
        .map(|(id, _)| uuid::Uuid::from(*id))
        .ok_or_else(|| {
            TemperError::Vault(format!(
                "resource '{slug}' not found in manifest — sync first to use --edges"
            ))
        })?;

    let edges: Vec<temper_core::types::graph::GraphEdgeRow> = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .edges(resource_id)
                .await
                .map_err(crate::commands::client_err)
        })
    })?;

    if edges.is_empty() {
        if format != "json" {
            println!("\nEdges: (none)");
        }
        return Ok(());
    }

    if format == "json" {
        let json = serde_json::to_string_pretty(&edges).unwrap_or_default();
        println!("{json}");
    } else {
        println!("\nEdges:");
        let outgoing: Vec<_> = edges.iter().filter(|e| e.direction == "outgoing").collect();
        let incoming: Vec<_> = edges.iter().filter(|e| e.direction == "incoming").collect();

        if !outgoing.is_empty() {
            println!("  outgoing:");
            for e in &outgoing {
                println!(
                    "    {} \u{2192} {} ({})",
                    e.label, e.peer_slug, e.peer_title
                );
            }
        }
        if !incoming.is_empty() {
            println!("  incoming:");
            for e in &incoming {
                println!(
                    "    {} \u{2190} {} ({})",
                    e.label, e.peer_slug, e.peer_title
                );
            }
        }
    }

    Ok(())
}

// `find_resource_file` retired in favor of `crate::lookup::find_resource`
// (typed DocType, owner-aware, manifest-aware id resolution, and no
// slugify-collapse on input — closes C.1 from the 2026-05-09 audit sweep).

/// Parameters for resource update.
pub struct UpdateParams<'a> {
    pub slug: &'a str,
    pub doc_type: Option<&'a str>,
    pub type_from: Option<&'a str>,
    pub type_to: Option<&'a str>,
    pub context: Option<&'a str>,
    pub context_to: Option<&'a str>,
    // Base schema fields
    pub title: Option<&'a str>,
    pub tags: &'a [String],
    pub aliases: &'a [String],
    pub relates_to: &'a [String],
    pub references: &'a [String],
    pub depends_on: &'a [String],
    pub extends: &'a [String],
    pub preceded_by: &'a [String],
    pub derived_from: &'a [String],
    // Task-specific fields
    pub stage: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub seq: Option<i64>,
    pub branch: Option<&'a str>,
    pub pr: Option<&'a str>,
    // Goal-specific fields
    pub status: Option<&'a str>,
    /// Body source flag: `None` (rely on stdin auto-detection — non-empty piped
    /// stdin updates the body; empty implicit stdin means no body update),
    /// `Some("-")` (explicit stdin; errors if empty), or `Some("@<path>")`
    /// (read from file; errors if empty). Applies in both local and cloud mode.
    pub body: Option<String>,
}

/// Build a partial `ManagedMeta` from update CLI flags. Returns `None` if no
/// managed-meta-mutating flags were passed.
///
/// `title` is a managed-meta scalar (it lands as `temper-title` in
/// frontmatter); B4 added it here so the surface-side dispatch can hand a
/// partial `ManagedMeta` to the backend's `apply_updates` translator without
/// dropping bare `--title` updates on the floor.
fn build_partial_managed_meta_from_args(
    params: &UpdateParams<'_>,
) -> Option<temper_core::types::ManagedMeta> {
    let any_set = params.title.is_some()
        || params.stage.is_some()
        || params.mode.is_some()
        || params.effort.is_some()
        || params.goal.is_some()
        || params.seq.is_some()
        || params.branch.is_some()
        || params.pr.is_some()
        || params.status.is_some();
    if !any_set {
        return None;
    }
    Some(temper_core::types::ManagedMeta {
        title: params.title.map(String::from),
        stage: params.stage.map(String::from),
        mode: params.mode.map(String::from),
        effort: params.effort.map(String::from),
        goal: params.goal.map(String::from),
        seq: params.seq,
        branch: params.branch.map(String::from),
        pr: params.pr.map(String::from),
        status: params.status.map(String::from),
        ..Default::default()
    })
}

/// Build a partial `open_meta` JSON object from update CLI list flags. Returns
/// `None` if no open-meta list flags were passed.
fn build_partial_open_meta_from_args(params: &UpdateParams<'_>) -> Option<serde_json::Value> {
    let mut obj = serde_json::Map::new();
    if !params.tags.is_empty() {
        obj.insert("tags".to_string(), serde_json::json!(params.tags));
    }
    if !params.aliases.is_empty() {
        obj.insert("aliases".to_string(), serde_json::json!(params.aliases));
    }
    if !params.relates_to.is_empty() {
        obj.insert(
            "relates-to".to_string(),
            serde_json::json!(params.relates_to),
        );
    }
    if !params.references.is_empty() {
        obj.insert(
            "references".to_string(),
            serde_json::json!(params.references),
        );
    }
    if !params.depends_on.is_empty() {
        obj.insert(
            "depends-on".to_string(),
            serde_json::json!(params.depends_on),
        );
    }
    if !params.extends.is_empty() {
        obj.insert("extends".to_string(), serde_json::json!(params.extends));
    }
    if !params.preceded_by.is_empty() {
        obj.insert(
            "preceded-by".to_string(),
            serde_json::json!(params.preceded_by),
        );
    }
    if !params.derived_from.is_empty() {
        obj.insert(
            "derived-from".to_string(),
            serde_json::json!(params.derived_from),
        );
    }
    if obj.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(obj))
    }
}

/// Build a `MoveSpec` from `--type-to` / `--context-to` CLI flags. Returns
/// `None` when neither is set; otherwise returns `Some` with whichever fields
/// were provided. Translates CLI move flags into the `UpdateResource.move_to`
/// operations field.
fn build_move_spec_from_args(
    params: &UpdateParams<'_>,
) -> Option<temper_core::operations::MoveSpec> {
    if params.context_to.is_none() && params.type_to.is_none() {
        return None;
    }
    Some(temper_core::operations::MoveSpec {
        context_to: params.context_to.map(String::from),
        type_to: params.type_to.map(String::from),
    })
}

/// Update a resource's frontmatter fields.
///
/// Surface responsibilities:
///
/// 1. Doctype + type-to validation (clap-side polish; produces a friendlier
///    error than letting `validate_update` surface a `BadRequest`).
/// 2. Per-flag schema validation against `schema::updatable_fields` —
///    rejects bad enum values (e.g. `--stage frobnicate`) with a
///    user-targeted message before the operations layer ever sees them.
/// 3. `--body` flag resolution (stdin/file → `Option<String>`).
/// 4. Build an `UpdateResource` command and dispatch through `build_backend`.
/// 5. Render output — mode-implicit via event presence:
///    - Local (VaultFileWritten present): `"Updated: {rel_path}"` to stderr +
///      `ResourceUpdate` discovery event.
///    - Cloud (no VaultFileWritten): JSON `{"temper-slug": ..., "content_hash": ...}`
///      to stdout. Agent-workflow contract per CLAUDE.md (show-edit-cat cycle).
pub fn update(config: &Config, params: &UpdateParams<'_>) -> Result<()> {
    use std::io::IsTerminal;

    use temper_core::operations::{BodyUpdate, ResourceRef, UpdateResource};

    // 1. Resolve current type from --type or --type-from (one is required).
    let current_type = params
        .doc_type
        .or(params.type_from)
        .ok_or_else(|| TemperError::Project("--type or --type-from is required".into()))?;
    let _ = temper_core::frontmatter::DocType::from_str(current_type)?;
    if let Some(tt) = params.type_to {
        let _ = temper_core::frontmatter::DocType::from_str(tt)?;
    }

    // 2. Per-flag schema validation.
    validate_update_args(params, current_type)?;

    // 3. --body resolution.
    let stdin_is_tty = std::io::stdin().is_terminal();
    let resolved_body = crate::actions::body_source::resolve_body_source(
        params.body.as_deref(),
        stdin_is_tty,
        std::io::stdin(),
    )?;

    let ctx = require_context(config, params.context)?;

    // 4. Build the UpdateResource cmd.
    let cmd = UpdateResource {
        resource: ResourceRef::scoped(
            config.owner_for_context(&ctx),
            &ctx,
            current_type,
            params.slug,
        ),
        body: resolved_body.map(BodyUpdate::new),
        managed_meta: build_partial_managed_meta_from_args(params),
        open_meta: build_partial_open_meta_from_args(params),
        move_to: build_move_spec_from_args(params),
        origin: surface_for_state(),
    };

    // 5. Acquire backend + dispatch.
    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.update_resource(cmd))?;

    // 6. Mode-implicit rendering. If a VaultFileWritten event is present
    //    (local mode), render rel_path + emit discovery event. Otherwise
    //    (cloud mode), print the agent-facing JSON with content_hash.
    if has_vault_file_event(&output.events) {
        let rel_path = vault_file_path_from_events(&output.events).unwrap_or_default();

        // Discovery event: emit agent-facing ResourceUpdate telemetry.
        let final_slug = std::path::Path::new(&rel_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let final_ctx = if output.value.context_name.is_empty() {
            ctx.clone()
        } else {
            output.value.context_name.clone()
        };
        let final_type = if output.value.doc_type_name.is_empty() {
            current_type.to_string()
        } else {
            output.value.doc_type_name.clone()
        };
        let event = Event::ResourceUpdate {
            ts: Local::now().to_rfc3339(),
            doc_type: final_type,
            slug: final_slug,
            context: final_ctx,
        };
        if let Err(e) = discovery::append_event(&config.state_dir, &event) {
            tracing::warn!("Failed to append discovery event: {e}");
        }

        output::success(format!("Updated: {rel_path}"));
    } else {
        // Cloud mode: print {temper-slug, content_hash} JSON to stdout for
        // the show-edit-cat agent workflow contract (per CLAUDE.md).
        let slug_display = output
            .value
            .slug
            .clone()
            .unwrap_or_else(|| output.value.id.to_string());
        let hash_display = output.value.body_hash.as_deref().unwrap_or("").to_string();
        println!(
            "{}",
            serde_json::json!({
                "temper-slug": slug_display,
                "content_hash": hash_display,
            })
        );
    }

    Ok(())
}

/// Per-flag schema validation for `update`. Lifted from the pre-B4 surface
/// code so the friendlier per-flag error messages survive the migration.
/// Only validates scalar managed-meta flags; array fields and `title` (a
/// base-schema field valid on all doctypes) are skipped.
fn validate_update_args(params: &UpdateParams<'_>, current_type: &str) -> Result<()> {
    // Build list of scalar field updates: (frontmatter_key, value) — a
    // direct lift of the pre-B4 inline assembly so the validation loop
    // semantics are unchanged.
    let scalar_updates: Vec<(&str, String)> = [
        ("temper-title", params.title.map(String::from)),
        ("temper-stage", params.stage.map(String::from)),
        ("temper-mode", params.mode.map(String::from)),
        ("temper-effort", params.effort.map(String::from)),
        ("temper-goal", params.goal.map(String::from)),
        ("temper-branch", params.branch.map(String::from)),
        ("temper-pr", params.pr.map(String::from)),
        ("temper-status", params.status.map(String::from)),
        ("temper-seq", params.seq.map(|s| s.to_string())),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|val| (k, val)))
    .collect();

    if scalar_updates.is_empty() {
        return Ok(());
    }

    let schema_fields = schema::updatable_fields(current_type)?;

    // Base fields valid on all types (from base.schema.json).
    const BASE_FIELDS: &[&str] = &["temper-title"];

    for (field_name, value) in &scalar_updates {
        if BASE_FIELDS.contains(field_name) {
            continue;
        }
        match schema_fields.iter().find(|(n, _)| n == field_name) {
            Some((_name, schema_prop)) => {
                if let Some(err) = schema::validate_field_value(field_name, value, schema_prop) {
                    return Err(TemperError::Project(err));
                }
            }
            None => {
                let flag = field_name.strip_prefix("temper-").unwrap_or(field_name);
                return Err(TemperError::Project(format!(
                    "--{flag} is not valid for type '{current_type}'"
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod list_pipeline_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_config(tmp: &TempDir) -> Config {
        let vault_root = tmp.path().to_path_buf();
        fs::create_dir_all(vault_root.join(".temper")).unwrap();
        Config {
            state_dir: vault_root.join(".temper"),
            vault_root,
            contexts: vec!["temper".into(), "default".into()],
            subscriptions: Vec::new(),
            skill_output: PathBuf::from("/tmp/skill"),
            profile_slug: None,
        }
    }

    fn write_resource(
        config: &Config,
        ctx: &str,
        doc_type: &str,
        slug: &str,
        updated: &str,
        extras: &str,
    ) {
        let vault_layout = Vault::new(&config.vault_root);
        let owner = config.owner_for_context(ctx);
        let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
        fs::create_dir_all(&dir).unwrap();
        let content = format!(
            "---\ntemper-id: \"id-{slug}\"\ntemper-type: {doc_type}\ntemper-context: {ctx}\ntemper-slug: {slug}\ntemper-title: \"Title {slug}\"\ntemper-updated: \"{updated}\"\n{extras}---\n\nbody\n"
        );
        fs::write(dir.join(format!("{slug}.md")), content).unwrap();
    }

    #[test]
    fn scan_rows_sorts_descending_by_updated() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "a",
            "2026-04-01T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );
        write_resource(
            &config,
            "temper",
            "task",
            "b",
            "2026-04-07T00:00:00Z",
            "temper-stage: done\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let mut sorted = rows;
        sort_rows(&mut sorted);
        assert_eq!(sorted[0].slug_for_tests(), "b");
        assert_eq!(sorted[1].slug_for_tests(), "a");
    }

    #[test]
    fn scan_rows_skips_non_markdown_and_files_without_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        let vault_layout = Vault::new(&config.vault_root);
        let owner = config.owner_for_context("temper");
        let dir = vault_layout.doc_type_dir(&owner, "temper", "task");
        fs::create_dir_all(&dir).unwrap();
        // Non-md file
        fs::write(dir.join("notes.txt"), "not markdown").unwrap();
        // Markdown without frontmatter
        fs::write(dir.join("raw.md"), "no frontmatter here").unwrap();
        // Valid resource
        write_resource(
            &config,
            "temper",
            "task",
            "ok",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slug_for_tests(), "ok");
    }

    #[test]
    fn scan_rows_all_contexts_when_none_specified() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "session",
            "one",
            "2026-04-01T00:00:00Z",
            "",
        );
        write_resource(
            &config,
            "default",
            "session",
            "two",
            "2026-04-02T00:00:00Z",
            "",
        );

        let rows = scan_rows(&config, "session", None).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn filter_rows_respects_stage() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "x",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );
        write_resource(
            &config,
            "temper",
            "task",
            "y",
            "2026-04-07T00:00:00Z",
            "temper-stage: in-progress\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let filtered = filter_rows(
            rows,
            ListFilters {
                stage: Some("in-progress"),
                goal: None,
                status: None,
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].slug_for_tests(), "y");
    }

    #[test]
    fn filter_rows_respects_goal() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "in-core",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );
        write_resource(
            &config,
            "temper",
            "task",
            "in-other",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: other\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let filtered = filter_rows(
            rows,
            ListFilters {
                stage: None,
                goal: Some("core"),
                status: None,
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].slug_for_tests(), "in-core");
    }

    #[test]
    fn filter_rows_respects_status() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "goal",
            "g1",
            "2026-04-07T00:00:00Z",
            "temper-status: active\ntemper-seq: 10\n",
        );
        write_resource(
            &config,
            "temper",
            "goal",
            "g2",
            "2026-04-07T00:00:00Z",
            "temper-status: completed\ntemper-seq: 20\n",
        );

        let rows = scan_rows(&config, "goal", Some("temper")).unwrap();
        let filtered = filter_rows(
            rows,
            ListFilters {
                stage: None,
                goal: None,
                status: Some("completed"),
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].slug_for_tests(), "g2");
    }

    #[test]
    fn truncate_to_limit() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        for i in 0..5 {
            write_resource(
                &config,
                "temper",
                "session",
                &format!("s{i}"),
                &format!("2026-04-0{}T00:00:00Z", i + 1),
                "",
            );
        }
        let mut rows = scan_rows(&config, "session", Some("temper")).unwrap();
        sort_rows(&mut rows);
        rows.truncate(2);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn render_no_tty_emits_tab_header_and_rows() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "only",
            "2026-04-07T00:00:00Z",
            "temper-stage: in-progress\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::NoTty,
        })
        .unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            "Context\tType\tSlug\tUpdated\tStage\tMode\tEffort\tGoal"
        );
        assert!(lines[1].starts_with("temper\ttask\tonly\t2026-04-07"));
    }

    #[test]
    fn render_pretty_has_table_structure() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "alpha",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Pretty,
        })
        .unwrap();
        let lines: Vec<&str> = out.lines().collect();
        // header | separator | 1 data row
        assert_eq!(lines.len(), 3, "expected 3 lines got: {out}");
        assert!(lines[0].starts_with('|'), "header should start with pipe");
        assert!(lines[1].contains("---"), "separator row contains dashes");
        assert!(lines[2].contains("alpha"), "data row should contain slug");
    }

    #[test]
    fn render_json_emits_full_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "research",
            "note",
            "2026-04-07T00:00:00Z",
            "",
        );
        let out = render_list(&RenderListParams {
            doc_type: "research",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Json,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["temper-slug"], "note");
        assert_eq!(arr[0]["temper-title"], "Title note");
        assert_eq!(arr[0]["temper-type"], "research");
    }

    #[test]
    fn render_json_empty_list_emits_empty_array() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Json,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed, serde_json::json!([]));
    }

    #[test]
    fn render_list_empty_pretty_returns_empty_string() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        // No resources written — the pipeline should return an empty body
        // so the CLI entry can render a "No X resources found" hint instead
        // of a header-only table.
        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Pretty,
        })
        .unwrap();
        assert!(
            out.trim().is_empty(),
            "expected empty body for Pretty empty-list, got: {out:?}"
        );
    }

    #[test]
    fn render_list_empty_no_tty_returns_empty_string() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        let out = render_list(&RenderListParams {
            doc_type: "goal",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::NoTty,
        })
        .unwrap();
        assert!(
            out.trim().is_empty(),
            "expected empty body for NoTty empty-list, got: {out:?}"
        );
    }

    #[test]
    fn render_list_respects_limit() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        for i in 0..5 {
            write_resource(
                &config,
                "temper",
                "session",
                &format!("s{i}"),
                &format!("2026-04-0{}T00:00:00Z", i + 1),
                "",
            );
        }
        let out = render_list(&RenderListParams {
            doc_type: "session",
            config: &config,
            context: Some("temper"),
            limit: Some(2),
            filters: ListFilters::default(),
            format: OutputFormat::Json,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
    }
}

#[cfg(test)]
mod build_helpers_tests {
    use super::*;

    /// Construct an `UpdateParams` with all optional/list fields defaulted.
    /// Tests override only the fields they exercise.
    fn empty_update_params<'a>(slug: &'a str) -> UpdateParams<'a> {
        UpdateParams {
            slug,
            doc_type: None,
            type_from: None,
            type_to: None,
            context: None,
            context_to: None,
            title: None,
            tags: &[],
            aliases: &[],
            relates_to: &[],
            references: &[],
            depends_on: &[],
            extends: &[],
            preceded_by: &[],
            derived_from: &[],
            stage: None,
            mode: None,
            effort: None,
            goal: None,
            seq: None,
            branch: None,
            pr: None,
            status: None,
            body: None,
        }
    }

    #[test]
    fn build_move_spec_returns_none_when_both_flags_unset() {
        let params = empty_update_params("foo");
        assert!(build_move_spec_from_args(&params).is_none());
    }

    #[test]
    fn build_move_spec_returns_some_with_only_context_to_when_only_context_to_set() {
        let mut params = empty_update_params("foo");
        params.context_to = Some("temper");
        let spec = build_move_spec_from_args(&params).expect("expected Some");
        assert_eq!(spec.context_to, Some("temper".to_string()));
        assert_eq!(spec.type_to, None);
    }

    #[test]
    fn build_move_spec_returns_some_with_both_when_both_set() {
        let mut params = empty_update_params("foo");
        params.context_to = Some("temper");
        params.type_to = Some("concept");
        let spec = build_move_spec_from_args(&params).expect("expected Some");
        assert_eq!(spec.context_to, Some("temper".to_string()));
        assert_eq!(spec.type_to, Some("concept".to_string()));
    }

    /// `title` is a managed-meta field (`temper-title`) and must propagate
    /// through `build_partial_managed_meta_from_args` so the B4 surface-side
    /// dispatch can hand a partial `ManagedMeta` (carrying `title`) to the
    /// backend's `apply_updates` translator. Pre-B4 the helper omitted
    /// `title`; this test guards against re-introducing that gap.
    #[test]
    fn build_partial_managed_meta_from_args_includes_title_when_set() {
        let mut params = empty_update_params("foo");
        params.title = Some("Renamed Resource");
        let mm = build_partial_managed_meta_from_args(&params).expect("expected Some");
        assert_eq!(mm.title.as_deref(), Some("Renamed Resource"));
    }

    /// Regression guard: a bare `--title` (no other managed flags) must still
    /// trip the `any_set` short-circuit so the helper returns `Some(..)`. If
    /// `any_set` were to omit `title`, callers passing only `--title` would
    /// see `None` and the title update would silently no-op.
    #[test]
    fn build_partial_managed_meta_from_args_returns_some_when_only_title_set() {
        let mut params = empty_update_params("foo");
        params.title = Some("Solo title");
        assert!(
            build_partial_managed_meta_from_args(&params).is_some(),
            "title-only must trip any_set"
        );
    }
}

#[cfg(test)]
mod render_create_output_tests {
    use temper_core::operations::CommandOutput;
    use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
    use temper_core::types::resource::ResourceRow;

    use super::render_create_output_to_string;

    pub(super) fn make_resource_row(
        slug: &str,
        doc_type: &str,
        title: &str,
        context: &str,
    ) -> ResourceRow {
        ResourceRow {
            id: ResourceId(uuid::Uuid::nil()),
            kb_context_id: ContextId(uuid::Uuid::nil()),
            kb_doc_type_id: DocTypeId(uuid::Uuid::nil()),
            origin_uri: "test://origin".to_string(),
            title: title.to_string(),
            slug: Some(slug.to_string()),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: context.to_string(),
            doc_type_name: doc_type.to_string(),
            owner_handle: "@me".to_string(),
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
            managed_hash: None,
            open_hash: None,
        }
    }

    #[test]
    fn render_create_output_task_json_matches_legacy_shape() {
        let row = make_resource_row("2026-05-14-test", "task", "Test", "temper");
        let output = CommandOutput::new(row);
        let json = render_create_output_to_string(&output, "task", "json")
            .expect("rendering task JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "task");
        assert_eq!(parsed["temper-slug"], "2026-05-14-test");
        assert_eq!(parsed["temper-title"], "Test");
        assert_eq!(parsed["temper-context"], "temper");
        // Exactly 4 fields — no extras leaking in.
        assert_eq!(
            parsed.as_object().unwrap().len(),
            4,
            "task JSON must have exactly 4 fields"
        );
    }

    #[test]
    fn render_create_output_goal_json_matches_legacy_shape() {
        let row = make_resource_row("test-goal", "goal", "Test Goal", "temper");
        let output = CommandOutput::new(row);
        let json = render_create_output_to_string(&output, "goal", "json")
            .expect("rendering goal JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["temper-title"], "Test Goal");
        assert_eq!(parsed["temper-slug"], "test-goal");
        assert_eq!(parsed["temper-context"], "temper");
        assert_eq!(parsed["temper-status"], "active");
        // seq is None → JSON null
        assert!(parsed["temper-seq"].is_null());
        // Exactly 5 fields.
        assert_eq!(
            parsed.as_object().unwrap().len(),
            5,
            "goal JSON must have exactly 5 fields"
        );
    }

    #[test]
    fn render_create_output_goal_json_includes_seq_when_set() {
        let mut row = make_resource_row("test-goal-seq", "goal", "Goal With Seq", "temper");
        row.seq = Some(3);
        let output = CommandOutput::new(row);
        let json = render_create_output_to_string(&output, "goal", "json")
            .expect("rendering goal JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["temper-seq"], 3);
    }

    /// Build a `CommandOutput<ResourceRow>` that carries a `VaultFileWritten`
    /// event with the canonical rel_path the on-disk write would have used.
    /// Used by tests that assert the local-mode legacy JSON shape
    /// (`path` field populated). Cloud-mode tests use `CommandOutput::new(row)`
    /// directly — no event, empty `path` field.
    fn output_with_vault_file(row: ResourceRow, rel_path: &str) -> CommandOutput<ResourceRow> {
        use temper_core::operations::DomainEvent;
        CommandOutput {
            value: row,
            events: vec![DomainEvent::VaultFileWritten {
                path: rel_path.to_string(),
            }],
        }
    }

    #[test]
    fn render_create_output_session_json_matches_legacy_shape() {
        let row = make_resource_row("2026-05-14-my-session", "session", "My Session", "temper");
        let output = output_with_vault_file(row, "@me/temper/session/2026-05-14-my-session.md");
        let json = render_create_output_to_string(&output, "session", "json")
            .expect("rendering session JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["title"], "My Session");
        assert_eq!(parsed["context"], "temper");
        // Path is populated from the VaultFileWritten event (local-mode).
        assert_eq!(
            parsed["path"],
            "@me/temper/session/2026-05-14-my-session.md"
        );
        // date field is today's date — just verify it parses and is non-empty.
        let date = parsed["date"].as_str().expect("date must be a string");
        assert!(!date.is_empty(), "date must be non-empty");
        assert_eq!(date.len(), 10, "date must be %Y-%m-%d (10 chars)");
        // Exactly 4 fields.
        assert_eq!(
            parsed.as_object().unwrap().len(),
            4,
            "session JSON must have exactly 4 fields"
        );
    }

    #[test]
    fn render_create_output_session_cloud_mode_emits_empty_path() {
        // Cloud-mode: no VaultFileWritten event → `path` field empty. Agents
        // can detect cloud mode by checking for an empty path; no fictional
        // file path is emitted.
        let row = make_resource_row("2026-05-14-my-session", "session", "My Session", "temper");
        let output = CommandOutput::new(row);
        let json = render_create_output_to_string(&output, "session", "json")
            .expect("rendering session JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["path"], "", "cloud mode must emit empty path");
    }

    #[test]
    fn render_create_output_research_json_matches_legacy_shape() {
        let row = make_resource_row(
            "2026-05-14-my-research",
            "research",
            "My Research",
            "temper",
        );
        let output = output_with_vault_file(row, "@me/temper/research/2026-05-14-my-research.md");
        let json = render_create_output_to_string(&output, "research", "json")
            .expect("rendering research JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["title"], "My Research");
        // Legacy research uses "project" (not "context").
        assert_eq!(parsed["project"], "temper");
        assert_eq!(
            parsed["path"],
            "@me/temper/research/2026-05-14-my-research.md"
        );
        // id is the UUID string of the ResourceId.
        let id = parsed["id"].as_str().expect("id must be a string");
        assert!(!id.is_empty(), "id must be non-empty");
        // slug field present.
        assert_eq!(parsed["slug"], "2026-05-14-my-research");
        let date = parsed["date"].as_str().expect("date must be a string");
        assert_eq!(date.len(), 10, "date must be %Y-%m-%d");
        // Exactly 6 fields: title, project, path, date, id, slug.
        assert_eq!(
            parsed.as_object().unwrap().len(),
            6,
            "research JSON must have exactly 6 fields"
        );
    }

    #[test]
    fn render_create_output_concept_json_matches_legacy_shape() {
        let row = make_resource_row("my-concept", "concept", "My Concept", "temper");
        let output = output_with_vault_file(row, "@me/temper/concept/my-concept.md");
        let json = render_create_output_to_string(&output, "concept", "json")
            .expect("rendering concept JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["doc_type"], "concept");
        assert_eq!(parsed["title"], "My Concept");
        assert_eq!(parsed["slug"], "my-concept");
        assert_eq!(parsed["context"], "temper");
        assert_eq!(parsed["path"], "@me/temper/concept/my-concept.md");
        let id = parsed["id"].as_str().expect("id must be a string");
        assert!(!id.is_empty(), "id must be non-empty");
        let date = parsed["date"].as_str().expect("date must be a string");
        assert_eq!(date.len(), 10, "date must be %Y-%m-%d");
        // Exactly 7 fields: doc_type, title, slug, context, path, date, id.
        assert_eq!(
            parsed.as_object().unwrap().len(),
            7,
            "concept JSON must have exactly 7 fields"
        );
    }

    #[test]
    fn render_create_output_decision_json_matches_legacy_shape() {
        let row = make_resource_row(
            "2026-05-14-my-decision",
            "decision",
            "My Decision",
            "temper",
        );
        let output = output_with_vault_file(row, "@me/temper/decision/2026-05-14-my-decision.md");
        let json = render_create_output_to_string(&output, "decision", "json")
            .expect("rendering decision JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["doc_type"], "decision");
        assert_eq!(parsed["title"], "My Decision");
        assert_eq!(parsed["slug"], "2026-05-14-my-decision");
        assert_eq!(parsed["context"], "temper");
        assert_eq!(
            parsed["path"],
            "@me/temper/decision/2026-05-14-my-decision.md"
        );
        let id = parsed["id"].as_str().expect("id must be a string");
        assert!(!id.is_empty(), "id must be non-empty");
        let date = parsed["date"].as_str().expect("date must be a string");
        assert_eq!(date.len(), 10, "date must be %Y-%m-%d");
        // Exactly 7 fields: doc_type, title, slug, context, path, date, id.
        assert_eq!(
            parsed.as_object().unwrap().len(),
            7,
            "decision JSON must have exactly 7 fields"
        );
    }

    #[test]
    fn render_create_output_non_json_format_returns_empty_string() {
        let row = make_resource_row("2026-05-14-test", "task", "Test", "temper");
        let output = CommandOutput::new(row);
        // Non-JSON format: function prints a success line and returns "".
        // We can't easily capture stdout in unit tests, but we can confirm
        // the return value is empty and no error is returned.
        let result = render_create_output_to_string(&output, "task", "text")
            .expect("non-JSON format should not error");
        assert!(
            result.is_empty(),
            "non-JSON format must return empty string"
        );
    }

    #[test]
    fn render_create_output_invalid_doctype_returns_error() {
        let row = make_resource_row("test", "task", "Test", "temper");
        let output = CommandOutput::new(row);
        let result = render_create_output_to_string(&output, "bogus", "json");
        assert!(result.is_err(), "invalid doctype must return an error");
    }
}
