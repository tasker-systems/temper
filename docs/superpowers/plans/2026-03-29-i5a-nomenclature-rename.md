# I5a: Nomenclature Rename — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename ticket→task, milestone→goal, project→context, scope→mode+effort throughout the CLI, vault, skill, docs, and memories. Remove local HNSW/embedding code. Clean break migration of existing vault files.

**Architecture:** Mechanical rename across ~30 CLI source files, 6 test files, 2 templates, the skill generator, config structs, and the knowledge vault. Local embedding stack (candle, instant-distance, HNSW, chunker) is deleted entirely. Search and context commands are gutted (will be rebuilt as cloud-routed in I5d).

**Tech Stack:** Rust (clap, serde), YAML frontmatter, knowledge vault markdown files

**Spec:** `docs/superpowers/specs/2026-03-29-i5-temper-developer-experience-design.md`

---

### Task 1: Commit Knowledge Vault Snapshot

Before any destructive changes, commit the vault as-is for a clean rollback point.

**Files:**
- Modify: `/Users/petetaylor/projects/knowledge/` (git commit only, no file changes)

- [ ] **Step 1: Commit current vault state**

```bash
cd ~/projects/knowledge
git add -A
git commit -m "snapshot: pre-nomenclature-rename vault state"
```

- [ ] **Step 2: Verify commit**

Run: `cd ~/projects/knowledge && git log --oneline -1`
Expected: Commit with "snapshot: pre-nomenclature-rename vault state"

---

### Task 2: Rename Vault Directories and Frontmatter

Rename `tickets/` → `tasks/`, `milestones/` → `goals/`. Update frontmatter in all files: `project:` → `context:`, `milestone:` → `goal:`, `type: ticket` → `type: task`, `type: milestone` → `type: goal`, `scope:` → `mode:` + `effort:`.

**Files:**
- Rename: `~/projects/knowledge/tickets/` → `~/projects/knowledge/tasks/`
- Rename: `~/projects/knowledge/milestones/` → `~/projects/knowledge/goals/`
- Modify: All `.md` files under `tasks/` and `goals/` (frontmatter updates)
- Modify: All `.md` files under `sessions/` that reference `milestone:` or `project:` in frontmatter

- [ ] **Step 1: Rename directories**

```bash
cd ~/projects/knowledge
mv tickets tasks
mv milestones goals
```

- [ ] **Step 2: Write a frontmatter migration script**

Create a temporary script at `~/projects/knowledge/.temper/migrate-frontmatter.sh`:

```bash
#!/bin/bash
set -euo pipefail

# Migrate task files (formerly tickets)
find tasks -name '*.md' -exec sed -i '' \
  -e 's/^type: ticket$/type: task/' \
  -e 's/^project: /context: /' \
  -e 's/^milestone: /goal: /' \
  -e 's/^scope: patch$/mode: build\neffort: small/' \
  -e 's/^scope: feature$/mode: build\neffort: medium/' \
  -e 's/^scope: epic$/mode: plan\neffort: large/' \
  -e 's/^scope: null$/mode: build\neffort: medium/' \
  {} +

# Migrate goal files (formerly milestones)
find goals -name '*.md' -exec sed -i '' \
  -e 's/^type: milestone$/type: goal/' \
  -e 's/^project: /context: /' \
  {} +

# Migrate session files
find sessions -name '*.md' -exec sed -i '' \
  -e 's/^project: /context: /' \
  -e 's/^milestone: /goal: /' \
  {} +

# Migrate research files
find research -name '*.md' -exec sed -i '' \
  -e 's/^project: /context: /' \
  -e 's/^milestone: /goal: /' \
  {} +

echo "Migration complete. Review with: git diff"
```

- [ ] **Step 3: Run migration script**

```bash
cd ~/projects/knowledge
chmod +x .temper/migrate-frontmatter.sh
bash .temper/migrate-frontmatter.sh
```

- [ ] **Step 4: Verify migration**

```bash
cd ~/projects/knowledge
# Should find zero remaining old-style frontmatter
grep -r '^type: ticket$' tasks/ || echo "OK: no old ticket types"
grep -r '^type: milestone$' goals/ || echo "OK: no old milestone types"
grep -r '^scope: ' tasks/ || echo "OK: no old scope fields"
grep -r '^project: ' tasks/ sessions/ research/ goals/ || echo "OK: no old project fields"
# Should find new-style frontmatter
grep -c '^type: task$' tasks/**/*.md
grep -c '^type: goal$' goals/**/*.md
```

