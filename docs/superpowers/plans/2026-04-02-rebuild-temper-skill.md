# Rebuild Temper Skill — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the monolithic temper command file with a modular skill directory — router, workflow files, subagent guidance, session lifecycle, and project init — generated and installed by the temper CLI.

**Architecture:** The command wrapper (`~/.claude/commands/temper.md`) becomes a thin entrypoint that invokes the skill. The skill (`~/.claude/skills/temper/`) is a directory of markdown files: SKILL.md routes by mode/effort, workflow files provide per-combination playbooks, and a `guidance/` directory offers extension points. Askama templates generate dynamic files; `include_str!()` embeds static content.

**Tech Stack:** Rust, Askama 0.12, clap 4, sha2

**Spec:** `docs/superpowers/specs/2026-04-02-rebuild-temper-skill-design.md`

---

## File Map

**Create:**
- `crates/temper-cli/templates/skill.md` — Askama template for SKILL.md
- `crates/temper-cli/templates/command-wrapper.md` — Askama template for command wrapper
- `crates/temper-cli/skill-content/reference.md` — Static CLI reference
- `crates/temper-cli/skill-content/subagent-guidance.md` — Static 10 principles
- `crates/temper-cli/skill-content/session-lifecycle.md` — Static lifecycle patterns
- `crates/temper-cli/skill-content/workflows/build-small.md` — Static workflow
- `crates/temper-cli/skill-content/workflows/build-medium.md` — Static workflow
- `crates/temper-cli/skill-content/workflows/build-large.md` — Static workflow
- `crates/temper-cli/skill-content/workflows/plan-small.md` — Static workflow
- `crates/temper-cli/skill-content/workflows/plan-medium.md` — Static workflow
- `crates/temper-cli/skill-content/workflows/plan-large.md` — Static workflow

**Modify:**
- `crates/temper-core/src/types/config.rs` — Update `SkillConfig` default output path
- `crates/temper-cli/src/templates.rs` — Add `SkillTemplate` and `CommandWrapperTemplate`
- `crates/temper-cli/src/commands/skill.rs` — Rewrite generate/install/check for directory output
- `crates/temper-cli/src/cli.rs` — Update `SkillAction::Install` args
- `crates/temper-cli/src/main.rs` — Wire updated install path logic

**Test:**
- `crates/temper-cli/src/commands/skill.rs` — Unit tests in `#[cfg(test)] mod tests`
- `crates/temper-core/src/types/config.rs` — Update existing config test

---

### Task 1: Write Static Skill Content Files

Create all 9 static markdown files that will be embedded via `include_str!()`. These are the behavioral content of the skill — no Rust code yet.

**Files:**
- Create: `crates/temper-cli/skill-content/reference.md`
- Create: `crates/temper-cli/skill-content/subagent-guidance.md`
- Create: `crates/temper-cli/skill-content/session-lifecycle.md`

- [ ] **Step 1: Create the skill-content directory structure**

```bash
mkdir -p crates/temper-cli/skill-content/workflows
```

- [ ] **Step 2: Write `reference.md`**

Create `crates/temper-cli/skill-content/reference.md` with the CLI command reference. This is the non-behavioral documentation extracted from the current command file. Include:

- Invocation rules (always `temper` from PATH, never cargo run)
- Command table with syntax and brief descriptions for: search, context, session (save, list), task (create, list, move, done, show, start), goal list, note create, research save, normalize, events, warmup, index, status
- Stages: backlog, in-progress, done, cancelled
- Mode definitions: plan (research/design/discovery) and build (implementation)
- Effort definitions: small (single session), medium (multi-step bounded), large (multi-session)
- Discovery workflow: search → context → targeted reads
- Template access via `--show-template`

Do NOT include any workflow routing logic, session lifecycle patterns, or subagent guidance. This file is reference only.

- [ ] **Step 3: Write `subagent-guidance.md`**

Create `crates/temper-cli/skill-content/subagent-guidance.md` with the 10 universal principles from the spec. Structure:

```markdown
# Subagent Guidance

Principles for any subagent dispatched during temper workflows. When dispatching subagents,
include all applicable principles verbatim in the subagent prompt. Do not summarize or
paraphrase — subagents need the full text to follow them.

## Foundational Principles

### SG-1: Follow Existing Patterns
...

### SG-2: Single Responsibility
...
```

Include all 10 principles (SG-1 through SG-10) with their full descriptions from the spec. After the principles, include the wrong→right table:

```markdown
## Quick Reference

| Wrong | Right |
|-------|-------|
| Silently swallow errors with defaults | Return specific errors with context |
| Build a new abstraction for one use | Inline it, extract later if repeated |
| Claim "tests pass" without running them | Run, read output, report result |
| Propose complex solution without checking | List existing tools/abstractions first |
| Declare a failure "not our problem" | Prove external causation before dismissing |
| Skip reading sibling files before editing | Read the file AND a neighbor first |
```

End with the domain-aware application note:

```markdown
## Domain Applicability

- **Software tasks:** All 10 principles apply.
- **Non-software tasks:** SG-1 (follow patterns), SG-5 (don't over-build), SG-6 (verify), SG-10 (checkpoint) apply. The rest are software-specific.
```

- [ ] **Step 4: Write `session-lifecycle.md`**

Create `crates/temper-cli/skill-content/session-lifecycle.md` with the session lifecycle content from the spec. Include session start checklist, session end pattern (with the full stdin pipe example), mid-session drift detection table, and checkpoint pattern.

- [ ] **Step 5: Verify all three files are well-formed markdown**

```bash
wc -l crates/temper-cli/skill-content/reference.md crates/temper-cli/skill-content/subagent-guidance.md crates/temper-cli/skill-content/session-lifecycle.md
```

Expected: Each file should be substantive (reference ~60-80 lines, subagent-guidance ~120-150 lines, session-lifecycle ~80-100 lines).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/skill-content/reference.md crates/temper-cli/skill-content/subagent-guidance.md crates/temper-cli/skill-content/session-lifecycle.md
git commit -m "feat(skill): add static skill content — reference, subagent guidance, session lifecycle"
```

---

### Task 2: Write Static Workflow Files

Create the six workflow markdown files — one per mode/effort combination.

**Files:**
- Create: `crates/temper-cli/skill-content/workflows/build-small.md`
- Create: `crates/temper-cli/skill-content/workflows/build-medium.md`
- Create: `crates/temper-cli/skill-content/workflows/build-large.md`
- Create: `crates/temper-cli/skill-content/workflows/plan-small.md`
- Create: `crates/temper-cli/skill-content/workflows/plan-medium.md`
- Create: `crates/temper-cli/skill-content/workflows/plan-large.md`

- [ ] **Step 1: Write `build-small.md`**

```markdown
# Build/Small Workflow

## When This Applies
Single-session implementation work. The task is well-defined, scope is clear,
no design decisions needed. Get in, build it, verify, get out.

## Steps
1. Read task content and project fundamentals (`guidance/fundamentals.md` if it exists)
2. Read `subagent-guidance.md` — apply all principles
3. Implement directly with tests
4. Run verification commands from project fundamentals (or project-standard test/lint)
5. Commit

## Completion
Pipe session content via stdin:
` ``bash
cat <<'EOF' | temper session save "<summary>" --task <slug> --state done
## Goal
...
## What Happened
...
## Decisions
...
## Connections
...
## Next Steps
...
EOF
` ``
```

(Remove the space before the triple backtick — formatting escape.)

- [ ] **Step 2: Write `build-medium.md`**

This workflow includes discovery, optional brainstorming, planning, and implementation. Key structure:

```markdown
# Build/Medium Workflow

## When This Applies
Multi-step implementation. Needs design decisions, touches multiple files/components,
benefits from a plan before coding.

## Steps
1. Read task content and project fundamentals
2. Discovery: `temper search` and `temper context` for related work
3. Read `subagent-guidance.md`
4. **Design phase:** If the user opted into a brainstorming skill, invoke it now.
   Otherwise, outline the approach inline:
   - What components are affected?
   - What's the implementation order?
   - What are the risks?
   Present to user for approval before coding.
5. **Planning phase:** If the user opted into a planning skill, invoke it now.
   Otherwise, list the implementation steps with files and verification.
