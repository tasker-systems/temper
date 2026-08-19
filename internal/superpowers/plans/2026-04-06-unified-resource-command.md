# Unified Resource Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **REQUIRED:** Before writing any code, read `/Users/petetaylor/.claude/skills/temper/subagent-guidance.md` and follow all 10 principles (SG-1 through SG-10). In particular: read sibling files before editing (SG-1), don't over-build (SG-5), verify before claiming done (SG-6).

**Goal:** Replace per-doctype CLI commands with a unified `temper resource {create,list,show,update}` command, add concept/decision doctypes, and clean up dead code/config.

**Architecture:** A single `ResourceAction` enum in clap replaces `TaskAction`, `GoalAction`, `SessionAction`, `ResearchAction`, `NoteAction`. The `resource update` command uses embedded jsonschemas to validate which flags are legal per doctype. Type-specific create/list logic is delegated to existing action modules refactored to accept common parameter structs.

**Tech Stack:** Rust (clap, serde, serde_json, askama), jsonschema validation via `crates/temper-core/src/schema.rs`

**Spec:** `docs/superpowers/specs/2026-04-06-unified-resource-command-design.md`

---

## Task 1: Cleanup — Remove Dead Config and Code

Remove `[sync.auto]` config, `LEGACY_FIELD_MAP` from doctor_fix, stale template copies, and `temper-legacy-id`.

**Files:**
- Modify: `crates/temper-core/src/types/config.rs:91-111` (remove `SyncAutoConfig`, `SyncSubscriptionsConfig` inline `contexts` into `UnifiedSyncConfig`)
- Modify: `crates/temper-core/src/types/mod.rs:45-46` (remove `SyncAutoConfig` export)
- Modify: `crates/temper-cli/src/commands/init.rs:83` (remove `[sync.auto]` from template)
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs:143-190` (remove `LEGACY_FIELD_MAP` and `fix_legacy_fields`)
- Modify: `crates/temper-core/src/schema.rs:59-78` (remove `temper-legacy-id` from `KNOWN_TEMPER_FIELDS`)
- Modify: `crates/temper-core/src/schema.rs:80-99` (remove `legacy_id` from `LEGACY_FIELDS`)
- Modify: `crates/temper-core/src/types/managed_meta.rs:31-32` (remove `legacy_id` field)
- Modify: `crates/temper-core/schemas/base.schema.json:33-36` (remove `temper-legacy-id` property)
- Delete: `crates/temper-cli/src/templates/research.md`
- Delete: `crates/temper-cli/src/templates/task.md`
- Delete: `crates/temper-cli/src/templates/session.md`
- Delete: `crates/temper-cli/src/templates/goal.md`

- [ ] **Step 1: Read all files to understand current state**

Read these files before modifying:
- `crates/temper-core/src/types/config.rs` (full file)
- `crates/temper-core/src/types/mod.rs`
- `crates/temper-cli/src/commands/init.rs`
- `crates/temper-cli/src/actions/doctor_fix.rs`
- `crates/temper-core/src/schema.rs`
- `crates/temper-core/src/types/managed_meta.rs`
- `crates/temper-core/schemas/base.schema.json`

- [ ] **Step 2: Remove `SyncAutoConfig` and simplify `UnifiedSyncConfig`**

In `crates/temper-core/src/types/config.rs`, delete the `SyncAutoConfig` struct (lines 91-95) and the `SyncSubscriptionsConfig` struct (lines 98-102). Flatten `UnifiedSyncConfig` to hold `contexts` directly:

```rust
/// Sync config — which contexts are synced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnifiedSyncConfig {
    #[serde(default)]
    pub subscriptions: SyncSubscriptions,
}

/// Sync subscriptions — which contexts are synced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSubscriptions {
    #[serde(default)]
    pub contexts: Vec<String>,
}
```

Note: Keep the `subscriptions` nesting so existing `[sync.subscriptions]` TOML sections still parse. Only remove `[sync.auto]`. Update the re-exports in `crates/temper-core/src/types/mod.rs` to remove `SyncAutoConfig`.

Also delete `CloudConfig` and `SyncConfig` (lines 59-81) if they have no remaining callers — grep first.

- [ ] **Step 3: Remove `[sync.auto]` from init template**

In `crates/temper-cli/src/commands/init.rs`, find the two occurrences of `[sync.auto]` (around lines 83 and 125) and remove those TOML lines from the config template strings. The config template should go from:

```toml
[sync.auto]
doctypes = ["task", "goal", "session", "research", "concepts", "decisions"]

[sync.subscriptions]
```

To just:

```toml
[sync.subscriptions]
```

- [ ] **Step 4: Remove LEGACY_FIELD_MAP and fix_legacy_fields from doctor_fix.rs**

In `crates/temper-cli/src/actions/doctor_fix.rs`:
- Delete the `LEGACY_FIELD_MAP` static (lines 143-164)
- Delete the `fix_legacy_fields()` function (lines 170-190)
- Find and remove any callers of `fix_legacy_fields()` in the same file
- Keep `infer_temper_id` and all other functions

- [ ] **Step 5: Remove temper-legacy-id from schemas and types**

In `crates/temper-core/schemas/base.schema.json`, remove:
```json
"temper-legacy-id": {
  "type": "string",
  "description": "Previous UUID from migration"
},
```

In `crates/temper-core/src/schema.rs`:
- Remove `"temper-legacy-id"` from `KNOWN_TEMPER_FIELDS` (line ~66)
- Remove `("legacy_id", "temper-legacy-id")` from `LEGACY_FIELDS` (line ~91)

In `crates/temper-core/src/types/managed_meta.rs`:
- Remove the `legacy_id` field and its serde rename attribute (line ~31-32)
- Update any tests that reference `legacy_id`

- [ ] **Step 6: Delete stale src/templates/ copies**

```bash
rm crates/temper-cli/src/templates/research.md
rm crates/temper-cli/src/templates/task.md
rm crates/temper-cli/src/templates/session.md
rm crates/temper-cli/src/templates/goal.md
```

Verify nothing imports or references these files:
```bash
cargo make check
```

- [ ] **Step 7: Run tests and fix any breakage**

```bash
cargo make check
cargo make test
```

Fix any compile errors or test failures from the removals. Update tests in `config.rs` that assert on `sync.auto` fields.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "chore: remove dead config (sync.auto), legacy field map, stale templates, temper-legacy-id"
```