- [ ] **Step 5: Handle scope: null edge cases**

Some tickets may have `scope: null` or no scope at all. Manually review any files that didn't match the sed patterns:

```bash
cd ~/projects/knowledge
# Find task files missing both mode and effort
for f in tasks/**/*.md; do
  if ! grep -q '^mode:' "$f" 2>/dev/null; then
    echo "MISSING mode: $f"
  fi
done
```

For any files missing `mode`/`effort`, add `mode: build` and `effort: medium` as defaults.

- [ ] **Step 6: Clean up migration script and commit**

```bash
cd ~/projects/knowledge
rm .temper/migrate-frontmatter.sh
git add -A
git commit -m "rename: ticket→task, milestone→goal, project→context, scope→mode+effort

Clean break migration of all vault frontmatter.
Directories: tickets/→tasks/, milestones/→goals/
Fields: type, project, milestone, scope all updated."
```

---

### Task 3: Update temper.toml Configuration

The vault's `temper.toml` references directory names for tickets and milestones.

**Files:**
- Modify: `~/projects/knowledge/temper.toml`

- [ ] **Step 1: Read current temper.toml**

Read the file to see current structure.

- [ ] **Step 2: Update directory references**

Replace `tickets` → `tasks` and `milestones` → `goals` in the vault config section. The exact fields depend on what's in the file — update all references to the old directory names.

- [ ] **Step 3: Commit**

```bash
cd ~/projects/knowledge
git add temper.toml
git commit -m "config: update temper.toml directory names for rename"
```

---

### Task 4: Remove Local Embedding and Indexing Code

Delete the local HNSW/candle embedding stack from temper-cli.

**Files:**
- Delete: `crates/temper-cli/src/embedder.rs`
- Delete: `crates/temper-cli/src/hnsw.rs`
- Delete: `crates/temper-cli/src/chunker.rs`
- Delete: `crates/temper-cli/src/registry.rs`
- Delete: `crates/temper-cli/src/commands/index.rs`
- Delete: `crates/temper-cli/src/actions/index.rs`
- Delete: `crates/temper-cli/src/commands/search.rs`
- Delete: `crates/temper-cli/src/actions/search.rs`
- Delete: `crates/temper-cli/src/commands/context.rs`
- Delete: `crates/temper-cli/src/actions/context.rs`
- Delete: `crates/temper-cli/tests/index_test.rs`
- Delete: `crates/temper-cli/tests/embedder_test.rs`
- Delete: `crates/temper-cli/tests/hnsw_test.rs`
- Delete: `crates/temper-cli/tests/chunker_test.rs` (if exists)
- Delete: `crates/temper-cli/tests/research_test.rs` (if it only tests indexing)
- Modify: `crates/temper-cli/Cargo.toml` (remove dependencies)
- Modify: `crates/temper-cli/src/lib.rs` (remove module declarations)
- Modify: `crates/temper-cli/src/commands/mod.rs` (remove module declarations)
- Modify: `crates/temper-cli/src/actions/mod.rs` (remove module declarations)
- Modify: `crates/temper-cli/src/cli.rs` (remove Index, Search, Context commands)

