use std::collections::HashMap;
use std::path::Path;

use askama::Template;
use clap::CommandFactory;
use sha2::{Digest, Sha256};

use crate::cli::Cli;
use crate::config::{self, Config};
use crate::error::{Result, TemperError};
use crate::output;
use crate::templates::{CommandWrapperTemplate, SkillTemplate};

// ── Static content (compiled into the binary) ────────────────────────────────

static SUBAGENT_GUIDANCE_MD: &str = include_str!("../../skill-content/subagent-guidance.md");
static SESSION_LIFECYCLE_MD: &str = include_str!("../../skill-content/session-lifecycle.md");
static KNOWLEDGE_BASE_MD: &str = include_str!("../../../../agent-skills/knowledge-base.md");
static WF_BUILD_SMALL: &str = include_str!("../../skill-content/workflows/build-small.md");
static WF_BUILD_MEDIUM: &str = include_str!("../../skill-content/workflows/build-medium.md");
static WF_BUILD_LARGE: &str = include_str!("../../skill-content/workflows/build-large.md");
static WF_PLAN_SMALL: &str = include_str!("../../skill-content/workflows/plan-small.md");
static WF_PLAN_MEDIUM: &str = include_str!("../../skill-content/workflows/plan-medium.md");
static WF_PLAN_LARGE: &str = include_str!("../../skill-content/workflows/plan-large.md");

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
4. Reach for `--meta-only` / `--fields` on `resource show` and `resource list`
   when you need cheap orientation rather than full bodies (see below)

Search first, read second. Don't guess at file paths.

## Orientation Projection (cheap reads)

The read-side projection flags let you peek at structure without paying for
full bodies — useful when triaging a context, comparing a few resources, or
deciding whether to read more deeply.

| Pattern | What it returns |
|---------|-----------------|
| `temper resource show <ref> --meta-only` | Frontmatter (managed + open) and hashes; no body. Calls `GET /api/resources/<id>/meta`. |
| `temper resource list --type <t> --context <ctx> --meta-only` | Meta-tier rows instead of full row payloads. |
| `--fields <a,b,c>` on either of the above | Subselects top-level response keys. The anchor key (`id` or `resource_id`) is always preserved. Pipe through `jq` for nested projection. |
| `temper resource show <ref> --edges` | Adds the graph edges connected to this resource. Mutually exclusive with `--meta-only`. |

## Vault Projection (local cache)

The vault directory is a **read-only projection cache** of cloud state. To
refresh missing or stale projected files for a context:

```bash
temper pull <context>
```