---

## Task 2: Add Concept and Decision Templates and Schema Updates

Add Askama templates for concept and decision doctypes, add `date` to concept schema, add template structs.

**Files:**
- Create: `crates/temper-cli/templates/concept.md`
- Create: `crates/temper-cli/templates/decision.md`
- Modify: `crates/temper-core/schemas/concept.schema.json`
- Modify: `crates/temper-cli/src/templates.rs` (add ConceptTemplate, DecisionTemplate)

- [ ] **Step 1: Read existing templates and schemas for patterns**

Read:
- `crates/temper-cli/templates/research.md` (closest pattern for concept/decision)
- `crates/temper-cli/templates/session.md`
- `crates/temper-cli/src/templates.rs`
- `crates/temper-core/schemas/concept.schema.json`
- `crates/temper-core/schemas/decision.schema.json`

- [ ] **Step 2: Add date to concept schema**

In `crates/temper-core/schemas/concept.schema.json`, add the `date` property and make it required:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/concept.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "concept" },
    "slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    },
    "date": {
      "type": "string",
      "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$",
      "description": "Date concept was recorded (YYYY-MM-DD)"
    }
  },
  "required": ["slug", "date"],
  "additionalProperties": true
}
```

- [ ] **Step 3: Create concept template**

Create `crates/temper-cli/templates/concept.md`:

```
---
temper-provisional-id: "{{ id }}"
temper-type: concept
temper-context: "{{ project }}"
date: {{ date }}
title: "{{ title }}"
slug: "{{ slug }}"
---

# {{ title }}
```

- [ ] **Step 4: Create decision template**

Create `crates/temper-cli/templates/decision.md`:

```
---
temper-provisional-id: "{{ id }}"
temper-type: decision
temper-context: "{{ project }}"
date: {{ date }}
title: "{{ title }}"
slug: "{{ slug }}"
---

# {{ title }}

## Context

## Decision

## Consequences
```

- [ ] **Step 5: Add template structs**

In `crates/temper-cli/src/templates.rs`, add after the existing template structs:

```rust
#[derive(Template)]
#[template(path = "concept.md")]
pub struct ConceptTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub date: &'a str,
    pub project: &'a str,
    pub slug: &'a str,
}

#[derive(Template)]
#[template(path = "decision.md")]
pub struct DecisionTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub date: &'a str,
    pub project: &'a str,
    pub slug: &'a str,
}
```

- [ ] **Step 6: Verify templates compile**

```bash
cargo make check
```

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/templates/concept.md crates/temper-cli/templates/decision.md crates/temper-core/schemas/concept.schema.json crates/temper-cli/src/templates.rs
git commit -m "feat: add concept and decision templates, add date to concept schema"
```

---

## Task 3: Build the ResourceAction Enum and CLI Parsing

Replace per-doctype command enums with a single `ResourceAction` enum. Wire up `main.rs` dispatch.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (replace command enums)
- Modify: `crates/temper-cli/src/main.rs` (replace dispatch match arms)

- [ ] **Step 1: Read current cli.rs and main.rs fully**

Read:
- `crates/temper-cli/src/cli.rs` (full file — all enums)
- `crates/temper-cli/src/main.rs` (full file — all dispatch)

- [ ] **Step 2: Define ResourceAction enum in cli.rs**

Replace `TaskAction`, `GoalAction`, `SessionAction`, `ResearchAction`, `NoteAction` with:

```rust
#[derive(Subcommand)]
pub enum ResourceAction {
    /// Create a new resource
    Create {
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Resource title
        #[arg(long)]
        title: Option<String>,
        /// Context name
        #[arg(long)]
        context: Option<String>,
        /// Parent goal slug (task only)
        #[arg(long)]
        goal: Option<String>,
        /// Work mode: plan or build (task only)
        #[arg(long)]
        mode: Option<String>,
        /// Work effort: small, medium, large (task only)
        #[arg(long)]
        effort: Option<String>,
        /// Override auto-generated slug (goal only)
        #[arg(long)]
        slug: Option<String>,
        /// Print the raw template and exit
        #[arg(long)]
        show_template: bool,
        #[arg(long, hide = true)]
        stdin: bool,
        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// List resources of a given type
    List {
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Maximum results
        #[arg(long)]
        limit: Option<usize>,
        /// Filter by stage (task only)
        #[arg(long)]
        stage: Option<String>,
        /// Filter by goal (task only)
        #[arg(long)]
        goal: Option<String>,
        /// Filter by status (goal only)
        #[arg(long)]
        status: Option<String>,
        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Show a resource's content
    Show {
        /// Resource slug
        slug: String,
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Update a resource's frontmatter fields
    Update {
        /// Resource slug
        slug: String,
        /// Current resource type (for lookup)
        #[arg(long)]
        r#type: Option<String>,
        /// Current resource type when changing type (use with --type-to)
        #[arg(long)]
        type_from: Option<String>,
        /// New resource type (converts the resource)
        #[arg(long)]
        type_to: Option<String>,
        /// Context to search in
        #[arg(long)]
        context: Option<String>,
        /// Move resource to a new context
        #[arg(long)]
        context_to: Option<String>,
        // --- Base schema fields ---
        /// Update title
        #[arg(long)]
        title: Option<String>,
        /// Add tag (repeatable)
        #[arg(long)]
        tags: Vec<String>,
        /// Add alias (repeatable)
        #[arg(long)]
        aliases: Vec<String>,
        /// Add relates-to reference (repeatable)
        #[arg(long)]
        relates_to: Vec<String>,
        /// Add reference (repeatable)
        #[arg(long)]
        references: Vec<String>,
        /// Add depends-on reference (repeatable)
        #[arg(long)]
        depends_on: Vec<String>,
        // --- Task-specific fields ---
        /// Task stage (backlog, in-progress, done, cancelled)
        #[arg(long)]
        stage: Option<String>,
        /// Task mode (plan, build)
        #[arg(long)]
        mode: Option<String>,
        /// Task effort (small, medium, large)
        #[arg(long)]
        effort: Option<String>,
        /// Task goal slug
        #[arg(long)]
        goal: Option<String>,
        /// Task sequence number
        #[arg(long)]
        seq: Option<i64>,
        /// Git branch
        #[arg(long)]
        branch: Option<String>,
        /// Pull request URL
        #[arg(long)]
        pr: Option<String>,
        // --- Goal-specific fields ---
        /// Goal status (active, completed, paused, cancelled)
        #[arg(long)]
        status: Option<String>,
    },
}
```