- [ ] **Step 1: Delete embedding/indexing source files**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
rm -f crates/temper-cli/src/embedder.rs
rm -f crates/temper-cli/src/hnsw.rs
rm -f crates/temper-cli/src/chunker.rs
rm -f crates/temper-cli/src/registry.rs
rm -f crates/temper-cli/src/commands/index.rs
rm -f crates/temper-cli/src/actions/index.rs
rm -f crates/temper-cli/src/commands/search.rs
rm -f crates/temper-cli/src/actions/search.rs
rm -f crates/temper-cli/src/commands/context.rs
rm -f crates/temper-cli/src/actions/context.rs
```

- [ ] **Step 2: Delete related test files**

```bash
rm -f crates/temper-cli/tests/index_test.rs
rm -f crates/temper-cli/tests/embedder_test.rs
rm -f crates/temper-cli/tests/hnsw_test.rs
rm -f crates/temper-cli/tests/chunker_test.rs
rm -f crates/temper-cli/tests/research_test.rs
```

- [ ] **Step 3: Remove dependencies from Cargo.toml**

Remove these lines from `crates/temper-cli/Cargo.toml`:
- `candle-core`
- `candle-nn`
- `candle-transformers`
- `tokenizers`
- `hf-hub`
- `instant-distance`
- `bincode` (if only used for index serialization)

- [ ] **Step 4: Remove module declarations from lib.rs**

Remove `pub mod embedder;`, `pub mod hnsw;`, `pub mod chunker;`, `pub mod registry;` from `crates/temper-cli/src/lib.rs`.

- [ ] **Step 5: Remove module declarations from commands/mod.rs and actions/mod.rs**

Remove `pub mod index;`, `pub mod search;`, `pub mod context;` from both `crates/temper-cli/src/commands/mod.rs` and `crates/temper-cli/src/actions/mod.rs`.

- [ ] **Step 6: Remove Index, Search, Context commands from cli.rs**

In `crates/temper-cli/src/cli.rs`, remove the `Index`, `Search`, and `Context` variants from the `Commands` enum and their associated argument structs.

- [ ] **Step 7: Remove dispatch arms from main.rs**

In `crates/temper-cli/src/main.rs`, remove the `Commands::Index { .. }`, `Commands::Search { .. }`, and `Commands::Context { .. }` match arms.

- [ ] **Step 8: Fix any remaining references**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
cargo check --all-features 2>&1 | head -50
```

Fix any compilation errors from dangling imports or references to removed modules. Common places: `commands/warmup.rs` (may reference search/index), `actions/index.rs` references in other actions.

- [ ] **Step 9: Run tests**

```bash
cargo test -p temper-cli 2>&1 | tail -20
```

Expected: All remaining tests pass. Some tests may fail if they import removed modules — delete those test files or fix imports.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "remove: local HNSW embedding and indexing stack

Removed candle, instant-distance, embedder, chunker, registry,
and index/search/context commands. Cloud search (I5d) replaces
local search. Drops ~6 heavy dependencies."
```

---

### Task 5: Remove TUI Code

The TUI is being retired (SvelteKit web UI replaces it). Remove it now as part of the clean break.

**Files:**
- Delete: `crates/temper-cli/src/tui/` (entire directory)
- Modify: `crates/temper-cli/src/lib.rs` (remove `pub mod tui;`)
- Modify: `crates/temper-cli/src/cli.rs` (remove `Tui` command variant)
- Modify: `crates/temper-cli/src/main.rs` (remove `Commands::Tui` dispatch)
- Modify: `crates/temper-cli/Cargo.toml` (remove `ratatui`, `crossterm`, and related TUI deps)

- [ ] **Step 1: Delete TUI directory**

```bash
rm -rf crates/temper-cli/src/tui/
```

- [ ] **Step 2: Remove TUI module declaration and command**

Remove `pub mod tui;` from `lib.rs`. Remove `Tui` variant from `Commands` enum in `cli.rs`. Remove `Commands::Tui` match arm from `main.rs`.

- [ ] **Step 3: Remove TUI dependencies from Cargo.toml**

Remove `ratatui`, `crossterm`, and any other TUI-only dependencies.

- [ ] **Step 4: Fix compilation and test**

```bash
cargo check -p temper-cli && cargo test -p temper-cli 2>&1 | tail -20
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "remove: TUI code and ratatui dependencies

SvelteKit web UI replaces the terminal UI. Removes ratatui,
crossterm, and all TUI rendering code."
```

---

### Task 6: Rename Types — TicketInfo → TaskInfo, MilestoneInfo → GoalInfo

Rename the core data types and their fields.

**Files:**
- Modify: `crates/temper-cli/src/actions/types.rs`
- Modify: All files importing `TicketInfo` or `MilestoneInfo`

- [ ] **Step 1: Rename TicketInfo to TaskInfo in types.rs**

In `crates/temper-cli/src/actions/types.rs`:

```rust
// Before
pub struct TicketInfo {
    // ...
    pub project: String,
    pub milestone: String,
    pub scope: Option<String>,
    // ...
}