`rm`'ing a projected file has no server effect — it just creates a local cache
miss. To actually delete a resource server-side, run `temper resource delete
<ref> [--force]` (the `<ref>` is the resource's `ref` field from `list`/`show`).

## Context Requirement

| Verb | `--context` required? |
|------|----------------------|
| `resource list` | optional (omitting lists across all contexts) |
| `resource show` | **required** |
| `resource create` | **required** |
| `resource update` | **required** |
| `resource delete` | **required** |

Omitting `--context` where it is required surfaces the error
`Project error: no context specified — use --context <name>`.

## Template Access

Use `--show-template` on `resource create` to display the expected frontmatter and body
structure without creating anything:

```bash
temper resource create --type session --show-template
temper resource create --type task --show-template
temper resource create --type research --show-template
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

/// Generate the reference.md content from clap's command tree.
pub fn generate_reference() -> String {
    let cmd = Cli::command();
    let mut rows = Vec::new();
    collect_command_rows(&cmd, "", &mut rows);

    let mut out = String::new();
    out.push_str("# CLI Reference\n\n");
    out.push_str("## Invocation\n\n");
    out.push_str("**Always run `temper` directly from PATH.** Never use `cargo run -p temper-cli`, `python`,\n");
    out.push_str("full paths, or any indirect invocation method — even when working inside the temper source\n");
    out.push_str(
        "repository. The installed binary may differ from the in-development code, and that is\n",
    );
    out.push_str("intentional: we use the installed CLI to manage our own workflow while evolving the crate.\n\n");
    out.push_str("**Before running any temper command**, verify the binary exists:\n");
    out.push_str("```bash\nwhich temper\n```\n");
    out.push_str("If `temper` is not on PATH, **stop and warn the user**:\n");
    out.push_str("> \"The `temper` binary is not installed or not on PATH. Install it with\n");
    out.push_str("> `cargo install --path crates/temper-cli` or ensure `~/.cargo/bin` is in your PATH.\"\n\n");
    out.push_str("Do not fall back to `cargo run` as a workaround.\n\n");
    out.push_str("## Commands\n\n");
    out.push_str("| Command | Syntax |\n");
    out.push_str("|---------|--------|\n");
    for (name, syntax) in &rows {
        out.push_str(&format!("| {} | `{}` |\n", name, syntax));
    }
    out.push_str("\nPipe content via stdin for `resource create` (all types accept stdin body).\n");
    out.push_str(REFERENCE_FOOTER);
    out
}

/// Recursively collect (command_name, syntax_string) rows from the clap command tree.
fn collect_command_rows(cmd: &clap::Command, prefix: &str, rows: &mut Vec<(String, String)>) {
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let name = sub.get_name();
        let full_name = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{} {}", prefix, name)
        };

        // If this command has subcommands, recurse into them
        let child_subs: Vec<_> = sub.get_subcommands().filter(|c| !c.is_hide_set()).collect();
        if !child_subs.is_empty() {
            // If subcommand is optional, also emit a row for the parent command
            if !sub.is_subcommand_required_set() {
                let syntax = build_syntax(&full_name, sub);
                rows.push((full_name.clone(), syntax));
            }
            collect_command_rows(sub, &full_name, rows);
        } else {
            let syntax = build_syntax(&full_name, sub);
            rows.push((full_name, syntax));
        }
    }
}

/// Build a syntax string like `temper task create --title <title> [--context <ctx>]`
fn build_syntax(full_name: &str, cmd: &clap::Command) -> String {
    let mut parts = vec![format!("temper {}", full_name)];

    for arg in cmd.get_arguments() {
        // Skip hidden args
        if arg.is_hide_set() {
            continue;
        }
        let id = arg.get_id().as_str();
        // Skip --format (implementation detail)
        if id == "format" {
            continue;
        }
        // Skip global --help and --version
        if id == "help" || id == "version" || id == "vault" {
            continue;
        }

        if arg.is_positional() {
            if arg.is_required_set() {
                parts.push(format!("<{}>", id));
            } else {
                parts.push(format!("[<{}>]", id));
            }
        } else {
            // Flag/option arg
            let long = arg
                .get_long()
                .map(|l| format!("--{}", l))
                .unwrap_or_else(|| format!("--{}", id));

            let takes_value = !matches!(
                arg.get_action(),
                clap::ArgAction::SetTrue | clap::ArgAction::SetFalse | clap::ArgAction::Count
            );
            if takes_value {
                if arg.is_required_set() {
                    parts.push(format!("{} <{}>", long, id));
                } else {
                    parts.push(format!("[{} <{}>]", long, id));
                }
            } else {
                // Boolean flag
                parts.push(format!("[{}]", long));
            }
        }
    }

    parts.join(" ")
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Generate all skill files as a map of relative_path → content.
pub fn generate_skill_files(config: &Config) -> Result<HashMap<String, String>> {
    let hash = compute_config_hash()?;
    generate_skill_files_with_hash(config, &hash)
}

/// Returns the generated reference.md content for stdout preview.
pub fn generate(_config: &Config) -> Result<String> {
    Ok(generate_reference())
}

/// Report of what changed during `install`.
///
/// Lets the caller distinguish between a real refresh and a no-op so agents
/// (and humans) can tell whether an install actually did anything.
#[derive(Debug, Default)]
pub struct InstallReport {
    pub total: usize,
    pub changed: Vec<String>,
}

impl InstallReport {
    pub fn is_no_op(&self) -> bool {
        self.changed.is_empty()
    }
}

/// Install skill directory and command wrapper.
///
/// 1. Generate all skill files
/// 2. Write skill files (except command-wrapper.md) into `skill_dir`
/// 3. Write command-wrapper.md to `~/.claude/commands/temper.md`
///
/// Skips writes when the destination already matches the generated content,
/// returning an `InstallReport` that lists every file whose bytes changed.
pub fn install(config: &Config, skill_dir: &Path) -> Result<InstallReport> {
    let files = generate_skill_files(config)?;
    let mut report = InstallReport {
        total: files.len(),
        changed: Vec::new(),
    };

    // Ensure skill_dir and subdirectories exist
    for sub in &["workflows", "guidance"] {
        let dir = skill_dir.join(sub);
        std::fs::create_dir_all(&dir).map_err(|e| {
            TemperError::Config(format!("cannot create directory {}: {}", dir.display(), e))
        })?;
    }

    // Write all files except command-wrapper.md into skill_dir
    for (rel_path, content) in &files {
        if rel_path == "command-wrapper.md" {
            continue;
        }
        let dest = skill_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TemperError::Config(format!(
                    "cannot create parent dir for {}: {}",
                    dest.display(),
                    e
                ))
            })?;
        }
        if write_if_changed(&dest, content)? {
            report.changed.push(rel_path.clone());
        }
    }

    // Write command-wrapper.md to ~/.claude/commands/temper.md
    if let Some(wrapper_content) = files.get("command-wrapper.md") {
        let home = dirs::home_dir()
            .ok_or_else(|| TemperError::Config("cannot determine home directory".to_string()))?;
        let commands_dir = home.join(".claude/commands");
        std::fs::create_dir_all(&commands_dir).map_err(|e| {
            TemperError::Config(format!("cannot create {}: {}", commands_dir.display(), e))
        })?;
        let wrapper_path = commands_dir.join("temper.md");
        if write_if_changed(&wrapper_path, wrapper_content)? {
            report.changed.push("command-wrapper.md".to_string());
        }
    }

    Ok(report)
}

/// Write `content` to `dest` only if the on-disk bytes differ. Returns
/// `true` if a write happened (or the file did not previously exist).
fn write_if_changed(dest: &Path, content: &str) -> Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(dest) {
        if existing == content {
            return Ok(false);
        }
    }
    std::fs::write(dest, content)
        .map_err(|e| TemperError::Config(format!("cannot write {}: {}", dest.display(), e)))?;
    Ok(true)
}

/// Check skill installation status.
pub fn check(config: &Config) -> Result<()> {
    // 1. Check skill directory exists
    let skill_dir = &config.skill_output;
    if !skill_dir.exists() {
        output::status_icon(
            false,
            format!("Skill directory: NOT FOUND ({})", skill_dir.display()),
        );
        output::hint("  Run: temper skill install");
        return Ok(());
    }

    output::status_icon(true, format!("Skill directory: {}", skill_dir.display()));

    // 2. Check expected files
    let expected_files = [
        "SKILL.md",
        "reference.md",
        "subagent-guidance.md",
        "session-lifecycle.md",
        "knowledge-base.md",
        "workflows/build-small.md",
        "workflows/build-medium.md",
        "workflows/build-large.md",
        "workflows/plan-small.md",
        "workflows/plan-medium.md",
        "workflows/plan-large.md",
    ];

    let mut all_present = true;
    for file in &expected_files {
        let path = skill_dir.join(file);
        if !path.exists() {
            output::status_icon(false, format!("Missing: {}", file));
            all_present = false;
        }
    }
    if all_present {
        output::status_icon(
            true,
            format!("All {} skill files present", expected_files.len()),
        );
    }

    // 3. Check config hash staleness in SKILL.md
    let skill_md_path = skill_dir.join("SKILL.md");
    if skill_md_path.exists() {
        let existing = std::fs::read_to_string(&skill_md_path)
            .map_err(|e| TemperError::Config(format!("cannot read SKILL.md: {}", e)))?;

        let embedded_hash = extract_config_hash(&existing);
        let current_hash = compute_config_hash()?;

        match embedded_hash {
            Some(h) if h == current_hash => {
                output::status_icon(true, "Hash: up to date");
            }
            Some(h) => {
                output::status_icon(false, "Hash: STALE");
                output::plain(format!("  Embedded: {}", h));
                output::plain(format!("  Current:  {}", current_hash));
                output::hint("  Run: temper skill install");
            }
            None => {
                output::warning("Hash: UNKNOWN (no config-hash comment found)");
            }
        }
    }

    // 5. Check command wrapper at ~/.claude/commands/temper.md
    let wrapper_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".claude/commands/temper.md");

    if wrapper_path.exists() {
        output::status_icon(true, format!("Command wrapper: {}", wrapper_path.display()));
    } else {
        output::status_icon(
            false,
            format!("Command wrapper: NOT FOUND ({})", wrapper_path.display()),
        );
        output::hint("  Run: temper skill install");
    }

    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Generate all skill files with a pre-computed config hash (for testability).
pub fn generate_skill_files_with_hash(
    config: &Config,
    hash: &str,
) -> Result<HashMap<String, String>> {
    let vault_path = config.vault_root.display().to_string();
    let context_list = format_context_list(&config.contexts);

    let skill_template = SkillTemplate {
        config_hash: hash,
        vault_path: &vault_path,
        context_list: &context_list,
    };

    let wrapper_template = CommandWrapperTemplate { config_hash: hash };

    let mut files = HashMap::new();

    files.insert(
        "SKILL.md".to_string(),
        skill_template
            .render()
            .map_err(|e| TemperError::Config(format!("template render error: {}", e)))?,
    );

    files.insert(
        "command-wrapper.md".to_string(),
        wrapper_template
            .render()
            .map_err(|e| TemperError::Config(format!("template render error: {}", e)))?,
    );

    files.insert("reference.md".to_string(), generate_reference());
    files.insert(
        "subagent-guidance.md".to_string(),
        SUBAGENT_GUIDANCE_MD.to_string(),
    );
    files.insert(
        "session-lifecycle.md".to_string(),
        SESSION_LIFECYCLE_MD.to_string(),
    );

    files.insert(
        "knowledge-base.md".to_string(),
        KNOWLEDGE_BASE_MD.to_string(),
    );

    files.insert(
        "workflows/build-small.md".to_string(),
        WF_BUILD_SMALL.to_string(),
    );
    files.insert(
        "workflows/build-medium.md".to_string(),
        WF_BUILD_MEDIUM.to_string(),
    );
    files.insert(
        "workflows/build-large.md".to_string(),
        WF_BUILD_LARGE.to_string(),
    );
    files.insert(
        "workflows/plan-small.md".to_string(),
        WF_PLAN_SMALL.to_string(),
    );
    files.insert(
        "workflows/plan-medium.md".to_string(),
        WF_PLAN_MEDIUM.to_string(),
    );
    files.insert(
        "workflows/plan-large.md".to_string(),
        WF_PLAN_LARGE.to_string(),
    );

    Ok(files)
}

/// Compute SHA256 hash of the global config file.
fn compute_config_hash() -> Result<String> {
    let config_path = config::global_config_path();
    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| TemperError::Config(format!("cannot read config: {}", e)))?;
    Ok(format!("{:x}", Sha256::digest(config_content.as_bytes())))
}

/// Format contexts as sorted markdown list items.
pub fn format_context_list(contexts: &[String]) -> String {
    if contexts.is_empty() {
        return "(no contexts configured)".to_string();
    }
    let mut sorted = contexts.to_vec();
    sorted.sort();
    sorted
        .iter()
        .map(|ctx| format!("- `{ctx}`"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the config hash from a `<!-- config-hash: ... -->` comment.
pub fn extract_config_hash(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("<!-- config-hash: ") {
            if let Some(hash) = rest.strip_suffix(" -->") {
                return Some(hash.to_string());
            }
        }
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    fn test_config() -> Config {
        Config {
            vault_root: PathBuf::from("/tmp/test-vault"),
            state_dir: PathBuf::from("/tmp/test-vault/.temper"),
            contexts: vec!["alpha".to_string(), "beta".to_string()],
            subscriptions: Vec::new(),
            skill_output: PathBuf::from("/tmp/test-skill-output"),
            profile_slug: None,
        }
    }

    #[test]
    fn test_generate_skill_files_contains_expected_keys() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();

        assert!(files.contains_key("SKILL.md"));
        assert!(files.contains_key("reference.md"));
        assert!(files.contains_key("subagent-guidance.md"));
        assert!(files.contains_key("session-lifecycle.md"));
        assert!(files.contains_key("knowledge-base.md"));
        assert!(files.contains_key("workflows/build-small.md"));
        assert!(files.contains_key("workflows/build-medium.md"));
        assert!(files.contains_key("workflows/build-large.md"));
        assert!(files.contains_key("workflows/plan-small.md"));
        assert!(files.contains_key("workflows/plan-medium.md"));
        assert!(files.contains_key("workflows/plan-large.md"));
        assert!(files.contains_key("command-wrapper.md"));
    }

    #[test]
    fn test_generate_skill_md_contains_vault_and_contexts() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let skill_md = &files["SKILL.md"];

        assert!(skill_md.contains("/tmp/test-vault"));
        assert!(skill_md.contains("alpha"));
        assert!(skill_md.contains("beta"));
        assert!(skill_md.contains("config-hash: testhash"));
    }

    #[test]
    fn test_generate_command_wrapper_contains_hash() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let wrapper = &files["command-wrapper.md"];

        assert!(wrapper.contains("config-hash: testhash"));
        assert!(wrapper.contains("Invoke the temper skill"));
    }

    #[test]
    fn test_format_context_list_sorted() {
        let contexts = vec![
            "zebra".to_string(),
            "alpha".to_string(),
            "middle".to_string(),
        ];
        let result = format_context_list(&contexts);
        assert!(result.starts_with("- `alpha`"));
        assert!(result.contains("- `middle`"));
        assert!(result.ends_with("- `zebra`"));
    }

    #[test]
    fn test_format_context_list_empty() {
        let result = format_context_list(&[]);
        assert_eq!(result, "(no contexts configured)");
    }

    #[test]
    fn test_extract_config_hash_found() {
        let content = "<!-- config-hash: abc123 -->\n---\nname: temper\n---";
        assert_eq!(extract_config_hash(content), Some("abc123".to_string()));
    }

    #[test]
    fn test_extract_config_hash_not_found() {
        let content = "---\nname: temper\n---";
        assert_eq!(extract_config_hash(content), None);
    }

    #[test]
    fn test_generate_reference_contains_all_commands() {
        let reference = generate_reference();
        assert!(
            reference.contains("| init |"),
            "should contain init command"
        );
        assert!(
            reference.contains("| resource create |"),
            "should contain resource create"
        );
        assert!(
            reference.contains("| resource list |"),
            "should contain resource list"
        );
        assert!(
            reference.contains("| resource show |"),
            "should contain resource show"
        );
        assert!(reference.contains("| warmup |"), "should contain warmup");
        assert!(reference.contains("| search |"), "should contain search");
    }

    #[test]
    fn test_generate_reference_shows_actual_flags() {
        let reference = generate_reference();
        // These flags were recently added - they MUST appear
        assert!(
            reference.contains("--stage"),
            "should contain --stage flag from task list"
        );
        assert!(
            reference.contains("--limit"),
            "should contain --limit flag from session list"
        );
    }

    #[test]
    fn test_generate_reference_excludes_hidden_args() {
        let reference = generate_reference();
        // --stdin is hidden in clap definitions
        assert!(
            !reference.contains("--stdin"),
            "should NOT contain hidden --stdin flag"
        );
    }

    #[test]
    fn test_generate_reference_excludes_format_flag() {
        let reference = generate_reference();
        // --format is an implementation detail, should not appear in syntax column
        assert!(
            !reference.contains("--format"),
            "should NOT contain --format flag in syntax column"
        );
    }

    #[test]
    fn test_generate_reference_has_footer_sections() {
        let reference = generate_reference();
        assert!(
            reference.contains("## Task Stages"),
            "should have Task Stages section"
        );
        assert!(reference.contains("## Modes"), "should have Modes section");
        assert!(
            reference.contains("## Skill-Only Commands"),
            "should have Skill-Only Commands"
        );
        assert!(
            reference.contains("## Orientation Projection"),
            "should have Orientation Projection section"
        );
        assert!(
            reference.contains("## Context Requirement"),
            "should have Context Requirement section"
        );
    }

    #[test]
    fn test_generate_skill_files_uses_generated_reference() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let reference = &files["reference.md"];
        // Should contain generated commands, not stale static content
        assert!(
            reference.contains("--stage"),
            "installed reference.md should have --stage"
        );
    }
}