Update `Commands` enum: remove `Task`, `Goal`, `Session`, `Research`, `Note` variants. Add:

```rust
    /// Manage resources (tasks, goals, sessions, research, concepts, decisions)
    Resource {
        #[command(subcommand)]
        action: ResourceAction,
    },
```

Delete the old enums: `TaskAction`, `GoalAction`, `SessionAction`, `ResearchAction`, `NoteAction`.

- [ ] **Step 3: Update main.rs dispatch**

Replace all the old match arms (`Commands::Task { action }`, `Commands::Goal { action }`, etc.) with a single `Commands::Resource { action }` arm that dispatches based on the `type` field.

For `ResourceAction::Create`:
```rust
Commands::Resource { action } => {
    let config = temper_cli::config::load(cli.vault.as_deref())?;
    match action {
        ResourceAction::Create {
            r#type,
            title,
            context,
            goal,
            mode,
            effort,
            slug,
            show_template,
            stdin: _,
            format,
        } => {
            if show_template {
                let content = temper_cli::vault::get_template(&r#type)?;
                print!("{content}");
                return Ok(());
            }
            let title = title.ok_or_else(|| {
                temper_cli::error::TemperError::Project(
                    "--title is required for resource create".into(),
                )
            })?;
            temper_cli::commands::resource::create(
                &config, &r#type, &title,
                context.as_deref(), goal.as_deref(),
                mode.as_deref(), effort.as_deref(),
                slug.as_deref(), &format,
            )
        }
        ResourceAction::List { r#type, context, limit, stage, goal, status, format } => {
            temper_cli::commands::resource::list(
                &config, &r#type, context.as_deref(), limit,
                stage.as_deref(), goal.as_deref(), status.as_deref(), &format,
            )
        }
        ResourceAction::Show { slug, r#type, context, format } => {
            temper_cli::commands::resource::show(
                &config, &r#type, &slug, context.as_deref(), &format,
            )
        }
        ResourceAction::Update { slug, r#type, type_from, type_to, context, context_to,
            title, tags, aliases, relates_to, references, depends_on,
            stage, mode, effort, goal, seq, branch, pr, status,
        } => {
            temper_cli::commands::resource::update(
                &config, &slug,
                r#type.as_deref(), type_from.as_deref(), type_to.as_deref(),
                context.as_deref(), context_to.as_deref(),
                title.as_deref(), &tags, &aliases, &relates_to, &references, &depends_on,
                stage.as_deref(), mode.as_deref(), effort.as_deref(),
                goal.as_deref(), seq, branch.as_deref(), pr.as_deref(),
                status.as_deref(),
            )
        }
    }
}
```