6. Implement per plan
7. Full verification (test, lint, build per project fundamentals)
8. Commit

## Completion
Pipe session content via stdin (same pattern as build-small).
```

- [ ] **Step 3: Write `build-large.md`**

Like build-medium but explicitly multi-session. Key additions: sub-task creation, session boundaries, "each session works one task, saves, creates next."

- [ ] **Step 4: Write `plan-small.md`**

Quick research workflow. Steps: read task → search/read → write findings → session save.

- [ ] **Step 5: Write `plan-medium.md`**

Discovery → brainstorm problem space (not implementation) → design spec → session save. References brainstorming skill conditionally.

- [ ] **Step 6: Write `plan-large.md`**

Deep discovery → map problem space → goal roadmap → first actionable task → session save. Emphasizes: code only if user pushes. The roadmap guides session work.

- [ ] **Step 7: Verify all six workflow files exist and are well-formed**

```bash
ls -la crates/temper-cli/skill-content/workflows/
wc -l crates/temper-cli/skill-content/workflows/*.md
```

Expected: 6 files, each 40-80 lines.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/skill-content/workflows/
git commit -m "feat(skill): add workflow files for all mode/effort combinations"
```

---

### Task 3: Create Askama Templates for SKILL.md and Command Wrapper

Two new askama templates for the files that need dynamic content.

**Files:**
- Create: `crates/temper-cli/templates/skill.md`
- Create: `crates/temper-cli/templates/command-wrapper.md`
- Modify: `crates/temper-cli/src/templates.rs`

- [ ] **Step 1: Write the SKILL.md askama template**

Create `crates/temper-cli/templates/skill.md`:

```
{% raw %}<!-- config-hash: {{ config_hash }} -->
---
name: temper
description: Use when managing knowledge vault tasks, sessions, or search — task start/create/done, session save, semantic search, context discovery, or any /temper command invocation
---

# Temper Workflow Skill

Vault: {{ vault_path }}

## Contexts
{{ context_list }}

## How This Skill Works

This is a modular skill. SKILL.md (this file) is the router — it tells you what to
read and when. Behavioral content lives in supporting files. Do NOT read all files
upfront; read only what the current task requires.

### Supporting Files
- `reference.md` — CLI commands, stages, mode/effort definitions
- `subagent-guidance.md` — 10 universal principles for dispatched subagents
- `session-lifecycle.md` — Session start/end patterns, drift detection, checkpoints

### Workflow Files (`workflows/`)
One file per mode/effort combination. Read only the one that matches the current task.

### Extension Files (`guidance/`)
User-created guidance files. Read and apply any files found here.
`guidance/fundamentals.md` contains project-specific principles if it exists.

## On Task Start

1. Read the task content — extract mode and effort
2. If mode or effort is missing, ask: "What mode (plan/build) and effort (small/medium/large)?"
3. Infer or ask the domain: "What kind of work is this? (a) Software development, (b) Writing/documentation, (c) Research/analysis, (d) Design/architecture, (e) Something else"
4. Check for `guidance/fundamentals.md`:
   - If it exists, read it and apply its principles
   - If it doesn't, offer: "This context has no project fundamentals. Want to set them up? (`/temper init`)"
5. Check auto-memory for user plugin preferences (skills they've said they rely on)
6. Scan for installed skills: check `~/.claude/skills/` and plugins cache
7. Ask: "I found [list]. Want subagents to use any of these? Any other quality gates?"
8. Read `workflows/{mode}-{effort}.md` and follow it

## On Other Commands

For non-task-start invocations (search, session save, etc.), read `reference.md`
for command syntax and follow standard patterns.

## Subagent Dispatch

Before dispatching any subagent:
1. Read `subagent-guidance.md`
2. Include all applicable principles in the subagent prompt (verbatim, not summarized)
3. Include project fundamentals from `guidance/fundamentals.md` if available
4. Include any user-selected plugin skills

## Session Lifecycle

Read `session-lifecycle.md` for:
- Session start checklist
- Session end save pattern
- Mid-session drift detection
- Checkpoint prompts{% endraw %}
```

- [ ] **Step 2: Write the command wrapper askama template**

Create `crates/temper-cli/templates/command-wrapper.md`:

```
{% raw %}<!-- config-hash: {{ config_hash }} -->
---
name: temper
description: Knowledge vault operations — context lookup, session notes, task management, semantic search
---

Invoke the temper skill to handle this request.

ARGUMENTS: $ARGUMENTS{% endraw %}
```

- [ ] **Step 3: Add template structs to `templates.rs`**

Add to `crates/temper-cli/src/templates.rs`:

```rust
#[derive(Template)]
#[template(path = "skill.md")]
pub struct SkillTemplate<'a> {
    pub config_hash: &'a str,
    pub vault_path: &'a str,
    pub context_list: &'a str,
}

#[derive(Template)]
#[template(path = "command-wrapper.md")]
pub struct CommandWrapperTemplate<'a> {
    pub config_hash: &'a str,
}
```

- [ ] **Step 4: Verify templates compile**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli
```

Expected: Clean compilation. Askama validates templates at compile time.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/templates/skill.md crates/temper-cli/templates/command-wrapper.md crates/temper-cli/src/templates.rs
git commit -m "feat(skill): add askama templates for SKILL.md and command wrapper"
```

---

### Task 4: Update Config Default and CLI Args

Change the default skill output path from the old command file to the new skill directory, and update the CLI arg structure.

**Files:**
- Modify: `crates/temper-core/src/types/config.rs`
- Modify: `crates/temper-cli/src/cli.rs`

- [ ] **Step 1: Update the default skill output path**

In `crates/temper-core/src/types/config.rs`, change `default_skill_output()`:

```rust
fn default_skill_output() -> String {
    "~/.claude/skills/temper".to_string()
}
```

- [ ] **Step 2: Update the config test**

In the same file, update the test assertion that checks the default:

Find the assertion:
```rust
assert_eq!(config.skill.output, "~/.claude/commands/temper.md");
```

If this test uses a fixture TOML that hardcodes the old path, update the fixture too. If it relies on the default, the test should now expect `~/.claude/skills/temper`.

Check the test fixture to determine which case applies:

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && grep -n "skill" crates/temper-core/src/types/config.rs | grep -i "test\|assert\|fixture\|commands"
```

- [ ] **Step 3: Simplify `SkillAction::Install` in `cli.rs`**

In `crates/temper-cli/src/cli.rs`, update the `Install` variant. Remove the `global` and `context` args (legacy from single-file install). Keep `path` as the override:

```rust
#[derive(Subcommand)]
pub enum SkillAction {
    /// Generate skill content (preview to stdout)
    Generate,
    /// Install skill directory and command wrapper
    Install {
        /// Override install directory (default: ~/.claude/skills/temper)
        #[arg(long)]
        path: Option<String>,
    },
    /// Check skill installation status
    Check,
}
```

- [ ] **Step 4: Verify compilation**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli
```

Expected: Compilation will fail in `main.rs` because it still references the old `SkillAction::Install` fields. That's expected — we fix it in Task 5.

- [ ] **Step 5: Commit config and CLI changes**

```bash
git add crates/temper-core/src/types/config.rs crates/temper-cli/src/cli.rs
git commit -m "feat(skill): update default output to skill directory, simplify install args"
```

---

### Task 5: Rewrite `skill.rs` — Generate and Install

Replace the monolithic `generate()` with a directory-based generation and install flow.

**Files:**
- Modify: `crates/temper-cli/src/commands/skill.rs`
- Modify: `crates/temper-cli/src/main.rs`

- [ ] **Step 1: Write the test for `generate_skill_files`**

Add a `#[cfg(test)] mod tests` block at the bottom of `crates/temper-cli/src/commands/skill.rs`:

```rust
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
            skill_output: PathBuf::from("/tmp/test-skill-output"),
            skill_framework: "superpowers".to_string(),
        }
    }

    #[test]
    fn test_generate_skill_files_contains_expected_keys() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();

        // Must produce SKILL.md, command wrapper, and all static files
        assert!(files.contains_key("SKILL.md"));
        assert!(files.contains_key("reference.md"));
        assert!(files.contains_key("subagent-guidance.md"));
        assert!(files.contains_key("session-lifecycle.md"));
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
        assert!(skill_md.contains("config-hash:"));
    }

    #[test]
    fn test_generate_command_wrapper_contains_hash() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let wrapper = &files["command-wrapper.md"];

        assert!(wrapper.contains("config-hash:"));
        assert!(wrapper.contains("Invoke the temper skill"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-cli -- test_generate_skill_files 2>&1 | head -20
```

Expected: FAIL — `generate_skill_files` doesn't exist yet.

- [ ] **Step 3: Implement `generate_skill_files`**

Replace the contents of `crates/temper-cli/src/commands/skill.rs`. Keep `extract_config_hash` and `check` (updated), replace `generate` and `install`:

```rust
use std::collections::HashMap;
use std::path::Path;

use askama::Template;
use sha2::{Digest, Sha256};

use crate::config::{self, Config};
use crate::error::{Result, TemperError};
use crate::output;
use crate::templates::{CommandWrapperTemplate, SkillTemplate};

/// Generate all skill files as a map of relative_path → content.
pub fn generate_skill_files(config: &Config) -> Result<HashMap<String, String>> {
    let config_hash = compute_config_hash()?;
    generate_skill_files_with_hash(config, &config_hash)
}

/// Testable version that accepts a pre-computed hash.
fn generate_skill_files_with_hash(
    config: &Config,
    config_hash: &str,
) -> Result<HashMap<String, String>> {
    let vault_path = config.vault_root.display().to_string();
    let context_list = format_context_list(&config.contexts);

    let mut files = HashMap::new();

    // Templated files
    let skill_md = SkillTemplate {
        config_hash,
        vault_path: &vault_path,
        context_list: &context_list,
    }
    .render()
    .map_err(|e| TemperError::Config(format!("failed to render SKILL.md: {e}")))?;
    files.insert("SKILL.md".to_string(), skill_md);

    let command_wrapper = CommandWrapperTemplate {
        config_hash,
    }
    .render()
    .map_err(|e| TemperError::Config(format!("failed to render command wrapper: {e}")))?;
    files.insert("command-wrapper.md".to_string(), command_wrapper);

    // Static files
    files.insert(
        "reference.md".to_string(),
        include_str!("../../skill-content/reference.md").to_string(),
    );
    files.insert(
        "subagent-guidance.md".to_string(),
        include_str!("../../skill-content/subagent-guidance.md").to_string(),
    );
    files.insert(
        "session-lifecycle.md".to_string(),
        include_str!("../../skill-content/session-lifecycle.md").to_string(),
    );
    files.insert(
        "workflows/build-small.md".to_string(),
        include_str!("../../skill-content/workflows/build-small.md").to_string(),
    );
    files.insert(
        "workflows/build-medium.md".to_string(),
        include_str!("../../skill-content/workflows/build-medium.md").to_string(),
    );
    files.insert(
        "workflows/build-large.md".to_string(),
        include_str!("../../skill-content/workflows/build-large.md").to_string(),
    );
    files.insert(
        "workflows/plan-small.md".to_string(),
        include_str!("../../skill-content/workflows/plan-small.md").to_string(),
    );
    files.insert(
        "workflows/plan-medium.md".to_string(),
        include_str!("../../skill-content/workflows/plan-medium.md").to_string(),
    );
    files.insert(
        "workflows/plan-large.md".to_string(),
        include_str!("../../skill-content/workflows/plan-large.md").to_string(),
    );

    Ok(files)
}

/// Install the skill directory and command wrapper.
///
/// Writes all generated files to `skill_dir`. The command wrapper is written
/// to `~/.claude/commands/temper.md`. Files in `guidance/` are never overwritten.
pub fn install(config: &Config, skill_dir: &Path) -> Result<()> {
    let files = generate_skill_files(config)?;

    // Create directory structure
    let workflows_dir = skill_dir.join("workflows");
    let guidance_dir = skill_dir.join("guidance");
    std::fs::create_dir_all(&workflows_dir)
        .map_err(|e| TemperError::Config(format!("cannot create {}: {e}", workflows_dir.display())))?;
    std::fs::create_dir_all(&guidance_dir)
        .map_err(|e| TemperError::Config(format!("cannot create {}: {e}", guidance_dir.display())))?;

    // Write skill files (skip command-wrapper.md — it goes elsewhere)
    for (relative_path, content) in &files {
        if relative_path == "command-wrapper.md" {
            continue;
        }
        let dest = skill_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| TemperError::Config(format!("cannot create {}: {e}", parent.display())))?;
        }
        std::fs::write(&dest, content)
            .map_err(|e| TemperError::Config(format!("cannot write {}: {e}", dest.display())))?;
    }

    // Write command wrapper to ~/.claude/commands/temper.md
    let commands_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".claude/commands");
    std::fs::create_dir_all(&commands_dir)
        .map_err(|e| TemperError::Config(format!("cannot create {}: {e}", commands_dir.display())))?;
    let command_path = commands_dir.join("temper.md");
    if let Some(wrapper_content) = files.get("command-wrapper.md") {
        std::fs::write(&command_path, wrapper_content)
            .map_err(|e| TemperError::Config(format!("cannot write {}: {e}", command_path.display())))?;
    }

    Ok(())
}

/// Backward-compatible generate that returns SKILL.md content for stdout preview.
pub fn generate(config: &Config) -> Result<String> {
    let files = generate_skill_files(config)?;
    files
        .get("SKILL.md")
        .cloned()
        .ok_or_else(|| TemperError::Config("SKILL.md not found in generated files".to_string()))
}

fn compute_config_hash() -> Result<String> {
    let config_path = config::global_config_path();
    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| TemperError::Config(format!("cannot read config: {e}")))?;
    Ok(format!("{:x}", Sha256::digest(config_content.as_bytes())))
}

fn format_context_list(contexts: &[String]) -> String {
    if contexts.is_empty() {
        return "(no contexts configured)".to_string();
    }
    let mut sorted = contexts.to_vec();
    sorted.sort();
    sorted.iter().map(|c| format!("- `{c}`")).collect::<Vec<_>>().join("\n")
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

/// Check skill installation status.
pub fn check(config: &Config) -> Result<()> {
    // Check superpowers installation only when relevant
    if config.skill_framework == "superpowers" {
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
    }

    // Check skill directory
    let skill_dir = &config.skill_output;
    if !skill_dir.exists() {
        output::status_icon(false, format!("Skill directory: NOT FOUND ({})", skill_dir.display()));
        output::hint("  Run: temper skill install");
        return Ok(());
    }

    output::status_icon(true, format!("Skill directory: {}", skill_dir.display()));

    // Check expected files
    let expected_files = [
        "SKILL.md",
        "reference.md",
        "subagent-guidance.md",
        "session-lifecycle.md",
        "workflows/build-small.md",
        "workflows/build-medium.md",
        "workflows/build-large.md",
        "workflows/plan-small.md",
        "workflows/plan-medium.md",
        "workflows/plan-large.md",
    ];
    for file in &expected_files {
        let path = skill_dir.join(file);
        if path.exists() {
            output::status_icon(true, format!("  {file}"));
        } else {
            output::status_icon(false, format!("  {file}: MISSING"));
        }
    }

    // Check hash staleness
    let skill_md_path = skill_dir.join("SKILL.md");
    if skill_md_path.exists() {
        let content = std::fs::read_to_string(&skill_md_path)
            .map_err(|e| TemperError::Config(format!("cannot read SKILL.md: {e}")))?;
        let embedded_hash = extract_config_hash(&content);
        let current_hash = compute_config_hash()?;

        match embedded_hash {
            Some(h) if h == current_hash => {
                output::status_icon(true, "Hash: up to date");
            }
            Some(h) => {
                output::status_icon(false, "Hash: STALE");
                output::plain(format!("  Embedded: {h}"));
                output::plain(format!("  Current:  {current_hash}"));
                output::hint("  Run: temper skill install");
            }
            None => {
                output::warning("Hash: UNKNOWN (no config-hash comment found)");
            }
        }
    }

    // Check command wrapper
    let command_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".claude/commands/temper.md");
    if command_path.exists() {
        output::status_icon(true, format!("Command wrapper: {}", command_path.display()));
    } else {
        output::status_icon(false, format!("Command wrapper: NOT FOUND ({})", command_path.display()));
    }

    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-cli -- test_generate_skill 2>&1
```

Expected: All 3 tests pass. Note: `test_generate_skill_files_contains_expected_keys` will fail if `compute_config_hash()` can't read the global config. If that happens, refactor to accept hash as parameter or mock the config path in tests. A pragmatic fix:

Add a `generate_skill_files_with_hash` internal function that accepts the hash, and have `generate_skill_files` call it after computing the hash. Tests call `generate_skill_files_with_hash` directly with a test hash.

- [ ] **Step 5: Update `main.rs` to wire the new install signature**

In `crates/temper-cli/src/main.rs`, update the `SkillAction::Install` match arm:

```rust
SkillAction::Install { path } => {
    let skill_dir = if let Some(p) = path {
        std::path::PathBuf::from(p)
    } else {
        config.skill_output.clone()
    };
    temper_cli::commands::skill::install(&config, &skill_dir)?;
    temper_cli::output::success(format!(
        "Skill installed: {}",
        skill_dir.display()
    ));
    Ok(())
}
```

- [ ] **Step 6: Verify full compilation**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli
```

Expected: Clean compilation.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/skill.rs crates/temper-cli/src/main.rs
git commit -m "feat(skill): rewrite skill generation for modular directory output"
```

---

### Task 6: Integration Test — Full Install Cycle

Test the complete install flow: generate files → write to temp directory → verify structure.

**Files:**
- Modify: `crates/temper-cli/src/commands/skill.rs` (add integration test)

- [ ] **Step 1: Write the install integration test**

Add to the `#[cfg(test)] mod tests` in `skill.rs`:

```rust
#[test]
fn test_install_creates_expected_directory_structure() {
    let config = test_config();
    let tmp = tempfile::tempdir().unwrap();
    let skill_dir = tmp.path().join("temper-skill");

    // Override HOME for command wrapper (we can't write to real ~/.claude in tests)
    // Instead, just verify the skill directory structure
    // The command wrapper write will fail gracefully in test — we test it separately

    // Create the skill directory files manually using generate_skill_files
    let files = generate_skill_files_with_hash(&config, "testhash123").unwrap();

    // Write skill files
    let workflows_dir = skill_dir.join("workflows");
    let guidance_dir = skill_dir.join("guidance");
    std::fs::create_dir_all(&workflows_dir).unwrap();
    std::fs::create_dir_all(&guidance_dir).unwrap();

    for (relative_path, content) in &files {
        if relative_path == "command-wrapper.md" {
            continue;
        }
        let dest = skill_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&dest, content).unwrap();
    }

    // Verify structure
    assert!(skill_dir.join("SKILL.md").exists());
    assert!(skill_dir.join("reference.md").exists());
    assert!(skill_dir.join("subagent-guidance.md").exists());
    assert!(skill_dir.join("session-lifecycle.md").exists());
    assert!(skill_dir.join("workflows/build-small.md").exists());
    assert!(skill_dir.join("workflows/plan-large.md").exists());
    assert!(skill_dir.join("guidance").is_dir());

    // Verify SKILL.md has expected content
    let skill_content = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    assert!(skill_content.contains("testhash123"));
    assert!(skill_content.contains("/tmp/test-vault"));
}

#[test]
fn test_install_preserves_guidance_files() {
    let config = test_config();
    let tmp = tempfile::tempdir().unwrap();
    let skill_dir = tmp.path().join("temper-skill");
    let guidance_dir = skill_dir.join("guidance");
    std::fs::create_dir_all(&guidance_dir).unwrap();

    // Pre-create a user guidance file
    let user_file = guidance_dir.join("fundamentals.md");
    std::fs::write(&user_file, "# My Fundamentals\nDo not overwrite me.").unwrap();

    // Run install (write files manually as above since real install touches HOME)
    let files = generate_skill_files_with_hash(&config, "testhash456").unwrap();
    for (relative_path, content) in &files {
        if relative_path == "command-wrapper.md" {
            continue;
        }
        let dest = skill_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        // Don't overwrite files in guidance/
        if relative_path.starts_with("guidance/") && dest.exists() {
            continue;
        }
        std::fs::write(&dest, content).unwrap();
    }

    // Verify user file preserved
    let content = std::fs::read_to_string(&user_file).unwrap();
    assert!(content.contains("Do not overwrite me."));
}
```

- [ ] **Step 2: Run the integration tests**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-cli -- test_install 2>&1
```

Expected: Both tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/skill.rs
git commit -m "test(skill): add integration tests for install directory structure"
```

---

### Task 7: Manual Verification — Install and Inspect

Run the actual `temper skill install` command and verify the output.

**Files:** None modified — verification only.

- [ ] **Step 1: Run `temper skill install`**

```bash
temper skill install
```

Expected: Success message with path `~/.claude/skills/temper`.

- [ ] **Step 2: Verify the directory structure**

```bash
find ~/.claude/skills/temper -type f | sort
```

Expected:
```
/Users/petetaylor/.claude/skills/temper/SKILL.md
/Users/petetaylor/.claude/skills/temper/reference.md
/Users/petetaylor/.claude/skills/temper/session-lifecycle.md
/Users/petetaylor/.claude/skills/temper/subagent-guidance.md
/Users/petetaylor/.claude/skills/temper/workflows/build-large.md
/Users/petetaylor/.claude/skills/temper/workflows/build-medium.md
/Users/petetaylor/.claude/skills/temper/workflows/build-small.md
/Users/petetaylor/.claude/skills/temper/workflows/plan-large.md
/Users/petetaylor/.claude/skills/temper/workflows/plan-medium.md
/Users/petetaylor/.claude/skills/temper/workflows/plan-small.md
```

- [ ] **Step 3: Verify the command wrapper was updated**

```bash
head -5 ~/.claude/commands/temper.md
```

Expected: Config hash comment, frontmatter with `name: temper`, "Invoke the temper skill" body.

- [ ] **Step 4: Verify `temper skill check` reports healthy**

```bash
temper skill check
```

Expected: All green checkmarks for directory, all files, hash up to date, command wrapper present.

- [ ] **Step 5: Verify SKILL.md contains correct vault path and contexts**

```bash
grep "Vault:" ~/.claude/skills/temper/SKILL.md
grep -A 10 "## Contexts" ~/.claude/skills/temper/SKILL.md
```

Expected: Vault path matches config, all four contexts listed.

- [ ] **Step 6: Run full test suite**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo test --workspace 2>&1 | tail -20
```

Expected: All tests pass, no regressions.

---

### Task 8: Update Global Config Test Fixture

Ensure the config round-trip test reflects the new default.

**Files:**
- Modify: `crates/temper-core/src/types/config.rs`

- [ ] **Step 1: Check current test fixture**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && grep -A 5 "\[skill\]" crates/temper-core/src/types/config.rs
```

Determine if the test TOML fixture hardcodes the old path or uses defaults.

- [ ] **Step 2: Update fixture if needed**

If the fixture has `output = "~/.claude/commands/temper.md"`, change to `output = "~/.claude/skills/temper"`.

Update the corresponding assertion:

```rust
assert_eq!(config.skill.output, "~/.claude/skills/temper");
```

- [ ] **Step 3: Run config tests**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-core -- config 2>&1
```

Expected: All config tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/config.rs
git commit -m "test(config): update skill output default in test fixture"
```

---

## Post-Implementation Notes

- **Project init flow** (`/temper init` for `guidance/fundamentals.md`) is specified in the design but is NOT part of this implementation plan. It should be a separate build task — the skill's SKILL.md already references it and offers the prompt, but the guided flow itself is agent-side behavior, not generated Rust code.
- **tasker-mcp integration:** This skill architecture is the foundation for temper-mcp. When MCP tools are built, the skill and CLI remain the preferred interface. A future task should fold MCP awareness into the plugin discovery step.
- After installing, start a new Claude Code session and test `/temper task start <slug>` to verify the skill loads and routes correctly through the new modular structure.