// After
pub struct TaskInfo {
    // ...
    pub context: String,
    pub goal: String,
    pub mode: Option<String>,
    pub effort: Option<String>,
    // ...
}
```

- [ ] **Step 2: Rename MilestoneInfo to GoalInfo in types.rs**

```rust
// Before
pub struct MilestoneInfo {
    // ...
    pub project: String,
    // ...
}

// After
pub struct GoalInfo {
    // ...
    pub context: String,
    // ...
}
```

- [ ] **Step 3: Update NormalizeSummary**

Rename `unscoped_tickets` to an appropriate field name (e.g., `tasks_without_effort`).

- [ ] **Step 4: Find and replace all references**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
grep -rn 'TicketInfo\|MilestoneInfo\|unscoped_tickets' crates/temper-cli/src/
```

Update every reference. Key files:
- `actions/ticket.rs` → all `TicketInfo` references
- `actions/milestone.rs` → all `MilestoneInfo` references
- `commands/ticket.rs` → imports and usage
- `commands/milestone.rs` → imports and usage
- `commands/status.rs` → summary display
- `commands/warmup.rs` → context loading
- `commands/normalize.rs` → normalize summary
- `commands/session.rs` → ticket/milestone references
- `actions/normalize.rs` → directory references

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p temper-cli 2>&1 | head -50
```

Fix any remaining type errors.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "rename: TicketInfo→TaskInfo, MilestoneInfo→GoalInfo, field renames

Fields: project→context, milestone→goal, scope→mode+effort"
```

---

### Task 7: Rename CLI Commands — ticket → task, milestone → goal, project → context

Rename the command enums, action enums, file names, and module declarations.

**Files:**
- Rename: `crates/temper-cli/src/commands/ticket.rs` → `crates/temper-cli/src/commands/task.rs`
- Rename: `crates/temper-cli/src/commands/milestone.rs` → `crates/temper-cli/src/commands/goal.rs`
- Rename: `crates/temper-cli/src/actions/ticket.rs` → `crates/temper-cli/src/actions/task.rs`
- Rename: `crates/temper-cli/src/actions/milestone.rs` → `crates/temper-cli/src/actions/goal.rs`
- Modify: `crates/temper-cli/src/cli.rs` (command enum renames)
- Modify: `crates/temper-cli/src/main.rs` (dispatch renames)
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`

- [ ] **Step 1: Rename source files**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
mv crates/temper-cli/src/commands/ticket.rs crates/temper-cli/src/commands/task.rs
mv crates/temper-cli/src/commands/milestone.rs crates/temper-cli/src/commands/goal.rs
mv crates/temper-cli/src/actions/ticket.rs crates/temper-cli/src/actions/task.rs
mv crates/temper-cli/src/actions/milestone.rs crates/temper-cli/src/actions/goal.rs
```

- [ ] **Step 2: Rename test files**

```bash
mv crates/temper-cli/tests/ticket_test.rs crates/temper-cli/tests/task_test.rs 2>/dev/null
mv crates/temper-cli/tests/actions_ticket_test.rs crates/temper-cli/tests/actions_task_test.rs 2>/dev/null
mv crates/temper-cli/tests/actions_milestone_test.rs crates/temper-cli/tests/actions_goal_test.rs 2>/dev/null
mv crates/temper-cli/tests/session_ticket_test.rs crates/temper-cli/tests/session_task_test.rs 2>/dev/null
```

- [ ] **Step 3: Update module declarations in mod.rs files**

In `crates/temper-cli/src/commands/mod.rs`:
- `pub mod ticket;` → `pub mod task;`
- `pub mod milestone;` → `pub mod goal;`

In `crates/temper-cli/src/actions/mod.rs`:
- `pub mod ticket;` → `pub mod task;`
- `pub mod milestone;` → `pub mod goal;`

- [ ] **Step 4: Rename command enums in cli.rs**

In `crates/temper-cli/src/cli.rs`:
- `Commands::Ticket` → `Commands::Task`
- `TicketAction` → `TaskAction`
- `Commands::Milestone` → `Commands::Goal`
- `MilestoneAction` → `GoalAction`
- `Commands::Project` → `Commands::Context`
- `ProjectAction` → `ContextAction`
- All `--project` help text → `--context`
- All `--scope` args → split into `--mode` and `--effort`
- `--milestone` arg → `--goal`
- Update clap `#[command(name = "...")]` annotations