- [ ] **Step 4: Verify it compiles (will fail — resource module doesn't exist yet)**

```bash
cargo check -p temper-cli 2>&1 | head -20
```

Expected: error about missing `commands::resource` module. This is correct — we build it in the next task.

- [ ] **Step 5: Commit (WIP — won't compile yet)**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "refactor: replace per-doctype command enums with unified ResourceAction

WIP: commands::resource module not yet implemented"
```

---

## Task 4: Build the Resource Command Module — Create and List

Implement `commands::resource.rs` with `create()` and `list()` functions that delegate to existing action modules.

**Files:**
- Create: `crates/temper-cli/src/commands/resource.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (add `pub mod resource;`)

- [ ] **Step 1: Read existing command modules for patterns**

Read:
- `crates/temper-cli/src/commands/mod.rs`
- `crates/temper-cli/src/commands/research.rs` (save function — will be refactored)
- `crates/temper-cli/src/commands/session.rs:26-141` (save function)
- `crates/temper-cli/src/actions/task.rs:128-212` (create function)
- `crates/temper-cli/src/actions/goal.rs:107-144` (create function)
- `crates/temper-cli/src/commands/note.rs` (create function)

- [ ] **Step 2: Add module declaration**

In `crates/temper-cli/src/commands/mod.rs`, add:
```rust
pub mod resource;
```

- [ ] **Step 3: Write resource::create()**

Create `crates/temper-cli/src/commands/resource.rs`:

```rust
use crate::config::Config;
use crate::error::{Result, TemperError};

/// Valid resource types.
const VALID_TYPES: &[&str] = &["task", "goal", "session", "research", "concept", "decision"];

fn validate_type(doc_type: &str) -> Result<()> {
    if !VALID_TYPES.contains(&doc_type) {
        return Err(TemperError::Project(format!(
            "unknown resource type '{}'; expected one of: {}",
            doc_type,
            VALID_TYPES.join(", ")
        )));
    }
    Ok(())
}

pub fn create(
    config: &Config,
    doc_type: &str,
    title: &str,
    context: Option<&str>,
    goal: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    slug: Option<&str>,
    format: &str,
) -> Result<()> {
    validate_type(doc_type)?;

    match doc_type {
        "task" => {
            let ctx = require_context(config, context)?;
            crate::commands::task::create(config, &ctx, title, goal, mode, effort)?;
            Ok(())
        }
        "goal" => {
            let ctx = require_context(config, context)?;
            crate::commands::goal::create(config, &ctx, title, slug, format)?;
            Ok(())
        }
        "session" => {
            let stdin_content = crate::vault::read_stdin_if_piped();
            crate::commands::session::save(
                config,
                Some(title),
                context,
                stdin_content.as_deref(),
                None, // task
                None, // state
                format,
            )
        }
        "research" => {
            let stdin_content = crate::vault::read_stdin_if_piped();
            crate::commands::research::save(
                config,
                title,
                context,
                stdin_content.as_deref(),
                format,
            )
        }
        "concept" | "decision" => {
            create_simple_resource(config, doc_type, title, context, format)
        }
        _ => unreachable!(), // validate_type catches this
    }
}

/// Create a concept or decision resource using the template + doc_type_dir pattern.
fn create_simple_resource(
    config: &Config,
    doc_type: &str,
    title: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use askama::Template;
    use chrono::Local;

    let context_name = context.unwrap_or("general");
    let today = Local::now().format("%Y-%m-%d").to_string();
    let id = crate::ids::generate_id();
    let slug = crate::vault::slugify(title);

    let content = match doc_type {
        "concept" => {
            let tmpl = crate::templates::ConceptTemplate {
                id: &id,
                title,
                date: &today,
                project: context_name,
                slug: &slug,
            };
            tmpl.render()
                .map_err(|e| TemperError::Vault(format!("template error: {e}")))?
        }
        "decision" => {
            let tmpl = crate::templates::DecisionTemplate {
                id: &id,
                title,
                date: &today,
                project: context_name,
                slug: &slug,
            };
            tmpl.render()
                .map_err(|e| TemperError::Vault(format!("template error: {e}")))?
        }
        _ => unreachable!(),
    };

    // Determine filename: concept uses {slug}.md, decision uses {date}-{slug}.md
    let filename = match doc_type {
        "concept" => format!("{slug}.md"),
        "decision" => format!("{today}-{slug}.md"),
        _ => unreachable!(),
    };

    let dir = config.doc_type_dir(context_name, doc_type);
    let note_path = dir.join(&filename);

    let stdin_content = crate::vault::read_stdin_if_piped();

    if note_path.exists() {
        // If stdin is piped, update the body
        if let Some(body) = &stdin_content {
            let existing = std::fs::read_to_string(&note_path)?;
            let updated = crate::vault::replace_body(&existing, body);
            std::fs::write(&note_path, updated)?;
            let relative = note_path.strip_prefix(&config.vault_root).unwrap_or(&note_path);
            crate::output::success(format!("Updated: {}", relative.display()));
        }
        return Ok(());
    }

    // Apply stdin body if piped
    let content = if let Some(body) = &stdin_content {
        crate::vault::replace_body(&content, body)
    } else {
        content
    };

    crate::vault::write_note(&note_path, &content)?;

    let relative = note_path.strip_prefix(&config.vault_root).unwrap_or(&note_path);
    let relative_str = relative.to_string_lossy();

    if format == "json" {
        let json = serde_json::json!({
            "type": doc_type,
            "title": title,
            "context": context_name,
            "path": relative_str,
            "date": today,
            "id": id,
            "slug": slug,
        });
        println!("{}", serde_json::to_string_pretty(&json).unwrap_or_default());
    } else {
        crate::output::success(format!("Created: {relative_str}"));
    }

    let ts = Local::now().to_rfc3339();
    let event = crate::discovery::Event::ResourceCreate {
        ts,
        doc_type: doc_type.to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        context: context_name.to_string(),
    };
    if let Err(e) = crate::discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}

fn require_context(config: &Config, context: Option<&str>) -> Result<String> {
    let ctx = context.ok_or_else(|| {
        TemperError::Project("no context specified — use --context <name>".into())
    })?;
    Ok(crate::commands::resolve_context_with_fallback(config, ctx))
}
```

- [ ] **Step 4: Write resource::list()**

Add to `crates/temper-cli/src/commands/resource.rs`:

```rust
pub fn list(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
    limit: Option<usize>,
    stage: Option<&str>,
    goal: Option<&str>,
    status: Option<&str>,
    format: &str,
) -> Result<()> {
    validate_type(doc_type)?;

    match doc_type {
        "task" => crate::commands::task::list(config, context, goal, stage, format),
        "goal" => {
            let ctx = require_context(config, context)?;
            crate::commands::goal::list(config, &ctx, format)
        }
        "session" => crate::commands::session::list(config, context, limit, format),
        "research" | "concept" | "decision" => {
            list_simple_resources(config, doc_type, context, limit, format)
        }
        _ => unreachable!(),
    }
}

/// Generic list for resource types that don't have custom list logic.
fn list_simple_resources(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
    limit: Option<usize>,
    format: &str,
) -> Result<()> {
    let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    let mut entries: Vec<ResourceEntry> = Vec::new();

    for ctx in &contexts_to_scan {
        let dir = config.doc_type_dir(ctx, doc_type);
        if !dir.is_dir() {
            continue;
        }
        for file_entry in std::fs::read_dir(&dir)? {
            let file_entry = file_entry?;
            let path = file_entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                if let Some(entry) = parse_resource_entry(&path, ctx) {
                    entries.push(entry);
                }
            }
        }
    }

    entries.sort_by(|a, b| b.date.cmp(&a.date));
    entries.truncate(limit.unwrap_or(20));

    if format == "json" {
        let json = serde_json::to_string_pretty(&entries).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    if entries.is_empty() {
        crate::output::hint(format!("No {doc_type} resources found."));
        return Ok(());
    }

    crate::output::plain(format!("{:<12} {:<20} Title", "Date", "Context"));
    crate::output::dim("-".repeat(60));
    for entry in &entries {
        crate::output::plain(format!(
            "{:<12} {:<20} {}",
            entry.date, entry.context, entry.title
        ));
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct ResourceEntry {
    date: String,
    context: String,
    title: String,
    slug: String,
}

fn parse_resource_entry(path: &std::path::Path, context: &str) -> Option<ResourceEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    let fm = crate::vault::parse_frontmatter(&content)?;
    let title = fm.get("title")?.as_str()?.to_string();
    let date = fm
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let slug = fm
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(ResourceEntry {
        date,
        context: context.to_string(),
        title,
        slug,
    })
}
```

- [ ] **Step 5: Add `--limit` to goal list**

In `crates/temper-cli/src/commands/goal.rs`, modify the `list()` function to accept an optional `limit` parameter and truncate results. Update the signature and add `entries.truncate(limit.unwrap_or(20));` after sorting. Update the caller in `resource::list()` to pass `limit`.

- [ ] **Step 6: Verify compilation**

```bash
cargo check -p temper-cli 2>&1 | head -30
```

This will likely show errors about the `ResourceCreate` event variant not existing yet — that's expected. Create a minimal variant in `discovery.rs` to unblock:

```rust
#[serde(rename = "resource_create")]
ResourceCreate {
    ts: String,
    doc_type: String,
    title: String,
    path: String,
    context: String,
},
```

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/commands/goal.rs crates/temper-cli/src/discovery.rs
git commit -m "feat: add resource create and list commands

Delegates to existing type-specific action modules. Adds concept and
decision create support, generic list for simple resource types."
```

---

## Task 5: Build Resource Show and Update

Implement `resource::show()` and `resource::update()` with schema-driven validation.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (add show, update)
- Modify: `crates/temper-core/src/schema.rs` (add helper to extract updatable fields)

- [ ] **Step 1: Read schema validation code**

Read:
- `crates/temper-core/src/schema.rs` (full file — understand `load_schema`, `validate_frontmatter`)
- `crates/temper-cli/src/commands/task.rs` (show function — pattern for show)
- `crates/temper-cli/src/actions/task.rs:215-311` (move_task — pattern for field updates)

- [ ] **Step 2: Add schema helper for updatable field enumeration**

In `crates/temper-core/src/schema.rs`, add a function that extracts updatable field names and their schema constraints from a doctype schema:

```rust
/// Fields that are system-managed and cannot be updated via CLI.
pub static SYSTEM_MANAGED_FIELDS: &[&str] = &[
    "temper-id",
    "temper-provisional-id",
    "temper-type",
    "temper-created",
    "temper-updated",
    "temper-source",
    "slug",
];

/// Get the updatable field names for a doctype by reading the schema properties
/// and excluding system-managed fields.
pub fn updatable_fields(doc_type: &str) -> Result<Vec<(String, serde_json::Value)>> {
    let schema_str = match doc_type {
        "task" => TASK_SCHEMA,
        "goal" => GOAL_SCHEMA,
        "session" => SESSION_SCHEMA,
        "research" => RESEARCH_SCHEMA,
        "decision" => DECISION_SCHEMA,
        "concept" => CONCEPT_SCHEMA,
        other => {
            return Err(TemperError::Config(format!("unknown doctype '{other}'")))
        }
    };

    let schema: serde_json::Value = serde_json::from_str(schema_str)
        .map_err(|e| TemperError::Config(format!("schema parse error: {e}")))?;

    let mut fields = Vec::new();

    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (key, value) in props {
            if !SYSTEM_MANAGED_FIELDS.contains(&key.as_str()) {
                fields.push((key.clone(), value.clone()));
            }
        }
    }

    Ok(fields)
}

/// Validate a field value against a schema property definition.
/// Returns an error message if invalid, None if valid.
pub fn validate_field_value(field_name: &str, value: &str, schema_prop: &serde_json::Value) -> Option<String> {
    // Check enum constraint
    if let Some(enum_values) = schema_prop.get("enum") {
        if let Some(arr) = enum_values.as_array() {
            let valid: Vec<String> = arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !valid.contains(&value.to_string()) {
                return Some(format!(
                    "invalid value '{}' for --{}; expected one of: {}",
                    value,
                    field_name.strip_prefix("temper-").unwrap_or(field_name),
                    valid.join(", ")
                ));
            }
        }
    }
    // Check type constraint
    if let Some(type_val) = schema_prop.get("type") {
        if type_val == "integer" {
            if value.parse::<i64>().is_err() {
                return Some(format!(
                    "invalid value '{}' for --{}; expected integer",
                    value,
                    field_name.strip_prefix("temper-").unwrap_or(field_name),
                ));
            }
        }
    }
    None
}
```

- [ ] **Step 3: Write resource::show()**

Add to `crates/temper-cli/src/commands/resource.rs`:

```rust
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    validate_type(doc_type)?;

    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format),
        "session" => crate::commands::session::show(config, slug, context, format),
        _ => show_generic(config, doc_type, slug, context, format),
    }
}

