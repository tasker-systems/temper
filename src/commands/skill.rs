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
        "(no projects configured)".to_string()
    } else {
        project_lines.join("\n")
    };

    let content = format!(
        r#"<!-- config-hash: {hash} -->
---
name: temper
description: Knowledge vault operations — context lookup, session notes, ticket management, semantic search
---

# Temper — Vault Workflow Tool

Vault: {vault_path}

## Invocation

`temper` is an installed binary in `$PATH`. Always run it directly as `temper <subcommand>`.
Never use `cargo run`, `python`, full binary paths, or any other indirect method — even when
working inside the temper source repo.

## Projects
{project_list}

## Commands

- `temper search <query>` — Semantic search across indexed content (reach for this before grep/find)
- `temper context <topic> [--depth N]` — Traverse nearest neighbors for related content
- `temper session save [<title>] [--ticket <slug>] [--state <state>]` — Create/update session note, optionally link to ticket
- `temper session list` — List recent sessions
- `temper ticket create --title <t> [--project <p>] [--scope patch|feature|epic]` — Create ticket (stdin auto-detected)
- `temper ticket list [--project <p>] [--format json|text]` — List tickets
- `temper tui` — Interactive terminal UI (board, search, context, maintain)
- `temper ticket move <slug> --stage <s> [--project <p>] [--scope patch|feature|epic]` — Move ticket between stages or update scope
- `temper ticket done <slug> [--project <p>]` — Mark ticket done
- `temper ticket show <slug> [--project <p>] [--format json|text]` — Show ticket content
- `temper ticket start <slug> [--project <p>]` — Move to in-progress, show content, invoke brainstorming skill
- `temper milestone list [--project <p>]` — Roadmap view
- `temper note create <type> <title> [--project <p>]` — Create note from template (stdin auto-detected)
- `temper research save <title> [--project <p>]` — Create high-fidelity research note (stdin auto-detected)
- `temper normalize [--project <p>] [--dry-run]` — Repair vault structure drift
- `temper events [--project <p>] [--limit <n>]` — Show recent vault events
- `temper warmup [--project <p>]` — Context primer for new sessions
- `temper index` — Rebuild search index
- `temper status` — Vault overview

## Discovery

Before launching subagents to grep or find across the vault, use temper's semantic tools:

- `temper search "<query>"` uses embeddings to find conceptually related content — not just keyword matches
- `temper context <topic> --depth 2` traverses nearest neighbors in the HNSW index to surface related entities
- Workflow: search → context → targeted file reads. This is what the index is for.

Templates are available via `--show-template` on create/save commands.

## Stages

Tickets use four stages: `backlog`, `in-progress`, `done`, `cancelled`.

## Scope

Tickets have an optional `scope` field: `patch`, `feature`, or `epic`. Scope controls the workflow:

| Scope | Nature | Ceremony | Output |
|-------|--------|----------|--------|
| `patch` | Tactical | None — just do it | Delivered code |
| `feature` | Deliberate | Full Superpowers pipeline | Delivered code with design artifact |
| `epic` | Strategic | Deep discovery + roadmapping | Living milestone roadmap + first actionable ticket |

## Workflow Integration

When starting a session:
- Check for recent sessions: `temper session list --project <current>`
- Search for relevant context: `temper search "<topic>"`

When ending a session:
- Suggest: `temper session save --ticket <slug> --state done` (if working on a ticket)
- Or just: `temper session save`

When the user says `/temper ticket start <slug>`:
1. Run `temper ticket move <slug> --stage in-progress --project <p>`
2. Run `temper ticket show <slug>`
3. Check the `scope` field and route accordingly:

### Scope Routing

**If scope is set**, announce the workflow:
- **patch**: "Scoped as patch — implementing directly with tests, no spec or plan." Skip brainstorming.
- **feature**: "Scoped as feature — full superpowers pipeline." Invoke brainstorming skill.
- **epic**: "Scoped as epic — mapping the problem space to produce a milestone roadmap." Invoke brainstorming skill framed as strategic planning.

**If scope is missing**, ask briefly: "Does this feel like a patch, feature, or epic?" Then set it via `temper ticket move <slug> --scope <confirmed>`.

### Patch Workflow
1. Read ticket content
2. Implement directly with tests
3. `cargo test` / `cargo clippy`
4. Commit
5. `temper session save "<summary>" --ticket <slug> --state done`

### Feature Workflow
1. Read ticket content
2. `temper search` / `temper context` for discovery
3. Invoke superpowers:brainstorming (design the implementation)
4. Produce design spec, then invoke superpowers:writing-plans
5. Implement via plan execution
6. Full verification (tests, clippy, fmt)
7. `temper session save "<summary>" --ticket <slug> --state done`

### Epic Workflow
1. Read ticket content
2. Deep discovery — `temper search`, `temper context`, codebase exploration
3. Invoke superpowers:brainstorming (map the problem space, NOT design an implementation)
4. Produce a milestone roadmap via `temper milestone create`:
   - Throughline summary, sequenced deliverable chunks, validation gates, open questions
5. Create the FIRST actionable ticket under that milestone
6. `temper session save "<summary>" --ticket <slug>`
7. Code only if the user actively pushes for it

Epic philosophy: the roadmap guides session work, not ticket-spread. Each session: work the current ticket, learn, evolve the roadmap, create the next ticket.

### Mid-Session Drift Detection

Watch for scope mismatch:
- **Patch drifting up**: needing design decisions, touching 3+ files, considering multiple approaches → suggest feature
- **Feature drifting up**: needs decomposition into multiple deliverables, spans multiple sessions → suggest epic
- **Epic drifting down**: first ticket is obvious, roadmap has only 1-2 items → suggest feature or start work

On confirmation: `temper ticket move <slug> --scope <new>`

### Scope at Create Time

When creating a ticket without `--scope`, ask briefly: "Does this feel like a patch, feature, or epic?" Don't over-analyze — the user usually knows.

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