- [ ] **Step 5: Update main.rs dispatch**

Replace all `Commands::Ticket` → `Commands::Task`, `Commands::Milestone` → `Commands::Goal`, `Commands::Project` → `Commands::Context` match arms. Update function calls to use new module paths (`commands::task::`, `commands::goal::`).

- [ ] **Step 6: Update internal references in renamed files**

In `commands/task.rs` and `actions/task.rs`:
- All `ticket` variable names → `task`
- All `milestone` parameter names → `goal`
- All `project` parameter names → `context`
- All `scope` references → `mode`/`effort`
- Import paths: `actions::ticket::` → `actions::task::`
- Function names: `load_tickets` → `load_tasks`, `find_ticket` → `find_task`, `move_ticket` → `move_task`

In `commands/goal.rs` and `actions/goal.rs`:
- All `milestone` variable names → `goal`
- All `project` parameter names → `context`
- Function names: `load_milestones` → `load_goals`, `find_milestone` → `find_goal`
- `ensure_maintenance` → `ensure_maintenance` (name still makes sense)
- `count_tickets_by_stage` → `count_tasks_by_stage`

- [ ] **Step 7: Update project command to context**

Rename `commands/project.rs` internal references. The file name may stay as `project.rs` or rename to `context.rs` — rename it:

```bash
mv crates/temper-cli/src/commands/project.rs crates/temper-cli/src/commands/context.rs
```

Update `commands/mod.rs`: `pub mod project;` → `pub mod context;`

- [ ] **Step 8: Update remaining cross-references**

Files that reference tickets/milestones/projects in other commands:
- `commands/session.rs` — references to `--ticket` flag → `--task`, milestone references
- `commands/status.rs` — ticket/milestone counts and display
- `commands/check.rs` — `tickets_dir` → `tasks_dir`, `milestones_dir` → `goals_dir`
- `commands/warmup.rs` — ticket/milestone loading
- `commands/normalize.rs` — ticket/milestone directory references
- `actions/normalize.rs` — directory references

- [ ] **Step 9: Update test file contents**

In all renamed test files, update:
- Import paths (`use temper_cli::actions::ticket` → `use temper_cli::actions::task`)
- Type names (`TicketInfo` → `TaskInfo`)
- Variable names and assertions
- Test function names (e.g., `test_create_ticket` → `test_create_task`)

- [ ] **Step 10: Verify compilation and tests**

```bash
cargo check -p temper-cli && cargo test -p temper-cli 2>&1 | tail -30
```

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "rename: CLI commands ticket→task, milestone→goal, project→context

File renames, module declarations, command enums, dispatch arms,
function names, variable names, and test files all updated."
```

---

### Task 8: Rename Config Structs and Paths

Update VaultConfig, Config, and related path resolution.

**Files:**
- Modify: `crates/temper-cli/src/config.rs`

- [ ] **Step 1: Rename VaultConfig fields**

```rust
// Before
pub struct VaultConfig {
    pub tickets: String,
    pub milestones: String,
    // ...
}

// After
pub struct VaultConfig {
    pub tasks: String,
    pub goals: String,
    // ...
}
```

Update default functions: `default_tickets()` → `default_tasks()`, `default_milestones()` → `default_goals()`.

- [ ] **Step 2: Rename Config fields**

```rust
// Before
pub tickets_dir: PathBuf,
pub milestones_dir: PathBuf,

// After
pub tasks_dir: PathBuf,
pub goals_dir: PathBuf,
```

- [ ] **Step 3: Update all references to config fields**

```bash
grep -rn 'tickets_dir\|milestones_dir\|\.tickets\|\.milestones' crates/temper-cli/src/
```

Update every reference to use the new field names.

- [ ] **Step 4: Verify compilation and tests**

```bash
cargo check -p temper-cli && cargo test -p temper-cli 2>&1 | tail -20
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "rename: config fields tickets→tasks, milestones→goals"
```

---

### Task 9: Update Templates

Rename the ticket template to task template, update frontmatter fields.

**Files:**
- Rename: `crates/temper-cli/src/templates/ticket.md` → `crates/temper-cli/src/templates/task.md`
- Rename: `crates/temper-cli/src/templates/milestone.md` → `crates/temper-cli/src/templates/goal.md`
- Modify: Template contents (frontmatter fields)
- Modify: Any code that references template file names

- [ ] **Step 1: Rename template files**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
mv crates/temper-cli/src/templates/ticket.md crates/temper-cli/src/templates/task.md
mv crates/temper-cli/src/templates/milestone.md crates/temper-cli/src/templates/goal.md
```