/// Generic show for resource types that use slug-based file lookup.
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    let (path, _ctx) = find_resource_file(config, doc_type, slug, context)?;

    if format == "json" {
        let content = std::fs::read_to_string(&path)?;
        let relative = path.strip_prefix(&config.vault_root).unwrap_or(&path);
        let json = serde_json::json!({
            "type": doc_type,
            "slug": slug,
            "path": relative.to_string_lossy(),
            "content": content,
        });
        println!("{}", serde_json::to_string_pretty(&json).unwrap_or_default());
    } else {
        let content = std::fs::read_to_string(&path)?;
        print!("{content}");
    }

    Ok(())
}

/// Find a resource file by doc_type and slug, searching across contexts.
fn find_resource_file(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
) -> Result<(std::path::PathBuf, String)> {
    let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    let needle = crate::vault::slugify(slug);

    for ctx in &contexts_to_scan {
        let dir = config.doc_type_dir(ctx, doc_type);
        if !dir.is_dir() {
            continue;
        }
        for file_entry in std::fs::read_dir(&dir)? {
            let file_entry = file_entry?;
            let path = file_entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                // Match: exact stem, stem contains needle, or slug portion after date matches
                let slug_portion = if stem.len() > 11 && stem.as_bytes().get(10) == Some(&b'-') {
                    &stem[11..]
                } else {
                    &stem
                };
                if stem == needle || slug_portion == needle || slug_portion.contains(&needle) {
                    return Ok((path, ctx.clone()));
                }
            }
        }
    }

    Err(TemperError::Vault(format!(
        "{doc_type} not found: {slug}"
    )))
}
```

- [ ] **Step 4: Write resource::update() with schema validation**

Add to `crates/temper-cli/src/commands/resource.rs`:

```rust
pub fn update(
    config: &Config,
    slug: &str,
    doc_type: Option<&str>,
    type_from: Option<&str>,
    type_to: Option<&str>,
    context: Option<&str>,
    context_to: Option<&str>,
    title: Option<&str>,
    tags: &[String],
    aliases: &[String],
    relates_to: &[String],
    references: &[String],
    depends_on: &[String],
    stage: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    goal: Option<&str>,
    seq: Option<i64>,
    branch: Option<&str>,
    pr: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    // Resolve the current type
    let current_type = type_from.or(doc_type).ok_or_else(|| {
        TemperError::Project("--type or --type-from is required for resource update".into())
    })?;
    validate_type(current_type)?;

    if let Some(tt) = type_to {
        validate_type(tt)?;
    }

    // Find the resource file
    let (path, found_ctx) = find_resource_file(config, current_type, slug, context)?;
    let mut content = std::fs::read_to_string(&path)?;

    // Validate fields against schema
    let updatable = temper_core::schema::updatable_fields(current_type)?;
    let updatable_names: Vec<&str> = updatable.iter().map(|(k, _)| k.as_str()).collect();

    // Apply scalar field updates with schema validation
    let field_updates: Vec<(&str, &str)> = [
        ("title", title),
        ("temper-stage", stage),
        ("temper-mode", mode),
        ("temper-effort", effort),
        ("temper-goal", goal),
        ("temper-branch", branch),
        ("temper-pr", pr),
        ("temper-status", status),
    ]
    .iter()
    .filter_map(|(field, val)| val.map(|v| (*field, v)))
    .collect();

    for (field, value) in &field_updates {
        // Check if field is valid for this type (title is always valid via base)
        if *field != "title" && !updatable_names.contains(field) {
            return Err(TemperError::Project(format!(
                "--{} is not valid for type '{}'",
                field.strip_prefix("temper-").unwrap_or(field),
                current_type
            )));
        }

        // Validate enum/type constraints
        if let Some((_, schema_prop)) = updatable.iter().find(|(k, _)| k == field) {
            if let Some(err) = temper_core::schema::validate_field_value(field, value, schema_prop) {
                return Err(TemperError::Project(err));
            }
        }

        content = crate::vault::set_frontmatter_field(&content, field, value);
    }

    // Apply seq if provided
    if let Some(s) = seq {
        if !updatable_names.contains(&"temper-seq") {
            return Err(TemperError::Project(format!(
                "--seq is not valid for type '{current_type}'"
            )));
        }
        content = crate::vault::set_frontmatter_field(&content, "temper-seq", &s.to_string());
    }

    // Apply array field appends
    for val in tags {
        content = append_frontmatter_array(&content, "tags", val);
    }
    for val in aliases {
        content = append_frontmatter_array(&content, "aliases", val);
    }
    for val in relates_to {
        content = append_frontmatter_array(&content, "relates_to", val);
    }
    for val in references {
        content = append_frontmatter_array(&content, "references", val);
    }
    for val in depends_on {
        content = append_frontmatter_array(&content, "depends_on", val);
    }

    // Update timestamp
    let now = chrono::Local::now().to_rfc3339();
    content = crate::vault::set_frontmatter_field(&content, "temper-updated", &now);

    // Handle context move
    let mut final_path = path.clone();
    if let Some(new_ctx) = context_to {
        content = crate::vault::set_frontmatter_field(&content, "temper-context", new_ctx);
        let filename = path.file_name().unwrap();
        let new_dir = config.doc_type_dir(new_ctx, current_type);
        std::fs::create_dir_all(&new_dir)?;
        final_path = new_dir.join(filename);
    }

    // Handle type change
    if let Some(new_type) = type_to {
        content = crate::vault::set_frontmatter_field(&content, "temper-type", new_type);
        let ctx = context_to.unwrap_or(&found_ctx);
        let filename = final_path.file_name().unwrap().to_owned();
        let new_dir = config.doc_type_dir(ctx, new_type);
        std::fs::create_dir_all(&new_dir)?;
        final_path = new_dir.join(filename);
    }

    // Write updated content
    if final_path != path {
        // Moving file: write to new location, remove old
        std::fs::write(&final_path, &content)?;
        std::fs::remove_file(&path)?;
    } else {
        std::fs::write(&path, &content)?;
    }

    let relative = final_path.strip_prefix(&config.vault_root).unwrap_or(&final_path);
    crate::output::success(format!("Updated: {}", relative.display()));

    // Emit discovery event
    let ts = chrono::Local::now().to_rfc3339();
    let event = crate::discovery::Event::ResourceUpdate {
        ts,
        doc_type: type_to.unwrap_or(current_type).to_string(),
        slug: slug.to_string(),
        context: context_to.unwrap_or(&found_ctx).to_string(),
    };
    if let Err(e) = crate::discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}

