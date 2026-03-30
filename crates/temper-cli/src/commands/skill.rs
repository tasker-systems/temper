use std::path::Path;

use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::error::{Result, TemperError};
use crate::output;

/// Generate the skill file content as a string.
pub fn generate(config: &Config) -> Result<String> {
    let toml_path = config.vault_root.join("temper.toml");
    let toml_content = std::fs::read_to_string(&toml_path)
        .map_err(|e| TemperError::Config(format!("cannot read temper.toml: {}", e)))?;
    let hash = format!("{:x}", Sha256::digest(toml_content.as_bytes()));

    let vault_path = config.vault_root.display().to_string();

    let mut project_lines = Vec::new();
    let mut sorted_projects: Vec<_> = config.projects.values().collect();
    sorted_projects.sort_by_key(|p| &p.name);
    for project in sorted_projects {
        project_lines.push(format!("- `{}` — {}", project.name, project.path.display()));
    }
    let project_list = if project_lines.is_empty() {
        "(no contexts configured)".to_string()
    } else {
        project_lines.join("\n")
    };

    let content = format!(
        r#"<!-- config-hash: {hash} -->
---
name: temper
description: Knowledge vault operations — context lookup, session notes, task management, semantic search
---

# Temper — Vault Workflow Tool

Vault: {vault_path}

## Invocation

`temper` is an installed binary in `$PATH`. Always run it directly as `temper <subcommand>`.
Never use `cargo run`, `python`, full binary paths, or any other indirect method — even when
working inside the temper source repo.

## Contexts
{project_list}

## Commands

- `temper search <query>` — Semantic search across indexed content (reach for this before grep/find)
- `temper context <topic> [--depth N]` — Traverse nearest neighbors for related content
- `temper session save [<title>] [--task <slug>] [--state <state>]` — Create/update session note, optionally link to task (stdin auto-detected — pipe content to populate the body; without stdin, creates from template with placeholder text that must be edited)
- `temper session list` — List recent sessions
- `temper task create --title <t> [--context <c>] [--mode plan|build] [--effort small|medium|large]` — Create task (stdin auto-detected)
- `temper task list [--context <c>] [--format json|text]` — List tasks
- `temper task move <slug> --stage <s> [--context <c>] [--mode plan|build] [--effort small|medium|large]` — Move task between stages or update mode/effort
- `temper task done <slug> [--context <c>]` — Mark task done
- `temper task show <slug> [--context <c>] [--format json|text]` — Show task content
- `temper task start <slug> [--context <c>]` — Move to in-progress, show content, invoke brainstorming skill
- `temper goal list [--context <c>]` — Roadmap view
- `temper note create <type> <title> [--context <c>]` — Create note from template (stdin auto-detected)
- `temper research save <title> [--context <c>]` — Create high-fidelity research note (stdin auto-detected)
- `temper normalize [--context <c>] [--dry-run]` — Repair vault structure drift
- `temper events [--context <c>] [--limit <n>]` — Show recent vault events
- `temper warmup [--context <c>]` — Context primer for new sessions
- `temper index` — Rebuild search index
- `temper status` — Vault overview

## Discovery

Before launching subagents to grep or find across the vault, use temper's semantic tools:

- `temper search "<query>"` uses embeddings to find conceptually related content — not just keyword matches
- `temper context <topic> --depth 2` traverses nearest neighbors in the HNSW index to surface related entities
- Workflow: search → context → targeted file reads. This is what the index is for.

Templates are available via `--show-template` on create/save commands.

## Stages

Tasks use four stages: `backlog`, `in-progress`, `done`, `cancelled`.

## Mode and Effort

Tasks have two optional classification fields:

**Mode** (`--mode`): `plan` or `build`
- `plan` — research, design, discovery work; output is knowledge, not code
- `build` — implementation work; output is delivered code

**Effort** (`--effort`): `small`, `medium`, or `large`
- `small` — single focused session
- `medium` — multi-step but bounded
- `large` — multi-session, may spawn sub-tasks

## Workflow Integration

When starting a session:
- Check for recent sessions: `temper session list --context <current>`
- Search for relevant context: `temper search "<topic>"`

When ending a session:
- Pipe session content via stdin: `cat <<'EOF' | temper session save "<title>" --task <slug> --state done\n<content>\nEOF`
- Or create from template and edit after: `temper session save "<title>"` then edit the file
- **Important**: Without stdin, `temper session save` creates a template with placeholder text. You must either pipe content or edit the file afterward — otherwise the session note will be empty boilerplate.

When the user says `/temper task start <slug>`:
1. Run `temper task move <slug> --stage in-progress --context <c>`
2. Run `temper task show <slug>`
3. Check the `mode` and `effort` fields and route accordingly:

### Mode + Effort Routing

**If mode and effort are set**, announce the workflow:

| Mode | Effort | Workflow |
|------|--------|----------|
| `build` | `small` | Implement directly with tests |
| `build` | `medium` | Brainstorm → plan → implement |
| `build` | `large` | Brainstorm → plan → implement (multi-session) |
| `plan` | `small` | Quick research, write up findings |
| `plan` | `medium` | Brainstorm → design spec |
| `plan` | `large` | Deep discovery → goal roadmap → first actionable task |

**If mode or effort is missing**, ask briefly: "What mode (plan/build) and effort (small/medium/large)?" Then set via `temper task move <slug> --mode <m> --effort <e>`.

### build/small Workflow
1. Read task content
2. Implement directly with tests
3. `cargo test` / `cargo clippy`
4. Commit
5. Pipe session content: `cat <<'EOF' | temper session save "<summary>" --task <slug> --state done` with goal, what happened, decisions, connections, and next steps via stdin

### build/medium Workflow
1. Read task content
2. `temper search` / `temper context` for discovery
3. Invoke superpowers:brainstorming (design the implementation)
4. Produce design spec, then invoke superpowers:writing-plans
5. Implement via plan execution
6. Full verification (tests, clippy, fmt)
7. Pipe session content: `cat <<'EOF' | temper session save "<summary>" --task <slug> --state done` with goal, what happened, decisions, connections, and next steps via stdin

### build/large Workflow
1. Same as build/medium but expect multi-session execution
2. Create sub-tasks as work is decomposed
3. Each session: work the current task, learn, create the next task
4. Pipe session content after each session

### plan/small Workflow
1. Read task content
2. Quick research — `temper search`, targeted file reads
3. Write up findings
4. Pipe session content: `cat <<'EOF' | temper session save "<summary>" --task <slug> --state done` via stdin

### plan/medium Workflow
1. Read task content
2. `temper search` / `temper context` for discovery
3. Invoke superpowers:brainstorming (explore the problem space)
4. Produce design spec
5. Pipe session content: `cat <<'EOF' | temper session save "<summary>" --task <slug> --state done` via stdin

### plan/large Workflow
1. Read task content
2. Deep discovery — `temper search`, `temper context`, codebase exploration
3. Invoke superpowers:brainstorming (map the problem space, NOT design an implementation)
4. Produce a goal roadmap via `temper goal create`:
   - Throughline summary, sequenced deliverable chunks, validation gates, open questions
5. Create the FIRST actionable task under that goal
6. Pipe session content: `cat <<'EOF' | temper session save "<summary>" --task <slug>` with goal, what happened, decisions, connections, and next steps via stdin
7. Code only if the user actively pushes for it

plan/large philosophy: the roadmap guides session work, not task-spread. Each session: work the current task, learn, evolve the roadmap, create the next task.

### Mid-Session Drift Detection

Watch for mode/effort mismatch:
- **build/small drifting up**: needing design decisions, touching 3+ files, considering multiple approaches → suggest build/medium
- **build/medium drifting up**: needs decomposition into multiple deliverables, spans multiple sessions → suggest build/large
- **plan/large drifting down**: first task is obvious, roadmap has only 1-2 items → suggest plan/medium or start building

On confirmation: `temper task move <slug> --mode <new> --effort <new>`

### Mode + Effort at Create Time

When creating a task without `--mode`/`--effort`, ask briefly: "What mode (plan/build) and effort (small/medium/large)?" Don't over-analyze — the user usually knows.

Stdin is auto-detected — pipe content directly without flags.
"#,
        hash = hash,
        vault_path = vault_path,
        project_list = project_list,
    );

    Ok(content)
}