- [ ] **Step 2: Update task template frontmatter**

In `crates/temper-cli/src/templates/task.md`:

```yaml
---
type: task
title: "{{title}}"
slug: "{{slug}}"
context: "{{context}}"
goal: "{{goal}}"
stage: backlog
seq: {{seq}}
mode: {{mode}}
effort: {{effort}}
created: {{created}}
updated: {{updated}}
branch: null
pr: null
---
```

- [ ] **Step 3: Update goal template frontmatter**

In `crates/temper-cli/src/templates/goal.md`:

```yaml
---
type: goal
title: "{{title}}"
slug: "{{slug}}"
context: "{{context}}"
seq: {{seq}}
status: active
created: {{created}}
---
```

- [ ] **Step 4: Update template references in code**

Search for `ticket.md` and `milestone.md` template path references in the crate and update them:

```bash
grep -rn 'ticket\.md\|milestone\.md' crates/temper-cli/src/
```

- [ ] **Step 5: Update template rendering calls**

The `create()` functions in `actions/task.rs` and `actions/goal.rs` pass template variables. Update:
- `"project"` → `"context"`
- `"milestone"` → `"goal"`
- `"scope"` → split into `"mode"` and `"effort"`

- [ ] **Step 6: Verify and commit**

```bash
cargo check -p temper-cli && cargo test -p temper-cli 2>&1 | tail -20
git add -A
git commit -m "rename: templates ticket.md→task.md, milestone.md→goal.md"
```

---

### Task 10: Update Discovery Events

Rename event variants for the event log.

**Files:**
- Modify: `crates/temper-cli/src/discovery.rs`
- Modify: Any files that emit or match on event variants

- [ ] **Step 1: Rename event variants**

In `crates/temper-cli/src/discovery.rs`:
- `Event::TicketCreate` → `Event::TaskCreate`
- `Event::TicketMove` → `Event::TaskMove`
- `Event::MilestoneCreate` → `Event::GoalCreate`
- Any other ticket/milestone event variants

- [ ] **Step 2: Update event emission sites**

```bash
grep -rn 'TicketCreate\|TicketMove\|MilestoneCreate' crates/temper-cli/src/
```

Update all sites that emit these events.

- [ ] **Step 3: Update event display/formatting**

If events have display strings like "created ticket" or "moved ticket", update to "created task" / "moved task".

- [ ] **Step 4: Verify and commit**

```bash
cargo check -p temper-cli && cargo test -p temper-cli 2>&1 | tail -20
git add -A
git commit -m "rename: discovery events TicketCreate→TaskCreate, MilestoneCreate→GoalCreate"
```

---

### Task 11: Update Skill Generator

The skill generator in `commands/skill.rs` outputs the `/temper` skill content. This needs comprehensive updates.

**Files:**
- Modify: `crates/temper-cli/src/commands/skill.rs`

- [ ] **Step 1: Read the current skill.rs**

Read `crates/temper-cli/src/commands/skill.rs` to understand the full template.

- [ ] **Step 2: Update command reference section**

Replace all command documentation:
- `temper ticket create --title <t> [--project <p>] [--scope ...]` → `temper task create --title <t> [--context <c>] [--mode plan|build] [--effort small|medium|large]`
- `temper ticket list` → `temper task list`
- `temper ticket move <slug> --stage <s> [--project <p>]` → `temper task move <slug> --stage <s> [--context <c>]`
- `temper ticket done <slug>` → `temper task done <slug>`
- `temper ticket show <slug>` → `temper task show <slug>`
- `temper ticket start <slug>` → `temper task start <slug>`
- `temper milestone list` → `temper goal list`
- All `--project` flags → `--context`

- [ ] **Step 3: Update Scope section to Mode + Effort**

Replace the "Scope" section with:

```markdown
## Mode

Tasks have a `mode` field: `plan` or `build`.
- `plan` — outcome is a plan (research, design, roadmap)
- `build` — outcome is an artifact (code, document, design)

## Effort

Tasks have an `effort` field: `small`, `medium`, or `large`.

| Mode | Effort | Workflow |
|------|--------|----------|
| build | small | Implement directly with tests |
| build | medium | Brainstorm → plan → implement |
| build | large | Brainstorm → plan → implement (multi-session) |
| plan | small | Quick research, write up findings |
| plan | medium | Brainstorm → design spec |
| plan | large | Deep discovery → goal roadmap → first actionable task |
```

- [ ] **Step 4: Update Workflow Integration section**

Replace all references to scope routing (patch/feature/epic workflows) with mode+effort routing. Update the `temper task start` documentation to use the new routing table.

Replace all instances of:
- "ticket" → "task"
- "milestone" → "goal"
- "project" → "context"
- "patch" / "feature" / "epic" → appropriate mode+effort combinations
- `--project` → `--context`
- `--scope` → `--mode` / `--effort`

- [ ] **Step 5: Verify skill generation**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
cargo run -- skill generate 2>&1 | head -50
```

Review the generated output for any remaining old terminology.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "rename: skill generator updated for task/goal/context/mode+effort"
```

---

### Task 12: Scope Validation Updates

The old `valid_scopes` array in the create action needs to become mode+effort validation.

**Files:**
- Modify: `crates/temper-cli/src/actions/task.rs` (formerly ticket.rs)

- [ ] **Step 1: Replace scope validation**

In `actions/task.rs`, find the scope validation (was around line 143):

```rust
// Before
let valid_scopes = ["patch", "feature", "epic"];

// After
let valid_modes = ["plan", "build"];
let valid_efforts = ["small", "medium", "large"];
```

Update the validation logic to check both `mode` and `effort` independently.

- [ ] **Step 2: Update move command scope handling**

The `move_task` function may accept `--scope` to change scope. Update to accept `--mode` and `--effort` separately, updating the relevant frontmatter fields.

- [ ] **Step 3: Verify and commit**

```bash
cargo check -p temper-cli && cargo test -p temper-cli 2>&1 | tail -20
git add -A
git commit -m "rename: scope validation replaced with mode+effort validation"
```

---

### Task 13: Update Claude Memory and CLAUDE.md Files

Update project memories and documentation that reference old terminology.

**Files:**
- Modify: `~/.claude/projects/-Users-petetaylor-projects-tasker-systems-temper/memory/MEMORY.md`
- Modify: `~/.claude/projects/-Users-petetaylor-projects-tasker-systems-temper/memory/project_nomenclature_tasks_goals.md`
- Modify: `~/.claude/projects/-Users-petetaylor-projects-tasker-systems-temper/memory/project_tui_removal.md`
- Modify: Any other memory files with old terminology
- Modify: `tools/cargo-make/CLAUDE.md` (if it references old commands)

- [ ] **Step 1: Update MEMORY.md index**

Review and update any entries referencing tickets, milestones, or old scope terms.

- [ ] **Step 2: Update individual memory files**

Update `project_nomenclature_tasks_goals.md` to reflect that the rename is DONE (not planned).

Update `project_tui_removal.md` to reflect that TUI removal is DONE.

Remove or update any memory files that are now stale.

- [ ] **Step 3: Commit memory changes**

Memory files are outside the temper repo, so commit separately:

```bash
# Memory files are in the Claude projects dir, not version controlled
# Just update them in place
```

---

### Task 14: Final Verification

Run the full check suite and verify everything compiles and passes.

**Files:** None (verification only)

- [ ] **Step 1: Full cargo check**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
cargo check --all-features
```

- [ ] **Step 2: Full test suite**

```bash
cargo test --all-features 2>&1 | tail -30
```

- [ ] **Step 3: Clippy**

```bash
cargo clippy --all-features -- -D warnings 2>&1 | tail -20
```

- [ ] **Step 4: cargo make check**

```bash
cargo make check
```

- [ ] **Step 5: Rebuild and install temper**

```bash
cargo install --path crates/temper-cli
```

- [ ] **Step 6: Verify renamed commands work**

```bash
temper task list --context temper
temper goal list --context temper
temper context list
temper status
```

- [ ] **Step 7: Regenerate and install skill**

```bash
temper skill generate
temper skill install
```

- [ ] **Step 8: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: final cleanup from nomenclature rename"
```