/// Append a value to a YAML array field in frontmatter.
fn append_frontmatter_array(content: &str, field: &str, value: &str) -> String {
    let marker = format!("\n{}:", field);
    if content.contains(&marker) {
        // Field exists — append to the list
        if let Some(pos) = content.find(&marker) {
            let after_marker = pos + marker.len();
            let new_entry = format!("\n  - {value}");
            let mut result = content.to_string();
            result.insert_str(after_marker, &new_entry);
            result
        } else {
            content.to_string()
        }
    } else {
        // Insert field before closing --- of frontmatter
        let trimmed_start = if content.starts_with("---") { 3 } else { 0 };
        if let Some(close_pos) = content[trimmed_start..].find("\n---") {
            let insert_at = trimmed_start + close_pos;
            let new_field = format!("\n{}:\n  - {}", field, value);
            let mut result = content.to_string();
            result.insert_str(insert_at, &new_field);
            result
        } else {
            content.to_string()
        }
    }
}
```

- [ ] **Step 5: Add ResourceUpdate event variant**

In `crates/temper-cli/src/discovery.rs`, add alongside the `ResourceCreate` variant from Task 4:

```rust
#[serde(rename = "resource_update")]
ResourceUpdate {
    ts: String,
    doc_type: String,
    slug: String,
    context: String,
},
```

- [ ] **Step 6: Verify compilation and run tests**

```bash
cargo make check
cargo make test
```

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-core/src/schema.rs crates/temper-cli/src/discovery.rs
git commit -m "feat: add resource show and update with schema-driven validation

Update validates field names and enum values against jsonschemas.
Supports --context-to for moves and --type-from/--type-to for type changes."
```

---

## Task 6: Fix Research Path Bug and Wire Up Remaining Delegates

Fix the research save path bug and ensure all delegate functions work correctly through the resource command.

**Files:**
- Modify: `crates/temper-cli/src/commands/research.rs:24` (fix path)
- Modify: `crates/temper-cli/src/commands/goal.rs` (add limit parameter to list)

- [ ] **Step 1: Fix research path**

In `crates/temper-cli/src/commands/research.rs`, change line 24 from:

```rust
let research_dir = config.vault_root.join("research").join(context_name);
```

To:

```rust
let research_dir = config.doc_type_dir(context_name, "research");
```

- [ ] **Step 2: Fix research frontmatter field name**

In the same file, line 52 sets `project` instead of `temper-context`:

```rust
content = vault::set_frontmatter_field(&content, "project", context_name);
```

Change to:

```rust
content = vault::set_frontmatter_field(&content, "temper-context", context_name);
```

- [ ] **Step 3: Verify research create works end-to-end**

```bash
cargo make test
```

Run a manual test if possible:
```bash
echo "Test content" | cargo run -p temper-cli -- resource create --type research --title "test-research-path" --context temper
```