/// Write the generated skill file to `output_path`, creating parent dirs as needed.
pub fn install(config: &Config, output_path: &Path) -> Result<()> {
    let content = generate(config)?;

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            TemperError::Config(format!(
                "cannot create directories for {}: {}",
                output_path.display(),
                e
            ))
        })?;
    }

    std::fs::write(output_path, &content).map_err(|e| {
        TemperError::Config(format!(
            "cannot write skill file to {}: {}",
            output_path.display(),
            e
        ))
    })?;

    Ok(())
}

/// Check skill installation status.
/// 1. Checks if superpowers plugin is installed.
/// 2. Checks if the skill file exists at the configured output path.
/// 3. If it exists, compares the embedded config hash to detect staleness.
pub fn check(config: &Config) -> Result<()> {
    // Check superpowers installation
    let superpowers_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".claude/plugins/cache/claude-plugins-official/superpowers");

    if superpowers_path.exists() {
        output::status_icon(true, format!("Superpowers: {}", superpowers_path.display()));
    } else {
        output::status_icon(
            false,
            format!("Superpowers: NOT FOUND ({})", superpowers_path.display()),
        );
    }

    // Check skill file
    let skill_path = &config.skill_output;
    if !skill_path.exists() {
        output::status_icon(
            false,
            format!("Skill file: NOT FOUND ({})", skill_path.display()),
        );
        output::hint("  Run: temper skill install");
        return Ok(());
    }

    output::status_icon(true, format!("Skill file: {}", skill_path.display()));

    // Check for staleness by comparing hashes
    let existing = std::fs::read_to_string(skill_path)
        .map_err(|e| TemperError::Config(format!("cannot read skill file: {}", e)))?;

    let embedded_hash = extract_config_hash(&existing);

    // Compute current hash
    let toml_path = config.vault_root.join("temper.toml");
    let toml_content = std::fs::read_to_string(&toml_path)
        .map_err(|e| TemperError::Config(format!("cannot read temper.toml: {}", e)))?;
    let current_hash = format!("{:x}", Sha256::digest(toml_content.as_bytes()));

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

    Ok(())
}

fn extract_config_hash(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("<!-- config-hash: ") {
            if let Some(hash) = rest.strip_suffix(" -->") {
                return Some(hash.to_string());
            }
        }
    }
    None
}