Verify the file lands in `{vault}/temper/research/` not `{vault}/research/temper/`.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/research.rs crates/temper-cli/src/commands/goal.rs
git commit -m "fix: research create now writes to {context}/research/ instead of research/{context}/"
```

---

## Task 7: Consolidate Discovery Events

Replace per-type discovery events with generic `ResourceCreate` and `ResourceUpdate`.

**Files:**
- Modify: `crates/temper-cli/src/discovery.rs` (consolidate Event enum)
- Modify: `crates/temper-cli/src/commands/research.rs` (use ResourceCreate)
- Modify: `crates/temper-cli/src/commands/session.rs` (use ResourceCreate)
- Modify: `crates/temper-cli/src/commands/note.rs` (use ResourceCreate)
- Modify: `crates/temper-cli/src/actions/task.rs` (use ResourceCreate/ResourceUpdate)
- Modify: `crates/temper-cli/src/actions/goal.rs` (use ResourceCreate/ResourceUpdate)

- [ ] **Step 1: Read all event emission sites**

Grep for `Event::` across the CLI crate:
```bash
grep -rn "Event::" crates/temper-cli/src/
```

Read each file that emits events.

- [ ] **Step 2: Update Event enum**

In `crates/temper-cli/src/discovery.rs`, keep `ResourceCreate` and `ResourceUpdate` (added in Tasks 4-5). Remove `NoteCreate`, `TaskCreate`, `TaskMove`, `TaskDone`, `GoalCreate`, `GoalUpdate`. Keep `Normalize` (different concern).

The final enum should be:

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "resource_create")]
    ResourceCreate {
        ts: String,
        doc_type: String,
        title: String,
        path: String,
        context: String,
    },
    #[serde(rename = "resource_update")]
    ResourceUpdate {
        ts: String,
        doc_type: String,
        slug: String,
        context: String,
    },
    #[serde(rename = "normalize")]
    Normalize {
        ts: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        ids_backfilled: u32,
        files_moved: u32,
        stages_migrated: u32,
        slugs_fixed: u32,
        frontmatter_fixed: u32,
    },
}
```

- [ ] **Step 3: Update all event emission sites**

Replace each `Event::NoteCreate`, `Event::TaskCreate`, `Event::GoalCreate` with `Event::ResourceCreate`.
Replace each `Event::TaskMove`, `Event::TaskDone`, `Event::GoalUpdate` with `Event::ResourceUpdate`.

For each file, the pattern is the same — change the variant name and adjust fields.

Example in `actions/task.rs` for create:
```rust
let event = Event::ResourceCreate {
    ts,
    doc_type: "task".to_string(),
    title: title.to_string(),
    path: relative_str.to_string(),
    context: context.to_string(),
};
```

Example in `actions/task.rs` for move:
```rust
let event = Event::ResourceUpdate {
    ts,
    doc_type: "task".to_string(),
    slug: slug.to_string(),
    context: context.to_string(),
};
```

- [ ] **Step 4: Verify**

```bash
cargo make check
cargo make test
```

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/discovery.rs crates/temper-cli/src/commands/ crates/temper-cli/src/actions/
git commit -m "refactor: consolidate discovery events to ResourceCreate and ResourceUpdate"
```

---

## Task 8: Remove Old Command Modules

Now that all traffic routes through `resource`, remove the old per-type command modules and CLI enum variants.

**Files:**
- Delete: `crates/temper-cli/src/commands/note.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (remove `pub mod note;`)
- Modify: `crates/temper-cli/src/cli.rs` (verify old enums are gone)

Note: Do NOT delete `commands/task.rs`, `commands/session.rs`, `commands/research.rs`, `commands/goal.rs` — these still contain the delegated action logic (`create()`, `list()`, `show()`, `save()`, `move_task()`, etc.) that `resource.rs` calls. Only `note.rs` is fully replaced by `resource::create_simple_resource()`.

- [ ] **Step 1: Verify note.rs is no longer imported**

```bash
grep -rn "commands::note" crates/temper-cli/src/
```

If nothing references it (main.rs dispatch was removed in Task 3), proceed.

- [ ] **Step 2: Remove note.rs and its module declaration**

```bash
rm crates/temper-cli/src/commands/note.rs
```

In `crates/temper-cli/src/commands/mod.rs`, remove `pub mod note;`.

- [ ] **Step 3: Clean up unused imports**

Run `cargo make check` and fix any dead code warnings or unused import errors across the crate.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/
git commit -m "chore: remove note command module, now replaced by resource create"
```

---

## Task 9: Update Skill Generation and Reference Content

Update the skill generate/install to reflect the new `resource` command surface.

**Files:**
- Modify: `crates/temper-cli/src/commands/skill.rs:26-81` (update REFERENCE_FOOTER)
- Modify: `crates/temper-cli/src/commands/skill.rs:110-111` (update stdin note)
- Modify: `crates/temper-cli/src/commands/skill.rs:569-623` (update tests)

- [ ] **Step 1: Read skill.rs fully**

Read `crates/temper-cli/src/commands/skill.rs` — especially the `REFERENCE_FOOTER` static (lines 26-81) and `generate_reference()` function (lines 84-115).

Note: The command table in `reference.md` is auto-generated from clap's command tree — since we changed the `Commands` enum in Task 3, the table will automatically reflect the new `resource` subcommands. Only the footer and stdin note need manual updates.

- [ ] **Step 2: Update REFERENCE_FOOTER**

Replace the `REFERENCE_FOOTER` static content to reflect the new command surface:

```rust
static REFERENCE_FOOTER: &str = r#"
## Resource Types

| Type | Description |
|------|-------------|
| task | Work items with stage, mode, effort tracking |
| goal | High-level objectives that group tasks |
| session | Timestamped work session notes |
| research | Research notes and findings |
| concept | Named ideas, patterns, or domain terms |
| decision | Point-in-time choices with rationale (ADR-like) |

## Task Stages

| Stage | Meaning |
|-------|---------|
| backlog | Not yet started |
| in-progress | Actively being worked |
| done | Completed |
| cancelled | Abandoned or no longer relevant |

## Modes

| Mode | Purpose |
|------|---------|
| plan | Research, design, discovery -- understanding before building |
| build | Implementation -- producing artifacts |

## Effort Levels

| Effort | Scope |
|--------|-------|
| small | Single session, focused deliverable |
| medium | Multi-step, bounded to a clear outcome |
| large | Multi-session, may require decomposition |

## Discovery Workflow

1. `temper search "<topic>"` -- find relevant documents and notes
2. `temper context [<name>]` -- understand current context and recent activity
3. Use search results to guide targeted file reads

Search first, read second. Don't guess at file paths.

## Template Access

Use `--show-template` on resource create to display the expected frontmatter:

```bash
temper resource create --type session --show-template
temper resource create --type task --show-template
temper resource create --type decision --show-template
```

## Skill-Only Commands

These commands are handled by the skill routing layer, not the temper CLI directly.
They compose multiple CLI commands into guided workflows.

| Skill Command | What It Does |
|---------------|-------------|
| `task start <slug>` | Shows task, moves to in-progress, routes to workflow |
| `task resume <slug>` | Shows task, reads last session, continues workflow |
| `task create` | Guided interactive task creation with prompts |
| `session start` | Start a session without a predefined task |
"#;
```

- [ ] **Step 3: Update stdin pipe note**

In `generate_reference()` around line 111, change:

```rust
out.push_str(
    "\nPipe content via stdin for `session save`, `note create`, and `research save`.\n",
);
```

To:

```rust
out.push_str(
    "\nPipe content via stdin for `resource create` (all types accept stdin body).\n",
);
```

- [ ] **Step 4: Update skill.rs tests**

Update the test assertions in skill.rs to match the new command names:
- `test_generate_reference_contains_all_commands`: change `"task create"` → `"resource create"`, `"task list"` → `"resource list"`, `"session list"` → check for `"resource list"`
- Other tests should still pass since they check for flags that still exist

- [ ] **Step 5: Update skill SKILL.md template references**

Read `crates/temper-cli/skill-content/` files and the SKILL.md Askama template. Update any references to old commands (`temper task show`, `temper session save`, etc.) to use the new `temper resource` equivalents.

Check these files:
- `crates/temper-cli/templates/skill.md` (the Askama SKILL.md template)
- `crates/temper-cli/skill-content/session-lifecycle.md`
- `crates/temper-cli/skill-content/workflows/*.md`

- [ ] **Step 6: Verify and reinstall skill**

```bash
cargo make check
cargo make test
cargo run -p temper-cli -- skill generate | head -40  # preview
```

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/skill.rs crates/temper-cli/skill-content/ crates/temper-cli/templates/skill.md
git commit -m "docs: update skill reference and content for unified resource command"
```

---

## Task 10: Update temper-ui Docs Page

Update the CLI reference documentation in the SvelteKit UI.

**Files:**
- Modify: `packages/temper-ui/src/routes/docs/+page.svelte`

- [ ] **Step 1: Read current docs page**

Read `packages/temper-ui/src/routes/docs/+page.svelte` fully.

- [ ] **Step 2: Update CLI reference tables**

Replace the per-type command sections (Core, Content, Goals and Tasks, etc.) with sections organized around the new command surface:

- **Core Commands:** init, check, status, warmup, doctor, events (unchanged)
- **Resources:** resource create, resource list, resource show, resource update (with type examples)
- **Search:** search (unchanged)
- **Contexts and Skills:** context, skill (unchanged)
- **Cloud:** auth, sync, pull, remove (unchanged)

Show examples with different `--type` values so users understand the unified pattern.

- [ ] **Step 3: Verify build**

```bash
cd packages/temper-ui && bun run check && bun run build
```

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/routes/docs/+page.svelte
git commit -m "docs: update temper-ui CLI reference for unified resource command"
```

---

## Task 11: Integration Testing and Final Verification

End-to-end verification that all resource commands work correctly.

**Files:**
- Modify: existing test files as needed for compilation

- [ ] **Step 1: Build the CLI binary**

```bash
cargo build -p temper-cli
```

- [ ] **Step 2: Run full test suite**

```bash
cargo make check
cargo make test
```

Fix any remaining failures.

- [ ] **Step 3: Manual smoke tests**

Test each command manually against the local vault:

```bash
# Create
temper resource create --type concept --title "Test Concept" --context temper
temper resource create --type decision --title "Test Decision" --context temper

# List
temper resource list --type task --context temper --limit 3
temper resource list --type goal --context temper
temper resource list --type session --context temper --limit 3
temper resource list --type concept --context temper
temper resource list --type decision --context temper

# Show
temper resource show --type task <a-real-task-slug> --context temper

# Update
temper resource update --type concept test-concept --context temper --tags "test-tag"

# Show template
temper resource create --type decision --show-template
```

- [ ] **Step 4: Reinstall skill and verify**

```bash
temper skill install
temper skill check
```

- [ ] **Step 5: Clean up any test resources created**

Remove test concept and decision files created during smoke testing.

- [ ] **Step 6: Final commit if any fixes were needed**

```bash
cargo make check
cargo make test
git add -A
git commit -m "test: fix integration issues from unified resource command migration"
```

---

## Task 12: Session Save and Task Completion

Save a session note and mark the task as done.

- [ ] **Step 1: Save session**

```bash
cat <<'EOF' | temper resource create --type session --title "unified-resource-command-implementation" --context temper
## Goal
Replace per-doctype CLI commands with unified `temper resource {create,list,show,update}`,
add concept/decision doctypes, clean up dead config/code.

## What Happened
<fill in actual work done>

## Decisions
- Unified resource command with --type flag for all doctypes
- Schema-driven update validation from jsonschemas
- --type-from/--type-to for type mutation, --context-to for context moves
- Removed sync.auto dead config, LEGACY_FIELD_MAP, stale template copies
- Consolidated discovery events to ResourceCreate/ResourceUpdate

## Connections
- Task: 2026-04-06-standardize-doc-creation-commands-across-the-board-and-fix-the-research-save-bug
- Folded: 2026-04-05-add-temper-task-update-command-for-editing-task-frontmatter-fields
- Spec: docs/superpowers/specs/2026-04-06-unified-resource-command-design.md

## Next Steps
- Update the temper skill's SKILL.md router to use new command names
- Verify cloud sync handles context/type changes correctly
- Consider adding --remove flag for array fields (tags, relates-to, etc.)
EOF
```

- [ ] **Step 2: Mark task done**

```bash
temper resource update --type task 2026-04-06-standardize-doc-creation-commands-across-the-board-and-fix-the-research-save-bug --stage done --branch jct/cli-standardization-and-documentation
```

Also mark the folded task as done:
```bash
temper resource update --type task 2026-04-05-add-temper-task-update-command-for-editing-task-frontmatter-fields --stage done
```
